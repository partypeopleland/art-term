use crate::{
    backend::{
        Backend, HostEdit, HostSelection, ImportView, OpenShell, ProfileView, ShellListView,
        ShellView,
    },
    tab::{HistoryStatus, TabAction, TabClient, TabStatus, TerminalSize},
    tty::Tty,
};
use art_term_core::openab::profiles::HostProfile;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use std::{future::Future, path::PathBuf, time::Duration};
use tokio::task::JoinHandle;
use tui_term::widget::PseudoTerminal;
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalCommand {
    Hosts,
    Shells,
    Create,
    Previous,
    Next,
    Close,
    Up,
    Down,
    Follow,
    Recover,
    Import,
    Export,
    Help,
    Quit,
}

const BINDINGS: &[(char, LocalCommand, &str)] = &[
    ('h', LocalCommand::Hosts, "主機清單 / Hosts"),
    ('s', LocalCommand::Shells, "遠端 Shell 清單"),
    ('n', LocalCommand::Create, "建立並連線新 Shell"),
    ('p', LocalCommand::Previous, "上一個分頁"),
    ('l', LocalCommand::Next, "下一個分頁"),
    ('w', LocalCommand::Close, "關閉本機分頁（不刪遠端工作）"),
    ('u', LocalCommand::Up, "向上閱讀歷史"),
    ('d', LocalCommand::Down, "向下閱讀歷史"),
    ('f', LocalCommand::Follow, "回到即時輸出"),
    ('r', LocalCommand::Recover, "恢復原連線"),
    ('i', LocalCommand::Import, "匯入 Host / Screen"),
    ('e', LocalCommand::Export, "加密匯出全部 Host / Screen"),
    ('?', LocalCommand::Help, "操作說明 / Help"),
    ('q', LocalCommand::Quit, "離開（保留遠端工作）"),
];

