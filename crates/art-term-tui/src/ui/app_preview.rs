use super::*;

impl App {
    pub(super) fn close_viewer(&mut self) {
        if self.expanded {
            self.damage_images();
        }
        self.expanded = false;
        self.expanded_image = None;
        self.drag = None;
        self.viewport.reset();
    }
    pub(super) fn toggle_viewer(&mut self) {
        if let Some(notice) = self.display.notice() {
            self.status = notice.into();
            return;
        }
        if self.expanded {
            self.close_viewer();
        } else if self.source_frame.is_none() {
            self.focus = Focus::Preview;
            self.status = "請先在 [Desktop] 選擇桌面並等待截圖。".into();
        } else if self.layout.terminal_inner.width == 0 || self.layout.terminal_inner.height == 0 {
            self.status = "請放大終端視窗，以在右側開啟桌面預覽。".into();
        } else {
            self.expanded = true;
            self.viewport.reset();
            self.damage_images();
        }
        self.refresh_surfaces();
    }
    pub(super) fn surface_plan(&self) -> SurfacePlan {
        let visible = self.modal.is_none() && !matches!(self.page, Page::Help);
        SurfacePlan {
            thumbnail: if visible {
                self.layout.preview_inner
            } else {
                Rect::default()
            },
            expanded: (visible && self.expanded)
                .then(|| (self.layout.viewer_inner(), self.viewport.clone())),
        }
    }
    pub(super) fn refresh_surfaces(&mut self) {
        if self.modal.is_some() || matches!(self.page, Page::Help) {
            return;
        }
        let Some(source) = self.source_frame.clone() else {
            return;
        };
        let plan = self.surface_plan();
        let identity = (source.sequence, plan.clone());
        if self.render_request.as_ref() != Some(&identity) {
            if self.render_request.as_ref().is_some_and(|(_, old)| {
                old.expanded.as_ref().map(|(area, _)| area)
                    != plan.expanded.as_ref().map(|(area, _)| area)
            }) {
                self.damage_images();
                self.expanded_image = None;
            }
            self.render_request = Some(identity);
            if let Some(renderer) = &mut self.renderer {
                renderer.set_request(Some(RenderRequest {
                    frame: source,
                    plan,
                }));
            }
        }
    }
    pub(super) fn update_preview(&mut self) {
        if self.display.pixels().is_none() {
            return;
        }
        let target = self.tabs.get(self.tab).and_then(|tab| {
            self.selected_screen().map(|screen| PreviewTarget {
                tab: tab.id(),
                generation: self.generation,
                screen,
            })
        });
        if target != self.preview_target {
            self.invalidate_preview();
            self.preview_target = target.map(|mut target| {
                target.generation = self.generation;
                target
            });
        }
        let visible = self.modal.is_none() && !matches!(self.page, Page::Help);
        if let Some(worker) = &mut self.preview {
            worker.set_target(if visible {
                self.preview_target.clone()
            } else {
                None
            });
        }
        if let Some(update) = self
            .preview
            .as_mut()
            .and_then(|worker| worker.take_latest())
        {
            if self.preview_target.as_ref() == Some(&update.target)
                && self
                    .backend
                    .screen_is_current(&update.target.screen)
                    .unwrap_or(false)
            {
                if let Some(frame) = update.frame {
                    self.source_frame = Some(frame);
                } else if matches!(update.status, PreviewStatus::Failed(_)) {
                    self.close_viewer();
                    self.damage_images();
                    self.source_frame = None;
                    self.preview_image = None;
                    self.render_request = None;
                    if let Some(renderer) = &mut self.renderer {
                        renderer.set_request(None);
                    }
                }
                self.preview_status = Some(update.status);
            }
        }
        self.refresh_surfaces();
        if let Some(prepared) = self
            .renderer
            .as_mut()
            .and_then(|worker| worker.take_latest())
        {
            self.apply_surfaces(prepared);
        }
    }
    pub(super) fn apply_surfaces(&mut self, prepared: PreparedSurfaces) {
        if self.preview_target.as_ref() != Some(&prepared.target)
            || !self
                .render_request
                .as_ref()
                .is_some_and(|(sequence, plan)| {
                    prepared.sequence <= *sequence && prepared.plan == *plan
                })
            || self
                .last_rendered_sequence
                .is_some_and(|sequence| prepared.sequence < sequence)
        {
            return;
        }
        self.last_rendered_sequence = Some(prepared.sequence);
        self.preview_image = prepared.thumbnail;
        self.expanded_image = prepared.expanded;
        if let Some(error) = prepared.error {
            self.close_viewer();
            self.damage_images();
            self.source_frame = None;
            self.preview_image = None;
            self.render_request = None;
            self.preview_status = Some(PreviewStatus::Failed(error));
            if let Some(renderer) = &mut self.renderer {
                renderer.set_request(None);
            }
            self.status = error.to_string();
        }
    }
}
