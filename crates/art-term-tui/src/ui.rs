#[cfg(test)]
use crate::image_view::CellPixels;
use crate::{
    backend::{
        Backend, DesktopRequest, DesktopState, HostEdit, HostSelection, ImportView, OpenShell,
        ProfileView, ScreenEdit, ScreenSelection, ShellListView, ShellView, ToolReadiness,
    },
    display::{DisplayPreferences, DisplaySelection, TerminalProbe},
    image_view::{ImageViewport, PreparedSurfaces, RenderRequest, RenderWorker, SurfacePlan},
    keybindings::{BindingAction, Bindings},
    layout::{Focus, WorkspaceLayout},
    preview::{CapturedFrame, PreviewStatus, PreviewTarget, PreviewWorker},
    tab::{HistoryStatus, TabAction, TabClient, TabId, TabStatus, TerminalSize},
    tty::Tty,
};
use art_term_core::instance_mcp::DesktopProfile;
use art_term_core::openab::profiles::HostProfile;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use futures_util::StreamExt;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use ratatui_image::{protocol::Protocol, Image};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use tui_term::widget::{Cursor, PseudoTerminal};
use zeroize::Zeroizing;

use BindingAction as LocalCommand;

const COMMANDS: &[(LocalCommand, &str)] = &[
    (LocalCommand::Hosts, "Connections"),
    (LocalCommand::Terminal, "Terminal"),
    (LocalCommand::Preview, "Preview"),
    (LocalCommand::Screens, "Desktop"),
    (LocalCommand::NewHost, "New Host"),
    (LocalCommand::NewAgent, "New Agent"),
    (LocalCommand::NewScreen, "New Screen"),
    (LocalCommand::Create, "New Shell"),
    (LocalCommand::Edit, "Edit"),
    (LocalCommand::Delete, "Delete"),
    (LocalCommand::Test, "Test"),
    (LocalCommand::Refresh, "Refresh"),
    (LocalCommand::Import, "Import"),
    (LocalCommand::Export, "Export"),
    (LocalCommand::Previous, "Previous Tab"),
    (LocalCommand::Next, "Next Tab"),
    (LocalCommand::Close, "Close Tab"),
    (LocalCommand::Up, "History Up"),
    (LocalCommand::Down, "History Down"),
    (LocalCommand::Follow, "Follow"),
    (LocalCommand::Recover, "Recover"),
    (LocalCommand::Help, "Help"),
    (LocalCommand::Quit, "Quit"),
];

#[derive(Clone)]
enum HitAction {
    Command(LocalCommand),
    Focus(Focus),
    ViewerToggle,
    ViewerClose,
    ViewerFit,
    ViewerCanvas,
    Tab(usize),
    CloseTab(usize),
    Tree(String),
    Field(usize),
    Submit,
    Cancel,
    Confirm,
    ModalRow(usize),
    ToggleReplace,
}
#[derive(Default)]
struct HitMap {
    regions: Vec<(Rect, HitAction)>,
}
impl HitMap {
    fn add(&mut self, rect: Rect, action: HitAction) {
        if rect.width > 0 && rect.height > 0 {
            self.regions.push((rect, action));
        }
    }
    fn at(&self, x: u16, y: u16) -> Option<HitAction> {
        self.regions
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
            .map(|(_, a)| a.clone())
    }
}
#[derive(Clone)]
enum TreeAction {
    NewHost,
    NewAgent,
    NewScreen,
    NewShell(HostSelection),
    Refresh(HostSelection),
    Shell(HostSelection, ShellView),
    Screen(usize),
    Host(usize),
}
#[derive(Clone)]
struct TreeRow {
    key: String,
    label: String,
    action: Option<TreeAction>,
}
struct TreeLoad {
    host: HostSelection,
    task: JoinHandle<crate::backend::Result<ShellListView>>,
}

enum Page {
    Hosts,
    Terminal,
    Help,
}
enum Confirm {
    DeleteHost(HostSelection),
    DeleteScreen(ScreenSelection),
    Attach(ShellView),
    DeleteShell(ShellView),
    ClearTools(TabId),
}
enum Modal {
    Actions {
        row: usize,
    },
    Host(HostForm),
    Screen(ScreenForm),
    Screens {
        row: usize,
    },
    Confirm(Confirm),
    Transfer(TransferForm),
    Import {
        preview: ImportView,
        replace: bool,
        scroll: u16,
    },
}

struct Field {
    label: &'static str,
    value: Zeroizing<String>,
    secret: bool,
}
impl Field {
    fn new(label: &'static str, value: String, secret: bool) -> Self {
        Self {
            label,
            value: Zeroizing::new(value),
            secret,
        }
    }
    fn visible(&self) -> String {
        if self.secret {
            "*".repeat(self.value.chars().count())
        } else {
            safe(&self.value)
        }
    }
    fn input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Backspace => {
                self.value.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.value.clear()
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && self.value.len() + c.len_utf8() <= 4096 =>
            {
                self.value.push(c)
            }
            _ => (),
        }
    }
    fn paste(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.value.len() + c.len_utf8() > 4096 {
                break;
            }
            self.value.push(c);
        }
    }
}
struct HostForm {
    category: art_term_core::connections::Category,
    previous: Option<HostSelection>,
    fields: [Field; 3],
    active: usize,
}
impl HostForm {
    fn new(host: Option<&HostProfile>) -> Self {
        Self {
            previous: host.map(HostSelection::from),
            category: host
                .and_then(|h| h.metadata.as_ref())
                .map(|m| m.category.clone())
                .unwrap_or(art_term_core::connections::Category::Host),
            fields: [
                Field::new(
                    "名稱 / Name",
                    host.map(|h| h.name.clone()).unwrap_or_default(),
                    false,
                ),
                Field::new(
                    "HTTPS Runtime URL",
                    host.map(|h| h.base_url.clone()).unwrap_or_default(),
                    false,
                ),
                Field::new(
                    "管理憑證 / Credential（編輯時空白保留）",
                    String::new(),
                    true,
                ),
            ],
            active: 0,
        }
    }
    fn agent() -> Self {
        let mut form = Self::new(None);
        form.category = art_term_core::connections::Category::Agent;
        form
    }
    fn edit(mut self) -> HostEdit {
        HostEdit {
            category: self.category,
            previous: self.previous,
            name: std::mem::take(&mut *self.fields[0].value),
            base_url: std::mem::take(&mut *self.fields[1].value),
            credential: std::mem::take(&mut self.fields[2].value),
        }
    }
}
struct ScreenForm {
    previous: Option<ScreenSelection>,
    fields: [Field; 3],
    active: usize,
}
impl ScreenForm {
    fn new(screen: Option<&DesktopProfile>) -> Self {
        Self {
            previous: screen.map(|p| ScreenSelection {
                id: p.id.clone(),
                revision: p.revision,
            }),
            fields: [
                Field::new(
                    "名稱 / Name",
                    screen.map(|p| p.name.clone()).unwrap_or_default(),
                    false,
                ),
                Field::new(
                    "HTTPS MCP URL",
                    screen.map(|p| p.mcp_url.clone()).unwrap_or_default(),
                    false,
                ),
                Field::new("Token（編輯時空白保留）", String::new(), true),
            ],
            active: 0,
        }
    }
    fn edit(mut self) -> ScreenEdit {
        ScreenEdit {
            previous: self.previous,
            name: std::mem::take(&mut *self.fields[0].value),
            mcp_url: std::mem::take(&mut *self.fields[1].value),
            credential: std::mem::take(&mut self.fields[2].value),
        }
    }
}
struct TransferForm {
    export: bool,
    fields: Vec<Field>,
    active: usize,
}
impl TransferForm {
    fn new(export: bool) -> Self {
        let mut fields = vec![
            Field::new("檔案路徑 / File path", String::new(), false),
            Field::new("檔案密碼 / File password", String::new(), true),
        ];
        if export {
            fields.push(Field::new(
                "再次輸入檔案密碼 / Confirm password",
                String::new(),
                true,
            ));
        }
        Self {
            export,
            fields,
            active: 0,
        }
    }
}

enum JobResult {
    Profiles(crate::backend::Result<ProfileView>),
    Saved(crate::backend::Result<HostSelection>),
    ScreenSaved(crate::backend::Result<ScreenSelection>),
    Deleted(crate::backend::Result<()>),
    Tested(crate::backend::Result<usize>),
    Opened(crate::backend::Result<TabClient>),
    RemoteDeleted(crate::backend::Result<()>),
    Preview(crate::backend::Result<ImportView>),
    Imported(crate::backend::Result<crate::backend::ImportSummary>),
    Exported(crate::backend::Result<()>),
}
struct Pending {
    epoch: u64,
    task: JoinHandle<JobResult>,
}
struct ClosingTab {
    host: HostSelection,
    task: JoinHandle<()>,
}
struct App {
    backend: Backend,
    focus: Focus,
    layout: WorkspaceLayout,
    bindings: Bindings,
    hits: HitMap,
    paused: HashSet<TabId>,
    tree_selected: Option<String>,
    tree_offset: usize,
    catalog: HashMap<String, ShellListView>,
    catalog_errors: HashSet<String>,
    load_queue: VecDeque<HostSelection>,
    tree_load: Option<TreeLoad>,
    preview: Option<PreviewWorker>,
    preview_target: Option<PreviewTarget>,
    preview_image: Option<Protocol>,
    source_frame: Option<Arc<CapturedFrame>>,
    renderer: Option<RenderWorker>,
    render_request: Option<(u64, SurfacePlan)>,
    last_rendered_sequence: Option<u64>,
    expanded_image: Option<Protocol>,
    display: DisplaySelection,
    expanded: bool,
    viewport: ImageViewport,
    drag: Option<(u16, u16)>,
    image_damage: bool,
    images_visible: bool,
    preview_status: Option<PreviewStatus>,
    generation: u64,
    profiles: ProfileView,
    page: Page,
    modal: Option<Modal>,
    confirm_yes: bool,
    help_offset: u16,
    tabs: Vec<TabClient>,
    tab: usize,
    prefix: bool,
    status: String,
    pending: Option<Pending>,
    closing: Vec<JoinHandle<()>>,
    closing_tabs: Vec<ClosingTab>,
    epoch: u64,
    size: TerminalSize,
    quit: bool,
}

