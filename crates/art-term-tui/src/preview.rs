use crate::{
    backend::{Backend, ScreenSelection},
    tab::TabId,
};
use art_term_core::{image as image_limits, openab::screenshot::ShotImage};
use base64::{engine::general_purpose::STANDARD, Engine};
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use std::{
    future::Future,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle, time::Instant};

const CAPTURE_INTERVAL: Duration = Duration::from_secs(1);
const MAX_PREVIEW_COLUMNS: u16 = 160;
const MAX_PREVIEW_ROWS: u16 = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewTarget {
    pub tab: TabId,
    pub generation: u64,
    pub screen: ScreenSelection,
    pub area: Rect,
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

pub(crate) struct PreviewUpdate {
    pub target: PreviewTarget,
    pub status: PreviewStatus,
    pub image: Option<Protocol>,
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
        if target.area.is_empty() {
            tokio::select! {
                _ = stopping.changed() => (),
                changed = requests.changed() => if changed.is_err() { break; },
            }
            continue;
        }
        tokio::select! {
            _ = stopping.changed() => continue,
            changed = requests.changed() => { if changed.is_err() { break; } continue; },
            _ = tokio::time::sleep_until(next_capture) => (),
        }
        if !matches_target(&requests, &target) || *stopping.borrow() {
            continue;
        }
        let admissible = valid_area(target.area);
        if let Err(error) = admissible {
            publish(
                &latest,
                PreviewUpdate {
                    target,
                    status: PreviewStatus::Failed(error),
                    image: None,
                },
            );
            next_capture = Instant::now() + CAPTURE_INTERVAL;
            continue;
        }
        if !source.current(&target.screen) {
            publish(
                &latest,
                PreviewUpdate {
                    target,
                    status: PreviewStatus::Failed(PreviewError::Unavailable),
                    image: None,
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
                    image: None,
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
                let area = target.area;
                // Blocking preparation is joined, including when the selection changes.
                tokio::task::spawn_blocking(move || prepare(shot, area))
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
        let (status, image) = match result {
            Ok(image) => (PreviewStatus::Live, Some(image)),
            Err(error) => (PreviewStatus::Failed(error), None),
        };
        publish(
            &latest,
            PreviewUpdate {
                target,
                status,
                image,
            },
        );
    }
}

fn valid_area(area: Rect) -> Result<(), PreviewError> {
    if area.is_empty() || area.width > MAX_PREVIEW_COLUMNS || area.height > MAX_PREVIEW_ROWS {
        Err(PreviewError::AreaTooLarge)
    } else {
        Ok(())
    }
}

fn prepare(shot: ShotImage, area: Rect) -> Result<Protocol, PreviewError> {
    valid_area(area)?;
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
    Picker::halfblocks()
        .new_protocol(image, area.as_size(), Resize::Fit(None))
        .map_err(|_| PreviewError::InvalidImage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, style::Color, widgets::Widget};
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
            area: Rect::new(2, 2, 8, 4),
        }
    }

    #[test]
    fn real_image_preserves_colors_aspect_and_render_bounds() {
        let area = Rect::new(2, 2, 8, 4);
        let protocol = prepare(shot(), area).unwrap();
        // A square image occupies twice as many cell columns as cell rows.
        assert_eq!(protocol.size(), ratatui::layout::Size::new(4, 2));
        let bounds = Rect::new(0, 0, 12, 8);
        let mut buffer = Buffer::empty(bounds);
        ratatui_image::Image::new(&protocol).render(area, &mut buffer);
        let mut colors = Vec::new();
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let cell = &buffer[(x, y)];
                if !area.contains((x, y).into()) {
                    assert_eq!(cell.symbol(), " ");
                    assert_eq!(cell.fg, Color::Reset);
                } else {
                    colors.push(cell.fg);
                    colors.push(cell.bg);
                }
            }
        }
        assert!(colors.contains(&Color::Rgb(255, 0, 0)));
        assert!(colors.contains(&Color::Rgb(0, 0, 255)));
        assert!(
            colors.iter().any(
                |color| matches!(color, Color::Rgb(r, g, b) if *g > 180 && *r < 80 && *b < 80)
            ),
            "{colors:?}"
        );
    }

    #[test]
    fn preparation_rejects_untrusted_images_and_oversized_areas() {
        let area = Rect::new(0, 0, 8, 4);
        let mut malformed = shot();
        malformed.image_base64 = STANDARD.encode(b"not PNG");
        assert!(matches!(
            prepare(malformed, area),
            Err(PreviewError::InvalidImage)
        ));
        let mut wrong_dimensions = shot();
        wrong_dimensions.width += 1;
        assert!(matches!(
            prepare(wrong_dimensions, area),
            Err(PreviewError::InvalidImage)
        ));
        let mut unsupported = shot();
        unsupported.mime = "image/gif".into();
        assert!(matches!(
            prepare(unsupported, area),
            Err(PreviewError::InvalidImage)
        ));
        assert!(matches!(
            prepare(shot(), Rect::new(0, 0, 161, 80)),
            Err(PreviewError::AreaTooLarge)
        ));
        assert!(matches!(
            prepare(shot(), Rect::new(0, 0, 160, 81)),
            Err(PreviewError::AreaTooLarge)
        ));
        assert!(matches!(
            prepare(shot(), Rect::default()),
            Err(PreviewError::AreaTooLarge)
        ));
        let maximum = prepare(shot(), Rect::new(0, 0, 160, 80)).unwrap();
        assert!(maximum.size().width <= 160 && maximum.size().height <= 80);
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
        assert!(prepare(jpeg, Rect::new(0, 0, 8, 4)).is_ok());
        let mut excessive = shot();
        excessive.width = image_limits::MAX_DIMENSION + 1;
        assert!(matches!(
            prepare(excessive, Rect::new(0, 0, 8, 4)),
            Err(PreviewError::InvalidImage)
        ));
    }

    #[test]
    fn publication_identity_includes_tab_generation_screen_revision_and_area() {
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
        let mut changed = original.clone();
        changed.area.width += 1;
        alternatives.push(changed);
        let mut changed = original.clone();
        changed.area.x += 1;
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
            self.state.started.fetch_add(1, Ordering::SeqCst);
            self.state.permits.acquire().await.unwrap().forget();
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
        until(|| state.started.load(Ordering::SeqCst) == 2).await;
        state.permits.add_permits(1);
        let update = live(&mut worker).await;
        assert_eq!(update.target, target("second", 2));
        assert_eq!(update.status, PreviewStatus::Live);
        assert!(update.image.is_some());
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
        assert!(update.image.is_none());
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

    #[tokio::test]
    async fn hidden_and_huge_previews_do_not_capture() {
        let (source, state) = MockSource::new();
        let mut worker = PreviewWorker::start_source(source);
        let mut hidden = target("screen", 1);
        hidden.area = Rect::default();
        worker.set_target(Some(hidden));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(state.started.load(Ordering::SeqCst), 0);
        let mut huge = target("screen", 2);
        huge.area = Rect::new(0, 0, 200, 100);
        worker.set_target(Some(huge));
        assert_eq!(
            live(&mut worker).await.status,
            PreviewStatus::Failed(PreviewError::AreaTooLarge)
        );
        assert_eq!(state.started.load(Ordering::SeqCst), 0);
        worker.stop().await;
    }
}
