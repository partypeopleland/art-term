use super::*;

impl RuntimeSessions {
    pub(super) fn id(&self, prefix: &str) -> String {
        format!(
            "runtime-{prefix}-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        )
    }
    pub(super) fn issue(
        &self,
        binding: SessionBinding,
        grant: Option<SessionGrant>,
        attached: bool,
        desktop: Option<RuntimeMapping>,
    ) -> Result<TargetDto, ApiError> {
        let mut targets = self.targets.lock().map_err(|_| ApiError::Storage)?;
        // Officially the session name is the identity — a generation
        // change from an operator renew keeps the same target.
        if let Some((id, target)) = targets.iter().find(|(_, target)| {
            target.binding.profile == binding.profile && target.binding.name == binding.name
        }) {
            // Reused bindings keep the mapping frozen at first issue —
            // a later catalog view can never rebind them.
            return Ok(TargetDto {
                id: id.clone(),
                host_id: binding.profile.id.clone(),
                label: shell_label(&binding.name),
                subtitle: "Runtime Shell".into(),
                account_label: None,
                kind: "host".into(),
                running: true,
                phase: if attached { "attached" } else { "ready" }.into(),
                desktop_available: target.desktop.as_ref().is_some_and(|m| m.route.is_some()),
            });
        }
        // A changed profile or session incarnation invalidates the old target;
        // open connections retain their own immutable binding.
        targets.retain(|_, target| {
            target.binding.profile.id != binding.profile.id || target.binding.name != binding.name
        });
        if targets.len() >= 512 {
            return Err(ApiError::Capacity);
        }
        let id = self.id("target");
        let dto = TargetDto {
            id: id.clone(),
            host_id: binding.profile.id.clone(),
            label: shell_label(&binding.name),
            subtitle: "Runtime Shell".into(),
            account_label: None,
            kind: "host".into(),
            running: true,
            phase: if attached { "attached" } else { "ready" }.into(),
            desktop_available: desktop.as_ref().is_some_and(|m| m.route.is_some()),
        };
        targets.insert(
            id,
            Arc::new(Target {
                binding,
                grant: Mutex::new(grant),
                desktop,
                history_bootstrap: AtomicBool::new(false),
            }),
        );
        Ok(dto)
    }
    pub(super) fn connection(&self, id: &str) -> Reply<Arc<Connection>> {
        self.sessions
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(id)
            .cloned()
            .ok_or(SessionError::StateChanged.into())
    }
    pub(super) fn prune_list(
        &self,
        profile: &art_term_core::openab::profiles::HostProfile,
        names: &[&str],
    ) -> Result<(), ApiError> {
        let mut targets = self.targets.lock().map_err(|_| ApiError::Storage)?;
        targets.retain(|_, target| {
            target.binding.profile.id != profile.id
                || (target.binding.profile == *profile
                    && names.contains(&target.binding.name.as_str()))
        });
        Ok(())
    }
    pub fn is_target(id: &str) -> bool {
        id.starts_with("runtime-target-")
    }
    pub fn is_session(id: &str) -> bool {
        id.starts_with("runtime-session-")
    }
    pub async fn open(
        &self,
        hosts: &RuntimeHosts,
        target_id: &str,
        rows: u16,
        cols: u16,
    ) -> Reply<SessionDto> {
        let target = self
            .targets
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(target_id)
            .cloned()
            .ok_or(SessionError::StateChanged)?;
        let snapshot = hosts
            .store()
            .map_err(SessionError::from)?
            .snapshot(&target.binding.profile.id)
            .map_err(SessionError::from)?;
        if snapshot.profile != target.binding.profile {
            return Err(SessionError::StateChanged.into());
        }
        if self
            .sessions
            .lock()
            .map_err(|_| SessionError::Internal)?
            .len()
            >= 32
        {
            return Err(SessionError::Capacity.into());
        }
        let cached = target
            .grant
            .lock()
            .map_err(|_| SessionError::Internal)?
            .take();
        let grant = match cached {
            Some(grant) => grant,
            // Explicit user re-selection is the only path that mints a new
            // token — the official renew (no expected_* conditions) bumps
            // the remote generation and evicts any older attach.
            None => hosts
                .admin()
                .map_err(SessionError::from)?
                .renew(&snapshot, &target.binding.name)
                .await
                .map_err(SessionError::from)?,
        };
        target
            .binding
            .verify_grant(&grant, false)
            .map_err(SessionError::from)?;
        let handle = hosts.attach(snapshot, grant, 0, 1).await?;
        if !hosts
            .store()
            .map_err(SessionError::from)?
            .is_current(&target.binding.profile)
            .map_err(SessionError::from)?
        {
            handle.shutdown().await;
            return Err(SessionError::StateChanged.into());
        }
        handle.resize(rows, cols)?;
        let history_binding = handle.binding().clone();
        if target.history_bootstrap.swap(false, Ordering::SeqCst) {
            let key =
                history::key_from_name(&history_binding.name).ok_or(SessionError::Protocol)?;
            let version = history::HistoryVersion::from_name(&history_binding.name)
                .ok_or(SessionError::Protocol)?;
            // Only a target minted by the explicit new-history action can
            // reach this once. Relist/recovery never repeats shell input.
            handle.write(version.bootstrap_command(&key)?.as_bytes())?;
        }
        let (cancel, _) = watch::channel(false);
        let id = self.id("session");
        self.sessions
            .lock()
            .map_err(|_| SessionError::Internal)?
            .insert(
                id.clone(),
                Arc::new(Connection {
                    handle: Mutex::new(Some(handle)),
                    recovery: Recovery::default(),
                    cancel,
                    next_generation: AtomicU64::new(2),
                    // Frozen from the verified target — a later mapping
                    // change can never retarget this session's preview.
                    desktop_route: target.desktop.as_ref().and_then(|m| m.route.clone()),
                    desktop_enabled: AtomicBool::new(false),
                    selected_desktop: Mutex::new(None),
                    preview_cancel: watch::channel(0).0,
                    preview_operation: self.preview_operation.clone(),
                    owned_shells: self.owned_shells.clone(),
                    closed: AtomicBool::new(false),
                    desktop_operation: tokio::sync::Mutex::new(()),
                    history_binding: Some(history_binding),
                    history: tokio::sync::Mutex::new(None),
                    history_cancel: watch::channel(0).0,
                    native_interaction: tokio::sync::Mutex::new(()),
                    native_enabled: AtomicBool::new(false),
                    native_held: AtomicBool::new(false),
                    native_uncertain: AtomicBool::new(false),
                }),
            );
        Ok(SessionDto {
            id,
            target_id: target_id.into(),
            label: shell_label(&target.binding.name),
            remote_session: format!("openab:{}", target.binding.name),
        })
    }
    pub fn poll(&self, id: &str) -> Reply<GuiOutput> {
        let connection = self.connection(id)?;
        let guard = connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?;
        let handle = guard.as_ref().ok_or(SessionError::Busy)?;
        let poll = handle.poll()?;
        Ok(GuiOutput {
            bytes: vec![],
            exit_code: None,
            runtime: Some(RuntimeOutput {
                batch_id: poll.batch_id,
                local_generation: poll.local_generation,
                events: poll.events,
                ended: poll.ended,
                input_uncertain: poll.input_uncertain,
            }),
        })
    }
    pub fn ack(&self, id: &str, batch: u64, generation: u64) -> Reply<()> {
        let connection = self.connection(id)?;
        let guard = connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?;
        guard
            .as_ref()
            .ok_or(SessionError::Busy)?
            .ack(batch, generation)
            .map_err(Into::into)
    }
    pub(super) fn direct_write(connection: &Connection, bytes: &[u8]) -> Reply<()> {
        if connection.closed.load(Ordering::SeqCst) {
            return Err(SessionError::Cancelled.into());
        }
        let guard = connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?;
        guard
            .as_ref()
            .ok_or(SessionError::Busy)?
            .write(bytes)
            .map_err(Into::into)
    }
    pub async fn write(&self, id: &str, bytes: &[u8], terminal_reply: bool) -> Reply<()> {
        let connection = self.connection(id)?;
        // Replies belong to the outer tmux client, never the inner Shell or follow operation.
        // Android never enables native control and retains this original fast path.
        if terminal_reply || !connection.native_enabled.load(Ordering::SeqCst) {
            return Self::direct_write(&connection, bytes);
        }
        let history_generation = *connection.history_cancel.borrow();
        let _interaction = connection.native_interaction.lock().await;
        if *connection.history_cancel.borrow() != history_generation {
            return Err(SessionError::Cancelled.into());
        }
        if connection.closed.load(Ordering::SeqCst) {
            return Err(SessionError::Cancelled.into());
        }
        if connection.native_uncertain.load(Ordering::SeqCst) {
            return Err(SessionError::InputUncertain.into());
        }
        if !connection.native_held.load(Ordering::SeqCst) {
            return Self::direct_write(&connection, bytes);
        }
        if bytes.len() > 1024 * 1024 {
            return Err(SessionError::Capacity.into());
        }
        let mut slot = connection.history.lock().await;
        if connection.closed.load(Ordering::SeqCst)
            || *connection.history_cancel.borrow() != history_generation
        {
            return Err(SessionError::Cancelled.into());
        }
        let channel = &mut slot.as_mut().ok_or(SessionError::StateChanged)?.channel;
        let mut cancel = connection.cancel.subscribe();
        let mut history_cancel = connection.history_cancel.subscribe();
        let length = bytes.len().min(4096);
        connection.native_uncertain.store(true, Ordering::SeqCst);
        let result = tokio::select! { biased;
            _ = cancel.changed() => Err(SessionError::Cancelled),
            _ = history_cancel.changed() => Err(SessionError::Cancelled),
            result = channel.request(HistoryRequest::Follow { input: Some(STANDARD.encode(&bytes[..length])) }) => result,
        };
        match result {
            Ok(HistoryReply::View {
                held: false,
                offset: 0,
                available: true,
                ..
            }) => {
                connection.native_held.store(false, Ordering::SeqCst);
                connection.native_uncertain.store(false, Ordering::SeqCst);
                // Long CLI commands still remain one ordered interaction; no second sender.
                if length < bytes.len() {
                    Self::direct_write(&connection, &bytes[length..])?;
                }
                Ok(())
            }
            _ => {
                if let Some(mut owned) = slot.take() {
                    if owned.channel.close().await {
                        connection.native_held.store(false, Ordering::SeqCst);
                        connection.native_uncertain.store(false, Ordering::SeqCst);
                    }
                }
                // Atomic remote send may already have happened. No retry or original-PTY fallback.
                Err(SessionError::InputUncertain.into())
            }
        }
    }
    pub fn resize(&self, id: &str, rows: u16, cols: u16) -> Reply<()> {
        let connection = self.connection(id)?;
        let guard = connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?;
        guard
            .as_ref()
            .ok_or(SessionError::Busy)?
            .resize(rows, cols)
            .map_err(Into::into)
    }
    /// Transient-drop recovery: the SAME retained attach token plus the
    /// renderer ACK cursor (`?since=`). No renew/create/restart ever runs
    /// here — a dead token (401) or terminal close only gets a diagnostic
    /// list so the user can re-select; minting requires explicit open.
    pub async fn recover(&self, hosts: &RuntimeHosts, id: &str) -> Reply<()> {
        let connection = self.connection(id)?;
        connection
            .history_cancel
            .send_modify(|n| *n = n.wrapping_add(1));
        let _interaction = connection.native_interaction.lock().await;
        if let Some(mut owned) = connection.history.lock().await.take() {
            if owned.channel.close().await {
                connection.native_held.store(false, Ordering::SeqCst);
                connection.native_uncertain.store(false, Ordering::SeqCst);
            }
        }
        let mut old = connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?
            .take()
            .ok_or(SessionError::Busy)?;
        let binding = old.binding().clone();
        let cursor = old.rendered_cursor()?;
        let token = old.take_token().ok_or(SessionError::StateChanged)?;
        old.shutdown().await;
        if !hosts
            .store()
            .map_err(SessionError::from)?
            .is_current(&binding.profile)
            .map_err(SessionError::from)?
        {
            return Err(SessionError::StateChanged.into());
        }
        let generation = connection.next_generation.fetch_add(1, Ordering::SeqCst);
        let result = connection
            .recovery
            .resume(connection.cancel.subscribe(), || {
                let retry = Secret::new(token.expose().to_owned()).map_err(SessionError::from);
                let binding = binding.clone();
                async move {
                    let token = retry?;
                    hosts.attach_token(binding, token, cursor, generation).await
                }
            })
            .await;
        let handle = match result {
            Ok(handle) => handle,
            // 401 is deliberately ambiguous: one diagnostic list separates
            // "session gone" from "token stale". Either way recovery stops
            // and the user re-selects — no silent renew or rebind.
            Err(SessionError::Auth) => {
                let snapshot = hosts
                    .store()
                    .map_err(SessionError::from)?
                    .snapshot(&binding.profile.id)
                    .map_err(SessionError::from)?;
                if snapshot.profile != binding.profile {
                    return Err(SessionError::StateChanged.into());
                }
                return Err(match hosts
                    .admin()
                    .map_err(SessionError::from)?
                    .list(&snapshot)
                    .await
                {
                    Ok(list) => match binding.verify_list(&list) {
                        Ok(()) => SessionError::Auth,
                        Err(e) => SessionError::from(e),
                    },
                    Err(e) => SessionError::from(e),
                }
                .into());
            }
            Err(e) => return Err(e.into()),
        };
        if *connection.cancel.borrow()
            || !hosts
                .store()
                .map_err(SessionError::from)?
                .is_current(&binding.profile)
                .map_err(SessionError::from)?
        {
            handle.shutdown().await;
            return Err(SessionError::StateChanged.into());
        }
        *connection
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)? = Some(handle);
        Ok(())
    }
    /// Shared-desktop preview toggle for a runtime session. Enable is a
    /// readiness check through the session's frozen native or pinned Endpoint
    /// route; disable is local-only —
    /// the shared service is never released, probed, or stopped.
    pub async fn close(&self, id: &str) -> Reply<()> {
        let connection = self
            .sessions
            .lock()
            .map_err(|_| SessionError::Internal)?
            .remove(id);
        if let Some(connection) = connection {
            // Local flags only — a detach never touches the shared
            // desktop; the remote Shell keeps running on the host.
            connection.closed.store(true, Ordering::SeqCst);
            connection.desktop_enabled.store(false, Ordering::SeqCst);
            connection
                .preview_cancel
                .send_modify(|n| *n = n.wrapping_add(1));
            connection.cancel.send_replace(true);
            connection
                .history_cancel
                .send_modify(|n| *n = n.wrapping_add(1));
            let _interaction = connection.native_interaction.lock().await;
            if let Some(mut history) = connection.history.lock().await.take() {
                history.channel.close().await;
            }
            let handle = connection
                .handle
                .lock()
                .map_err(|_| SessionError::Internal)?
                .take();
            if let Some(handle) = handle {
                handle.shutdown().await;
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        if let Ok(mut targets) = self.targets.lock() {
            targets.clear();
        }
        let connections = self
            .sessions
            .lock()
            .map(|mut s| {
                s.drain()
                    .map(|(_, connection)| connection)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut tasks = tokio::task::JoinSet::new();
        for connection in connections {
            connection.closed.store(true, Ordering::SeqCst);
            connection.desktop_enabled.store(false, Ordering::SeqCst);
            connection
                .preview_cancel
                .send_modify(|n| *n = n.wrapping_add(1));
            connection.cancel.send_replace(true);
            connection
                .history_cancel
                .send_modify(|n| *n = n.wrapping_add(1));
            let _interaction = connection.native_interaction.lock().await;
            if let Some(mut history) = connection.history.lock().await.take() {
                history.channel.close().await;
            }
            let handle = connection.handle.lock().ok().and_then(|mut h| h.take());
            if let Some(handle) = handle {
                tasks.spawn(handle.shutdown());
            }
        }
        while tasks.join_next().await.is_some() {}
    }
}
