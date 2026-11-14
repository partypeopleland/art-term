use super::*;

impl App {
    pub(super) fn send(&mut self, action: TabAction) {
        if let Some(tab) = self.tabs.get(self.tab) {
            if let Err(error) = tab.try_send(action) {
                self.status = error.to_string();
            }
        }
    }
    pub(super) fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            if self.tab != index {
                self.invalidate_preview();
            }
            self.tab = index;
            self.navigate(Page::Terminal);
        }
    }
    pub(super) fn local(&mut self, command: LocalCommand) {
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
            LocalCommand::Settings => self.open_settings(),
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
    pub(super) fn key(&mut self, key: KeyEvent) {
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
        if was_prefix && key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.status = "已取消命令模式。".into();
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
    pub(super) fn binding_action(&mut self, action: BindingAction) {
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
    pub(super) fn bind_screen(&mut self, row: usize) {
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
    pub(super) fn selected_screen(&self) -> Option<ScreenSelection> {
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
    pub(super) fn desktop_status(&self) -> String {
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
    pub(super) fn modal_key(&mut self, mut key: KeyEvent) {
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
            self.restore_origin();
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
            Modal::Settings(settings) => {
                let row = settings.row;
                let target = settings.target.clone();
                match key.code {
                    KeyCode::Down => {
                        let len = self.settings_rows().len();
                        settings.row = (row + 1).min(len.saturating_sub(1));
                    }
                    KeyCode::Up => settings.row = row.saturating_sub(1),
                    KeyCode::Enter => match self.settings_rows().get(row).cloned() {
                        Some(SettingsRow::Target(candidate)) => settings.target = Some(candidate),
                        Some(SettingsRow::Action(action)) => {
                            self.settings_action(action, target, row);
                            return;
                        }
                        None => (),
                    },
                    _ => (),
                }
            }
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
            Modal::Confirm(_) if matches!(key.code, KeyCode::Char('n' | 'N')) => {
                self.restore_origin();
                return;
            }
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
    pub(super) fn mouse(&mut self, event: MouseEvent) {
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
                let settings_len = self.settings_rows().len() as isize;
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
                        Modal::Settings(settings) => {
                            settings.row = (settings.row as isize + amount)
                                .clamp(0, settings_len.saturating_sub(1))
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
    pub(super) fn hit(&mut self, action: HitAction) {
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
                    Some(Modal::Settings(settings)) => settings.row = index,
                    _ => return,
                }
                self.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
        }
    }

    pub(super) fn paste(&mut self, text: String) {
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
}
