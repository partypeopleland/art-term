use crate::{
    backend::{Backend, ScreenSelection},
    tab::TabId,
};
use art_term_core::{image as image_limits, openab::screenshot::ShotImage};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    future::Future,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle, time::Instant};

const CAPTURE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewTarget {
    pub tab: TabId,
    pub generation: u64,
    pub screen: ScreenSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewError {
    Unavailable,
    InvalidImage,
    AreaTooLarge,
}
impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "桌面暫時不可用；請檢查 Screen 設定與連線。",
            Self::InvalidImage => "桌面圖片無法顯示。",
            Self::AreaTooLarge => "預覽區超出可用尺寸。",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewStatus {
    Capturing,
    Live,
    Failed(PreviewError),
}

pub(crate) struct CapturedFrame {
    pub target: PreviewTarget,
    pub sequence: u64,
    pub pixels: image::DynamicImage,
}

pub(crate) struct PreviewUpdate {
    pub target: PreviewTarget,
    pub status: PreviewStatus,
    pub frame: Option<Arc<CapturedFrame>>,
}

type Latest = Arc<Mutex<Option<PreviewUpdate>>>;

pub(crate) struct PreviewWorker {
    target: watch::Sender<Option<PreviewTarget>>,
    shutdown: watch::Sender<bool>,
    latest: Latest,
    task: Option<JoinHandle<()>>,
}

impl PreviewWorker {
    pub(crate) fn start(backend: Backend) -> Self {
        Self::start_source(backend)
    }

    fn start_source(source: impl CaptureSource) -> Self {
        let (target, requests) = watch::channel(None);
        let (shutdown, stopping) = watch::channel(false);
        let latest = Arc::new(Mutex::new(None));
        let task = tokio::spawn(run(source, requests, stopping, latest.clone()));
        Self {
            target,
            shutdown,
            latest,
            task: Some(task),
        }
    }

    pub(crate) fn set_target(&mut self, target: Option<PreviewTarget>) {
        if *self.target.borrow() != target {
            *self
                .latest
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            self.target.send_replace(target);
        }
    }

    pub(crate) fn take_latest(&mut self) -> Option<PreviewUpdate> {
        let update = self
            .latest
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()?;
        (Some(&update.target) == self.target.borrow().as_ref()).then_some(update)
    }

    pub(crate) async fn stop(mut self) {
        self.set_target(None);
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

trait CaptureSource: Send + Sync + 'static {
    fn capture(
        &self,
        selection: ScreenSelection,
    ) -> impl Future<Output = Result<ShotImage, PreviewError>> + Send;
    fn current(&self, selection: &ScreenSelection) -> bool;
}
impl CaptureSource for Backend {
    fn capture(
        &self,
        selection: ScreenSelection,
    ) -> impl Future<Output = Result<ShotImage, PreviewError>> + Send {
        let snapshot = self.screen_snapshot(&selection);
        async move {
            let snapshot = snapshot.map_err(|_| PreviewError::Unavailable)?;
            art_term_core::instance_mcp::screenshot(snapshot)
                .await
                .map_err(|_| PreviewError::Unavailable)
        }
    }

    fn current(&self, selection: &ScreenSelection) -> bool {
        self.screen_is_current(selection).unwrap_or(false)
    }
}

fn matches_target(
    requests: &watch::Receiver<Option<PreviewTarget>>,
    target: &PreviewTarget,
) -> bool {
    requests.borrow().as_ref() == Some(target)
}

fn publish(latest: &Latest, update: PreviewUpdate) {
    *latest.lock().unwrap_or_else(|error| error.into_inner()) = Some(update);
}

async fn run(
    source: impl CaptureSource,
    mut requests: watch::Receiver<Option<PreviewTarget>>,
    mut stopping: watch::Receiver<bool>,
    latest: Latest,
) {
    let mut next_capture = Instant::now();
    let mut announced_target = None;
    let mut sequence = 0u64;
    loop {
        if *stopping.borrow() || requests.has_changed().is_err() {
            break;
        }
        let desired = requests.borrow_and_update().clone();
        let Some(target) = desired else {
            announced_target = None;
            tokio::select! {
                _ = stopping.changed() => (),
                changed = requests.changed() => if changed.is_err() { break; },
            }
            continue;
        };
        tokio::select! {
            _ = stopping.changed() => continue,
            changed = requests.changed() => { if changed.is_err() { break; } continue; },
            _ = tokio::time::sleep_until(next_capture) => (),
        }
        if !matches_target(&requests, &target) || *stopping.borrow() {
            continue;
        }
        if !source.current(&target.screen) {
            publish(
                &latest,
                PreviewUpdate {
                    target,
                    status: PreviewStatus::Failed(PreviewError::Unavailable),
                    frame: None,
                },
            );
            next_capture = Instant::now() + CAPTURE_INTERVAL;
            continue;
        }
        if announced_target.as_ref() != Some(&target) {
            publish(
                &latest,
                PreviewUpdate {
                    target: target.clone(),
                    status: PreviewStatus::Capturing,
                    frame: None,
                },
            );
            announced_target = Some(target.clone());
        }

        // A target change does not drop the core future before its MCP session closes.
        let captured = source.capture(target.screen.clone()).await;
        let result = match captured {
            Ok(shot)
                if matches_target(&requests, &target)
                    && !*stopping.borrow()
                    && source.current(&target.screen) =>
            {
                // Blocking preparation is joined, including when the selection changes.
                tokio::task::spawn_blocking(move || decode(shot))
                    .await
                    .unwrap_or(Err(PreviewError::InvalidImage))
            }
            Ok(_) => Err(PreviewError::Unavailable),
            Err(error) => Err(error),
        };
        next_capture = Instant::now() + CAPTURE_INTERVAL;
        if *stopping.borrow() || !matches_target(&requests, &target) {
            continue;
        }
        let result = if source.current(&target.screen) {
            result
        } else {
            Err(PreviewError::Unavailable)
        };
        let (status, frame) = match result {
            Ok(pixels) => {
                sequence = sequence.saturating_add(1);
                (
                    PreviewStatus::Live,
                    Some(Arc::new(CapturedFrame {
                        target: target.clone(),
                        sequence,
                        pixels,
                    })),
                )
            }
            Err(error) => (PreviewStatus::Failed(error), None),
        };
        publish(
            &latest,
            PreviewUpdate {
                target,
                status,
                frame,
            },
        );
    }
}

fn decode(shot: ShotImage) -> Result<image::DynamicImage, PreviewError> {
    if shot.image_base64.len() > image_limits::MAX_BASE64_LEN
        || shot.width == 0
        || shot.height == 0
        || shot.width > image_limits::MAX_DIMENSION
        || shot.height > image_limits::MAX_DIMENSION
    {
        return Err(PreviewError::InvalidImage);
    }
    let format = match shot.mime.as_str() {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        _ => return Err(PreviewError::InvalidImage),
    };
    let bytes = STANDARD
        .decode(&shot.image_base64)
        .map_err(|_| PreviewError::InvalidImage)?;
    if bytes.is_empty() || bytes.len() > image_limits::MAX_IMAGE_BYTES {
        return Err(PreviewError::InvalidImage);
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(image_limits::MAX_DIMENSION);
    limits.max_image_height = Some(image_limits::MAX_DIMENSION);
    limits.max_alloc = Some(image_limits::MAX_ALLOC_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| PreviewError::InvalidImage)?;
    if image.width() != shot.width || image.height() != shot.height {
        return Err(PreviewError::InvalidImage);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    fn shot() -> ShotImage {
        let mut image = image::RgbImage::new(40, 40);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb(if y < 20 {
                [255, 0, 0]
            } else if x < 20 {
                [0, 0, 255]
            } else {
                [0, 255, 0]
            });
        }
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        ShotImage {
            mime: "image/png".into(),
            image_base64: STANDARD.encode(bytes.into_inner()),
            width: 40,
            height: 40,
        }
    }

    fn target(id: &str, generation: u64) -> PreviewTarget {
        PreviewTarget {
            tab: TabId(generation),
            generation,
            screen: ScreenSelection {
                id: id.into(),
                revision: 1,
            },
        }
    }

    #[test]
    fn decode_preserves_source_and_rejects_invalid_images() {
        let decoded = decode(shot()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 40));
        let mut invalid = shot();
        invalid.width += 1;
        assert!(matches!(decode(invalid), Err(PreviewError::InvalidImage)));
        let mut invalid = shot();
        invalid.image_base64 = STANDARD.encode(b"not PNG");
        assert!(matches!(decode(invalid), Err(PreviewError::InvalidImage)));
    }

    #[test]
    fn jpeg_is_supported_and_declared_size_is_bounded() {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbImage::new(40, 40)
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .unwrap();
        let mut jpeg = shot();
        jpeg.mime = "image/jpeg".into();
        jpeg.image_base64 = STANDARD.encode(bytes.into_inner());
        assert!(decode(jpeg).is_ok());
        let mut excessive = shot();
        excessive.width = image_limits::MAX_DIMENSION + 1;
        assert!(matches!(decode(excessive), Err(PreviewError::InvalidImage)));
    }

    #[test]
    fn publication_identity_includes_tab_generation_and_screen_revision() {
        let original = target("screen", 1);
        let (sender, receiver) = watch::channel(Some(original.clone()));
        assert!(matches_target(&receiver, &original));
        let mut alternatives = Vec::new();
        let mut changed = original.clone();
        changed.tab = TabId(2);
        alternatives.push(changed);
        let mut changed = original.clone();
        changed.generation += 1;
        alternatives.push(changed);
        let mut changed = original.clone();
        changed.screen.id = "other".into();
        alternatives.push(changed);
        let mut changed = original.clone();
        changed.screen.revision += 1;
        alternatives.push(changed);
        for changed in alternatives {
            sender.send_replace(Some(changed));
            assert!(!matches_target(&receiver, &original));
        }
        sender.send_replace(None);
        assert!(!matches_target(&receiver, &original));
    }

    struct MockSource {
        state: Arc<MockState>,
    }
    struct MockState {
        permits: Semaphore,
        started: AtomicUsize,
        completed: AtomicUsize,
        started_at: Mutex<Vec<Instant>>,
        completed_at: Mutex<Vec<Instant>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        revision: AtomicU64,
    }
    impl MockSource {
        fn new() -> (Self, Arc<MockState>) {
            let state = Arc::new(MockState {
                permits: Semaphore::new(0),
                started: AtomicUsize::new(0),
                completed: AtomicUsize::new(0),
                started_at: Mutex::new(Vec::new()),
                completed_at: Mutex::new(Vec::new()),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                revision: AtomicU64::new(1),
            });
            (
                Self {
                    state: state.clone(),
                },
                state,
            )
        }
    }
    impl CaptureSource for MockSource {
        async fn capture(&self, _: ScreenSelection) -> Result<ShotImage, PreviewError> {
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.max_active.fetch_max(active, Ordering::SeqCst);
            self.state.started_at.lock().unwrap().push(Instant::now());
            self.state.started.fetch_add(1, Ordering::SeqCst);
            self.state.permits.acquire().await.unwrap().forget();
            self.state.completed_at.lock().unwrap().push(Instant::now());
            self.state.completed.fetch_add(1, Ordering::SeqCst);
            self.state.active.fetch_sub(1, Ordering::SeqCst);
            Ok(shot())
        }
        fn current(&self, screen: &ScreenSelection) -> bool {
            screen.revision == self.state.revision.load(Ordering::SeqCst)
        }
    }
    async fn until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("worker progress");
    }
    async fn live(worker: &mut PreviewWorker) -> PreviewUpdate {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(update) = worker.take_latest() {
                    if update.status != PreviewStatus::Capturing {
                        break update;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("preview update")
    }

    #[tokio::test]
    async fn automatic_capture_waits_thirty_seconds_after_completion() {
        let (source, state) = MockSource::new();
        let mut worker = PreviewWorker::start_source(source);
        worker.set_target(Some(target("screen", 1)));
        until(|| state.started.load(Ordering::SeqCst) == 1).await;
        state.permits.add_permits(1);
        assert_eq!(live(&mut worker).await.status, PreviewStatus::Live);
        let completed = Instant::now();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            state.started.load(Ordering::SeqCst),
            1,
            "no one-second polling"
        );
        tokio::time::sleep_until(completed + Duration::from_secs(29)).await;
        assert_eq!(
            state.started.load(Ordering::SeqCst),
            1,
            "no capture before thirty seconds"
        );
        until(|| state.started.load(Ordering::SeqCst) == 2).await;
        let interval = state.started_at.lock().unwrap()[1] - state.completed_at.lock().unwrap()[0];
        assert!(
            interval >= Duration::from_secs(30),
            "capture interval: {interval:?}"
        );
        state.permits.add_permits(1);
        assert_eq!(live(&mut worker).await.status, PreviewStatus::Live);
        worker.stop().await;
        assert_eq!(state.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn switching_discards_inflight_result_and_keeps_capture_single_flight() {
        let (source, state) = MockSource::new();
        let mut worker = PreviewWorker::start_source(source);
        let first = target("first", 1);
        worker.set_target(Some(first));
        until(|| state.started.load(Ordering::SeqCst) == 1).await;
        worker.set_target(Some(target("second", 2)));
        assert!(worker.take_latest().is_none());
        state.permits.add_permits(1);
        until(|| state.completed.load(Ordering::SeqCst) == 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            state.started.load(Ordering::SeqCst),
            1,
            "cadence starts after completion"
        );
        assert!(worker.take_latest().is_none());
        tokio::time::sleep(Duration::from_secs(30)).await;
        until(|| state.started.load(Ordering::SeqCst) == 2).await;
        state.permits.add_permits(1);
        let update = live(&mut worker).await;
        assert_eq!(update.target, target("second", 2));
        assert_eq!(update.status, PreviewStatus::Live);
        assert!(update.frame.is_some());
        worker.stop().await;
        assert_eq!(state.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn changed_revision_clears_image_and_never_publishes_obsolete_capture() {
        let (source, state) = MockSource::new();
        let mut worker = PreviewWorker::start_source(source);
        worker.set_target(Some(target("screen", 1)));
        until(|| state.started.load(Ordering::SeqCst) == 1).await;
        state.revision.store(2, Ordering::SeqCst);
        state.permits.add_permits(1);
        let update = live(&mut worker).await;
        assert_eq!(
            update.status,
            PreviewStatus::Failed(PreviewError::Unavailable)
        );
        assert!(update.frame.is_none());
        worker.stop().await;
    }

    #[tokio::test]
    async fn shutdown_clears_immediately_and_waits_for_capture_cleanup() {
        let (source, state) = MockSource::new();
        let mut worker = PreviewWorker::start_source(source);
        worker.set_target(Some(target("screen", 1)));
        until(|| state.started.load(Ordering::SeqCst) == 1).await;
        worker.set_target(None);
        assert!(worker.take_latest().is_none());
        let stopping = tokio::spawn(worker.stop());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!stopping.is_finished());
        assert_eq!(state.completed.load(Ordering::SeqCst), 0);
        state.permits.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), stopping)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.completed.load(Ordering::SeqCst), 1);
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
        assert_eq!(state.started.load(Ordering::SeqCst), 1);
    }
}
