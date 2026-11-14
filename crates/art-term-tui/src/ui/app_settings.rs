use super::*;

impl App {
    pub(super) fn open_settings(&mut self) {
        self.modal_origin = None;
        let target = if self.focus == Focus::Connections && !matches!(self.page, Page::Help) {
            match self.selected_action() {
                Some(TreeAction::Host(index)) => self
                    .profiles
                    .hosts
                    .get(index)
                    .map(|host| SettingsTarget::Host(HostSelection::from(host))),
                Some(TreeAction::Screen(index)) => self.profiles.screens.get(index).map(|screen| {
                    SettingsTarget::Screen(ScreenSelection {
                        id: screen.id.clone(),
                        revision: screen.revision,
                    })
                }),
                _ => None,
            }
        } else {
            None
        };
        self.show_modal(Modal::Settings(SettingsModal { row: 0, target }));
    }
    pub(super) fn resolve_settings_target(
        &self,
        target: &Option<SettingsTarget>,
    ) -> Option<SettingsTarget> {
        match target {
            Some(SettingsTarget::Host(selection)) => self
                .profiles
                .hosts
                .iter()
                .find(|host| host.id == selection.id && host.revision == selection.revision)
                .map(|host| SettingsTarget::Host(HostSelection::from(host))),
            Some(SettingsTarget::Screen(selection)) => self
                .profiles
                .screens
                .iter()
                .find(|screen| screen.id == selection.id && screen.revision == selection.revision)
                .map(|screen| {
                    SettingsTarget::Screen(ScreenSelection {
                        id: screen.id.clone(),
                        revision: screen.revision,
                    })
                }),
            None => None,
        }
    }
    pub(super) fn settings_rows(&self) -> Vec<SettingsRow> {
        SETTINGS_ACTIONS
            .iter()
            .map(|(action, _)| SettingsRow::Action(*action))
            .chain(
                self.profiles.hosts.iter().map(|host| {
                    SettingsRow::Target(SettingsTarget::Host(HostSelection::from(host)))
                }),
            )
            .chain(self.profiles.screens.iter().map(|screen| {
                SettingsRow::Target(SettingsTarget::Screen(ScreenSelection {
                    id: screen.id.clone(),
                    revision: screen.revision,
                }))
            }))
            .collect()
    }
    pub(super) fn settings_target_label(&self, target: &SettingsTarget) -> String {
        match target {
            SettingsTarget::Host(selection) => {
                self.profiles
                    .hosts
                    .iter()
                    .find(|host| host.id == selection.id)
                    .map(|host| {
                        let agent = host.metadata.as_ref().is_some_and(|m| {
                            m.category == art_term_core::connections::Category::Agent
                        });
                        let kind = if agent { "Agent" } else { "Host" };
                        match host
                            .metadata
                            .as_ref()
                            .and_then(|m| m.display_name.as_ref())
                            .filter(|name| !name.trim().is_empty())
                        {
                            Some(name) => format!("{kind}：{} / {}", safe(&host.name), safe(name)),
                            None => format!("{kind}：{}", safe(&host.name)),
                        }
                    })
                    .unwrap_or_else(|| "Host：?".into())
            }
            SettingsTarget::Screen(selection) => self
                .profiles
                .screens
                .iter()
                .find(|screen| screen.id == selection.id)
                .map(|screen| format!("Screen：{}", safe(&screen.name)))
                .unwrap_or_else(|| "Screen：?".into()),
        }
    }
    pub(super) fn settings_row_text(
        &self,
        row: &SettingsRow,
        resolved: &Option<SettingsTarget>,
    ) -> (String, bool) {
        match row {
            SettingsRow::Target(candidate) => (
                format!(
                    "{} {}",
                    if resolved.as_ref() == Some(candidate) {
                        "●"
                    } else {
                        "○"
                    },
                    self.settings_target_label(candidate)
                ),
                false,
            ),
            SettingsRow::Action(action) => {
                let label = SETTINGS_ACTIONS
                    .iter()
                    .find(|(candidate, _)| candidate == action)
                    .map(|(_, label)| *label)
                    .unwrap_or("?");
                let suffix = match action {
                    SettingsAction::Edit | SettingsAction::Delete | SettingsAction::Test
                        if resolved.is_none() =>
                    {
                        Some("（需先選擇連線）")
                    }
                    SettingsAction::Test if matches!(resolved, Some(SettingsTarget::Screen(_))) => {
                        Some("（僅適用 Host）")
                    }
                    _ => None,
                };
                let text = format!("{label}{}", suffix.unwrap_or(""));
                (text, suffix.is_some())
            }
        }
    }
    pub(super) fn restore_settings_modal(&mut self, row: usize, target: Option<SettingsTarget>) {
        self.modal = Some(Modal::Settings(SettingsModal { row, target }));
    }
    pub(super) fn open_settings_child(
        &mut self,
        modal: Modal,
        target: Option<SettingsTarget>,
        row: usize,
    ) {
        self.modal_origin = Some(ModalOrigin::Settings { target, row });
        self.show_modal(modal);
    }
    pub(super) fn restore_origin(&mut self) {
        let Some(ModalOrigin::Settings { target, row }) = self.modal_origin.take() else {
            return;
        };
        // A just-saved selection carries a revision the cached profiles have not
        // seen yet; keep it so the in-flight refresh can validate it. Commits
        // still revalidate through resolve_settings_target.
        let target = self.resolve_settings_target(&target).or(target);
        self.modal = Some(Modal::Settings(SettingsModal { row, target }));
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub(super) fn settings_action(
        &mut self,
        action: SettingsAction,
        target: Option<SettingsTarget>,
        row: usize,
    ) {
        let resolved = self.resolve_settings_target(&target);
        match action {
            SettingsAction::Back => (),
            SettingsAction::NewHost => {
                self.open_settings_child(Modal::Host(HostForm::new(None)), target, row)
            }
            SettingsAction::NewAgent => {
                self.open_settings_child(Modal::Host(HostForm::agent()), target, row)
            }
            SettingsAction::NewScreen => {
                self.open_settings_child(Modal::Screen(ScreenForm::new(None)), target, row)
            }
            SettingsAction::Import => {
                self.open_settings_child(Modal::Transfer(TransferForm::new(false)), target, row)
            }
            SettingsAction::Export => {
                self.open_settings_child(Modal::Transfer(TransferForm::new(true)), target, row)
            }
            SettingsAction::Edit => match &resolved {
                Some(SettingsTarget::Host(selection)) => {
                    if let Some(host) = self
                        .profiles
                        .hosts
                        .iter()
                        .find(|host| host.id == selection.id)
                    {
                        self.open_settings_child(
                            Modal::Host(HostForm::new(Some(host))),
                            target,
                            row,
                        );
                    } else {
                        self.restore_settings_modal(row, target);
                    }
                }
                Some(SettingsTarget::Screen(selection)) => {
                    if let Some(screen) = self
                        .profiles
                        .screens
                        .iter()
                        .find(|screen| screen.id == selection.id)
                    {
                        self.open_settings_child(
                            Modal::Screen(ScreenForm::new(Some(screen))),
                            target,
                            row,
                        );
                    } else {
                        self.restore_settings_modal(row, target);
                    }
                }
                None => {
                    self.status = "請先選擇要管理的連線。".into();
                    self.restore_settings_modal(row, target);
                }
            },
            SettingsAction::Test => match &resolved {
                Some(SettingsTarget::Host(selection)) => {
                    self.modal_origin = Some(ModalOrigin::Settings { target, row });
                    let backend = self.backend.clone();
                    let host = selection.clone();
                    self.job(async move { JobResult::Tested(backend.test_host(host).await) });
                }
                Some(SettingsTarget::Screen(_)) => {
                    self.status = "測試連線僅適用 Host。".into();
                    self.restore_settings_modal(row, target);
                }
                None => {
                    self.status = "請先選擇要管理的連線。".into();
                    self.restore_settings_modal(row, target);
                }
            },
            SettingsAction::Delete => match &resolved {
                Some(SettingsTarget::Host(selection)) => {
                    self.modal_origin = Some(ModalOrigin::Settings {
                        target: resolved.clone(),
                        row,
                    });
                    self.show_modal(Modal::Confirm(Confirm::DeleteHost(selection.clone())));
                }
                Some(SettingsTarget::Screen(selection)) => {
                    self.modal_origin = Some(ModalOrigin::Settings {
                        target: resolved.clone(),
                        row,
                    });
                    self.show_modal(Modal::Confirm(Confirm::DeleteScreen(selection.clone())));
                }
                None => {
                    self.status = "請先選擇要管理的連線。".into();
                    self.restore_settings_modal(row, target);
                }
            },
        }
    }
    pub(super) fn refresh_settings_target(&mut self) {
        let resolved = match &self.modal {
            Some(Modal::Settings(settings)) => Some(self.resolve_settings_target(&settings.target)),
            _ => None,
        };
        if let (Some(Modal::Settings(settings)), Some(target)) = (self.modal.as_mut(), resolved) {
            settings.target = target;
        }
    }
    pub(super) fn edit_selected(&mut self) {
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
    pub(super) fn delete_selected(&mut self) {
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
    pub(super) fn create(&mut self) {
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
}
