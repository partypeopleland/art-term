use crate::preview::{CapturedFrame, PreviewError, PreviewTarget};
use image::{imageops::FilterType, DynamicImage, GenericImage, RgbaImage};
use ratatui::layout::Rect;
use ratatui_image::protocol::{halfblocks::Halfblocks, sixel::Sixel, Protocol};
use std::sync::{Arc, Mutex};
use tokio::{sync::watch, task::JoinHandle};

const MAX_COLUMNS: u16 = 160;
const MAX_ROWS: u16 = 80;
const MAX_PIXELS: u64 = 4_000_000;
const CENTER_UNITS: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageMode {
    Compatible,
    Sixel { cell_width: u16, cell_height: u16 },
}
impl ImageMode {
    fn metrics(self) -> (u32, u32) {
        match self {
            Self::Compatible => (1, 2),
            Self::Sixel {
                cell_width,
                cell_height,
            } => (cell_width.into(), cell_height.into()),
        }
    }
    fn canvas(self, area: Rect) -> (u32, u32) {
        let (cw, ch) = self.metrics();
        let width = area.width as u32 * cw;
        let height = area.height as u32 * ch;
        // SIXEL emits six-pixel bands; round down so its last band stays inside the pane.
        let height = if matches!(self, Self::Sixel { .. }) {
            height / 6 * 6
        } else {
            height
        };
        (width, height)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImageViewport {
    zoom: u8,
    center_x: i64,
    center_y: i64,
}
impl ImageViewport {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn zoom_at(
        &mut self,
        steps: i16,
        point: (u16, u16),
        area: Rect,
        image: (u32, u32),
        mode: ImageMode,
    ) -> bool {
        if area.is_empty() || image.0 == 0 || image.1 == 0 || mode.canvas(area).1 == 0 {
            return false;
        }
        let before = self.clone();
        let (cw, ch) = mode.metrics();
        if cw == 0 || ch == 0 {
            return false;
        }
        let old = Geometry::new(self, area, image, mode);
        let px = (point.0.saturating_sub(area.x).min(area.width - 1) as f64 + 0.5) * cw as f64;
        let py = (point.1.saturating_sub(area.y).min(area.height - 1) as f64 + 0.5) * ch as f64;
        let anchor_x = old.center.0 + (px - old.canvas.0 / 2.0) / old.scale;
        let anchor_y = old.center.1 + (py - old.canvas.1 / 2.0) / old.scale;
        self.zoom = (self.zoom as i32 + steps as i32).clamp(0, 16) as u8;
        let new = Geometry::new(self, area, image, mode);
        self.set_center(
            anchor_x - (px - new.canvas.0 / 2.0) / new.scale,
            anchor_y - (py - new.canvas.1 / 2.0) / new.scale,
            &new,
            image,
        );
        *self != before
    }

    pub fn pan_cells(
        &mut self,
        dx: i16,
        dy: i16,
        area: Rect,
        image: (u32, u32),
        mode: ImageMode,
    ) -> bool {
        if area.is_empty() || image.0 == 0 || image.1 == 0 || mode.canvas(area).1 == 0 {
            return false;
        }
        let before = self.clone();
        let (cw, ch) = mode.metrics();
        if cw == 0 || ch == 0 {
            return false;
        }
        let g = Geometry::new(self, area, image, mode);
        self.set_center(
            g.center.0 - dx as f64 * cw as f64 / g.scale,
            g.center.1 - dy as f64 * ch as f64 / g.scale,
            &g,
            image,
        );
        *self != before
    }

    fn set_center(&mut self, x: f64, y: f64, g: &Geometry, image: (u32, u32)) {
        let half_x = (g.canvas.0 / g.scale / 2.0).min(image.0 as f64 / 2.0);
        let half_y = (g.canvas.1 / g.scale / 2.0).min(image.1 as f64 / 2.0);
        let x = x.clamp(half_x, image.0 as f64 - half_x);
        let y = y.clamp(half_y, image.1 as f64 - half_y);
        self.center_x = ((x / image.0 as f64 - 0.5) * CENTER_UNITS).round() as i64;
        self.center_y = ((y / image.1 as f64 - 0.5) * CENTER_UNITS).round() as i64;
    }
}

struct Geometry {
    canvas: (f64, f64),
    scale: f64,
    center: (f64, f64),
}
impl Geometry {
    fn new(view: &ImageViewport, area: Rect, image: (u32, u32), mode: ImageMode) -> Self {
        let raster = mode.canvas(area);
        let canvas = (raster.0 as f64, raster.1 as f64);
        let scale = (canvas.0 / image.0 as f64).min(canvas.1 / image.1 as f64)
            * 2f64.powf(view.zoom as f64 / 4.0);
        let half_x = (canvas.0 / scale / 2.0).min(image.0 as f64 / 2.0);
        let half_y = (canvas.1 / scale / 2.0).min(image.1 as f64 / 2.0);
        Self {
            canvas,
            scale,
            center: (
                ((view.center_x as f64 / CENTER_UNITS + 0.5) * image.0 as f64)
                    .clamp(half_x, image.0 as f64 - half_x),
                ((view.center_y as f64 / CENTER_UNITS + 0.5) * image.1 as f64)
                    .clamp(half_y, image.1 as f64 - half_y),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SurfacePlan {
    pub thumbnail: Rect,
    pub expanded: Option<(Rect, ImageViewport)>,
}
#[derive(Clone)]
pub(crate) struct RenderRequest {
    pub frame: Arc<CapturedFrame>,
    pub plan: SurfacePlan,
}
impl RenderRequest {
    fn same(&self, other: &Self) -> bool {
        self.frame.target == other.frame.target
            && self.frame.sequence == other.frame.sequence
            && self.plan == other.plan
    }
}
pub(crate) struct PreparedSurfaces {
    pub target: PreviewTarget,
    pub sequence: u64,
    pub plan: SurfacePlan,
    pub thumbnail: Option<Protocol>,
    pub expanded: Option<Protocol>,
    pub error: Option<PreviewError>,
}

pub(crate) struct RenderWorker {
    request: watch::Sender<Option<RenderRequest>>,
    latest: Arc<Mutex<Option<PreparedSurfaces>>>,
    task: Option<JoinHandle<()>>,
}
impl RenderWorker {
    pub(crate) fn start(mode: ImageMode) -> Self {
        let (request, mut receiver) = watch::channel::<Option<RenderRequest>>(None);
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        let task = tokio::spawn(async move {
            loop {
                if receiver.changed().await.is_err() {
                    break;
                }
                let Some(request) = receiver.borrow_and_update().clone() else {
                    continue;
                };
                let active = request.clone();
                let prepared = tokio::task::spawn_blocking(move || prepare(request, mode)).await;
                if receiver
                    .borrow()
                    .as_ref()
                    .is_some_and(|current| current.same(&active))
                {
                    let prepared = prepared.unwrap_or_else(|_| PreparedSurfaces {
                        target: active.frame.target.clone(),
                        sequence: active.frame.sequence,
                        plan: active.plan,
                        thumbnail: None,
                        expanded: None,
                        error: Some(PreviewError::InvalidImage),
                    });
                    *output.lock().unwrap_or_else(|e| e.into_inner()) = Some(prepared);
                }
            }
        });
        Self {
            request,
            latest,
            task: Some(task),
        }
    }
    pub(crate) fn set_request(&mut self, request: Option<RenderRequest>) {
        let same = match (self.request.borrow().as_ref(), request.as_ref()) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.request.send_replace(request);
        }
    }
    pub(crate) fn take_latest(&mut self) -> Option<PreparedSurfaces> {
        let prepared = self
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        self.request
            .borrow()
            .as_ref()
            .filter(|current| {
                current.frame.target == prepared.target
                    && current.frame.sequence == prepared.sequence
                    && current.plan == prepared.plan
            })
            .map(|_| prepared)
    }
    pub(crate) async fn stop(mut self) {
        self.set_request(None);
        // Closing the only sender wakes the loop after the current blocking task joins.
        let (placeholder, _) = watch::channel(None);
        drop(std::mem::replace(&mut self.request, placeholder));
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

fn prepare(request: RenderRequest, mode: ImageMode) -> PreparedSurfaces {
    let mut prepared = PreparedSurfaces {
        target: request.frame.target.clone(),
        sequence: request.frame.sequence,
        plan: request.plan.clone(),
        thumbnail: None,
        expanded: None,
        error: None,
    };
    let result = (|| {
        if !request.plan.thumbnail.is_empty() {
            prepared.thumbnail = Some(surface(
                &request.frame.pixels,
                request.plan.thumbnail,
                &ImageViewport::default(),
                mode,
            )?);
        }
        if let Some((area, view)) = request.plan.expanded {
            prepared.expanded = Some(surface(&request.frame.pixels, area, &view, mode)?);
        }
        Ok(())
    })();
    if let Err(error) = result {
        prepared.thumbnail = None;
        prepared.expanded = None;
        prepared.error = Some(error);
    }
    prepared
}

fn surface(
    image: &DynamicImage,
    area: Rect,
    viewport: &ImageViewport,
    mode: ImageMode,
) -> Result<Protocol, PreviewError> {
    let (cw, ch) = mode.metrics();
    let (width, height) = mode.canvas(area);
    if area.is_empty()
        || area.width > MAX_COLUMNS
        || area.height > MAX_ROWS
        || cw == 0
        || ch == 0
        || height == 0
        || width as u64 * height as u64 > MAX_PIXELS
    {
        return Err(PreviewError::AreaTooLarge);
    }
    if image.width() == 0 || image.height() == 0 {
        return Err(PreviewError::InvalidImage);
    }
    let g = Geometry::new(viewport, area, (image.width(), image.height()), mode);
    let crop_w = (g.canvas.0 / g.scale)
        .min(image.width() as f64)
        .round()
        .max(1.0) as u32;
    let crop_h = (g.canvas.1 / g.scale)
        .min(image.height() as f64)
        .round()
        .max(1.0) as u32;
    let x = (g.center.0 - crop_w as f64 / 2.0).round().max(0.0) as u32;
    let y = (g.center.1 - crop_h as f64 / 2.0).round().max(0.0) as u32;
    let cropped = image.crop_imm(
        x.min(image.width() - crop_w),
        y.min(image.height() - crop_h),
        crop_w,
        crop_h,
    );
    let output_w = (crop_w as f64 * g.scale).round().clamp(1.0, width as f64) as u32;
    let output_h = (crop_h as f64 * g.scale).round().clamp(1.0, height as f64) as u32;
    let resized = cropped.resize_exact(output_w, output_h, FilterType::Triangle);
    let mut canvas = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    canvas
        .copy_from(&resized, (width - output_w) / 2, (height - output_h) / 2)
        .map_err(|_| PreviewError::InvalidImage)?;
    let canvas = DynamicImage::ImageRgba8(canvas);
    match mode {
        ImageMode::Compatible => Halfblocks::new(canvas, area.as_size()).map(Protocol::Halfblocks),
        ImageMode::Sixel { .. } => Sixel::new(canvas, area.as_size(), false).map(Protocol::Sixel),
    }
    .map_err(|_| PreviewError::InvalidImage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::ScreenSelection, tab::TabId};
    use ratatui::{buffer::Buffer, style::Color, widgets::Widget};

    fn frame(sequence: u64) -> Arc<CapturedFrame> {
        let pixels = image::RgbImage::from_fn(80, 40, |x, _| {
            image::Rgb(if x < 40 { [255, 0, 0] } else { [0, 255, 0] })
        });
        Arc::new(CapturedFrame {
            target: PreviewTarget {
                tab: TabId(1),
                generation: 1,
                screen: ScreenSelection {
                    id: "fixture".into(),
                    revision: 1,
                },
            },
            sequence,
            pixels: DynamicImage::ImageRgb8(pixels),
        })
    }
    fn plan() -> SurfacePlan {
        SurfacePlan {
            thumbnail: Rect::new(2, 2, 20, 20),
            expanded: None,
        }
    }

    #[test]
    fn compatible_centers_letterbox_preserves_colors_and_bounds() {
        let area = plan().thumbnail;
        let protocol = surface(
            &frame(1).pixels,
            area,
            &ImageViewport::default(),
            ImageMode::Compatible,
        )
        .unwrap();
        assert_eq!(protocol.size(), area.as_size());
        let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 25));
        ratatui_image::Image::new(&protocol).render(area, &mut buffer);
        assert_eq!(buffer[(2, 2)].bg, Color::Rgb(0, 0, 0));
        assert_eq!(buffer[(4, 11)].bg, Color::Rgb(255, 0, 0));
        assert_eq!(buffer[(20, 11)].bg, Color::Rgb(0, 255, 0));
        assert_eq!(buffer[(0, 0)].bg, Color::Reset);
    }

    #[test]
    fn zoom_preserves_pointer_anchor_then_pan_clamps_and_reset_fits() {
        let area = Rect::new(0, 0, 100, 50);
        let size = (1000, 1000);
        let mut view = ImageViewport::default();
        let before = Geometry::new(&view, area, size, ImageMode::Compatible);
        let anchor = before.center.0 + (70.5 - before.canvas.0 / 2.0) / before.scale;
        assert!(view.zoom_at(4, (70, 25), area, size, ImageMode::Compatible));
        let after = Geometry::new(&view, area, size, ImageMode::Compatible);
        assert!(
            (anchor - (after.center.0 + (70.5 - after.canvas.0 / 2.0) / after.scale)).abs() < 0.01
        );
        view.pan_cells(i16::MAX, i16::MAX, area, size, ImageMode::Compatible);
        let edge = view.clone();
        assert!(!view.pan_cells(i16::MAX, i16::MAX, area, size, ImageMode::Compatible));
        assert_eq!(view, edge);
        view.zoom_at(i16::MAX, (50, 25), area, size, ImageMode::Compatible);
        assert_eq!(view.zoom, 16);
        view.zoom_at(i16::MIN, (50, 25), area, size, ImageMode::Compatible);
        assert_eq!(view, ImageViewport::default());
        view.reset();
        assert_eq!(view, ImageViewport::default());
    }

    #[test]
    fn zoom_crops_original_source_instead_of_thumbnail() {
        let pixels = image::RgbImage::from_fn(1000, 1000, |x, _| {
            image::Rgb(if (480..520).contains(&x) {
                [0, 255, 0]
            } else {
                [255, 0, 0]
            })
        });
        let source = DynamicImage::ImageRgb8(pixels);
        let area = Rect::new(0, 0, 20, 10);
        let mut view = ImageViewport::default();
        view.zoom_at(16, (9, 4), area, (1000, 1000), ImageMode::Compatible);
        let protocol = surface(&source, area, &view, ImageMode::Compatible).unwrap();
        let mut buffer = Buffer::empty(area);
        ratatui_image::Image::new(&protocol).render(area, &mut buffer);
        assert!(
            buffer
                .content
                .iter()
                .filter(|cell| cell.bg == Color::Rgb(0, 255, 0))
                .count()
                > 40
        );
    }

    #[test]
    fn raster_limits_are_checked_before_allocation() {
        for (area, mode) in [
            (Rect::new(0, 0, 161, 80), ImageMode::Compatible),
            (Rect::new(0, 0, 160, 81), ImageMode::Compatible),
            (
                Rect::new(0, 0, 160, 80),
                ImageMode::Sixel {
                    cell_width: 1000,
                    cell_height: 1000,
                },
            ),
            (
                Rect::new(0, 0, 10, 10),
                ImageMode::Sixel {
                    cell_width: 0,
                    cell_height: 10,
                },
            ),
        ] {
            assert!(matches!(
                surface(&frame(1).pixels, area, &ImageViewport::default(), mode),
                Err(PreviewError::AreaTooLarge)
            ));
        }
    }

    #[test]
    fn actual_sixel_roundtrip_preserves_raster_and_colors() {
        let mode = ImageMode::Sixel {
            cell_width: 8,
            cell_height: 16,
        };
        let Protocol::Sixel(encoded) = surface(
            &frame(1).pixels,
            Rect::new(0, 0, 10, 5),
            &ImageViewport::default(),
            mode,
        )
        .unwrap() else {
            panic!("native protocol");
        };
        assert!(!encoded.is_tmux);
        let start = encoded.data.find("\x1bP").unwrap();
        let decoded = icy_sixel::SixelImage::decode(encoded.data[start..].as_bytes()).unwrap();
        assert_eq!((decoded.width, decoded.height), (80, 78));
        assert!(decoded
            .pixels
            .chunks_exact(4)
            .any(|p| p[0] > 240 && p[1] < 15 && p[2] < 15));
        assert!(decoded
            .pixels
            .chunks_exact(4)
            .any(|p| p[1] > 240 && p[0] < 15 && p[2] < 15));
    }

    #[tokio::test]
    async fn renderer_coalesces_identical_requests_and_discards_replaced_source() {
        let mut worker = RenderWorker::start(ImageMode::Compatible);
        let request = RenderRequest {
            frame: frame(1),
            plan: plan(),
        };
        worker.set_request(Some(request.clone()));
        let rendered = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(result) = worker.take_latest() {
                    break result;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(rendered.sequence, 1);
        worker.set_request(Some(request));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(worker.take_latest().is_none());
        worker.set_request(Some(RenderRequest {
            frame: frame(2),
            plan: plan(),
        }));
        worker.set_request(None);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(worker.take_latest().is_none());
        worker.stop().await;
    }
}