enum Page {
    Hosts,
    Shells(ShellListView),
    Terminal,
    Help,
}
enum Confirm {
    DeleteHost(HostSelection),
    Attach(ShellView),
    DeleteShell(ShellView),
}
enum Modal {
    Host(HostForm),
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
    previous: Option<HostSelection>,
    fields: [Field; 3],
    active: usize,
}
impl HostForm {
    fn new(host: Option<&HostProfile>) -> Self {
        Self {
            previous: host.map(HostSelection::from),
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
    fn edit(mut self) -> HostEdit {
        HostEdit {
            previous: self.previous,
            name: std::mem::take(&mut *self.fields[0].value),
            base_url: std::mem::take(&mut *self.fields[1].value),
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
    Deleted(crate::backend::Result<()>),
    Tested(crate::backend::Result<usize>),
    Shells(crate::backend::Result<ShellListView>),
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
struct App {
    backend: Backend,
    profiles: ProfileView,
    page: Page,
    modal: Option<Modal>,
    host_row: usize,
    shell_row: usize,
    help_offset: u16,
    tabs: Vec<TabClient>,
    tab: usize,
    prefix: bool,
    status: String,
    pending: Option<Pending>,
    closing: Vec<JoinHandle<()>>,
    epoch: u64,
    size: TerminalSize,
    quit: bool,
}

impl App {
    fn new(backend: Backend) -> Self {
        Self {
            backend,
            profiles: ProfileView {
                hosts: Vec::new(),
                screens: Vec::new(),
            },
            page: Page::Hosts,
            modal: None,
            host_row: 0,
            shell_row: 0,
            help_offset: 0,
            tabs: Vec::new(),
            tab: 0,
            prefix: false,
            status: "載入主機清單…".into(),
            pending: None,
            closing: Vec::new(),
            epoch: 0,
            size: TerminalSize::new(24, 80),
            quit: false,
        }
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
    fn navigate(&mut self, page: Page) {
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
        self.dismiss_modal();
        self.modal = Some(modal);
        self.epoch = self.epoch.wrapping_add(1);
    }
    fn host(&self) -> Option<HostSelection> {
        match &self.page {
            Page::Shells(list) => Some(list.host.clone()),
            Page::Terminal => self
                .tabs
                .get(self.tab)
                .and_then(|tab| self.profiles.hosts.iter().find(|h| h.id == tab.host_id()))
                .map(HostSelection::from),
            _ => self
                .profiles
                .hosts
                .get(self.host_row)
                .map(HostSelection::from),
        }
    }
    fn shells(&mut self) {
        if let Some(host) = self.host() {
            let backend = self.backend.clone();
            self.job(async move { JobResult::Shells(backend.list_shells(host).await) });
        } else {
            self.status = "請先選擇或新增主機。".into();
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
            self.tab = index;
            self.navigate(Page::Terminal);
        }
    }
    fn local(&mut self, command: LocalCommand) {
        match command {
            LocalCommand::Hosts => {
                self.navigate(Page::Hosts);
                self.profiles();
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
                    let tab = self.tabs.remove(self.tab);
                    self.closing.push(tokio::spawn(async move {
                        tab.close().await;
                    }));
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
        if self.prefix {
            self.prefix = false;
            if prefix(key) {
                self.send(TabAction::Key(KeyEvent::new(
                    KeyCode::Char(']'),
                    KeyModifiers::CONTROL,
                )));
                return;
            }
            if let KeyCode::Char(c @ '1'..='9') = key.code {
                self.select_tab((c as u8 - b'1') as usize);
                return;
            }
            if let KeyCode::Char(c) = key.code {
                if let Some((_, command, _)) = BINDINGS.iter().find(|(binding, _, _)| *binding == c)
                {
                    self.local(*command);
                } else {
                    self.status = "未知快捷鍵，Ctrl-] ? 查看說明。".into();
                }
            }
            return;
        }
        if prefix(key) {
            self.prefix = true;
            return;
        }
        if self.modal.is_some() {
            self.modal_key(key);
            return;
        }
        match &self.page {
            Page::Terminal => self.send(TabAction::Key(key)),
            Page::Help => {
                if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
                    self.help_offset = self.help_offset.saturating_add(1).min(40);
                } else if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    self.help_offset = self.help_offset.saturating_sub(1);
                } else if key.code == KeyCode::PageDown {
                    self.help_offset = self.help_offset.saturating_add(10).min(40);
                } else if key.code == KeyCode::PageUp {
                    self.help_offset = self.help_offset.saturating_sub(10);
                } else if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    if self.tabs.is_empty() {
                        self.navigate(Page::Hosts);
                    } else {
                        self.navigate(Page::Terminal);
                    }
                }
            }
            Page::Hosts => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.host_row =
                        (self.host_row + 1).min(self.profiles.hosts.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => self.host_row = self.host_row.saturating_sub(1),
                KeyCode::Char('n') => self.show_modal(Modal::Host(HostForm::new(None))),
                KeyCode::Char('e') => {
                    if let Some(host) = self.profiles.hosts.get(self.host_row) {
                        self.show_modal(Modal::Host(HostForm::new(Some(host))));
                    }
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    if let Some(host) = self.host() {
                        self.show_modal(Modal::Confirm(Confirm::DeleteHost(host)));
                    }
                }
                KeyCode::Char('t') => {
                    if let Some(host) = self.host() {
                        let backend = self.backend.clone();
                        self.job(async move { JobResult::Tested(backend.test_host(host).await) });
                    }
                }
                KeyCode::Enter => self.shells(),
                KeyCode::Char('r') => self.profiles(),
                KeyCode::Char('i') => self.local(LocalCommand::Import),
                KeyCode::Char('x') => self.local(LocalCommand::Export),
                KeyCode::Char('?') => self.local(LocalCommand::Help),
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Esc if !self.tabs.is_empty() => self.navigate(Page::Terminal),
                _ => (),
            },
            Page::Shells(list) => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.shell_row = (self.shell_row + 1).min(list.shells.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.shell_row = self.shell_row.saturating_sub(1)
                }
                KeyCode::Enter => {
                    if let Some(shell) = list.shells.get(self.shell_row) {
                        self.show_modal(Modal::Confirm(Confirm::Attach(shell.clone())));
                    }
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    if let Some(shell) = list.shells.get(self.shell_row) {
                        self.show_modal(Modal::Confirm(Confirm::DeleteShell(shell.clone())));
                    }
                }
                KeyCode::Char('n') => self.create(),
                KeyCode::Char('r') => self.shells(),
                KeyCode::Esc => self.navigate(Page::Hosts),
                _ => (),
            },
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
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
        match &mut modal {
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
                    Confirm::DeleteHost(host) => {
                        self.job(async move { JobResult::Deleted(backend.delete_host(host).await) })
                    }
                    Confirm::Attach(shell) => {
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
    fn paste(&mut self, text: String) {
        if self.modal.is_some() {
            let text = Zeroizing::new(text);
            match self.modal.as_mut() {
                Some(Modal::Host(form)) => form.fields[form.active].paste(&text),
                Some(Modal::Transfer(form)) => form.fields[form.active].paste(&text),
                _ => (),
            }
        } else if matches!(self.page, Page::Terminal) {
            self.send(TabAction::Paste(text));
        }
    }
    async fn complete(&mut self) {
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
                self.profiles = profiles;
                self.host_row = self
                    .host_row
                    .min(self.profiles.hosts.len().saturating_sub(1));
                self.status = format!(
                    "{} Hosts，{} 個已保存 Screen",
                    self.profiles.hosts.len(),
                    self.profiles.screens.len()
                );
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
            JobResult::Shells(Ok(list)) if current => {
                self.shell_row = 0;
                self.navigate(Page::Shells(list));
                self.status = "選擇 Shell 並按 Enter；n 建立新工作，r 重新整理。".into();
            }
            JobResult::Shells(Ok(_)) => self.status = "先前的工作清單已完成；目前頁面保留。".into(),
            JobResult::Opened(Ok(tab)) => {
                self.tabs.push(tab);
                if current {
                    self.tab = self.tabs.len() - 1;
                    self.navigate(Page::Terminal);
                }
                self.status = "遠端 Shell 已連線。Ctrl-] ? 查看分頁快捷鍵。".into();
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
            | JobResult::Saved(Err(error))
            | JobResult::Deleted(Err(error))
            | JobResult::Tested(Err(error))
            | JobResult::Shells(Err(error))
            | JobResult::Opened(Err(error))
            | JobResult::RemoteDeleted(Err(error))
            | JobResult::Preview(Err(error))
            | JobResult::Imported(Err(error))
            | JobResult::Exported(Err(error)) => self.status = error.to_string(),
        }
    }
    fn resize(&mut self, area: Rect) {
        let Some(size) = terminal_size(area) else {
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
    async fn stop(&mut self) {
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
        for task in self.closing.drain(..) {
            let _ = task.await;
        }
    }
}

fn prefix(key: KeyEvent) -> bool {
    // Crossterm maps the legacy wire byte 0x1d to Ctrl-5.
    matches!(key.code, KeyCode::Char(']' | '5')) && key.modifiers == KeyModifiers::CONTROL
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
fn terminal_size(area: Rect) -> Option<TerminalSize> {
    let area = body(area);
    (area.width > 0 && area.height > 0).then(|| TerminalSize::new(area.height, area.width))
}
fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.height < 4 || area.width < 20 {
        frame.render_widget(Paragraph::new("終端太小，請放大視窗。"), area);
        return;
    }
    let content = body(area);
    let tabs: Vec<Span> = std::iter::once(Span::styled(
        " art-term ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
    .chain(app.tabs.iter().enumerate().map(|(index, tab)| {
        Span::styled(
            format!(" {}:{} ", index + 1, safe(tab.label())),
            if index == app.tab && matches!(app.page, Page::Terminal) {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            },
        )
    }))
    .collect();
    frame.render_widget(
        Paragraph::new(Line::from(tabs)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    match &app.page {
        Page::Hosts => {
            let items = app
                .profiles
                .hosts
                .iter()
                .map(|host| {
                    ListItem::new(vec![
                        Line::from(safe(&host.name)),
                        Line::from(Span::styled(
                            format!("  {}", safe(&host.base_url)),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                frame.render_widget(Paragraph::new("尚未新增主機 / No Hosts\n\n按 n 新增 Host；按 i 匯入其他裝置匯出的設定。\n\n這裡管理遠端 Runtime Shell，不會啟動本機 Shell。").block(panel("主機 / Hosts")).wrap(Wrap { trim: false }), content);
            } else {
                let mut state = ListState::default().with_selected(Some(app.host_row));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(panel("主機 / Hosts"))
                        .highlight_style(Style::default().bg(Color::DarkGray))
                        .highlight_symbol("▶ "),
                    content,
                    &mut state,
                );
            }
        }
        Page::Shells(list) => {
            let title = format!(
                "遠端 Shells{}",
                if list.draining {
                    "（主機維護中）"
                } else {
                    ""
                }
            );
            let items = list
                .shells
                .iter()
                .map(|shell| {
                    ListItem::new(format!(
                        "{}  [{}]",
                        safe(&shell.label),
                        if shell.attached {
                            "已連線，Enter 接管"
                        } else {
                            "可連線，Enter 選取"
                        }
                    ))
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                frame.render_widget(Paragraph::new("沒有遠端工作。按 n 建立並連線新的 Shell。\n\n若尚未準備通用歷史 helper，建立會清楚回報錯誤。").block(panel(title)), content);
            } else {
                let mut state = ListState::default().with_selected(Some(app.shell_row));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(panel(title))
                        .highlight_style(Style::default().bg(Color::DarkGray))
                        .highlight_symbol("▶ "),
                    content,
                    &mut state,
                );
            }
        }
        Page::Terminal => {
            if let Some(tab) = app.tabs.get(app.tab) {
                let view = tab.view();
                let terminal = Rect::new(
                    content.x,
                    content.y,
                    content.width.min(500),
                    content.height.min(240),
                );
                frame.render_widget(PseudoTerminal::new(view.screen.as_ref()), terminal);
            } else {
                frame.render_widget(
                    Paragraph::new("沒有開啟的終端分頁。Ctrl-] h 開啟主機清單。"),
                    content,
                );
            }
        }
        Page::Help => {
            let mut lines = vec![
                Line::from("先按 Ctrl-]，放開後再按以下按鍵。"),
                Line::from("Ctrl-] Ctrl-] 將原始 Ctrl-] 傳到遠端。"),
                Line::from("↑↓ / PgUp / PgDn 捲動說明；Esc 返回。"),
                Line::from("1..9 選擇指定分頁；Ctrl-C / Ctrl-D 保留給遠端 Shell。"),
                Line::from(""),
            ];
            lines.extend(
                BINDINGS
                    .iter()
                    .map(|(key, _, label)| Line::from(format!("  {key}    {label}"))),
            );
            lines.extend([
                Line::from(""),
                Line::from("主機清單：↑↓ 選取，Enter 工作清單，n 新增，e 編輯，d 刪除，t 測試。"),
                Line::from("工作清單：↑↓ 選取，Enter 連線／接管，n 建立，d 刪除遠端工作。"),
                Line::from("表單：Tab / Shift-Tab 移動，Ctrl-U 清空，Ctrl-S 儲存，Esc 取消。"),
                Line::from("連線會明確接管遠端工作；恢復連線只恢復原工作，不重播輸入。"),
                Line::from("歷史操作需要共同 helper 的 scroll/follow 能力；不支援時會回報。"),
                Line::from("Screen 設定隨加密匯入／匯出保留；此文字介面不提供圖形預覽。"),
                Line::from("Esc / q 返回。"),
            ]);
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel("操作說明 / Help"))
                    .scroll((app.help_offset, 0))
                    .wrap(Wrap { trim: false }),
                content,
            );
        }
    }
    let state = if matches!(app.page, Page::Terminal) {
        app.tabs
            .get(app.tab)
            .map(|tab| {
                let view = tab.view();
                format!(
                    "{} | {}{}",
                    status(&view.status),
                    history(&view.history),
                    view.notice
                        .map(|notice| format!(" | {}", safe(notice)))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_default()
    } else {
        format!(
            "{} Hosts | {} Screens",
            app.profiles.hosts.len(),
            app.profiles.screens.len()
        )
    };
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
    let footer = if app.prefix {
        "Ctrl-] 命令：h 主機 | s Shells | n 新工作 | p/l 分頁 | w 關閉 | ? 說明 | q 離開"
    } else if matches!(app.modal, Some(Modal::Import { .. })) {
        "↑↓ 捲動 | Space 重複項目策略 | y 確認匯入 | Esc 取消"
    } else if app.modal.is_some() {
        "Tab 移動 | Ctrl-U 清空 | Ctrl-S 儲存 | Esc 取消"
    } else {
        match app.page { Page::Hosts => "↑↓ 選取 | Enter Shells | n 新增 | e 編輯 | d 刪除 | t 測試 | i 匯入 | x 匯出 | Ctrl-] ? 說明", Page::Shells(_) => "↑↓ 選取 | Enter 連線/接管 | n 建立 | r 整理 | d 刪除遠端工作 | Esc 返回", _ => "Ctrl-] ? 操作說明 | Ctrl-] h 主機 | Ctrl-] 1..9 分頁 | Ctrl-] w 關閉 | Ctrl-] q 離開" }
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::Cyan)),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
    if let Some(modal) = &app.modal {
        draw_modal(modal, frame);
    }
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
fn draw_modal(modal: &Modal, frame: &mut Frame) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(88);
    let height = area.height.saturating_sub(4).min(20);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    match modal {
        Modal::Host(form) => draw_fields(
            &form.fields,
            form.active,
            if form.previous.is_some() {
                "編輯 Host / Edit"
            } else {
                "新增 Host / Add"
            },
            rect,
            frame,
        ),
        Modal::Transfer(form) => draw_fields(
            &form.fields,
            form.active,
            if form.export {
                "加密匯出全部 Host / Screen"
            } else {
                "匯入 Host / Screen（不含密碼可留白）"
            },
            rect,
            frame,
        ),
        Modal::Confirm(action) => {
            let message = match action {
                Confirm::DeleteHost(_) => {
                    "刪除此 Host 設定？原本機分頁會失效，遠端工作不會被刪除。"
                }
                Confirm::Attach(shell) => {
                    if shell.attached {
                        "接管此 Shell？目前其他連線可能中斷。"
                    } else {
                        "連線至選定的 Shell？此操作會重新取得連線權限。"
                    }
                }
                Confirm::DeleteShell(_) => "永久終止並刪除此遠端 Shell？這與關閉本機分頁不同。",
            };
            frame.render_widget(
                Paragraph::new(format!("\n{message}\n\ny 確認 / n 或 Esc 取消"))
                    .block(panel("確認 / Confirm"))
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
        Modal::Import {
            preview,
            replace,
            scroll,
        } => {
            let mut lines = vec![
                Line::from("確認本次將匯入的資料。既有其他項目保留。"),
                Line::from(format!(
                    "Space：{} 重複項目",
                    if *replace {
                        "更新可明確辨識的"
                    } else {
                        "略過"
                    }
                )),
                Line::from(""),
            ];
            lines.extend(preview.entries.iter().map(|entry| {
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
            }));
            lines.push(Line::from("y 確認匯入 / Esc 取消"));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel("匯入預覽 / Import preview"))
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
    }
}
fn draw_fields(
    fields: &[Field],
    active: usize,
    title: &'static str,
    rect: Rect,
    frame: &mut Frame,
) {
    let block = panel(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let constraints = fields
        .iter()
        .map(|_| Constraint::Length(3))
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let regions = Layout::vertical(constraints).split(inner);
    for (index, field) in fields.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(field.visible()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(field.label)
                    .border_style(Style::default().fg(if index == active {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    })),
            ),
            regions[index],
        );
    }
    frame.render_widget(
        Paragraph::new("Tab 移動；Enter 下一欄／最後一欄儲存；Ctrl-S 儲存；Esc 取消。")
            .wrap(Wrap { trim: false }),
        regions[fields.len()],
    );
}

pub(crate) async fn run(
    backend: Backend,
    startup_signals: crate::tty::StartupSignals,
) -> anyhow::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    startup_signals.disarm();
    let mut tty = Tty::enter()?;
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(40));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut app = App::new(backend);
    app.profiles();
    let result = async {
        while !app.quit {
            app.complete().await;
            let mut area = Rect::default();
            tty.terminal.draw(|frame| {
                area = frame.area();
                draw(&app, frame);
            })?;
            app.resize(area);
            tokio::select! {
                event = events.next() => match event {
                    Some(Ok(Event::Key(key))) => app.key(key),
                    Some(Ok(Event::Paste(text))) => app.paste(text),
                    Some(Ok(Event::Resize(_, _))) => (),
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
            Some(TerminalSize::new(27, 100))
        );
        assert_eq!(terminal_size(Rect::new(0, 0, 80, 2)), None);
        assert_eq!(terminal_size(Rect::new(0, 0, 0, 10)), None);
    }
    #[test]
    fn local_prefix_does_not_capture_remote_control_keys() {
        assert!(prefix(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::CONTROL
        )));
        assert!(prefix(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::CONTROL
        )));
        assert!(!prefix(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )));
        assert!(!prefix(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!prefix(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL
        )));
        let mut keys = std::collections::HashSet::new();
        for (key, _, _) in BINDINGS {
            assert!(keys.insert(key));
        }
    }
    #[test]
    fn visible_labels_never_forward_terminal_control_sequences() {
        assert_eq!(safe("host\u{1b}]52;data\u{7}"), "host�]52;data�");
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
        let vault =
            crate::vault::Vault::initialize(paths, Zeroizing::new("fixture-vault-password".into()))
                .unwrap();
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
}
