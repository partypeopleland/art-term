use super::*;

impl App {
    pub(super) fn rows(&self) -> Vec<TreeRow> {
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
    pub(super) fn selected_action(&self) -> Option<TreeAction> {
        let key = self.tree_selected.as_ref()?;
        self.rows()
            .into_iter()
            .find(|row| &row.key == key)
            .and_then(|row| row.action)
    }
    pub(super) fn move_tree(&mut self, amount: i16) {
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
    pub(super) fn host(&self) -> Option<HostSelection> {
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
    pub(super) fn queue_host(&mut self, host: HostSelection) {
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
    pub(super) fn shells(&mut self) {
        if let Some(host) = self.host() {
            self.queue_host(host);
            self.navigate(Page::Hosts);
        }
    }
    pub(super) fn activate_tree(&mut self) {
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
    pub(super) fn open_shell(&mut self, shell: ShellView, confirmed: bool) {
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
}
