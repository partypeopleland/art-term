use super::*;

impl App {
    pub(super) async fn complete(&mut self) {
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
                self.refresh_settings_target();
            }
            JobResult::ScreenSaved(Ok(selection)) => {
                if let Some(ModalOrigin::Settings { target, .. }) = &mut self.modal_origin {
                    *target = Some(SettingsTarget::Screen(selection));
                }
                self.profiles();
            }
            JobResult::Saved(Ok(selection)) => {
                if let Some(ModalOrigin::Settings { target, .. }) = &mut self.modal_origin {
                    *target = Some(SettingsTarget::Host(selection));
                }
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
        if self.modal.is_none() {
            self.restore_origin();
        }
    }
    pub(super) fn resize(&mut self, area: Rect) {
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
    pub(super) fn damage_images(&mut self) {
        if self.display.pixels().is_some() {
            self.image_damage = true;
        }
    }
}
