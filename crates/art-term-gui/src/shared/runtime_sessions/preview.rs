use super::*;

impl RuntimeSessions {
    pub async fn desktop(
        &self,
        hosts: &RuntimeHosts,
        id: &str,
        enabled: bool,
        preview: &dyn RuntimePreview,
    ) -> anyhow::Result<DesktopStateDto> {
        let conn = self
            .connection(id)
            .map_err(|f| anyhow::anyhow!("{}", f.message))?;
        if !enabled {
            conn.desktop_enabled.store(false, Ordering::SeqCst);
            conn.preview_cancel.send_modify(|n| *n = n.wrapping_add(1));
        }
        let _operation = conn.desktop_operation.lock().await;
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst), "session closed");
        let has_direct_selection = conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?
            .is_some();
        if has_direct_selection {
            if enabled {
                let epoch = *conn.preview_cancel.borrow();
                let snapshot = Self::direct_snapshot(hosts, &conn)?;
                let selected = snapshot.profile.clone();
                art_term_core::instance_mcp::screenshot(snapshot).await?;
                anyhow::ensure!(
                    !conn.closed.load(Ordering::SeqCst)
                        && *conn.preview_cancel.borrow() == epoch
                        && Self::direct_snapshot(hosts, &conn)?.profile == selected,
                    "桌面已切換"
                );
            }
            conn.desktop_enabled.store(enabled, Ordering::SeqCst);
            return Ok(DesktopStateDto {
                enabled,
                message: if enabled {
                    "App 直接預覽已啟用"
                } else {
                    "預覽已停止；桌面工具保留"
                }
                .into(),
            });
        }
        let route = conn
            .desktop_route
            .clone()
            .context("此 Runtime Shell 沒有映射的共享桌面")?;
        if enabled {
            let state = match &route {
                DesktopRoute::Native => preview.native_status(hosts, &conn).await?,
            };
            conn.desktop_enabled.store(true, Ordering::SeqCst);
            if conn.closed.load(Ordering::SeqCst) {
                conn.desktop_enabled.store(false, Ordering::SeqCst);
                anyhow::bail!("session closed while desktop enabled; nothing was provisioned");
            }
            Ok(state)
        } else {
            conn.desktop_enabled.store(false, Ordering::SeqCst);
            Ok(DesktopStateDto {
                enabled: false,
                message: "桌面預覽已關閉；共享桌面不受影響".into(),
            })
        }
    }
    /// Bounded screenshot through the session's frozen route —
    /// globally serialized and closed/enabled fenced.
    pub async fn screenshot(
        &self,
        hosts: &RuntimeHosts,
        id: &str,
        preview: &dyn RuntimePreview,
    ) -> anyhow::Result<ScreenshotDto> {
        let conn = self
            .connection(id)
            .map_err(|f| anyhow::anyhow!("{}", f.message))?;
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst) && conn.desktop_enabled.load(Ordering::SeqCst),
            "desktop is not enabled for this session"
        );
        let _operation = self
            .screenshot_operation
            .try_lock()
            .map_err(|_| anyhow::anyhow!("screenshot already in progress"))?;
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst), "session closed");
        let has_direct_selection = conn
            .selected_desktop
            .lock()
            .map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?
            .is_some();
        if has_direct_selection {
            return Self::instance_screenshot(hosts, &conn).await;
        }
        let shot = match conn
            .desktop_route
            .clone()
            .context("此 Runtime Shell 沒有映射的共享桌面")?
        {
            DesktopRoute::Native => preview.native_screenshot(hosts, &conn).await?,
        };
        anyhow::ensure!(
            !conn.closed.load(Ordering::SeqCst) && conn.desktop_enabled.load(Ordering::SeqCst),
            "desktop changed while screenshot was in progress"
        );
        Ok(shot)
    }
    /// Capture the CURRENT live handle binding plus a fresh HostStore
    /// snapshot for one native-route request. The synchronous handle
    /// mutex is taken and released inside this call — never held across
    /// an await — and the binding comes from the live handle, never a
    /// copy frozen before a renew bumped the remote generation. A stale
    /// or edited profile fails closed instead of minting a mismatched
    /// credential/binding pair.
    pub(super) fn native_binding(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
    ) -> anyhow::Result<(SessionBinding, HostSnapshot)> {
        let binding = conn
            .handle
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal state unavailable"))?
            .as_ref()
            .context("session is not attached")?
            .binding()
            .clone();
        let snapshot = hosts
            .store()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .snapshot(&binding.profile.id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Self::verify_preview_binding(&binding, &binding, &snapshot.profile)?;
        Ok((binding, snapshot))
    }
    /// Post-request revalidation for the native route: the live handle
    /// must still carry the same binding (a concurrent recover/renew
    /// would have replaced it) and the profile must still be the
    /// current store entry — a cross-generation reply never lands.
    pub(super) fn native_still_current(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
        binding: &SessionBinding,
    ) -> anyhow::Result<()> {
        let current = conn
            .handle
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal state unavailable"))?
            .as_ref()
            .context("session is not attached")?
            .binding()
            .clone();
        let snapshot = hosts
            .store()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .snapshot(&binding.profile.id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Self::verify_preview_binding(binding, &current, &snapshot.profile)
    }
    pub(super) fn verify_preview_binding(
        expected: &SessionBinding,
        current: &SessionBinding,
        profile: &art_term_core::openab::profiles::HostProfile,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            expected == current,
            "session binding changed during capture"
        );
        anyhow::ensure!(
            &expected.profile == profile,
            "host profile changed during capture"
        );
        Ok(())
    }
    pub(super) async fn native_preview(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
        mode: Mode,
    ) -> anyhow::Result<(SessionBinding, serde_json::Value)> {
        let mut cancel = conn.preview_cancel.subscribe();
        let _operation = tokio::select! { _ = cancel.changed() => anyhow::bail!("preview cancelled"), guard = tokio::time::timeout(std::time::Duration::from_secs(30), conn.preview_operation.lock()) => guard.map_err(|_| anyhow::anyhow!("preview wait timed out"))? };
        let (binding, snapshot) = Self::native_binding(hosts, conn)?;
        let ownership = Arc::new(OwnedShells {
            shells: conn.owned_shells.clone(),
            profile_id: ownership_key(&binding.profile),
        });
        let api = hosts.admin().map_err(|e| anyhow::anyhow!("{e}"))?;
        // Retry only recorded ownership for this exact profile; foreign prefixed names are untouched.
        let pending = conn
            .owned_shells
            .lock()
            .map_err(|_| anyhow::anyhow!("preview state unavailable"))?
            .get(&ownership_key(&binding.profile))
            .cloned()
            .unwrap_or_default();
        for name in pending.into_iter().take(4) {
            tokio::select! {
                _ = cancel.changed() => anyhow::bail!("preview cancelled"),
                deleted = tokio::time::timeout(std::time::Duration::from_secs(2), api.delete(&snapshot, &name)) => {
                    if matches!(deleted, Ok(Ok(())) | Ok(Err(ApiError::SessionMissing))) { ownership.unmark(&name); }
                }
            }
        }
        let json = screenshot::run(
            &api,
            snapshot,
            mode,
            ownership,
            cancel,
            |binding, token, since, generation| {
                hosts.attach_token(binding, token, since, generation)
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", SessionFailure::from(e).message))?;
        Self::native_still_current(hosts, conn, &binding)?;
        Ok((binding, json))
    }
    pub(super) async fn native_status(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
    ) -> anyhow::Result<DesktopStateDto> {
        let (_, json) = Self::native_preview(hosts, conn, Mode::Status).await?;
        anyhow::ensure!(
            screenshot::decode_status(&json)?,
            "shared desktop is not ready"
        );
        Ok(DesktopStateDto {
            enabled: true,
            message: "桌面已啟用".into(),
        })
    }
    pub(super) async fn native_screenshot(
        hosts: &RuntimeHosts,
        conn: &Arc<Connection>,
    ) -> anyhow::Result<ScreenshotDto> {
        let (binding, json) = Self::native_preview(hosts, conn, Mode::Screenshot).await?;
        let shot = screenshot::decode_screenshot(json).await?;
        Self::native_still_current(hosts, conn, &binding)?;
        Ok(ScreenshotDto {
            desktop_id: None,
            desktop_revision: None,
            mime: shot.mime,
            base64: shot.image_base64,
            source: format!("{} / shared runtime desktop", binding.profile.name),
            captured_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
    }
}
