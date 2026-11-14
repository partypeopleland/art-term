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
    (LocalCommand::Settings, "連線設定 / Settings"),
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
#[derive(Clone, Debug, PartialEq)]
enum SettingsTarget {
    Host(HostSelection),
    Screen(ScreenSelection),
}
#[derive(Clone, Copy, PartialEq)]
enum SettingsAction {
    NewHost,
    NewAgent,
    NewScreen,
    Edit,
    Test,
    Delete,
    Import,
    Export,
    Back,
}
const SETTINGS_ACTIONS: &[(SettingsAction, &str)] = &[
    (SettingsAction::NewHost, "New Host / 新增 Host"),
    (SettingsAction::NewAgent, "New Agent / 新增 Agent"),
    (SettingsAction::NewScreen, "New Screen / 新增 Screen"),
    (SettingsAction::Edit, "Edit target / 編輯所選連線"),
    (SettingsAction::Test, "Test target / 測試所選連線"),
    (SettingsAction::Delete, "Delete target / 刪除所選連線"),
    (SettingsAction::Import, "Import / 匯入連線"),
    (SettingsAction::Export, "Export / 匯出全部連線"),
    (SettingsAction::Back, "Back / 返回"),
];
#[derive(Clone)]
enum SettingsRow {
    Target(SettingsTarget),
    Action(SettingsAction),
}
struct SettingsModal {
    row: usize,
    target: Option<SettingsTarget>,
}
enum ModalOrigin {
    Settings {
        target: Option<SettingsTarget>,
        row: usize,
    },
}
enum Modal {
    Actions {
        row: usize,
    },
    Settings(SettingsModal),
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
    modal_origin: Option<ModalOrigin>,
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

#[path = "ui/app_events.rs"]
mod app_events;
#[path = "ui/app_input.rs"]
mod app_input;
#[path = "ui/app_preview.rs"]
mod app_preview;
#[path = "ui/app_settings.rs"]
mod app_settings;
#[path = "ui/app_state.rs"]
mod app_state;
#[path = "ui/app_tree.rs"]
mod app_tree;

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
#[path = "ui/render.rs"]
mod render;
use render::*;

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
#[path = "ui_tests.rs"]
mod tests;
