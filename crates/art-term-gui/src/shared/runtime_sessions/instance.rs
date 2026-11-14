use super::*;

impl RuntimeSessions {
    pub(super) async fn configure_tools(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
        payload: zeroize::Zeroizing<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let _guard = conn.preview_operation.lock().await;
        let (binding, snapshot) = Self::native_binding(hosts, conn)?;
        let ownership = Arc::new(OwnedShells {
            shells: conn.owned_shells.clone(),
            profile_id: ownership_key(&binding.profile),
        });
        let result = screenshot::configure(
            &hosts
                .admin()
                .map_err(|_| anyhow::anyhow!("Runtime 不可用"))?,
            snapshot,
            &binding.name,
            payload,
            ownership,
            conn.preview_cancel.subscribe(),
            |binding, token, since, generation| {
                hosts.attach_token(binding, token, since, generation)
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("受控端工具設定未確認成功；請確認通用工具入口已安裝"))?;
        Self::native_still_current(hosts, conn, &binding)?;
        anyhow::ensure!(result["ok"] == true, "受控端工具設定失敗");
        Ok(result)
    }
    pub(super) fn direct_snapshot(
        hosts: &RuntimeHosts,
        conn: &Connection,
    ) -> anyhow::Result<art_term_core::instance_mcp::DesktopSnapshot> {
        let selected = conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?
            .clone()
            .context("請先選擇 instance-mcp 桌面")?;
        let snapshot = hosts
            .store()
            .map_err(|_| anyhow::anyhow!("設定不可用"))?
            .desktop(&selected.id)
            .map_err(|_| anyhow::anyhow!("桌面設定已變更，請重新選擇"))?;
        anyhow::ensure!(snapshot.profile == selected, "桌面設定已變更，請重新選擇");
        Ok(snapshot)
    }

    pub(super) async fn instance_screenshot(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
    ) -> anyhow::Result<ScreenshotDto> {
        let epoch = *conn.preview_cancel.borrow();
        let mut cancel = conn.preview_cancel.subscribe();
        let snapshot = Self::direct_snapshot(hosts, conn)?;
        let selected = snapshot.profile.clone();
        let shot = tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面預覽已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => result?,
        };
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst)
                && conn.desktop_enabled.load(Ordering::SeqCst)
                && *conn.preview_cancel.borrow() == epoch
                && Self::direct_snapshot(hosts, conn)?.profile == selected,
            "桌面或連線已變更"
        );
        Ok(ScreenshotDto {
            mime: shot.mime,
            base64: shot.image_base64,
            source: selected.name,
            desktop_id: Some(selected.id),
            desktop_revision: Some(selected.revision),
            captured_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64,
        })
    }
    pub(super) fn tools_confirmed(
        result: &serde_json::Value,
        profile: &art_term_core::instance_mcp::DesktopProfile,
    ) -> bool {
        let Some(count) = result["adapters"].as_u64().filter(|n| *n > 0) else {
            return false;
        };
        result["ok"] == true
            && result["targets"].as_array().is_some_and(|targets| {
                targets.len() as u64 == count
                    && targets.iter().all(|target| {
                        target["id"] == profile.id
                            && target["revision"] == profile.revision
                            && target["generation"].as_u64().is_some_and(|n| n > 0)
                    })
            })
    }
    pub(super) async fn configure_selected(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
        profile: &art_term_core::instance_mcp::DesktopProfile,
    ) -> anyhow::Result<bool> {
        let snapshot = hosts
            .store()
            .map_err(|_| anyhow::anyhow!("設定不可用"))?
            .desktop(&profile.id)
            .map_err(|_| anyhow::anyhow!("桌面已變更"))?;
        anyhow::ensure!(snapshot.profile == *profile, "桌面已變更");
        let payload = zeroize::Zeroizing::new(serde_json::json!({"op":"configure","id":profile.id,"revision":profile.revision,"name":profile.name,"url":profile.mcp_url,"token":snapshot.token.expose()}).to_string());
        // Preview is independent of an installed/running adapter. No false tool-ready claim.
        Ok(Self::configure_tools(hosts, conn, payload)
            .await
            .map(|result| Self::tools_confirmed(&result, profile))
            .unwrap_or(false))
    }
    pub async fn restore_instance(
        &self,
        hosts: &RuntimeHosts,
        id: &str,
    ) -> anyhow::Result<Option<InstanceSelection>> {
        let conn = self
            .connection(id)
            .map_err(|_| anyhow::anyhow!("連線已結束"))?;
        let _guard = conn.desktop_operation.lock().await;
        let epoch = *conn.preview_cancel.borrow();
        let mut cancel = conn.preview_cancel.subscribe();
        let status = Self::configure_tools(
            hosts,
            &conn,
            zeroize::Zeroizing::new("{\"op\":\"current\"}".into()),
        )
        .await
        .context("Shell broker 查詢失敗")?;
        let store = hosts.store().map_err(|_| anyhow::anyhow!("設定不可用"))?;
        let desktops = store.desktops().context("桌面清單讀取失敗")?;
        let Some(selection) = crate::instance_restore::resolve(&status, &desktops)? else {
            return Ok(None);
        };
        let snapshot = store
            .desktop(&selection.profile.id)
            .context("桌面憑證讀取失敗")?;
        anyhow::ensure!(snapshot.profile == selection.profile, "桌面設定已變更");
        tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面恢復已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => { result.context("桌面預覽重新驗證失敗")?; }
        }
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch,
            "桌面恢復已取消"
        );
        // Restoring a local preview must not change the live Shell's identity,
        // credentials or generation, including an assignment from another App.
        let probe = zeroize::Zeroizing::new(
            serde_json::json!({"op":"verify","expected_targets":selection.broker_targets})
                .to_string(),
        );
        let tool_ready = Self::configure_tools(hosts, &conn, probe)
            .await
            .is_ok_and(|result| result["targets"] == status["targets"]);
        let current = Self::configure_tools(
            hosts,
            &conn,
            zeroize::Zeroizing::new("{\"op\":\"current\"}".into()),
        )
        .await
        .context("Shell broker 重新核對失敗")?;
        let latest = store
            .desktop(&selection.profile.id)
            .context("桌面憑證讀取失敗")?
            .profile;
        let profile = selection.confirm(
            &current,
            &latest,
            epoch,
            *conn.preview_cancel.borrow(),
            conn.closed.load(Ordering::SeqCst),
        )?;
        *conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = Some(profile.clone());
        conn.desktop_enabled.store(true, Ordering::SeqCst);
        Ok(Some(InstanceSelection {
            desktop: profile,
            tool_ready,
        }))
    }
    pub async fn select_instance(
        &self,
        hosts: &RuntimeHosts,
        id: &str,
        desktop_id: &str,
        revision: u64,
    ) -> anyhow::Result<InstanceSelection> {
        let conn = self
            .connection(id)
            .map_err(|_| anyhow::anyhow!("連線已結束"))?;
        conn.desktop_enabled.store(false, Ordering::SeqCst);
        conn.preview_cancel.send_modify(|n| *n = n.wrapping_add(1));
        let epoch = *conn.preview_cancel.borrow();
        let _guard = conn.desktop_operation.lock().await;
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch,
            "桌面選擇已取消"
        );
        // A live binding, rather than a saved Host capability, authorizes this per-tab selection.
        Self::native_binding(hosts, &conn)?;
        let previous = conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?
            .take();
        if previous.is_some() {
            let _ = Self::configure_tools(
                hosts,
                &conn,
                zeroize::Zeroizing::new("{\"op\":\"clear\"}".into()),
            )
            .await;
        }
        let snapshot = hosts
            .store()
            .map_err(|_| anyhow::anyhow!("設定不可用"))?
            .desktop(desktop_id)
            .map_err(|_| anyhow::anyhow!("桌面已變更"))?;
        anyhow::ensure!(snapshot.profile.revision == revision, "桌面已變更");
        let profile = snapshot.profile.clone();
        let mut cancel = conn.preview_cancel.subscribe();
        tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面選擇已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => { result?; }
        }
        anyhow::ensure!(*conn.preview_cancel.borrow() == epoch, "桌面選擇已取消");
        let tool_ready = Self::configure_selected(hosts, &conn, &profile).await?;
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst)
                && *conn.preview_cancel.borrow() == epoch
                && hosts
                    .store()
                    .map_err(|_| anyhow::anyhow!("設定不可用"))?
                    .desktop(desktop_id)
                    .map_err(|_| anyhow::anyhow!("桌面已變更"))?
                    .profile
                    == profile,
            "桌面或連線已變更"
        );
        *conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = Some(profile.clone());
        conn.desktop_enabled.store(true, Ordering::SeqCst);
        Ok(InstanceSelection {
            desktop: profile,
            tool_ready,
        })
    }
    pub async fn clear_instance(&self, hosts: &RuntimeHosts, id: &str) -> anyhow::Result<()> {
        let conn = self
            .connection(id)
            .map_err(|_| anyhow::anyhow!("連線已結束"))?;
        conn.desktop_enabled.store(false, Ordering::SeqCst);
        conn.preview_cancel.send_modify(|n| *n = n.wrapping_add(1));
        let _guard = conn.desktop_operation.lock().await;
        *conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = None;
        Self::configure_tools(
            hosts,
            &conn,
            zeroize::Zeroizing::new("{\"op\":\"clear\"}".into()),
        )
        .await?;
        Ok(())
    }
}