impl App {
    #[cfg(test)]
    fn new(backend: Backend) -> Self {
        Self::with_bindings(backend, Bindings::default())
    }
    #[cfg(test)]
    fn with_bindings(backend: Backend, bindings: Bindings) -> Self {
        Self::with_display(
            backend,
            bindings,
            DisplaySelection::Available(CellPixels::new(8, 16).unwrap()),
        )
    }
    fn with_display(backend: Backend, bindings: Bindings, display: DisplaySelection) -> Self {
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
    async fn set_display(&mut self, display: DisplaySelection) {
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
    fn job(&mut self, future: impl Future<Output = JobResult> + Send + 'static) {
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
    fn profiles(&mut self) {
        let backend = self.backend.clone();
        self.job(async move { JobResult::Profiles(backend.profiles().await) });
    }
    fn invalidate_preview(&mut self) {
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
    fn navigate(&mut self, page: Page) {
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
    fn dismiss_modal(&mut self) {
        if let Some(Modal::Import { preview, .. }) = self.modal.take() {
            let backend = self.backend.clone();
            self.closing.push(tokio::spawn(async move {
                backend.cancel_import(preview.ticket).await;
            }));
        }
    }
    fn show_modal(&mut self, modal: Modal) {
        self.confirm_yes = false;
        self.close_viewer();
        self.damage_images();
        self.dismiss_modal();
        self.modal = Some(modal);
        self.epoch = self.epoch.wrapping_add(1);
    }
    fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for (agent, title) in [(false, "Computer"), (true, "Agent")] {
            rows.push(TreeRow {
                key: format!("category:{title}"),
                label: title.into(),
                action: None,
            });
            if !agent {
                rows.push(TreeRow {
                    key: "new-host".into(),
                    label: "  + New Host".into(),
                    action: Some(TreeAction::NewHost),
                });
            }
            if agent {
                rows.push(TreeRow {
                    key: "new-agent".into(),
                    label: "  + New Agent".into(),
                    action: Some(TreeAction::NewAgent),
                });
            }
            for (index, host) in self.profiles.hosts.iter().enumerate().filter(|(_, h)| {
                h.metadata
                    .as_ref()
                    .is_some_and(|m| m.category == art_term_core::connections::Category::Agent)
                    == agent
            }) {
                let selection = HostSelection::from(host);
                rows.push(TreeRow {
                    key: format!("host:{}", host.id),
                    label: match host
                        .metadata
                        .as_ref()
                        .and_then(|m| m.display_name.as_ref())
                        .filter(|name| !name.trim().is_empty())
                    {
                        Some(name) => format!("  {} / {}", safe(&host.name), safe(name)),
                        None => format!("  {}", safe(&host.name)),
                    },
                    action: Some(TreeAction::Host(index)),
                });
                rows.push(TreeRow {
                    key: format!("new-shell:{}", host.id),
                    label: "    + New Shell".into(),
                    action: Some(TreeAction::NewShell(selection.clone())),
                });
                if let Some(list) = self
                    .catalog
                    .get(&host.id)
                    .filter(|l| l.host.revision == host.revision)
                {
                    for shell in &list.shells {
                        rows.push(TreeRow {
                            key: format!("shell:{}:{}", host.id, shell.label),
                            label: format!(
                                "    {}{}",
                                safe(&shell.label),
                                if shell.attached { " ●" } else { "" }
                            ),
                            action: Some(TreeAction::Shell(selection.clone(), shell.clone())),
                        });
                    }
                    if list.draining {
                        rows.push(TreeRow {
                            key: format!("draining:{}", host.id),
                            label: "    主機維護中 / Draining".into(),
                            action: None,
                        });
                    }
                    if list.shells.is_empty() {
                        rows.push(TreeRow {
                            key: format!("empty:{}", host.id),
                            label: "    (No Shells)".into(),
                            action: None,
                        });
                    }
                } else {
                    rows.push(TreeRow {
                        key: format!("loading:{}", host.id),
                        label: if self.catalog_errors.contains(&host.id) {
                            "    unavailable / 請重試".into()
                        } else {
                            "    loading…".into()
                        },
                        action: None,
                    });
                }
                rows.push(TreeRow {
                    key: format!("refresh:{}", host.id),
                    label: "    Refresh Shells".into(),
                    action: Some(TreeAction::Refresh(selection)),
                });
            }
        }
        rows.push(TreeRow {
            key: "category:screen".into(),
            label: "Screen".into(),
            action: None,
        });
        rows.push(TreeRow {
            key: "new-screen".into(),
            label: "  + New Screen".into(),
            action: Some(TreeAction::NewScreen),
        });
        for (index, screen) in self.profiles.screens.iter().enumerate() {
            rows.push(TreeRow {
                key: format!("screen:{}", screen.id),
                label: format!("  ▣ {}", safe(&screen.name)),
                action: Some(TreeAction::Screen(index)),
            });
        }
        rows
    }
    fn selected_action(&self) -> Option<TreeAction> {
        let key = self.tree_selected.as_ref()?;
        self.rows()
            .into_iter()
            .find(|row| &row.key == key)
            .and_then(|row| row.action)
    }
    fn move_tree(&mut self, amount: i16) {
        let rows = self.rows();
        let indices: Vec<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.action.as_ref().map(|_| i))
            .collect();
        if indices.is_empty() {
            return;
        }
        let current = indices
            .iter()
            .position(|i| self.tree_selected.as_ref() == Some(&rows[*i].key))
            .unwrap_or(0);
        let next =
            (current as isize + amount as isize).clamp(0, indices.len() as isize - 1) as usize;
        self.tree_selected = Some(rows[indices[next]].key.clone());
    }
    fn host(&self) -> Option<HostSelection> {
        if self.focus == Focus::Connections {
            match self.selected_action() {
                Some(TreeAction::Host(index)) => {
                    return self.profiles.hosts.get(index).map(HostSelection::from)
                }
                Some(
                    TreeAction::NewShell(host)
                    | TreeAction::Refresh(host)
                    | TreeAction::Shell(host, _),
                ) => return Some(host),
                _ => (),
            }
        }
        self.tabs
            .get(self.tab)
            .and_then(|t| self.profiles.hosts.iter().find(|h| h.id == t.host_id()))
            .or(self.profiles.hosts.first())
            .map(HostSelection::from)
    }
    fn queue_host(&mut self, host: HostSelection) {
        if let Some(queued) = self
            .load_queue
            .iter_mut()
            .find(|queued| queued.id == host.id)
        {
            *queued = host;
        } else {
            self.load_queue.push_back(host);
        }
    }
    fn shells(&mut self) {
        if let Some(host) = self.host() {
            self.queue_host(host);
            self.navigate(Page::Hosts);
        }
    }
    fn activate_tree(&mut self) {
        match self.selected_action() {
            Some(TreeAction::NewHost) => self.show_modal(Modal::Host(HostForm::new(None))),
            Some(TreeAction::NewAgent) => self.show_modal(Modal::Host(HostForm::agent())),
            Some(TreeAction::NewScreen) => self.show_modal(Modal::Screen(ScreenForm::new(None))),
            Some(TreeAction::NewShell(host)) => {
                let backend = self.backend.clone();
                let size = self.size;
                self.job(async move {
                    JobResult::Opened(backend.open(OpenShell::Create { host }, size).await)
                });
            }
            Some(TreeAction::Refresh(host)) => self.queue_host(host),
            Some(TreeAction::Host(index)) => {
                if let Some(host) = self.profiles.hosts.get(index) {
                    self.tree_selected = Some(format!("new-shell:{}", host.id));
                }
            }
            Some(TreeAction::Shell(_, shell)) => {
                self.open_shell(shell, false);
            }
            Some(TreeAction::Screen(index)) => self.bind_screen(index + 1),
            None => (),
        }
    }
    fn open_shell(&mut self, shell: ShellView, confirmed: bool) {
        if let Some(index) = self.tabs.iter().position(|tab| {
            tab.binding() == shell.selection.binding()
                && !matches!(tab.view().status, TabStatus::Ended(_))
        }) {
            self.select_tab(index);
            return;
        }
        if !confirmed {
            self.show_modal(Modal::Confirm(Confirm::Attach(shell)));
            return;
        }
        let backend = self.backend.clone();
        let size = self.size;
        self.job(async move {
            JobResult::Opened(
                backend
                    .open(
                        OpenShell::Attach {
                            selection: shell.selection,
                            confirmed: true,
                        },
                        size,
                    )
                    .await,
            )
        });
    }
    fn edit_selected(&mut self) {
        match self.selected_action() {
            Some(TreeAction::Screen(i)) => {
                if let Some(p) = self.profiles.screens.get(i) {
                    self.show_modal(Modal::Screen(ScreenForm::new(Some(p))));
                }
            }
            _ => {
                if let Some(h) = self
                    .host()
                    .and_then(|h| self.profiles.hosts.iter().find(|p| p.id == h.id))
                {
                    self.show_modal(Modal::Host(HostForm::new(Some(h))));
                }
            }
        }
    }
    fn delete_selected(&mut self) {
        match self.selected_action() {
            Some(TreeAction::Screen(i)) => {
                if let Some(p) = self.profiles.screens.get(i) {
                    self.show_modal(Modal::Confirm(Confirm::DeleteScreen(ScreenSelection {
                        id: p.id.clone(),
                        revision: p.revision,
                    })));
                }
            }
            Some(TreeAction::Shell(_, shell)) => {
                self.show_modal(Modal::Confirm(Confirm::DeleteShell(shell)))
            }
            _ => {
                if let Some(h) = self.host() {
                    self.show_modal(Modal::Confirm(Confirm::DeleteHost(h)));
                }
            }
        }
    }
    fn create(&mut self) {
        if let Some(host) = self.host() {
            let backend = self.backend.clone();
            let size = self.size;
            self.job(async move {
                JobResult::Opened(backend.open(OpenShell::Create { host }, size).await)
            });
        } else {
            self.status = "請先選擇或新增主機。".into();
        }
    }
    fn send(&mut self, action: TabAction) {
        if let Some(tab) = self.tabs.get(self.tab) {
            if let Err(error) = tab.try_send(action) {
                self.status = error.to_string();
            }
        }
    }
    fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            if self.tab != index {
                self.invalidate_preview();
            }
            self.tab = index;
            self.navigate(Page::Terminal);
        }
    }
    fn local(&mut self, command: LocalCommand) {
        match command {
            LocalCommand::Terminal => {
                self.navigate(Page::Terminal);
            }
            LocalCommand::Preview => {
                self.focus = Focus::Preview;
            }
            LocalCommand::Screens => self.show_modal(Modal::Screens { row: 0 }),
            LocalCommand::Hosts => {
                self.navigate(Page::Hosts);
                self.profiles();
            }
            LocalCommand::Actions => self.show_modal(Modal::Actions { row: 0 }),
            LocalCommand::Edit => self.edit_selected(),
            LocalCommand::Delete => self.delete_selected(),
            LocalCommand::NewHost => self.show_modal(Modal::Host(HostForm::new(None))),
            LocalCommand::NewAgent => self.show_modal(Modal::Host(HostForm::agent())),
            LocalCommand::NewScreen => self.show_modal(Modal::Screen(ScreenForm::new(None))),
            LocalCommand::Refresh => {
                self.profiles();
            }
            LocalCommand::Test => {
                if let Some(host) = self.host() {
                    let backend = self.backend.clone();
                    self.job(async move { JobResult::Tested(backend.test_host(host).await) });
                }
            }
            LocalCommand::Shells => self.shells(),
            LocalCommand::Create => self.create(),
            LocalCommand::Previous if !self.tabs.is_empty() => {
                self.select_tab((self.tab + self.tabs.len() - 1) % self.tabs.len())
            }
            LocalCommand::Next if !self.tabs.is_empty() => {
                self.select_tab((self.tab + 1) % self.tabs.len())
            }
            LocalCommand::Close => {
                if !self.tabs.is_empty() {
                    self.invalidate_preview();
                    let tab = self.tabs.remove(self.tab);
                    self.paused.remove(&tab.id());
                    let host = self
                        .profiles
                        .hosts
                        .iter()
                        .find(|host| host.id == tab.host_id())
                        .map(HostSelection::from);
                    let task = tokio::spawn(async move {
                        tab.close().await;
                    });
                    if let Some(host) = host {
                        self.closing_tabs.push(ClosingTab { host, task });
                    } else {
                        self.closing.push(task);
                    }
                    self.tab = self.tab.min(self.tabs.len().saturating_sub(1));
                    if self.tabs.is_empty() {
                        self.navigate(Page::Hosts);
                    }
                    self.status = "本機分頁已關閉，遠端工作仍保留。".into();
                }
            }
            LocalCommand::Up => self.send(TabAction::Scroll(-12)),
            LocalCommand::Down => self.send(TabAction::Scroll(12)),
            LocalCommand::Follow => self.send(TabAction::Follow),
            LocalCommand::Recover => self.send(TabAction::Recover),
            LocalCommand::Import => self.show_modal(Modal::Transfer(TransferForm::new(false))),
            LocalCommand::Export => self.show_modal(Modal::Transfer(TransferForm::new(true))),
            LocalCommand::Help => self.navigate(Page::Help),
            LocalCommand::Quit => self.quit = true,
            _ => (),
        }
    }
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if self.expanded && key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.close_viewer();
            return;
        }
        if self.modal.is_some() {
            self.prefix = false;
            let key = self.bindings.navigation_key(key);
            self.modal_key(key);
            return;
        }
        let was_prefix = self.prefix;
        self.prefix = false;
        if self.bindings.is_leader(key) {
            if was_prefix && self.focus == Focus::Terminal {
                self.send(TabAction::Key(self.bindings.literal_leader()));
            } else {
                self.prefix = true;
            }
            return;
        }
        if !was_prefix
            && self.focus == Focus::Connections
            && !matches!(self.page, Page::Help)
            && key.modifiers.is_empty()
        {
            match key.code {
                KeyCode::Up => {
                    self.move_tree(-1);
                    return;
                }
                KeyCode::Down => {
                    self.move_tree(1);
                    return;
                }
                KeyCode::Enter => {
                    self.activate_tree();
                    return;
                }
                _ => (),
            }
        }
        if let Some(action) = self.bindings.action(key, was_prefix) {
            self.binding_action(action);
            return;
        }
        if was_prefix {
            self.status = "未知快捷鍵；點擊 Help 查看目前設定。".into();
            return;
        }
        if self.expanded {
            return;
        }
        if self.focus == Focus::Terminal && matches!(self.page, Page::Terminal) {
            self.send(TabAction::Key(key));
            return;
        }
        let key = if self.focus == Focus::Connections
            && matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Enter)
        {
            key
        } else {
            self.bindings.navigation_key(key)
        };
        if matches!(self.page, Page::Help) {
            match key.code {
                KeyCode::Down => self.help_offset = self.help_offset.saturating_add(1),
                KeyCode::Up => self.help_offset = self.help_offset.saturating_sub(1),
                KeyCode::Esc => self.navigate(if self.tabs.is_empty() {
                    Page::Hosts
                } else {
                    Page::Terminal
                }),
                _ => (),
            }
        } else if self.focus == Focus::Preview {
            if key.code == KeyCode::Enter {
                self.local(LocalCommand::Screens);
            }
        } else {
            match key.code {
                KeyCode::Down => self.move_tree(1),
                KeyCode::Up => self.move_tree(-1),
                KeyCode::Enter => self.activate_tree(),
                KeyCode::Esc if !self.tabs.is_empty() => self.navigate(Page::Terminal),
                _ => (),
            }
        }
    }
    fn binding_action(&mut self, action: BindingAction) {
        match action {
            BindingAction::ViewerToggle => {
                self.toggle_viewer();
                return;
            }
            BindingAction::ViewerFit => {
                self.viewport.reset();
                self.refresh_surfaces();
                return;
            }
            BindingAction::FocusNext => {
                self.focus = match self.focus {
                    Focus::Connections => Focus::Preview,
                    Focus::Preview => Focus::Terminal,
                    Focus::Terminal => Focus::Connections,
                };
                self.page = if self.focus == Focus::Terminal {
                    Page::Terminal
                } else {
                    Page::Hosts
                };
                return;
            }
            BindingAction::TreeUp => {
                if self.focus == Focus::Connections {
                    self.move_tree(-1);
                }
                return;
            }
            BindingAction::TreeDown => {
                if self.focus == Focus::Connections {
                    self.move_tree(1);
                }
                return;
            }
            BindingAction::TreeActivate => {
                if self.focus == Focus::Connections {
                    self.activate_tree();
                }
                return;
            }
            _ => (),
        }
        let index = match action {
            BindingAction::Tab1 => Some(0),
            BindingAction::Tab2 => Some(1),
            BindingAction::Tab3 => Some(2),
            BindingAction::Tab4 => Some(3),
            BindingAction::Tab5 => Some(4),
            BindingAction::Tab6 => Some(5),
            BindingAction::Tab7 => Some(6),
            BindingAction::Tab8 => Some(7),
            BindingAction::Tab9 => Some(8),
            _ => None,
        };
        if let Some(index) = index {
            self.select_tab(index);
        } else {
            self.local(action);
        }
    }
    fn bind_screen(&mut self, row: usize) {
        self.invalidate_preview();
        let Some(tab) = self.tabs.get(self.tab) else {
            self.status = "先開啟 Shell 分頁，再選擇 Screen。".into();
            return;
        };
        let id = tab.id();
        if row == 0 {
            self.paused.insert(id);
            self.status = "預覽已停止；Shell 桌面工具仍保留。".into();
        } else if row <= self.profiles.screens.len() {
            let screen = &self.profiles.screens[row - 1];
            match self.backend.desktop_request(
                id,
                DesktopRequest::Select(ScreenSelection {
                    id: screen.id.clone(),
                    revision: screen.revision,
                }),
            ) {
                Ok(()) => {
                    self.paused.remove(&id);
                    self.status = "桌面設定中；請查看目前分頁的工具狀態。".into();
                }
                Err(error) => self.status = error.to_string(),
            }
        } else if row == self.profiles.screens.len() + 1 {
            self.show_modal(Modal::Confirm(Confirm::ClearTools(id)));
        } else if row == self.profiles.screens.len() + 2 {
            self.paused.remove(&id);
        } else {
            self.show_modal(Modal::Screen(ScreenForm::new(None)));
        }
    }
    fn selected_screen(&self) -> Option<ScreenSelection> {
        let id = self.tabs.get(self.tab)?.id();
        if self.paused.contains(&id) {
            return None;
        }
        let view = self.backend.desktop_view(id).ok()?;
        if let DesktopState::Selected { screen, .. } = view.state {
            self.backend
                .screen_is_current(&screen)
                .ok()
                .filter(|current| *current)
                .map(|_| screen)
        } else {
            None
        }
    }
    fn desktop_status(&self) -> String {
        let Some(tab) = self.tabs.get(self.tab) else {
            return "Desktop: 先開啟 Shell".into();
        };
        if tab.view().status != TabStatus::Live {
            return "Desktop: 工作連線未就緒，工具未確認".into();
        }
        let Ok(view) = self.backend.desktop_view(tab.id()) else {
            return "Desktop: unavailable".into();
        };
        if view.busy {
            return "Desktop: 設定中…".into();
        }
        match view.state {
            DesktopState::Selected { screen, tools } => {
                let name = self
                    .profiles
                    .screens
                    .iter()
                    .find(|p| p.id == screen.id && p.revision == screen.revision)
                    .map(|p| safe(&p.name));
                match name {
                    Some(name) => format!(
                        "Desktop: {} | {}{}",
                        name,
                        if tools == ToolReadiness::Ready {
                            "Tools ready"
                        } else {
                            "Tools unconfirmed"
                        },
                        if self.paused.contains(&tab.id()) {
                            " | 預覽停止"
                        } else {
                            ""
                        }
                    ),
                    None => "Desktop: 桌面已變更，請重新選擇".into(),
                }
            }
            DesktopState::Unselected => "Desktop: 未設定，點擊 [Desktop]".into(),
            DesktopState::Unknown => format!(
                "Desktop: {}",
                view.notice.unwrap_or("工具狀態未確認，請選擇桌面")
            ),
        }
    }
    fn modal_key(&mut self, mut key: KeyEvent) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            self.epoch = self.epoch.wrapping_add(1);
            if let Modal::Import { preview, .. } = modal {
                let backend = self.backend.clone();
                self.closing.push(tokio::spawn(async move {
                    backend.cancel_import(preview.ticket).await;
                }));
            }
            return;
        }
        if matches!(modal, Modal::Confirm(_)) {
            if matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Tab
                    | KeyCode::BackTab
            ) {
                self.confirm_yes = !self.confirm_yes;
                self.modal = Some(modal);
                return;
            }
            if key.code == KeyCode::Enter {
                key = KeyEvent::new(
                    KeyCode::Char(if self.confirm_yes { 'y' } else { 'n' }),
                    KeyModifiers::NONE,
                );
            }
        }
        match &mut modal {
            Modal::Actions { row } => match key.code {
                KeyCode::Down => *row = (*row + 1).min(COMMANDS.len() - 1),
                KeyCode::Up => *row = row.saturating_sub(1),
                KeyCode::Enter => {
                    let action = COMMANDS[*row].0;
                    self.local(action);
                    return;
                }
                _ => (),
            },
            Modal::Screens { row } => match key.code {
                KeyCode::Down => *row = (*row + 1).min(self.profiles.screens.len() + 3),
                KeyCode::Up => *row = row.saturating_sub(1),
                KeyCode::Enter => {
                    self.bind_screen(*row);
                    return;
                }
                _ => (),
            },
            Modal::Screen(form) => {
                if submit(key, form.active, form.fields.len()) && self.pending.is_none() {
                    let Modal::Screen(form) = modal else {
                        unreachable!()
                    };
                    let backend = self.backend.clone();
                    let edit = form.edit();
                    self.job(
                        async move { JobResult::ScreenSaved(backend.save_screen(edit).await) },
                    );
                    return;
                }
                field_key(key, &mut form.active, &mut form.fields);
            }
            Modal::Host(form) => {
                if submit(key, form.active, form.fields.len()) {
                    if self.pending.is_some() {
                        self.status = "請等待目前操作完成後再儲存。".into();
                        self.modal = Some(modal);
                        return;
                    }
                    let Modal::Host(form) = modal else {
                        unreachable!()
                    };
                    let backend = self.backend.clone();
                    let edit = form.edit();
                    self.job(async move { JobResult::Saved(backend.save_host(edit).await) });
                    return;
                }
                field_key(key, &mut form.active, &mut form.fields);
            }
            Modal::Transfer(form) => {
                if submit(key, form.active, form.fields.len()) {
                    if self.pending.is_some() {
                        self.status = "請等待目前操作完成。".into();
                        self.modal = Some(modal);
                        return;
                    }
                    if form.fields[0].value.is_empty() {
                        self.status = "請輸入檔案路徑。".into();
                        self.modal = Some(modal);
                        return;
                    }
                    if form.export
                        && (form.fields[1].value.is_empty()
                            || *form.fields[1].value != *form.fields[2].value)
                    {
                        self.status = "檔案密碼不可空白，兩次輸入須相同。".into();
                        self.modal = Some(modal);
                        return;
                    }
                    let Modal::Transfer(mut form) = modal else {
                        unreachable!()
                    };
                    let path = PathBuf::from(&*form.fields[0].value);
                    let password = std::mem::take(&mut form.fields[1].value);
                    let backend = self.backend.clone();
                    if form.export {
                        self.job(async move {
                            JobResult::Exported(backend.export(path, password).await)
                        });
                    } else {
                        let password = if password.is_empty() {
                            None
                        } else {
                            Some(password)
                        };
                        self.job(async move {
                            JobResult::Preview(backend.preview_import(path, password).await)
                        });
                    }
                    return;
                }
                field_key(key, &mut form.active, &mut form.fields);
            }
            Modal::Confirm(_) if matches!(key.code, KeyCode::Char('n' | 'N')) => return,
            Modal::Confirm(_) if matches!(key.code, KeyCode::Char('y' | 'Y')) => {
                if self.pending.is_some() {
                    self.status = "請等待目前操作完成後再確認。".into();
                    self.modal = Some(modal);
                    return;
                }
                let Modal::Confirm(action) = modal else {
                    unreachable!()
                };
                let backend = self.backend.clone();
                match action {
                    Confirm::DeleteScreen(screen) => {
                        self.invalidate_preview();
                        self.job(
                            async move { JobResult::Deleted(backend.delete_screen(screen).await) },
                        );
                    }
                    Confirm::ClearTools(id) => {
                        match self.backend.desktop_request(id, DesktopRequest::Clear) {
                            Ok(()) => self.status = "正在取消此 Shell 的桌面工具…".into(),
                            Err(error) => self.status = error.to_string(),
                        }
                        self.invalidate_preview();
                    }
                    Confirm::DeleteHost(host) => {
                        self.job(async move { JobResult::Deleted(backend.delete_host(host).await) })
                    }
                    Confirm::Attach(shell) => {
                        self.open_shell(shell, true);
                    }
                    Confirm::DeleteShell(shell) => self.job(async move {
                        JobResult::RemoteDeleted(backend.delete_shell(shell.selection, true).await)
                    }),
                }
                return;
            }
            Modal::Import { replace, .. } if key.code == KeyCode::Char(' ') => *replace = !*replace,
            Modal::Import {
                preview, scroll, ..
            } if key.code == KeyCode::Down => {
                *scroll = scroll
                    .saturating_add(1)
                    .min(preview.entries.len().min(u16::MAX as usize - 4) as u16 + 4)
            }
            Modal::Import { scroll, .. } if key.code == KeyCode::Up => {
                *scroll = scroll.saturating_sub(1)
            }
            Modal::Import { .. } if matches!(key.code, KeyCode::Char('y' | 'Y')) => {
                if self.pending.is_some() {
                    self.status = "請等待目前操作完成。".into();
                    self.modal = Some(modal);
                    return;
                }
                let Modal::Import {
                    preview, replace, ..
                } = modal
                else {
                    unreachable!()
                };
                let backend = self.backend.clone();
                self.job(async move {
                    JobResult::Imported(backend.apply_import(preview.ticket, replace).await)
                });
                return;
            }
            _ => (),
        }
        self.modal = Some(modal);
    }
    fn mouse(&mut self, event: MouseEvent) {
        self.prefix = false;
        if self.expanded && self.modal.is_none() && !matches!(self.page, Page::Help) {
            let canvas = self.layout.viewer_inner();
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if matches!(
                        self.hits.at(event.column, event.row),
                        Some(HitAction::ViewerCanvas)
                    ) {
                        self.drag = Some((event.column, event.row));
                        return;
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some((x, y)) = self.drag.replace((event.column, event.row)) {
                        if let (Some(source), Some(pixels)) =
                            (&self.source_frame, self.display.pixels())
                        {
                            self.viewport.pan_cells(
                                event.column as i16 - x as i16,
                                event.row as i16 - y as i16,
                                canvas,
                                (source.pixels.width(), source.pixels.height()),
                                pixels,
                            );
                            self.refresh_surfaces();
                        }
                    } else {
                        self.drag = None;
                    }
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.drag = None;
                    return;
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    if contains(canvas, event.column, event.row) =>
                {
                    if let (Some(source), Some(pixels)) =
                        (&self.source_frame, self.display.pixels())
                    {
                        self.viewport.zoom_at(
                            if event.kind == MouseEventKind::ScrollUp {
                                1
                            } else {
                                -1
                            },
                            (event.column, event.row),
                            canvas,
                            (source.pixels.width(), source.pixels.height()),
                            pixels,
                        );
                        self.refresh_surfaces();
                    }
                    return;
                }
                _ => (),
            }
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(action) = self.hits.at(event.column, event.row) {
                    self.hit(action);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let amount = if event.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                if let Some(modal) = self.modal.as_mut() {
                    match modal {
                        Modal::Actions { row } => {
                            *row = (*row as isize + amount).clamp(0, COMMANDS.len() as isize - 1)
                                as usize
                        }
                        Modal::Screens { row } => {
                            *row = (*row as isize + amount)
                                .clamp(0, self.profiles.screens.len() as isize + 3)
                                as usize
                        }
                        Modal::Import { scroll, .. } => {
                            *scroll =
                                (*scroll as i32 + amount as i32).clamp(0, u16::MAX as i32) as u16
                        }
                        _ => (),
                    }
                } else if matches!(self.page, Page::Help) {
                    self.help_offset = (self.help_offset as i32 + amount as i32 * 3)
                        .clamp(0, u16::MAX as i32) as u16;
                } else if contains(self.layout.connections, event.column, event.row) {
                    self.focus = Focus::Connections;
                    self.page = Page::Hosts;
                    self.move_tree(amount as i16);
                } else if !self.expanded
                    && contains(self.layout.terminal_inner, event.column, event.row)
                {
                    self.focus = Focus::Terminal;
                    self.page = Page::Terminal;
                    self.send(TabAction::Wheel(MouseEvent {
                        column: event.column - self.layout.terminal_inner.x,
                        row: event.row - self.layout.terminal_inner.y,
                        ..event
                    }));
                }
            }
            _ => (),
        }
    }
    fn hit(&mut self, action: HitAction) {
        match action {
            HitAction::Command(command) => self.binding_action(command),
            HitAction::Focus(focus) => {
                self.focus = focus;
                self.page = if focus == Focus::Terminal {
                    Page::Terminal
                } else {
                    Page::Hosts
                };
            }
            HitAction::ViewerToggle => self.toggle_viewer(),
            HitAction::ViewerClose => self.close_viewer(),
            HitAction::ViewerFit => {
                self.viewport.reset();
                self.drag = None;
                self.refresh_surfaces();
            }
            HitAction::ViewerCanvas => (),
            HitAction::Tab(index) => self.select_tab(index),
            HitAction::CloseTab(index) => {
                self.select_tab(index);
                self.local(LocalCommand::Close);
            }
            HitAction::Tree(key) => {
                self.focus = Focus::Connections;
                self.page = Page::Hosts;
                self.tree_selected = Some(key);
                self.activate_tree();
            }
            HitAction::Field(index) => match self.modal.as_mut() {
                Some(Modal::Host(form)) if index < form.fields.len() => form.active = index,
                Some(Modal::Screen(form)) if index < form.fields.len() => form.active = index,
                Some(Modal::Transfer(form)) if index < form.fields.len() => form.active = index,
                _ => (),
            },
            HitAction::Submit => {
                self.modal_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            }
            HitAction::Cancel => self.modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            HitAction::Confirm => {
                self.modal_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            }
            HitAction::ToggleReplace => {
                self.modal_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            }
            HitAction::ModalRow(index) => {
                match self.modal.as_mut() {
                    Some(Modal::Actions { row } | Modal::Screens { row }) => *row = index,
                    _ => return,
                }
                self.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
        }
    }

    fn paste(&mut self, text: String) {
        if self.modal.is_some() {
            let text = Zeroizing::new(text);
            match self.modal.as_mut() {
                Some(Modal::Screen(form)) => form.fields[form.active].paste(&text),
                Some(Modal::Host(form)) => form.fields[form.active].paste(&text),
                Some(Modal::Transfer(form)) => form.fields[form.active].paste(&text),
                _ => (),
            }
        } else if !self.expanded
            && self.focus == Focus::Terminal
            && matches!(self.page, Page::Terminal)
        {
            self.send(TabAction::Paste(text));
        }
    }
    async fn complete(&mut self) {
        if self
            .tree_load
            .as_ref()
            .is_some_and(|load| load.task.is_finished())
        {
            let load = self.tree_load.take().unwrap();
            if self
                .profiles
                .hosts
                .iter()
                .any(|h| h.id == load.host.id && h.revision == load.host.revision)
            {
                match load.task.await {
                    Ok(Ok(list)) => {
                        self.catalog_errors.remove(&load.host.id);
                        self.catalog.insert(load.host.id, list);
                    }
                    _ => {
                        self.catalog_errors.insert(load.host.id);
                    }
                }
            }
        }
        if self.tree_load.is_none() {
            if let Some(host) = self.load_queue.pop_front() {
                let backend = self.backend.clone();
                let requested = host.clone();
                let task = tokio::spawn(async move {
                    tokio::time::timeout(Duration::from_secs(4), backend.list_shells(requested))
                        .await
                        .unwrap_or(Err(crate::backend::BackendError::Busy))
                });
                self.tree_load = Some(TreeLoad { host, task });
            }
        }
        let rows = self.rows();
        if !rows
            .iter()
            .any(|r| self.tree_selected.as_ref() == Some(&r.key) && r.action.is_some())
        {
            self.tree_selected = rows.into_iter().find(|r| r.action.is_some()).map(|r| r.key);
        }
        let mut index = 0;
        while index < self.closing_tabs.len() {
            if self.closing_tabs[index].task.is_finished() {
                let closed = self.closing_tabs.swap_remove(index);
                let _ = closed.task.await;
                if let Some(host) = self
                    .profiles
                    .hosts
                    .iter()
                    .find(|host| host.id == closed.host.id)
                {
                    self.queue_host(HostSelection::from(host));
                }
            } else {
                index += 1;
            }
        }
        self.closing.retain(|task| !task.is_finished());
        if !self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.task.is_finished())
        {
            return;
        }
        let pending = self.pending.take().unwrap();
        let current = pending.epoch == self.epoch;
        let result = match pending.task.await {
            Ok(result) => result,
            Err(_) => {
                self.status = "操作未完成，請重新整理後確認結果。".into();
                return;
            }
        };
        match result {
            JobResult::Profiles(Ok(profiles)) => {
                self.catalog.retain(|id, list| {
                    profiles
                        .hosts
                        .iter()
                        .any(|h| &h.id == id && h.revision == list.host.revision)
                });
                self.profiles = profiles;
                for host in self.profiles.hosts.clone() {
                    self.queue_host(HostSelection::from(&host));
                }
                self.status = format!(
                    "{} Hosts，{} 個已保存 Screen",
                    self.profiles.hosts.len(),
                    self.profiles.screens.len()
                );
            }
            JobResult::ScreenSaved(Ok(_)) => {
                self.profiles();
            }
            JobResult::Saved(Ok(_)) => {
                self.profiles();
            }
            JobResult::Deleted(Ok(())) => {
                self.profiles();
            }
            JobResult::Tested(Ok(count)) => {
                self.status = format!("主機連線正常，遠端工作數量 {count}。")
            }
            JobResult::Opened(Ok(tab)) => {
                if let Some(host) = self.profiles.hosts.iter().find(|h| h.id == tab.host_id()) {
                    self.queue_host(HostSelection::from(host));
                }
                self.tabs.push(tab);
                if current {
                    self.tab = self.tabs.len() - 1;
                    self.navigate(Page::Terminal);
                }
                self.status = "遠端 Shell 已連線；點擊右上 [Desktop] 設定此連線的桌面。".into();
            }
            JobResult::RemoteDeleted(Ok(())) => {
                self.status = "遠端工作已刪除。".into();
                if current {
                    self.shells();
                }
            }
            JobResult::Preview(Ok(preview)) => {
                if current {
                    self.show_modal(Modal::Import {
                        preview,
                        replace: false,
                        scroll: 0,
                    });
                    self.status = "檢查匯入清單後按 y 確認，Esc 取消。".into();
                } else {
                    let backend = self.backend.clone();
                    self.closing.push(tokio::spawn(async move {
                        backend.cancel_import(preview.ticket).await;
                    }));
                }
            }
            JobResult::Imported(Ok(summary)) => {
                self.status = format!(
                    "匯入完成，新增 {}、更新 {}、略過 {}。",
                    summary.added, summary.updated, summary.skipped
                );
                self.profiles();
            }
            JobResult::Exported(Ok(())) => self.status = "已加密匯出；原有檔案不會被覆寫。".into(),
            JobResult::Profiles(Err(error))
            | JobResult::ScreenSaved(Err(error))
            | JobResult::Saved(Err(error))
            | JobResult::Deleted(Err(error))
            | JobResult::Tested(Err(error))
            | JobResult::Opened(Err(error))
            | JobResult::RemoteDeleted(Err(error))
            | JobResult::Preview(Err(error))
            | JobResult::Imported(Err(error))
            | JobResult::Exported(Err(error)) => self.status = error.to_string(),
        }
    }
    fn resize(&mut self, area: Rect) {
        let visible = self.modal.is_none() && !matches!(self.page, Page::Help);
        if visible != self.images_visible {
            self.damage_images();
            self.images_visible = visible;
        }
        let layout = WorkspaceLayout::new(area, self.focus);
        if layout != self.layout {
            self.damage_images();
            self.drag = None;
            self.expanded_image = None;
            self.layout = layout;
            if self.layout.terminal_inner.width == 0 || self.layout.terminal_inner.height == 0 {
                self.close_viewer();
            }
        }
        self.update_preview();
        let Some(size) = self.layout.terminal_size() else {
            return;
        };
        if size != self.size {
            self.size = size;
            for tab in &self.tabs {
                if let Err(error) = tab.try_send(TabAction::Resize(size)) {
                    self.status = error.to_string();
                }
            }
        }
    }
    fn damage_images(&mut self) {
        if self.display.pixels().is_some() {
            self.image_damage = true;
        }
    }
    fn close_viewer(&mut self) {
        if self.expanded {
            self.damage_images();
        }
        self.expanded = false;
        self.expanded_image = None;
        self.drag = None;
        self.viewport.reset();
    }
    fn toggle_viewer(&mut self) {
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
    fn surface_plan(&self) -> SurfacePlan {
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
    fn refresh_surfaces(&mut self) {
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
    fn update_preview(&mut self) {
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
    fn apply_surfaces(&mut self, prepared: PreparedSurfaces) {
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
    async fn stop(&mut self) {
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

fn submit(key: KeyEvent, active: usize, count: usize) -> bool {
    (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL))
        || (key.code == KeyCode::Enter && active + 1 == count)
}
fn field_key(key: KeyEvent, active: &mut usize, fields: &mut [Field]) {
    match key.code {
        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => *active = (*active + 1) % fields.len(),
        KeyCode::BackTab | KeyCode::Up => *active = (*active + fields.len() - 1) % fields.len(),
        _ => fields[*active].input(key),
    }
}
fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
fn body(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(3),
    )
}
#[cfg(test)]
fn terminal_size(area: Rect) -> Option<TerminalSize> {
    WorkspaceLayout::new(area, Focus::Terminal).terminal_size()
}
fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}
fn elide(text: &str, width: u16) -> String {
    let mut text = safe(text);
    if Span::raw(&text).width() <= width as usize {
        return text;
    }
    while !text.is_empty() && Span::raw(&text).width() + 1 > width as usize {
        text.pop();
    }
    if width > 0 {
        text.push('…');
    }
    text
}
fn button(frame: &mut Frame, hits: &mut HitMap, area: Rect, label: &str, action: HitAction) {
    frame.render_widget(
        Paragraph::new(elide(label, area.width)).style(Style::default().fg(Color::Cyan)),
        area,
    );
    hits.add(area, action);
}
fn draw(app: &App, frame: &mut Frame) -> HitMap {
    let area = frame.area();
    let mut hits = HitMap::default();
    if area.height < 4 || area.width < 20 {
        frame.render_widget(Paragraph::new("終端太小，請放大視窗。"), area);
        return hits;
    }
    let layout = WorkspaceLayout::new(area, app.focus);
    let mut x = area.x;
    let desktop_width = 9.min(area.width);
    for (label, command) in [
        ("[Actions]", LocalCommand::Actions),
        ("[Connections]", LocalCommand::Hosts),
        ("[Preview]", LocalCommand::Preview),
        ("[Terminal]", LocalCommand::Terminal),
        ("[Help]", LocalCommand::Help),
    ] {
        let width = Span::raw(label).width() as u16 + 1;
        if x + width > area.right() - desktop_width {
            break;
        }
        button(
            frame,
            &mut hits,
            Rect::new(x, area.y, width, 1),
            label,
            HitAction::Command(command),
        );
        x += width;
    }
    button(
        frame,
        &mut hits,
        Rect::new(area.right() - desktop_width, area.y, desktop_width, 1),
        "[Desktop]",
        HitAction::Command(LocalCommand::Screens),
    );
    let border = |title: String, focus: Focus| {
        panel(title).border_style(Style::default().fg(if app.focus == focus {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
    };
    if layout.connections.width > 0 {
        let block = border("Connections / 連線".into(), Focus::Connections);
        let inner = block.inner(layout.connections);
        frame.render_widget(block, layout.connections);
        hits.add(layout.connections, HitAction::Focus(Focus::Connections));
        let rows = app.rows();
        let selected = rows
            .iter()
            .position(|r| app.tree_selected.as_ref() == Some(&r.key));
        let offset = selected
            .map(|i| i.saturating_sub(inner.height.saturating_sub(1) as usize))
            .unwrap_or(0)
            .max(
                app.tree_offset
                    .min(rows.len().saturating_sub(inner.height as usize)),
            );
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(offset)
            .take(inner.height as usize)
        {
            let rect = Rect::new(inner.x, inner.y + (index - offset) as u16, inner.width, 1);
            let chosen = Some(index) == selected;
            let text = format!("{}{}", if chosen { "▶" } else { " " }, row.label);
            frame.render_widget(
                Paragraph::new(elide(&text, inner.width)).style(if chosen {
                    Style::default().bg(Color::DarkGray)
                } else if row.action.is_none() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                }),
                rect,
            );
            if row.action.is_some() {
                hits.add(rect, HitAction::Tree(row.key.clone()));
            }
        }
    }
    if layout.preview.width > 0 {
        let name = app
            .selected_screen()
            .and_then(|b| app.profiles.screens.iter().find(|p| p.id == b.id))
            .map(|p| safe(&p.name))
            .unwrap_or_else(|| "停止".into());
        frame.render_widget(
            border(format!("Desktop / {name}"), Focus::Preview),
            layout.preview,
        );
        hits.add(layout.preview, HitAction::ViewerToggle);
        if let Some(image) = app
            .preview_image
            .as_ref()
            .filter(|_| app.modal.is_none() && !matches!(app.page, Page::Help))
        {
            frame.render_widget(Image::new(image), layout.preview_inner);
        } else {
            let message = if let Some(notice) = app.display.notice() {
                format!("{notice}\n{}", app.desktop_status())
            } else if app.source_frame.is_some() {
                "準備圖片…".into()
            } else {
                match app.preview_status {
                    Some(PreviewStatus::Capturing) => "擷取中…".into(),
                    Some(PreviewStatus::Failed(error)) => error.to_string(),
                    _ => format!(
                        "點擊右上 [Desktop]\n選擇此 Shell 使用的桌面。\n{}",
                        app.desktop_status()
                    ),
                }
            };
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }),
                layout.preview_inner,
            );
        }
    }
    if layout.tabs.width > 0 {
        let slots = (layout.tabs.width / 24).max(1) as usize;
        let start = app.tab / slots * slots;
        let slot_width = (layout.tabs.width / slots as u16).max(1);
        for (slot, (index, tab)) in app
            .tabs
            .iter()
            .enumerate()
            .skip(start)
            .take(slots)
            .enumerate()
        {
            let rect = Rect::new(
                layout.tabs.x + slot as u16 * slot_width,
                layout.tabs.y,
                slot_width,
                1,
            );
            let close_width = 3.min(rect.width);
            let title_width = rect.width.saturating_sub(close_width);
            let label = elide(&format!("{}:{}", index + 1, tab.label()), title_width);
            frame.render_widget(
                Paragraph::new(label).style(if index == app.tab {
                    Style::default().bg(Color::Cyan).fg(Color::Black)
                } else {
                    Style::default().fg(Color::Gray)
                }),
                Rect::new(rect.x, rect.y, title_width, 1),
            );
            hits.add(
                Rect::new(rect.x, rect.y, title_width, 1),
                HitAction::Tab(index),
            );
            button(
                frame,
                &mut hits,
                Rect::new(rect.right() - close_width, rect.y, close_width, 1),
                "[x]",
                HitAction::CloseTab(index),
            );
        }
    }
    if layout.terminal.width > 0 {
        let title = elide(
            &if app.expanded {
                "Desktop | 拖曳平移 · 滾輪縮放".into()
            } else {
                format!("Terminal | {}", app.desktop_status())
            },
            layout.terminal.width.saturating_sub(2),
        );
        frame.render_widget(border(title, Focus::Terminal), layout.terminal);
        hits.add(layout.terminal, HitAction::Focus(Focus::Terminal));
        if app.expanded {
            let canvas = layout.viewer_inner();
            hits.add(canvas, HitAction::ViewerCanvas);
            if let Some(image) = &app.expanded_image {
                frame.render_widget(Image::new(image), canvas);
            } else {
                frame.render_widget(Paragraph::new("準備圖片…"), canvas);
            }
            let x = layout.terminal.right().saturating_sub(4);
            button(
                frame,
                &mut hits,
                Rect::new(x, layout.terminal.y, 3, 1),
                "[X]",
                HitAction::ViewerClose,
            );
            if layout.terminal.width >= 14 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(x - 6, layout.terminal.y, 5, 1),
                    "[Fit]",
                    HitAction::ViewerFit,
                );
            }
        } else if let Some(tab) = app.tabs.get(app.tab) {
            frame.render_widget(
                PseudoTerminal::new(tab.view().screen.as_ref())
                    .cursor(Cursor::default().visibility(false)),
                layout.terminal_inner,
            );
        } else {
            frame.render_widget(Paragraph::new("左側選擇 New Shell 或現有 Shell，按 Enter 或滑鼠點選。\n桌面設定在右上 [Desktop]。"),layout.terminal_inner);
        }
    }
    if matches!(app.page, Page::Help) {
        let content = body(area);
        frame.render_widget(Clear, content);
        hits = HitMap::default();
        let mut lines = vec![
            Line::from("滑鼠點選區塊與分頁；連線樹固定展開，↑↓移動、Enter執行。"),
            Line::from("[Actions] 包含新增／編輯／刪除、匯入匯出及歷史操作。"),
            Line::from(
                "點擊左下桌面圖片切換右側預覽；拖曳平移、滾輪縮放、[Fit] 還原、[X] 或 Esc 關閉。",
            ),
            Line::from("本機與 SSH 都偵測 Sixel 與實際像素；失敗只停預覽，桌面工具仍可使用。"),
            Line::from("display.toml 的舊 compatible 值視為 auto，不再使用字格圖片。"),
            Line::from(format!(
                "快捷鍵可在 keybindings.toml 自訂；目前前綴 {}。",
                app.bindings.leader_label()
            )),
        ];
        lines.extend(COMMANDS.iter().map(|(command, label)| {
            Line::from(format!("{}  {}", app.bindings.label(*command), label))
        }));
        lines.extend([
            Line::from("桌面預覽與遠端工具狀態分開顯示；Tools ready 才代表 broker 配置已確認。"),
            Line::from(
                "Stop 只停止預覽；Clear tools 取消此 Shell 的桌面工具。關頁不刪除遠端工作。",
            ),
            Line::from(
                "只有 Terminal 焦點會送普通按鍵與貼上；Shift+滑鼠可使用終端本身的文字選取。",
            ),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel("Help / 說明"))
                .scroll((app.help_offset, 0))
                .wrap(Wrap { trim: false }),
            content,
        );
        let close = Rect::new(
            content.right().saturating_sub(10),
            content.y,
            10.min(content.width),
            1,
        );
        button(
            frame,
            &mut hits,
            close,
            "[Close]",
            HitAction::Command(if app.tabs.is_empty() {
                LocalCommand::Hosts
            } else {
                LocalCommand::Terminal
            }),
        );
    }
    let state = app
        .tabs
        .get(app.tab)
        .map(|t| {
            let v = t.view();
            format!(
                "{} | {}{}",
                status(&v.status),
                history(&v.history),
                v.notice
                    .map(|n| format!(" | {}", safe(n)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| {
            format!(
                "{} Hosts | {} Screens",
                app.profiles.hosts.len(),
                app.profiles.screens.len()
            )
        });
    frame.render_widget(
        Paragraph::new(format!(
            "{}  {}{}",
            state,
            if app.pending.is_some() {
                "Working… "
            } else {
                ""
            },
            safe(&app.status)
        ))
        .style(Style::default().fg(Color::Gray)),
        Rect::new(area.x, area.bottom() - 2, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(if app.prefix {
            format!(
                "{} 命令模式 | 點擊 Help 查看目前快捷鍵",
                app.bindings.leader_label()
            )
        } else {
            "滑鼠點選 | ↑↓ / Enter 操作連線樹 | [Actions] 全部操作 | [Desktop] 桌面設定".into()
        })
        .style(Style::default().fg(Color::Cyan)),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
    if let Some(modal) = &app.modal {
        hits = draw_modal(app, modal, frame);
    } else if let Some(position) = terminal_cursor(app, layout.terminal_inner) {
        frame.set_cursor_position(position);
    }
    hits
}

fn terminal_cursor(app: &App, area: Rect) -> Option<(u16, u16)> {
    if app.focus != Focus::Terminal
        || !matches!(app.page, Page::Terminal)
        || app.expanded
        || app.modal.is_some()
    {
        return None;
    }
    let view = app.tabs.get(app.tab)?.view();
    if !matches!(view.status, TabStatus::Live)
        || matches!(
            view.history,
            HistoryStatus::Held { .. } | HistoryStatus::Uncertain
        )
        || view.screen.hide_cursor()
        || view.screen.scrollback() > 0
    {
        return None;
    }
    screen_cursor(&view.screen, area)
}
fn screen_cursor(screen: &vt100::Screen, area: Rect) -> Option<(u16, u16)> {
    let (row, column) = screen.cursor_position();
    (!screen.hide_cursor() && screen.scrollback() == 0 && row < area.height && column < area.width)
        .then_some((area.x + column, area.y + row))
}
fn field_cursor(field: &Field, region: Rect, bordered: bool) -> Option<(u16, u16)> {
    let area = if bordered {
        panel("").inner(region)
    } else {
        region
    };
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let width = Span::raw(field.visible()).width()
        + if bordered {
            0
        } else {
            Span::raw(format!("{}: ", field.label)).width()
        };
    Some((area.x + (width as u16).min(area.width - 1), area.y))
}

fn status(value: &TabStatus) -> String {
    match value {
        TabStatus::Live => "Live".into(),
        TabStatus::Recovering => "恢復連線中".into(),
        TabStatus::InputUncertain => "輸入結果不確定，已停止送入".into(),
        TabStatus::Ended(error) => format!("已斷線：{}", safe(error)),
    }
}
fn history(value: &HistoryStatus) -> String {
    match value {
        HistoryStatus::Unsupported => "無歷史控制".into(),
        HistoryStatus::Following => "即時輸出".into(),
        HistoryStatus::Held { offset } => format!("歷史閱讀 -{offset}"),
        HistoryStatus::Uncertain => "歷史控制狀態不確定".into(),
    }
}
fn modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(88);
    let height = area.height.saturating_sub(4).min(24);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn draw_modal(app: &App, modal: &Modal, frame: &mut Frame) -> HitMap {
    let rect = modal_rect(frame.area());
    let mut hits = HitMap::default();
    frame.render_widget(Clear, rect);
    match modal {
        Modal::Host(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if matches!(form.category, art_term_core::connections::Category::Agent) {
                    if form.previous.is_some() {
                        "編輯 Agent / Edit"
                    } else {
                        "新增 Agent / New Agent"
                    }
                } else if form.previous.is_some() {
                    "編輯 Host / Edit"
                } else {
                    "新增 Host / Add"
                },
                rect,
                frame,
            )
        }
        Modal::Screen(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if form.previous.is_some() {
                    "編輯 Screen"
                } else {
                    "新增 Screen"
                },
                rect,
                frame,
            )
        }
        Modal::Transfer(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if form.export {
                    "加密匯出全部 Host / Screen"
                } else {
                    "匯入 Host / Screen（不含密碼可留白）"
                },
                rect,
                frame,
            )
        }
        Modal::Actions { row } | Modal::Screens { row } => {
            let actions = matches!(modal, Modal::Actions { .. });
            let labels = if actions {
                COMMANDS
                    .iter()
                    .map(|(command, label)| format!("{}  {}", label, app.bindings.label(*command)))
                    .collect::<Vec<_>>()
            } else {
                std::iter::once("停止預覽 / Stop".into())
                    .chain(app.profiles.screens.iter().map(|p| safe(&p.name)))
                    .chain([
                        "取消桌面工具 / Clear tools".into(),
                        "恢復預覽 / Resume preview".into(),
                        "新增 Screen / New Screen".into(),
                    ])
                    .collect::<Vec<String>>()
            };
            let block = panel(if actions {
                "Actions / 所有操作"
            } else {
                "Screen：選擇此 Shell 的桌面"
            });
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let detail_rows = if !actions && inner.height >= 5 { 2 } else { 0 };
            let height = inner.height.saturating_sub(1 + detail_rows) as usize;
            let offset = row.saturating_sub(height.saturating_sub(1));
            for (i, label) in labels.iter().enumerate().skip(offset).take(height) {
                let region = Rect::new(inner.x, inner.y + (i - offset) as u16, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(elide(label, inner.width)).style(if i == *row {
                        Style::default().bg(Color::DarkGray)
                    } else {
                        Style::default()
                    }),
                    region,
                );
                hits.add(region, HitAction::ModalRow(i));
            }
            if detail_rows > 0 {
                let notice = app
                    .tabs
                    .get(app.tab)
                    .and_then(|t| app.backend.desktop_view(t.id()).ok())
                    .and_then(|view| view.notice)
                    .unwrap_or("Stop 只停止預覽；Clear tools 才會取消工具設定。");
                frame.render_widget(
                    Paragraph::new(elide(&app.desktop_status(), inner.width)),
                    Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
                );
                frame.render_widget(
                    Paragraph::new(elide(notice, inner.width)),
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                );
            }
            if inner.height > 0 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(inner.x, inner.bottom() - 1, inner.width.min(10), 1),
                    "[Cancel]",
                    HitAction::Cancel,
                );
            }
        }
        Modal::Confirm(action) => {
            let message = match action {
                Confirm::DeleteScreen(_) => {
                    "刪除此 Screen？本機預覽將停止；不會清除遠端 Shell 的現有工具設定。"
                }
                Confirm::DeleteHost(_) => "刪除此 Host 設定？本機分頁會失效，遠端工作不會被刪除。",
                Confirm::Attach(shell) => {
                    if shell.attached {
                        "接管此 Shell？目前其他連線可能中斷。"
                    } else {
                        "連線至選定的 Shell？此操作會重新取得連線權限。"
                    }
                }
                Confirm::DeleteShell(_) => "永久終止並刪除此遠端 Shell？這與關閉本機分頁不同。",
                Confirm::ClearTools(_) => {
                    "取消此 Shell 的桌面工具？其他 Shell 與共享桌面不受影響。"
                }
            };
            let block = panel("確認 / Confirm");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                ),
            );
            if inner.height > 1 {
                frame.render_widget(
                    Paragraph::new(if app.confirm_yes {
                        "↑↓ 選擇 [Yes]；Enter 確認"
                    } else {
                        "↑↓ 選擇 [Cancel]；Enter 取消"
                    })
                    .style(Style::default().fg(Color::Yellow)),
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                );
            }
            if inner.height > 0 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(
                        inner.x,
                        inner.bottom() - 1,
                        (if inner.width >= 18 {
                            8
                        } else {
                            inner.width.saturating_sub(8)
                        })
                        .min(inner.width),
                        1,
                    ),
                    "[Yes]",
                    HitAction::Confirm,
                );
                if inner.width >= 14 {
                    button(
                        frame,
                        &mut hits,
                        Rect::new(
                            if inner.width >= 18 {
                                inner.x + 9
                            } else {
                                inner.right() - 8
                            },
                            inner.bottom() - 1,
                            8,
                            1,
                        ),
                        "[Cancel]",
                        HitAction::Cancel,
                    );
                }
            }
        }
        Modal::Import {
            preview,
            replace,
            scroll,
        } => {
            let block = panel("匯入預覽 / Import preview");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let lines = preview
                .entries
                .iter()
                .map(|entry| {
                    Line::from(format!(
                        "{} {} [{}] {}",
                        entry.kind,
                        safe(&entry.name),
                        entry.status,
                        if entry.has_secret {
                            "含加密憑證"
                        } else {
                            "不含憑證"
                        }
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                ),
            );
            if inner.height > 1 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                    if *replace {
                        "[Replace duplicates: ON]"
                    } else {
                        "[Replace duplicates: OFF]"
                    },
                    HitAction::ToggleReplace,
                );
                button(
                    frame,
                    &mut hits,
                    Rect::new(
                        inner.x,
                        inner.bottom() - 1,
                        (if inner.width >= 18 {
                            8
                        } else {
                            inner.width.saturating_sub(8)
                        })
                        .min(inner.width),
                        1,
                    ),
                    if inner.width >= 15 { "[Apply]" } else { "[OK]" },
                    HitAction::Confirm,
                );
                if inner.width >= 14 {
                    button(
                        frame,
                        &mut hits,
                        Rect::new(
                            if inner.width >= 18 {
                                inner.x + 9
                            } else {
                                inner.right() - 8
                            },
                            inner.bottom() - 1,
                            8,
                            1,
                        ),
                        "[Cancel]",
                        HitAction::Cancel,
                    );
                }
            }
        }
    }
    hits
}
fn draw_fields(
    fields: &[Field],
    active: usize,
    title: &'static str,
    rect: Rect,
    frame: &mut Frame,
) -> HitMap {
    let mut hits = HitMap::default();
    let block = panel(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let field_height = if inner.height >= fields.len() as u16 * 3 + 1 {
        3
    } else {
        1
    };
    for (index, field) in fields.iter().enumerate() {
        let y = inner.y + index as u16 * field_height;
        if y + field_height > inner.bottom().saturating_sub(1) {
            break;
        }
        let region = Rect::new(inner.x, y, inner.width, field_height);
        let style = Style::default().fg(if index == active {
            Color::Cyan
        } else {
            Color::DarkGray
        });
        if field_height == 3 {
            frame.render_widget(
                Paragraph::new(field.visible()).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(field.label)
                        .border_style(style),
                ),
                region,
            );
        } else {
            frame.render_widget(
                Paragraph::new(elide(
                    &format!("{}: {}", field.label, field.visible()),
                    inner.width,
                ))
                .style(style),
                region,
            );
        }
        hits.add(region, HitAction::Field(index));
        if index == active {
            if let Some(position) = field_cursor(field, region, field_height == 3) {
                frame.set_cursor_position(position);
            }
        }
    }
    if inner.height > 0 {
        button(
            frame,
            &mut hits,
            Rect::new(
                inner.x,
                inner.bottom() - 1,
                (if inner.width >= 18 {
                    8
                } else {
                    inner.width.saturating_sub(8)
                })
                .min(inner.width),
                1,
            ),
            "[Save]",
            HitAction::Submit,
        );
        if inner.width >= 14 {
            button(
                frame,
                &mut hits,
                Rect::new(
                    if inner.width >= 18 {
                        inner.x + 9
                    } else {
                        inner.right() - 8
                    },
                    inner.bottom() - 1,
                    8,
                    1,
                ),
                "[Cancel]",
                HitAction::Cancel,
            );
        }
    }
    hits
}

