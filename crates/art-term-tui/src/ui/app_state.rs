use super::*;

impl App {
    #[cfg(test)]
    pub(super) fn new(backend: Backend) -> Self {
        Self::with_bindings(backend, Bindings::default())
    }
    #[cfg(test)]
    pub(super) fn with_bindings(backend: Backend, bindings: Bindings) -> Self {
        Self::with_display(
            backend,
            bindings,
            DisplaySelection::Available(CellPixels::new(8, 16).unwrap()),
        )
    }
    pub(super) fn with_display(
        backend: Backend,
        bindings: Bindings,
        display: DisplaySelection,
    ) -> Self {
        let pixels = display.pixels();
        let preview = pixels.map(|_| PreviewWorker::start(backend.clone()));
        Self {
            focus: Focus::Connections,
            layout: WorkspaceLayout::new(Rect::default(), Focus::Connections),
            bindings,
            hits: HitMap::default(),
            paused: HashSet::new(),
            tree_selected: None,
            tree_offset: 0,
            catalog: HashMap::new(),
            catalog_errors: HashSet::new(),
            load_queue: VecDeque::new(),
            tree_load: None,
            preview,
            preview_target: None,
            preview_image: None,
            source_frame: None,
            renderer: pixels.map(RenderWorker::start),
            render_request: None,
            last_rendered_sequence: None,
            expanded_image: None,
            display,
            expanded: false,
            viewport: ImageViewport::default(),
            drag: None,
            image_damage: false,
            images_visible: true,
            preview_status: None,
            generation: 0,
            backend,
            profiles: ProfileView {
                hosts: Vec::new(),
                screens: Vec::new(),
            },
            page: Page::Hosts,
            modal: None,
            modal_origin: None,
            confirm_yes: false,
            help_offset: 0,
            tabs: Vec::new(),
            tab: 0,
            prefix: false,
            status: "載入主機清單…".into(),
            pending: None,
            closing: Vec::new(),
            closing_tabs: Vec::new(),
            epoch: 0,
            size: TerminalSize::new(24, 80),
            quit: false,
        }
    }
    pub(super) async fn set_display(&mut self, display: DisplaySelection) {
        let pixels = display.pixels();
        if self.display.pixels() != pixels {
            self.invalidate_preview();
            if let Some(worker) = self.preview.take() {
                worker.stop().await;
            }
            if let Some(renderer) = self.renderer.take() {
                renderer.stop().await;
            }
            self.preview = pixels.map(|_| PreviewWorker::start(self.backend.clone()));
            self.renderer = pixels.map(RenderWorker::start);
        }
        self.display = display;
    }
    pub(super) fn job(&mut self, future: impl Future<Output = JobResult> + Send + 'static) {
        if self.pending.is_some() {
            self.status = "操作進行中，請稍候。終端分頁仍可切換與輸入。".into();
            return;
        }
        self.pending = Some(Pending {
            epoch: self.epoch,
            task: tokio::spawn(future),
        });
        self.status = "處理中… / Working…".into();
    }
    pub(super) fn profiles(&mut self) {
        let backend = self.backend.clone();
        self.job(async move { JobResult::Profiles(backend.profiles().await) });
    }
    pub(super) fn invalidate_preview(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.preview_target = None;
        self.last_rendered_sequence = None;
        self.close_viewer();
        self.damage_images();
        self.source_frame = None;
        self.render_request = None;
        self.expanded_image = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_request(None);
        }
        self.preview_image = None;
        self.preview_status = None;
        if let Some(worker) = &mut self.preview {
            worker.set_target(None);
        }
    }
    pub(super) fn navigate(&mut self, page: Page) {
        self.close_viewer();
        if matches!(page, Page::Help) || matches!(self.page, Page::Help) {
            self.damage_images();
        }
        self.focus = if matches!(page, Page::Terminal) {
            Focus::Terminal
        } else {
            Focus::Connections
        };
        self.dismiss_modal();
        self.page = page;
        self.help_offset = 0;
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub(super) fn dismiss_modal(&mut self) {
        if let Some(Modal::Import { preview, .. }) = self.modal.take() {
            let backend = self.backend.clone();
            self.closing.push(tokio::spawn(async move {
                backend.cancel_import(preview.ticket).await;
            }));
        }
    }
    pub(super) fn show_modal(&mut self, modal: Modal) {
        self.confirm_yes = false;
        self.close_viewer();
        self.damage_images();
        self.dismiss_modal();
        self.modal = Some(modal);
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub(super) async fn stop(&mut self) {
        self.load_queue.clear();
        if let Some(load) = self.tree_load.take() {
            load.task.abort();
            let _ = load.task.await;
        }
        if let Some(worker) = self.preview.take() {
            worker.stop().await;
        }
        if let Some(renderer) = self.renderer.take() {
            renderer.stop().await;
        }
        if let Some(pending) = self.pending.take() {
            pending.task.abort();
            let _ = pending.task.await;
        }
        if let Some(Modal::Import { preview, .. }) = self.modal.take() {
            self.backend.cancel_import(preview.ticket).await;
        }
        for tab in self.tabs.drain(..) {
            self.closing.push(tokio::spawn(async move {
                tab.close().await;
            }));
        }
        for closed in self.closing_tabs.drain(..) {
            let _ = closed.task.await;
        }
        for task in self.closing.drain(..) {
            let _ = task.await;
        }
    }
}