pub(crate) async fn run(
    backend: Backend,
    startup_signals: crate::tty::StartupSignals,
    bindings: Bindings,
    preferences: Result<DisplayPreferences, String>,
) -> anyhow::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    startup_signals.disarm();
    let mut tty = Tty::enter_with_native_images(false)?;
    let mut events = EventStream::new();
    let mut probe = TerminalProbe::new(preferences, Instant::now());
    let mut tick = tokio::time::interval(Duration::from_millis(40));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut app = App::with_display(backend, bindings, probe.initial_selection());
    if let Some(selection) = probe.write_queries(tty.terminal.backend_mut()) {
        app.set_display(selection).await;
    }
    app.profiles();
    let result = async {
        while !app.quit {
            if let Some(selection) = probe.expire(Instant::now()) {
                let native = selection.pixels().is_some();
                app.set_display(selection).await;
                tty.set_native_images(native);
            }
            app.complete().await;
            tty.terminal.autoresize()?;
            app.resize(tty.terminal.get_frame().area());
            if std::mem::take(&mut app.image_damage) {
                // Fullscreen resize resets both buffers without clear()'s stdin cursor query.
                let area = tty.terminal.get_frame().area();
                tty.terminal.resize(area)?;
            }
            tty.terminal.draw(|frame| {
                app.hits = draw(&app, frame);
            })?;
            tokio::select! {
                event = events.next() => match event {
                    Some(Ok(Event::Key(key))) => app.key(key),
                    Some(Ok(Event::Paste(text))) => app.paste(text),
                    Some(Ok(Event::Mouse(event))) => app.mouse(event),
                    Some(Ok(Event::Resize(_, _))) => app.hits=HitMap::default(),
                    Some(Ok(Event::TerminalResponse(reply))) => {
                        if let Some(selection) = probe.observe(reply, Instant::now()) {
                            let native = selection.pixels().is_some();
                            app.set_display(selection).await;
                            tty.set_native_images(native);
                        }
                    },
                    Some(Err(error)) => return Err(anyhow::Error::from(error)),
                    None => break,
                    _ => (),
                },
                _ = tick.tick() => (),
                _ = terminate.recv() => break,
                _ = interrupt.recv() => break,
            }
        }
        Ok(())
    }
    .await;
    drop(tty);
    app.stop().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn joined_local_detach_refreshes_current_host_only_after_work_has_closed() {
        let directory = tempfile::Builder::new()
            .prefix("art-term-tree-detach-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
        let mut app = App::new(backend.clone());
        app.profiles.hosts.push(HostProfile {
            id: "host".into(),
            revision: 2,
            name: "Host".into(),
            base_url: "https://fixture.example".into(),
            credential_present: false,
            metadata: None,
        });
        app.tree_load = Some(TreeLoad {
            host: HostSelection {
                id: "host".into(),
                revision: 2,
            },
            task: tokio::spawn(std::future::pending()),
        });
        let (release, done) = tokio::sync::oneshot::channel();
        app.closing_tabs.push(ClosingTab {
            host: HostSelection {
                id: "host".into(),
                revision: 1,
            },
            task: tokio::spawn(async move {
                let _ = done.await;
            }),
        });
        app.complete().await;
        assert!(app.load_queue.is_empty());
        release.send(()).unwrap();
        while !app.closing_tabs[0].task.is_finished() {
            tokio::task::yield_now().await;
        }
        app.complete().await;
        assert_eq!(app.load_queue.len(), 1);
        assert_eq!(app.load_queue[0].revision, 2);
        app.stop().await;
        backend.shutdown().await;
    }

    #[tokio::test]
    async fn host_edit_queues_latest_revision_behind_stale_inflight_tree_read() {
        let directory = tempfile::Builder::new()
            .prefix("art-term-tree-revision-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
        let mut app = App::new(backend.clone());
        app.tree_load = Some(TreeLoad {
            host: HostSelection {
                id: "host".into(),
                revision: 1,
            },
            task: tokio::spawn(std::future::pending()),
        });
        app.queue_host(HostSelection {
            id: "host".into(),
            revision: 2,
        });
        app.queue_host(HostSelection {
            id: "host".into(),
            revision: 3,
        });
        assert_eq!(app.load_queue.len(), 1);
        assert_eq!(app.load_queue[0].revision, 3);
        app.stop().await;
        backend.shutdown().await;
    }

    #[tokio::test]
    async fn every_narrow_form_and_confirmation_has_a_visible_mouse_cancel() {
        let directory = tempfile::Builder::new()
            .prefix("art-term-modal-cancel-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
        let mut app = App::new(backend.clone());
        app.resize(Rect::new(0, 0, 20, 14));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 14)).unwrap();
        for modal in [
            Modal::Host(HostForm::new(None)),
            Modal::Screen(ScreenForm::new(None)),
            Modal::Transfer(TransferForm::new(true)),
            Modal::Confirm(Confirm::DeleteHost(HostSelection {
                id: "host".into(),
                revision: 1,
            })),
            Modal::Import {
                preview: ImportView {
                    ticket: 1,
                    entries: Vec::new(),
                },
                replace: false,
                scroll: 0,
            },
        ] {
            app.show_modal(modal);
            terminal.draw(|frame| app.hits = draw(&app, frame)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("[Cancel]"), "{text}");
            assert!(app
                .hits
                .regions
                .iter()
                .any(|(_, action)| matches!(action, HitAction::Cancel)));
        }
        app.stop().await;
        backend.shutdown().await;
    }

    #[test]
    fn secret_fields_mask_unicode_and_clear_without_returning_plaintext() {
        let mut field = Field::new("secret", "秘密-value".into(), true);
        assert_eq!(field.visible(), "********");
        field.input(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(field.value.is_empty());
        field.paste("token\n\u{1b}value");
        assert_eq!(&*field.value, "tokenvalue");
    }
    #[test]
    fn chrome_is_not_part_of_remote_dimensions_and_tiny_windows_suspend_resize() {
        assert_eq!(
            terminal_size(Rect::new(0, 0, 100, 30)),
            Some(TerminalSize::new(24, 65))
        );
        assert_eq!(terminal_size(Rect::new(0, 0, 80, 2)), None);
        assert_eq!(terminal_size(Rect::new(0, 0, 0, 10)), None);
    }
    #[test]
    fn visible_labels_never_forward_terminal_control_sequences() {
        assert_eq!(safe("host\u{1b}]52;data\u{7}"), "host�]52;data�");
    }

    #[tokio::test]
    async fn three_panes_render_together_and_modal_input_cannot_navigate_or_quit() {
        let directory = tempfile::Builder::new()
            .prefix("art-term-ui-workspace-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
        let mut app = App::new(backend.clone());
        app.resize(Rect::new(0, 0, 100, 30));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                app.hits = draw(&app, frame);
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Connections"));
        assert!(text.contains("Desktop"));
        assert!(text.contains("Terminal"));
        app.show_modal(Modal::Host(HostForm::new(None)));
        app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.quit);
        assert!(!app.prefix);
        assert!(matches!(app.modal, Some(Modal::Host(_))));
        app.stop().await;
        backend.shutdown().await;
    }

    #[tokio::test]
    async fn slow_import_preview_does_not_replace_a_newer_host_draft() {
        let directory = tempfile::Builder::new()
            .prefix("art-term-ui-draft-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let vault = crate::vault::Vault::open_account(paths).unwrap();
        let backend = Backend::start(vault).unwrap();
        let mut app = App::new(backend.clone());
        app.job(async {
            JobResult::Preview(Ok(ImportView {
                ticket: 999,
                entries: Vec::new(),
            }))
        });
        app.show_modal(Modal::Host(HostForm::new(None)));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !app.pending.as_ref().unwrap().task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        app.complete().await;
        assert!(app.pending.is_none());
        assert!(matches!(app.modal, Some(Modal::Host(_))));
        app.stop().await;
        backend.shutdown().await;
    }
    async fn test_app() -> (tempfile::TempDir, Backend, App) {
        let directory = tempfile::Builder::new()
            .prefix("art-term-ui-mouse-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths =
            crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml")))
                .unwrap();
        let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
        let app = App::new(backend.clone());
        (directory, backend, app)
    }
    fn draw_test(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        app.resize(Rect::new(0, 0, width, height));
        terminal
            .draw(|frame| {
                app.hits = draw(app, frame);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    fn click(app: &mut App, x: u16, y: u16) {
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    }
    #[test]
    fn hit_regions_ignore_empty_cells_and_choose_topmost_visible_control() {
        let mut hits = HitMap::default();
        hits.add(Rect::new(0, 0, 0, 5), HitAction::Confirm);
        hits.add(Rect::new(2, 2, 5, 3), HitAction::Focus(Focus::Terminal));
        hits.add(Rect::new(4, 3, 2, 1), HitAction::Cancel);
        assert!(hits.at(1, 2).is_none());
        assert!(hits.at(7, 2).is_none());
        assert!(matches!(
            hits.at(3, 3),
            Some(HitAction::Focus(Focus::Terminal))
        ));
        assert!(matches!(hits.at(4, 3), Some(HitAction::Cancel)));
    }
    #[tokio::test]
    async fn tree_is_permanently_expanded_and_selection_survives_inserted_children() {
        let (_directory, backend, mut app) = test_app().await;
        app.profiles.screens.push(DesktopProfile {
            id: "screen-a".into(),
            revision: 1,
            name: "Screen A".into(),
            mcp_url: "http://127.0.0.1:9/mcp".into(),
            token_present: true,
        });
        app.tree_selected = Some("screen:screen-a".into());
        let before = app
            .rows()
            .iter()
            .position(|r| r.key == "screen:screen-a")
            .unwrap();
        app.profiles.hosts.push(HostProfile {
            id: "host-a".into(),
            revision: 1,
            name: "Host A".into(),
            base_url: "https://fixture.invalid".into(),
            credential_present: true,
            metadata: None,
        });
        let rows = app.rows();
        assert!(
            rows.iter()
                .position(|r| r.key == "screen:screen-a")
                .unwrap()
                > before
        );
        assert_eq!(app.tree_selected.as_deref(), Some("screen:screen-a"));
        for header in ["Computer", "Agent", "Screen"] {
            assert!(rows.iter().any(|r| r.label == header && r.action.is_none()));
        }
        assert!(rows.iter().any(|r| r.key == "new-shell:host-a"));
        assert!(rows.iter().any(|r| r.key == "refresh:host-a"));
        let mut keys = HashSet::new();
        assert!(rows.iter().all(|row| keys.insert(row.key.clone())));
        app.tree_selected = Some("new-host".into());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(app.modal, Some(Modal::Host(_))));
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn mouse_focus_and_form_controls_are_local_and_modal_exclusive() {
        let (_directory, backend, mut app) = test_app().await;
        draw_test(&mut app, 100, 30);
        click(&mut app, 80, 10);
        assert_eq!(app.focus, Focus::Terminal);
        draw_test(&mut app, 100, 30);
        click(&mut app, 5, 24);
        assert_eq!(app.focus, Focus::Preview);
        app.show_modal(Modal::Host(HostForm::new(None)));
        draw_test(&mut app, 100, 30);
        click(&mut app, 1, 0);
        assert!(matches!(app.modal, Some(Modal::Host(_))));
        assert!(!app.quit);
        let field = app
            .hits
            .regions
            .iter()
            .find_map(|(rect, action)| matches!(action, HitAction::Field(2)).then_some(*rect))
            .unwrap();
        click(&mut app, field.x, field.y);
        app.paste("private-fixture-token".into());
        let Some(Modal::Host(form)) = &app.modal else {
            panic!("form lost")
        };
        assert_eq!(form.active, 2);
        assert_eq!(&*form.fields[2].value, "private-fixture-token");
        assert!(!draw_test(&mut app, 100, 30).contains("private-fixture-token"));
        let cancel = app
            .hits
            .regions
            .iter()
            .find_map(|(rect, action)| matches!(action, HitAction::Cancel).then_some(*rect))
            .unwrap();
        click(&mut app, cancel.x, cancel.y);
        assert!(app.modal.is_none());
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn mouse_action_menu_reaches_every_command_and_narrow_desktop_remains_visible() {
        let (_directory, backend, mut app) = test_app().await;
        let tiny = draw_test(&mut app, 20, 14);
        assert!(tiny.contains("[Actions]"));
        assert!(tiny.contains("[Desktop]"));
        let text = draw_test(&mut app, 60, 14);
        assert!(text.contains("[Desktop]"));
        assert!(text.contains("[Actions]"));
        for row in 0..COMMANDS.len() {
            app.show_modal(Modal::Actions { row });
            draw_test(&mut app, 60, 14);
            assert!(
                app.hits
                    .regions
                    .iter()
                    .any(|(_, a)| matches!(a,HitAction::ModalRow(i) if *i==row)),
                "action {} is unreachable",
                COMMANDS[row].1
            );
            assert!(app
                .hits
                .regions
                .iter()
                .all(|(r, _)| r.right() <= 60 && r.bottom() <= 14));
        }
        app.show_modal(Modal::Actions {
            row: COMMANDS.len() - 1,
        });
        draw_test(&mut app, 60, 14);
        let quit = app
            .hits
            .regions
            .iter()
            .find_map(|(r, a)| {
                matches!(a,HitAction::ModalRow(i) if *i==COMMANDS.len()-1).then_some(*r)
            })
            .unwrap();
        click(&mut app, quit.x, quit.y);
        assert!(app.quit);
        app.stop().await;
        backend.shutdown().await;
    }
    #[test]
    fn unicode_tab_labels_are_clipped_in_terminal_cells() {
        assert_eq!(elide("中文abc", 4), "中…");
        assert_eq!(elide("abc", 3), "abc");
        assert_eq!(elide("中文", 0), "");
    }
    fn seed_frame(app: &mut App) -> Arc<CapturedFrame> {
        app.layout = WorkspaceLayout::new(Rect::new(0, 0, 100, 30), app.focus);
        let target = PreviewTarget {
            tab: TabId(1),
            generation: app.generation,
            screen: ScreenSelection {
                id: "fixture".into(),
                revision: 1,
            },
        };
        let frame = Arc::new(CapturedFrame {
            target: target.clone(),
            sequence: 1,
            pixels: image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                800,
                400,
                image::Rgb([20, 120, 240]),
            )),
        });
        app.preview_target = Some(target);
        app.source_frame = Some(frame.clone());
        app.preview_status = Some(PreviewStatus::Live);
        app.preview_image = Some(Protocol::Sixel(
            ratatui_image::protocol::sixel::Sixel::new(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    31,
                    14,
                    image::Rgb([20, 120, 240]),
                )),
                app.layout.preview_inner.as_size(),
                false,
            )
            .unwrap(),
        ));
        frame
    }
    fn draw_without_update(app: &mut App) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.hits = draw(app, frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    fn mouse_event(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
        app.mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    }
    #[tokio::test]
    async fn unavailable_native_preview_has_no_workers_and_keeps_screen_controls_accessible() {
        let (_directory, backend, mut previous) = test_app().await;
        previous.stop().await;
        let notice = "原生桌面預覽未啟用；桌面工具仍可使用。終端未宣告支援 Sixel。";
        let mut app = App::with_display(
            backend.clone(),
            Bindings::default(),
            DisplaySelection::Unavailable(notice.into()),
        );
        app.resize(Rect::new(0, 0, 100, 30));
        assert!(app.preview.is_none());
        assert!(app.renderer.is_none());
        assert!(app.preview_target.is_none());
        app.toggle_viewer();
        assert!(!app.expanded);
        assert_eq!(app.status, notice);
        let text: String = draw_without_update(&mut app).split_whitespace().collect();
        assert!(text.contains("桌面工具仍可使用"));
        assert!(text.contains("終端未宣告支援Sixel"));
        app.local(LocalCommand::Screens);
        assert!(matches!(app.modal, Some(Modal::Screens { .. })));
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn completing_native_probe_preserves_screen_selection_ui_and_existing_workers() {
        let (_directory, backend, mut previous) = test_app().await;
        previous.stop().await;
        let mut app = App::with_display(
            backend.clone(),
            Bindings::default(),
            DisplaySelection::Unavailable("正在偵測原生圖片能力".into()),
        );
        app.local(LocalCommand::Screens);
        assert!(matches!(app.modal, Some(Modal::Screens { .. })));
        app.set_display(DisplaySelection::Available(
            CellPixels::new(10, 20).unwrap(),
        ))
        .await;
        assert!(app.preview.is_some());
        assert!(app.renderer.is_some());
        assert!(matches!(app.modal, Some(Modal::Screens { .. })));
        app.modal = None;
        let original = seed_frame(&mut app);
        let generation = app.generation;
        app.set_display(DisplaySelection::Available(
            CellPixels::new(10, 20).unwrap(),
        ))
        .await;
        assert_eq!(app.generation, generation);
        assert!(Arc::ptr_eq(app.source_frame.as_ref().unwrap(), &original));
        app.set_display(DisplaySelection::Unavailable(
            "偵測失敗；桌面工具仍可使用".into(),
        ))
        .await;
        assert!(app.preview.is_none());
        assert!(app.renderer.is_none());
        assert!(app.source_frame.is_none());
        app.local(LocalCommand::Screens);
        assert!(matches!(app.modal, Some(Modal::Screens { .. })));
        app.stop().await;
        backend.shutdown().await;
    }
    fn prepared_frame(app: &App, sequence: u64, color: [u8; 3]) -> PreparedSurfaces {
        let plan = app.render_request.as_ref().unwrap().1.clone();
        let thumbnail = Protocol::Sixel(
            ratatui_image::protocol::sixel::Sixel::new(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    6,
                    6,
                    image::Rgb(color),
                )),
                plan.thumbnail.as_size(),
                false,
            )
            .unwrap(),
        );
        PreparedSurfaces {
            target: app.preview_target.clone().unwrap(),
            sequence,
            plan,
            thumbnail: Some(thumbnail),
            expanded: None,
            error: None,
        }
    }
    fn thumbnail_bytes(app: &App) -> &str {
        let Some(Protocol::Sixel(image)) = &app.preview_image else {
            panic!("native thumbnail missing");
        };
        &image.data
    }
    #[tokio::test]
    async fn completed_frames_stay_monotonic_while_capture_and_presentation_advance() {
        let (_directory, backend, mut app) = test_app().await;
        seed_frame(&mut app);
        let plan = app.surface_plan();
        app.render_request = Some((10, plan.clone()));
        app.apply_surfaces(prepared_frame(&app, 8, [255, 0, 0]));
        assert_eq!(app.last_rendered_sequence, Some(8));
        let latest = thumbnail_bytes(&app).to_owned();
        app.apply_surfaces(prepared_frame(&app, 7, [0, 255, 0]));
        assert_eq!(thumbnail_bytes(&app), latest);
        assert_eq!(app.last_rendered_sequence, Some(8));

        let mut invalid = prepared_frame(&app, 9, [0, 255, 0]);
        invalid.target.generation += 1;
        app.apply_surfaces(invalid);
        let mut invalid = prepared_frame(&app, 9, [0, 255, 0]);
        invalid.plan.thumbnail.x += 1;
        app.apply_surfaces(invalid);
        app.apply_surfaces(prepared_frame(&app, 11, [0, 255, 0]));
        assert_eq!(thumbnail_bytes(&app), latest);
        assert_eq!(app.last_rendered_sequence, Some(8));

        let mut changed = plan;
        changed.thumbnail.x += 1;
        app.render_request = Some((10, changed));
        app.apply_surfaces(prepared_frame(&app, 7, [0, 255, 0]));
        assert_eq!(thumbnail_bytes(&app), latest);
        app.apply_surfaces(prepared_frame(&app, 8, [0, 255, 0]));
        assert_ne!(thumbnail_bytes(&app), latest);
        assert_eq!(app.last_rendered_sequence, Some(8));
        app.apply_surfaces(prepared_frame(&app, 9, [0, 0, 255]));
        assert_eq!(app.last_rendered_sequence, Some(9));

        let mut old_error = prepared_frame(&app, 8, [0, 0, 0]);
        old_error.error = Some(crate::preview::PreviewError::InvalidImage);
        app.apply_surfaces(old_error);
        assert!(app.source_frame.is_some());
        assert!(app.preview_image.is_some());
        let mut current_error = prepared_frame(&app, 9, [0, 0, 0]);
        current_error.error = Some(crate::preview::PreviewError::InvalidImage);
        app.apply_surfaces(current_error);
        assert!(app.source_frame.is_none());
        assert!(app.preview_image.is_none());
        assert_eq!(app.last_rendered_sequence, Some(9));
        app.invalidate_preview();
        assert_eq!(app.last_rendered_sequence, None);
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn cached_thumbnail_survives_focus_and_viewer_gestures_without_instruction_flash() {
        let (_directory, backend, mut app) = test_app().await;
        let source = seed_frame(&mut app);
        draw_without_update(&mut app);
        let generation = app.generation;
        app.hit(HitAction::Focus(Focus::Terminal));
        assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
        assert!(app.preview_image.is_some());
        assert_eq!(app.generation, generation);
        let text = draw_without_update(&mut app);
        assert!(!text.contains("選擇此 Shell 使用的桌面"));
        click(&mut app, 4, 23);
        assert!(app.expanded);
        assert_eq!(app.focus, Focus::Terminal);
        let text = draw_without_update(&mut app);
        assert!(text.contains("[X]"));
        assert!(text.contains("[Fit]"));
        assert!(!text.contains("選擇此 Shell 使用的桌面"));
        let canvas = app.layout.viewer_inner();
        let point = (canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
        let before = app.viewport.clone();
        mouse_event(&mut app, MouseEventKind::ScrollUp, point.0, point.1);
        assert_ne!(app.viewport, before);
        let zoomed = app.viewport.clone();
        click(&mut app, point.0, point.1);
        assert_eq!(app.drag, Some(point));
        mouse_event(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            point.0 + 3,
            point.1 + 2,
        );
        assert_ne!(app.viewport, zoomed);
        mouse_event(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            point.0 + 3,
            point.1 + 2,
        );
        assert!(app.drag.is_none());
        assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
        assert_eq!(app.generation, generation);
        app.hit(HitAction::ViewerFit);
        assert_eq!(app.viewport, ImageViewport::default());
        let close = app
            .hits
            .regions
            .iter()
            .find_map(|(r, a)| matches!(a, HitAction::ViewerClose).then_some(*r))
            .unwrap();
        click(&mut app, close.x, close.y);
        assert!(!app.expanded);
        assert!(app.preview_image.is_some());
        click(&mut app, 4, 23);
        assert!(app.expanded);
        click(&mut app, 4, 23);
        assert!(!app.expanded);
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn viewer_closes_on_modal_help_source_stop_and_narrow_layout_without_resizing_remote() {
        let (_directory, backend, mut app) = test_app().await;
        let source = seed_frame(&mut app);
        let size = app.layout.terminal_size();
        app.toggle_viewer();
        assert!(app.expanded);
        assert_eq!(app.layout.terminal_size(), size);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.expanded);
        app.toggle_viewer();
        app.show_modal(Modal::Host(HostForm::new(None)));
        assert!(!app.expanded);
        assert!(app.drag.is_none());
        assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
        app.dismiss_modal();
        app.toggle_viewer();
        app.navigate(Page::Help);
        assert!(!app.expanded);
        assert!(app.source_frame.is_some());
        app.navigate(Page::Terminal);
        app.toggle_viewer();
        app.invalidate_preview();
        assert!(!app.expanded);
        assert!(app.source_frame.is_none());
        assert!(app.preview_image.is_none());
        seed_frame(&mut app);
        app.layout = WorkspaceLayout::new(Rect::new(0, 0, 50, 12), Focus::Preview);
        app.toggle_viewer();
        assert!(!app.expanded);
        assert!(app.status.contains("放大"));
        app.stop().await;
        backend.shutdown().await;
    }
    #[tokio::test]
    async fn native_damage_tracks_visibility_geometry_and_source_without_clearing_focus() {
        let (_directory, backend, mut app) = test_app().await;
        seed_frame(&mut app);
        app.display = DisplaySelection::Available(CellPixels::new(8, 16).unwrap());
        app.image_damage = false;
        app.hit(HitAction::Focus(Focus::Terminal));
        assert!(!app.image_damage);
        app.toggle_viewer();
        assert!(std::mem::take(&mut app.image_damage));
        app.close_viewer();
        assert!(std::mem::take(&mut app.image_damage));
        app.show_modal(Modal::Host(HostForm::new(None)));
        assert!(std::mem::take(&mut app.image_damage));
        app.invalidate_preview();
        assert!(app.image_damage);
        app.stop().await;
        backend.shutdown().await;
    }
    #[test]
    fn native_cursor_uses_remote_cells_and_respects_hide_history_and_bounds() {
        let area = Rect::new(41, 3, 50, 20);
        let mut parser = vt100::Parser::new(20, 50, 100);
        parser.process(b"abc");
        assert_eq!(screen_cursor(parser.screen(), area), Some((44, 3)));
        parser.process("中文".as_bytes());
        assert_eq!(screen_cursor(parser.screen(), area), Some((48, 3)));
        parser.process(b"\x1b[5;7H");
        assert_eq!(screen_cursor(parser.screen(), area), Some((47, 7)));
        assert_eq!(screen_cursor(parser.screen(), Rect::new(0, 0, 6, 4)), None);
        parser.process(b"\x1b[?25l");
        assert_eq!(screen_cursor(parser.screen(), area), None);
        parser.process(b"\x1b[?25h");
        for _ in 0..30 {
            parser.process(b"line\r\n");
        }
        parser.screen_mut().set_scrollback(3);
        assert_eq!(screen_cursor(parser.screen(), area), None);
    }
    #[test]
    fn form_caret_uses_visible_unicode_or_mask_cells_and_stays_inside_field() {
        let field = Field::new("名稱", "ab中文".into(), false);
        assert_eq!(
            field_cursor(&field, Rect::new(5, 8, 20, 3), true),
            Some((12, 9))
        );
        let masked = Field::new("密碼", "ab中文".into(), true);
        assert_eq!(
            field_cursor(&masked, Rect::new(5, 8, 20, 3), true),
            Some((10, 9))
        );
        assert_eq!(
            field_cursor(&field, Rect::new(5, 8, 10, 1), false),
            Some((14, 8))
        );
        assert_eq!(field_cursor(&field, Rect::new(5, 8, 2, 3), true), None);
        assert_eq!(field_cursor(&field, Rect::new(5, 8, 0, 1), false), None);
    }
}
