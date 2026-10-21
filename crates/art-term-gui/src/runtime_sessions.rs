//! Native runtime routing; credentials/grants never cross the WebView boundary.
use crate::runtime_hosts::{HostError, RuntimeHosts};
use anyhow::Context;
use art_term_core::{
    openab::{
        api::{ApiError, SessionGrant},
        history::{self, HistoryChannel, HistoryReply, HistoryRequest},
        profiles::HostSnapshot,
        screenshot::{self, Mode, ShellOwnership},
        secret::Secret,
        session::{OutputEvent, Recovery, SessionBinding, SessionError, SessionHandle},
    },
    runtime_preview::{DesktopRoute, RuntimeMapping},
    session_io::{DesktopStateDto, HostDto, ScreenshotDto, SessionDto, TargetDto},
};
use serde::Serialize;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::State;
use tokio::sync::watch;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSelection {
    pub desktop: art_term_core::instance_mcp::DesktopProfile,
    pub tool_ready: bool,
}

/// Native Runtime preview bridge. Production operations enforce live binding,
/// origin, identity, revision and generation fences before and after HTTPS I/O.
/// Test doubles may override native methods for lifecycle verification.
pub trait RuntimePreview: Send + Sync {
    /// Exact canonical-origin → binding mapping (`Some` for an endpoint
    /// `runtime_origin` or a declared `runtime_agents` origin), or
    /// `None` when the origin is not an explicitly trusted binding.
    fn mapped<'a>(
        &'a self,
        canonical_origin: &'a str,
    ) -> Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    >;
    /// Readiness check on the shared desktop through the mapped
    /// endpoint — never a provisioning operation.
    #[cfg(test)]
    fn enable<'a>(
        &'a self,
        _endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        Box::pin(async { anyhow::bail!("test helper unavailable") })
    }
    /// One bounded, validated screenshot through the mapped endpoint.
    #[cfg(test)]
    fn screenshot<'a>(
        &'a self,
        _endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        Box::pin(async { anyhow::bail!("test helper unavailable") })
    }
    fn native_status<'a>(
        &'a self,
        hosts: &'a RuntimeHosts,
        conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        Box::pin(async move { RuntimeSessions::native_status(hosts, conn).await })
    }
    fn native_screenshot<'a>(
        &'a self,
        hosts: &'a RuntimeHosts,
        conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        Box::pin(async move { RuntimeSessions::native_screenshot(hosts, conn).await })
    }
}

struct Target {
    binding: SessionBinding,
    grant: Mutex<Option<SessionGrant>>,
    /// Frozen mapped-preview binding captured at issue; `None` means
    /// this target can never offer desktop preview.
    desktop: Option<RuntimeMapping>,
    history_bootstrap: AtomicBool,
}
struct HistorySlot {
    channel: HistoryChannel,
    _permit: tokio::sync::OwnedSemaphorePermit,
    active: Arc<Mutex<HashSet<String>>>,
    active_key: String,
}
struct OpeningHistory {
    active: Arc<Mutex<HashSet<String>>>,
    registered: Arc<Mutex<Option<String>>>,
    armed: bool,
}
impl Drop for OpeningHistory {
    fn drop(&mut self) {
        if self.armed {
            if let (Ok(key), Ok(mut active)) = (self.registered.lock(), self.active.lock()) {
                if let Some(key) = key.as_ref() { active.remove(key); }
            }
        }
    }
}
struct HistoryOwnership {
    owned: OwnedShells,
    active: Arc<Mutex<HashSet<String>>>,
    registered: Arc<Mutex<Option<String>>>,
}
impl ShellOwnership for HistoryOwnership {
    fn mark(&self, name: &str) -> Result<(), SessionError> {
        let key = format!("{}:{name}", self.owned.profile_id);
        self.active.lock().map_err(|_| SessionError::Internal)?.insert(key.clone());
        *self.registered.lock().map_err(|_| SessionError::Internal)? = Some(key.clone());
        if let Err(error) = self.owned.mark(name) {
            if let Ok(mut active) = self.active.lock() { active.remove(&key); }
            return Err(error);
        }
        Ok(())
    }
    fn unmark(&self, name: &str) {
        self.owned.unmark(name);
        if let Ok(mut active) = self.active.lock() { active.remove(&format!("{}:{name}", self.owned.profile_id)); }
    }
}
impl Drop for HistorySlot {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() { active.remove(&self.active_key); }
    }
}
/// Live runtime session connection state. Opaque outside this module —
/// the frozen desktop route, enable/close fences, and live handle
/// binding never leave it.
pub struct Connection {
    handle: Mutex<Option<SessionHandle>>,
    recovery: Recovery,
    cancel: watch::Sender<bool>,
    next_generation: AtomicU64,
    /// Frozen at open from the verified target — the only route a
    /// live session's preview may ever use (legacy endpoint or the
    /// runtime's own `runtime-mcp` admin API; never both).
    desktop_route: Option<DesktopRoute>,
    desktop_enabled: AtomicBool,
    selected_desktop: Mutex<Option<art_term_core::instance_mcp::DesktopProfile>>,
    preview_cancel: watch::Sender<u64>,
    preview_operation: Arc<tokio::sync::Mutex<()>>,
    owned_shells: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    closed: AtomicBool,
    desktop_operation: tokio::sync::Mutex<()>,
    history_binding: Option<SessionBinding>,
    history: tokio::sync::Mutex<Option<HistorySlot>>,
    history_cancel: watch::Sender<u64>,
    native_interaction: tokio::sync::Mutex<()>,
    native_enabled: AtomicBool,
    native_held: AtomicBool,
    native_uncertain: AtomicBool,
}
#[derive(Default)]
pub struct RuntimeSessions {
    targets: Mutex<HashMap<String, Arc<Target>>>,
    sessions: Mutex<HashMap<String, Arc<Connection>>>,
    next: AtomicU64,
    /// One in-flight screenshot across every runtime session.
    screenshot_operation: tokio::sync::Mutex<()>,
    preview_operation: Arc<tokio::sync::Mutex<()>>,
    /// App-owned screenshot Shells still pending remote cleanup, keyed by
    /// Host profile id. Marked names are hidden from the selectable list
    /// and retried on later operations instead of being forgotten.
    owned_shells: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    history_owned: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    history_active: Arc<Mutex<HashSet<String>>>,
    history_slots: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
}
/// Fixed-profile adapter over `RuntimeSessions::owned_shells` for the
/// core screenshot transport.
struct OwnedShells {
    shells: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    profile_id: String,
}
fn shell_label(name: &str) -> String {
    match (history::HistoryVersion::from_name(name), history::key_from_name(name)) {
        (Some(history::HistoryVersion::Paged), Some(key)) => format!("s-{}", &key[..8]),
        _ => name.to_owned(),
    }
}

fn ownership_key(profile: &art_term_core::openab::profiles::HostProfile) -> String {
    format!("{}:{}:{}", profile.id, profile.revision, profile.base_url)
}
impl ShellOwnership for OwnedShells {
    fn mark(&self, name: &str) -> Result<(), SessionError> {
        let mut shells = self.shells.lock().map_err(|_| SessionError::Internal)?;
        let total: usize = shells.values().map(HashSet::len).sum();
        if total >= 64 {
            return Err(SessionError::Capacity);
        }
        shells
            .entry(self.profile_id.clone())
            .or_default()
            .insert(name.into());
        Ok(())
    }
    fn unmark(&self, name: &str) {
        if let Ok(mut shells) = self.shells.lock() {
            if let Some(names) = shells.get_mut(&self.profile_id) {
                names.remove(name);
                if names.is_empty() {
                    shells.remove(&self.profile_id);
                }
            }
        }
    }
}
#[derive(Debug, Serialize)]
pub struct SessionFailure {
    kind: SessionError,
    message: &'static str,
}
impl From<SessionError> for SessionFailure {
    fn from(kind: SessionError) -> Self {
        let message = match kind {
            SessionError::Auth => "Shell 認證被拒絕。",
            SessionError::Tls => "WSS 憑證驗證失敗。",
            SessionError::GatewayUnavailable => "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。",
            SessionError::UpgradeRequired => "主機 runtime 需要升級。",
            SessionError::Replacement => "遠端 runtime 或 Shell 已更換，請重新選擇。",
            SessionError::Missing | SessionError::Exited => "遠端 Shell 已結束或不存在。",
            SessionError::TtlExpired => "遠端 Shell 已到期。",
            SessionError::TakenOver => "此 Shell 已由另一個連線接管。",
            SessionError::Renewed => "此 Shell 的連線憑證已由其他操作更新。",
            SessionError::OperatorKilled => "遠端 Shell 已被管理者結束。",
            SessionError::Capacity | SessionError::SlowConsumer => {
                "連線佇列或 runtime 容量已達上限。"
            }
            SessionError::InputUncertain => "輸入是否送達尚未確認，未自動重送。",
            SessionError::Timeout | SessionError::Transport | SessionError::RetryExhausted => {
                "Shell 連線中斷或逾時，請確認網路後重新連線。"
            }
            SessionError::StateChanged | SessionError::Busy | SessionError::Cancelled => {
                "連線狀態已變更，請稍後重新操作。"
            }
            _ => "Shell 通訊失敗。",
        };
        Self { kind, message }
    }
}
type Reply<T> = Result<T, SessionFailure>;
impl RuntimeSessions {
    fn id(&self, prefix: &str) -> String {
        format!(
            "runtime-{prefix}-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn issue(
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
                coding_cli: target.desktop.as_ref().and_then(|m| m.coding_cli.clone()),
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
            coding_cli: desktop.as_ref().and_then(|m| m.coding_cli.clone()),
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
    fn connection(&self, id: &str) -> Reply<Arc<Connection>> {
        self.sessions
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(id)
            .cloned()
            .ok_or(SessionError::StateChanged.into())
    }
    fn prune_list(
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
            let key = history::key_from_name(&history_binding.name).ok_or(SessionError::Protocol)?;
            let version = history::HistoryVersion::from_name(&history_binding.name).ok_or(SessionError::Protocol)?;
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
    fn direct_write(connection: &Connection, bytes: &[u8]) -> Reply<()> {
        if connection.closed.load(Ordering::SeqCst) { return Err(SessionError::Cancelled.into()); }
        let guard = connection.handle.lock().map_err(|_| SessionError::Internal)?;
        guard.as_ref().ok_or(SessionError::Busy)?.write(bytes).map_err(Into::into)
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
        if *connection.history_cancel.borrow() != history_generation { return Err(SessionError::Cancelled.into()); }
        if connection.closed.load(Ordering::SeqCst) { return Err(SessionError::Cancelled.into()); }
        if connection.native_uncertain.load(Ordering::SeqCst) { return Err(SessionError::InputUncertain.into()); }
        if !connection.native_held.load(Ordering::SeqCst) { return Self::direct_write(&connection, bytes); }
        if bytes.len() > 1024 * 1024 { return Err(SessionError::Capacity.into()); }
        let mut slot = connection.history.lock().await;
        if connection.closed.load(Ordering::SeqCst) || *connection.history_cancel.borrow() != history_generation { return Err(SessionError::Cancelled.into()); }
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
            Ok(HistoryReply::View { held: false, offset: 0, available: true, .. }) => {
                connection.native_held.store(false, Ordering::SeqCst);
                connection.native_uncertain.store(false, Ordering::SeqCst);
                // Long CLI commands still remain one ordered interaction; no second sender.
                if length < bytes.len() { Self::direct_write(&connection, &bytes[length..])?; }
                Ok(())
            }
            _ => {
                if let Some(mut owned) = slot.take() {
                    if owned.channel.close().await { connection.native_held.store(false, Ordering::SeqCst); connection.native_uncertain.store(false, Ordering::SeqCst); }
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
        connection.history_cancel.send_modify(|n| *n = n.wrapping_add(1));
        let _interaction = connection.native_interaction.lock().await;
        if let Some(mut owned) = connection.history.lock().await.take() {
            if owned.channel.close().await { connection.native_held.store(false, Ordering::SeqCst); connection.native_uncertain.store(false, Ordering::SeqCst); }
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
        let has_direct_selection = conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?.is_some();
        if has_direct_selection {
            if enabled {
                let epoch = *conn.preview_cancel.borrow();
                let snapshot = Self::direct_snapshot(hosts, &conn)?;
                let selected = snapshot.profile.clone();
                art_term_core::instance_mcp::screenshot(snapshot).await?;
                anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch && Self::direct_snapshot(hosts, &conn)?.profile == selected, "桌面已切換");
            }
            conn.desktop_enabled.store(enabled, Ordering::SeqCst);
            return Ok(DesktopStateDto { enabled, message: if enabled { "App 直接預覽已啟用" } else { "預覽已停止；桌面工具保留" }.into() });
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
        let has_direct_selection = conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?.is_some();
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
    fn native_binding(
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
    fn native_still_current(
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
    fn verify_preview_binding(
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
    async fn native_preview(
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
            api,
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
    async fn native_status(
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
    async fn native_screenshot(
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
    pub async fn history_request(&self, hosts: &RuntimeHosts, id: &str, request: HistoryRequest) -> Reply<HistoryReply> {
        let conn = self.connection(id)?;
        let Some(binding) = conn.history_binding.as_ref() else { return Ok(HistoryReply::Unsupported) };
        let Some(key) = history::key_from_name(&binding.name) else { return Ok(HistoryReply::Unsupported) };
        let version = history::HistoryVersion::from_name(&binding.name).ok_or(SessionError::Protocol)?;
        if matches!(request, HistoryRequest::Follow { input: Some(_) }) { return Err(SessionError::Protocol.into()); }
        let native = matches!(request, HistoryRequest::Scroll { .. } | HistoryRequest::Follow { .. });
        let closing_history = matches!(request, HistoryRequest::Close);
        let was_uncertain = conn.native_uncertain.load(Ordering::SeqCst);
        if native {
            // Prewarm must fence input too: another retained helper may already own copy-mode.
            conn.native_uncertain.store(true, Ordering::SeqCst);
            conn.native_enabled.store(true, Ordering::SeqCst);
        }
        if closing_history { conn.history_cancel.send_modify(|n| *n = n.wrapping_add(1)); }
        let history_generation = *conn.history_cancel.borrow();
        let _interaction = if native || closing_history || conn.native_enabled.load(Ordering::SeqCst) { Some(conn.native_interaction.lock().await) } else { None };
        if *conn.history_cancel.borrow() != history_generation { return Err(SessionError::Cancelled.into()); }
        let mut slot = if closing_history { conn.history.lock().await } else { conn.history.try_lock().map_err(|_| SessionError::Busy)? };
        if closing_history {
            if let Some(mut owned) = slot.take() {
                if owned.channel.close().await { conn.native_held.store(false, Ordering::SeqCst); conn.native_uncertain.store(false, Ordering::SeqCst); }
            }
            return Ok(HistoryReply::Unsupported);
        }
        if conn.closed.load(Ordering::SeqCst) { return Err(SessionError::Cancelled.into()) }
        if slot.is_none() && !matches!(request, HistoryRequest::Viewport | HistoryRequest::Scroll { .. } | HistoryRequest::Follow { .. }) {
            return Ok(HistoryReply::Error { error: "expired".into() });
        }
        // The frozen profile stays available while recovery temporarily takes
        // the work handle. Such a pause is retryable, never a new binding.
        if conn.handle.lock().map_err(|_| SessionError::Internal)?.is_none() { return Err(SessionError::Busy.into()) }
        let store = hosts.store().map_err(SessionError::from)?;
        if !store.is_current(&binding.profile).map_err(SessionError::from)? { return Err(SessionError::StateChanged.into()) }
        let snapshot = store.snapshot(&binding.profile.id).map_err(SessionError::from)?;
        if snapshot.profile != binding.profile { return Err(SessionError::StateChanged.into()) }
        let mut cancel = conn.cancel.subscribe();
        let mut history_cancel = conn.history_cancel.subscribe();
        let generation = conn.next_generation.load(Ordering::SeqCst);
        let operation = async {
            if slot.is_none() {
                let host_key = ownership_key(&binding.profile);
                let semaphore = {
                    let mut slots = self.history_slots.lock().map_err(|_| SessionError::Internal)?;
                    if slots.len() >= 512 && !slots.contains_key(&host_key) { return Err(SessionError::Capacity) }
                    slots.entry(host_key.clone()).or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(2))).clone()
                };
                let permit = semaphore.try_acquire_owned().map_err(|_| SessionError::Capacity)?;
                let mut opening = OpeningHistory { active: self.history_active.clone(), registered: Arc::new(Mutex::new(None)), armed: true };
                let ownership = Arc::new(HistoryOwnership {
                    owned: OwnedShells { shells: self.history_owned.clone(), profile_id: host_key.clone() },
                    active: self.history_active.clone(), registered: opening.registered.clone(),
                });
                let pending = self.history_owned.lock().map_err(|_| SessionError::Internal)?.get(&host_key).cloned().unwrap_or_default();
                let active = self.history_active.lock().map_err(|_| SessionError::Internal)?.clone();
                let api = hosts.admin().map_err(SessionError::from)?;
                for name in pending.into_iter().filter(|name| !active.contains(&format!("{host_key}:{name}"))).take(4) {
                    if matches!(tokio::time::timeout(std::time::Duration::from_secs(2), api.delete(&snapshot, &name)).await,
                        Ok(Ok(())) | Ok(Err(ApiError::SessionMissing))) { ownership.unmark(&name); }
                }
                let channel = HistoryChannel::open(api, snapshot, &key, version, ownership, |binding, token, since, generation| {
                    hosts.attach_token(binding, token, since, generation)
                }).await?;
                let active_key = format!("{host_key}:{}", channel.helper_name());
                self.history_active.lock().map_err(|_| SessionError::Internal)?.insert(active_key.clone());
                *slot = Some(HistorySlot { channel, _permit: permit, active: self.history_active.clone(), active_key });
                opening.armed = false;
            }
            slot.as_mut().ok_or(SessionError::Internal)?.channel.request(request).await
        };
        let result = tokio::select! { biased;
            _ = cancel.changed() => Err(SessionError::Cancelled),
            _ = history_cancel.changed() => Err(SessionError::Cancelled),
            result = operation => result,
        };
        let current = !conn.closed.load(Ordering::SeqCst) && generation == conn.next_generation.load(Ordering::SeqCst) && *conn.history_cancel.borrow() == history_generation && store.is_current(&binding.profile).map_err(SessionError::from)?;
        if result.is_err() || !current {
            if let Some(mut owned) = slot.take() {
                if owned.channel.close().await { conn.native_held.store(false, Ordering::SeqCst); conn.native_uncertain.store(false, Ordering::SeqCst); }
            }
        }
        if !current { return Err(SessionError::StateChanged.into()) }
        if let Ok(HistoryReply::View { held, .. }) = &result { conn.native_held.store(*held, Ordering::SeqCst); conn.native_uncertain.store(false, Ordering::SeqCst); }
        match result {
            Ok(HistoryReply::Unsupported) => {
                // An old helper never entered native mode; retain any earlier ambiguous lease fence.
                if native && !was_uncertain && slot.as_ref().is_some_and(|owned| !owned.channel.native_supported()) { conn.native_uncertain.store(false, Ordering::SeqCst); }
                Ok(HistoryReply::Error { error: "unsupported".into() })
            },
            Err(SessionError::Missing) => Ok(HistoryReply::Error { error: "unavailable".into() }),
            Err(SessionError::UpgradeRequired) => Ok(HistoryReply::Error { error: "unsupported".into() }),
            other => other.map_err(Into::into),
        }
    }
    async fn configure_tools(
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
            hosts
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
    fn direct_snapshot(
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

    async fn instance_screenshot(hosts: &RuntimeHosts, conn: &Arc<Connection>) -> anyhow::Result<ScreenshotDto> {
        let epoch = *conn.preview_cancel.borrow();
        let mut cancel = conn.preview_cancel.subscribe();
        let snapshot = Self::direct_snapshot(hosts, conn)?;
        let selected = snapshot.profile.clone();
        let shot = tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面預覽已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => result?,
        };
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && conn.desktop_enabled.load(Ordering::SeqCst)
            && *conn.preview_cancel.borrow() == epoch && Self::direct_snapshot(hosts, conn)?.profile == selected,
            "桌面或連線已變更");
        Ok(ScreenshotDto { mime: shot.mime, base64: shot.image_base64, source: selected.name,
            desktop_id: Some(selected.id), desktop_revision: Some(selected.revision),
            captured_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64 })
    }
    fn tools_confirmed(result: &serde_json::Value, profile: &art_term_core::instance_mcp::DesktopProfile) -> bool {
        let Some(count) = result["adapters"].as_u64().filter(|n| *n > 0) else { return false; };
        result["ok"] == true && result["targets"].as_array().is_some_and(|targets| targets.len() as u64 == count
            && targets.iter().all(|target| target["id"] == profile.id && target["revision"] == profile.revision
                && target["generation"].as_u64().is_some_and(|n| n > 0)))
    }
    async fn configure_selected(hosts: &RuntimeHosts, conn: &Arc<Connection>, profile: &art_term_core::instance_mcp::DesktopProfile) -> anyhow::Result<bool> {
        let snapshot = hosts.store().map_err(|_| anyhow::anyhow!("設定不可用"))?.desktop(&profile.id)
            .map_err(|_| anyhow::anyhow!("桌面已變更"))?;
        anyhow::ensure!(snapshot.profile == *profile, "桌面已變更");
        let payload = zeroize::Zeroizing::new(serde_json::json!({"op":"configure","id":profile.id,"revision":profile.revision,"name":profile.name,"url":profile.mcp_url,"token":snapshot.token.expose()}).to_string());
        // Preview is independent of an installed/running adapter. No false tool-ready claim.
        Ok(Self::configure_tools(hosts, conn, payload).await.map(|result| Self::tools_confirmed(&result, profile)).unwrap_or(false))
    }
    pub async fn restore_instance(&self, hosts: &RuntimeHosts, id: &str) -> anyhow::Result<Option<InstanceSelection>> {
        let conn = self.connection(id).map_err(|_| anyhow::anyhow!("連線已結束"))?;
        let _guard = conn.desktop_operation.lock().await;
        let epoch = *conn.preview_cancel.borrow();
        let mut cancel = conn.preview_cancel.subscribe();
        let status = Self::configure_tools(hosts, &conn, zeroize::Zeroizing::new("{\"op\":\"current\"}".into()))
            .await.context("Shell broker 查詢失敗")?;
        let store = hosts.store().map_err(|_| anyhow::anyhow!("設定不可用"))?;
        let desktops = store.desktops().context("桌面清單讀取失敗")?;
        let Some(selection) = crate::instance_restore::resolve(&status, &desktops)? else { return Ok(None) };
        let snapshot = store.desktop(&selection.profile.id).context("桌面憑證讀取失敗")?;
        anyhow::ensure!(snapshot.profile == selection.profile, "桌面設定已變更");
        tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面恢復已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => { result.context("桌面預覽重新驗證失敗")?; }
        }
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch, "桌面恢復已取消");
        // Restoring a local preview must not change the live Shell's identity,
        // credentials or generation, including an assignment from another App.
        let probe = zeroize::Zeroizing::new(serde_json::json!({"op":"verify","expected_targets":selection.broker_targets}).to_string());
        let tool_ready = Self::configure_tools(hosts, &conn, probe).await
            .is_ok_and(|result| result["targets"] == status["targets"]);
        let current = Self::configure_tools(hosts, &conn, zeroize::Zeroizing::new("{\"op\":\"current\"}".into()))
            .await.context("Shell broker 重新核對失敗")?;
        let latest = store.desktop(&selection.profile.id).context("桌面憑證讀取失敗")?.profile;
        let profile = selection.confirm(&current, &latest, epoch, *conn.preview_cancel.borrow(), conn.closed.load(Ordering::SeqCst))?;
        *conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = Some(profile.clone());
        conn.desktop_enabled.store(true, Ordering::SeqCst);
        Ok(Some(InstanceSelection { desktop: profile, tool_ready }))
    }
    pub async fn select_instance(&self, hosts: &RuntimeHosts, id: &str, desktop_id: &str, revision: u64) -> anyhow::Result<InstanceSelection> {
        let conn = self.connection(id).map_err(|_| anyhow::anyhow!("連線已結束"))?;
        conn.desktop_enabled.store(false, Ordering::SeqCst);
        conn.preview_cancel.send_modify(|n| *n = n.wrapping_add(1));
        let epoch = *conn.preview_cancel.borrow();
        let _guard = conn.desktop_operation.lock().await;
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch, "桌面選擇已取消");
        // A live binding, rather than a saved Host capability, authorizes this per-tab selection.
        Self::native_binding(hosts, &conn)?;
        let previous = conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))?.take();
        if previous.is_some() {
            let _ = Self::configure_tools(hosts, &conn, zeroize::Zeroizing::new("{\"op\":\"clear\"}".into())).await;
        }
        let snapshot = hosts.store().map_err(|_| anyhow::anyhow!("設定不可用"))?.desktop(desktop_id).map_err(|_| anyhow::anyhow!("桌面已變更"))?;
        anyhow::ensure!(snapshot.profile.revision == revision, "桌面已變更");
        let profile = snapshot.profile.clone();
        let mut cancel = conn.preview_cancel.subscribe();
        tokio::select! {
            _ = cancel.changed() => anyhow::bail!("桌面選擇已取消"),
            result = art_term_core::instance_mcp::screenshot(snapshot) => { result?; }
        }
        anyhow::ensure!(*conn.preview_cancel.borrow() == epoch, "桌面選擇已取消");
        let tool_ready = Self::configure_selected(hosts, &conn, &profile).await?;
        anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch
            && hosts.store().map_err(|_| anyhow::anyhow!("設定不可用"))?.desktop(desktop_id).map_err(|_| anyhow::anyhow!("桌面已變更"))?.profile == profile,
            "桌面或連線已變更");
        *conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = Some(profile.clone());
        conn.desktop_enabled.store(true, Ordering::SeqCst);
        Ok(InstanceSelection { desktop: profile, tool_ready })
    }
    pub async fn clear_instance(&self, hosts: &RuntimeHosts, id: &str) -> anyhow::Result<()> {
        let conn = self.connection(id).map_err(|_| anyhow::anyhow!("連線已結束"))?;
        conn.desktop_enabled.store(false, Ordering::SeqCst);
        conn.preview_cancel.send_modify(|n| *n = n.wrapping_add(1));
        let _guard = conn.desktop_operation.lock().await;
        *conn.selected_desktop.lock().map_err(|_| anyhow::anyhow!("桌面狀態不可用"))? = None;
        Self::configure_tools(hosts, &conn, zeroize::Zeroizing::new("{\"op\":\"clear\"}".into())).await?;
        Ok(())
    }
    pub async fn launch_instance(&self, hosts: &RuntimeHosts, id: &str) -> anyhow::Result<InstanceSelection> {
        let conn = self.connection(id).map_err(|_| anyhow::anyhow!("連線已結束"))?;
        let _guard = conn.desktop_operation.try_lock().map_err(|_| anyhow::anyhow!("桌面操作進行中"))?;
        let profile = Self::direct_snapshot(hosts, &conn)?.profile;
        let epoch = *conn.preview_cancel.borrow();
        if Self::configure_selected(hosts, &conn, &profile).await? {
            return Ok(InstanceSelection { desktop: profile, tool_ready: true });
        }
        let (binding, _) = Self::native_binding(hosts, &conn)?;
        let cli = binding.profile.metadata.as_ref().and_then(|m| m.coding_cli.clone())
            .unwrap_or(art_term_core::connections::CodingCli { id:"codex".into(), name:"Codex".into(), command:"codex".into(), args:vec![] });
        anyhow::ensure!(std::path::Path::new(&cli.command).file_name().and_then(|name| name.to_str()) == Some("codex"), "此 CLI 尚未支援桌面工具入口；本輪支援 Codex");
        let quote = |word: &str| format!("'{}'", word.replace('\'', "'\\''"));
        let words = std::iter::once(cli.command.as_str()).chain(cli.args.iter().map(String::as_str)).map(quote).collect::<Vec<_>>().join(" ");
        let command = format!("python3 \"$HOME/.local/share/art-term/tools/instance_bridge.py\" launch --binding {} -- {}\r", binding.name, words);
        anyhow::ensure!(*conn.preview_cancel.borrow() == epoch && Self::direct_snapshot(hosts, &conn)?.profile == profile, "桌面已切換");
        self.write(id, command.as_bytes(), false).await.map_err(|_| anyhow::anyhow!("無法啟動 CLI"))?;
        // Exactly one explicit launch. Only configuration is retried while Codex starts stdio.
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            anyhow::ensure!(!conn.closed.load(Ordering::SeqCst) && *conn.preview_cancel.borrow() == epoch && Self::direct_snapshot(hosts, &conn)?.profile == profile, "桌面已切換");
            if Self::configure_selected(hosts, &conn, &profile).await? { return Ok(InstanceSelection { desktop: profile, tool_ready: true }); }
        }
        anyhow::bail!("Codex 工具入口尚未確認啟動；請查看終端，桌面預覽仍可使用")
    }

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
            connection.history_cancel.send_modify(|n| *n = n.wrapping_add(1));
            let _interaction = connection.native_interaction.lock().await;
            if let Some(mut history) = connection.history.lock().await.take() { history.channel.close().await; }
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
            connection.history_cancel.send_modify(|n| *n = n.wrapping_add(1));
            let _interaction = connection.native_interaction.lock().await;
            if let Some(mut history) = connection.history.lock().await.take() { history.channel.close().await; }
            let handle = connection.handle.lock().ok().and_then(|mut h| h.take());
            if let Some(handle) = handle {
                tasks.spawn(handle.shutdown());
            }
        }
        while tasks.join_next().await.is_some() {}
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiOutput {
    pub bytes: Vec<u8>,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeOutput>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOutput {
    batch_id: u64,
    local_generation: u64,
    events: Vec<OutputEvent>,
    ended: Option<SessionError>,
    input_uncertain: bool,
}

#[tauri::command]
pub async fn gui_runtime_sessions(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    id: String,
    expected_revision: u64,
) -> Result<HostDto, HostError> {
    let snapshot = hosts.store()?.snapshot(&id)?;
    if snapshot.profile.revision != expected_revision {
        return Err(ApiError::StateChanged.into());
    }
    let list = hosts.admin()?.list(&snapshot).await?;
    if !hosts.store()?.is_current(&snapshot.profile)? {
        return Err(ApiError::StateChanged.into());
    }
    sessions.prune_list(
        &snapshot.profile,
        &list
            .sessions
            .iter()
            .filter(|item| item.alive)
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
    )?;
    // Exact canonical-origin mapping only — an unmapped runtime Host
    // simply reports no desktop availability (deny is the default).
    let mapping = if let Some(local) = snapshot.profile.metadata.as_ref() {
        Some(art_term_core::runtime_preview::mapping_from_metadata(
            &art_term_core::runtime_preview::resolve_metadata(
                Some(local),
                hosts.connections.as_ref(),
                &snapshot.profile.base_url,
            ),
        ))
    } else {
        preview
            .mapped(&snapshot.profile.base_url)
            .await
            .ok()
            .flatten()
    };
    let mut targets = Vec::new();
    for item in &list.sessions {
        if item.alive
            && !sessions
                .owned_shells
                .lock()
                .map_err(|_| ApiError::Storage)?
                .get(&ownership_key(&snapshot.profile))
                .is_some_and(|names| names.contains(&item.name))
            && !sessions.history_owned.lock().map_err(|_| ApiError::Storage)?
                .get(&ownership_key(&snapshot.profile)).is_some_and(|names| names.contains(&item.name))
        {
            let binding = SessionBinding::from_list(snapshot.profile.clone(), &list, &item.name)?;
            targets.push(sessions.issue(binding, None, item.attached, mapping.clone())?);
        }
    }
    Ok(HostDto {
        id,
        label: snapshot.profile.name,
        desktop_available: mapping.as_ref().is_some_and(|m| m.route.is_some()),
        targets,
    })
}
#[tauri::command]
pub async fn gui_runtime_create_session(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    id: String,
    expected_revision: u64,
    name: String,
) -> Result<TargetDto, HostError> {
    let snapshot = hosts.store()?.snapshot(&id)?;
    if snapshot.profile.revision != expected_revision {
        return Err(ApiError::StateChanged.into());
    }
    let grant = hosts.admin()?.create(&snapshot, &name).await?;
    let binding = SessionBinding::from_grant(snapshot.profile.clone(), &grant)?;
    if !hosts.store()?.is_current(&snapshot.profile)? {
        return Err(ApiError::StateChanged.into());
    }
    let mapping = if let Some(local) = snapshot.profile.metadata.as_ref() {
        Some(art_term_core::runtime_preview::mapping_from_metadata(
            &art_term_core::runtime_preview::resolve_metadata(
                Some(local),
                hosts.connections.as_ref(),
                &snapshot.profile.base_url,
            ),
        ))
    } else {
        preview
            .mapped(&snapshot.profile.base_url)
            .await
            .ok()
            .flatten()
    };
    sessions
        .issue(binding, Some(grant), false, mapping)
        .map_err(Into::into)
}
#[tauri::command]
pub async fn gui_runtime_create_history_session(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    id: String,
    expected_revision: u64,
) -> Result<TargetDto, HostError> {
    let native_sessions = sessions.inner().clone();
    let raw = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("th2-{}", &raw[..24]);
    let target = gui_runtime_create_session(hosts, sessions, preview, id, expected_revision, name).await?;
    let targets = native_sessions.targets.lock().map_err(|_| ApiError::Storage)?;
    targets.get(&target.id).ok_or(ApiError::StateChanged)?.history_bootstrap.store(true, Ordering::SeqCst);
    Ok(target)
}
#[tauri::command]
pub async fn gui_runtime_history(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
    request: HistoryRequest,
) -> Reply<HistoryReply> {
    sessions.history_request(&hosts, &session_id, request).await
}
#[tauri::command]
pub async fn gui_runtime_ack(
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
    batch_id: u64,
    local_generation: u64,
) -> Reply<()> {
    sessions.ack(&session_id, batch_id, local_generation)
}
#[tauri::command]
pub async fn gui_runtime_recover(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
) -> Reply<()> {
    sessions.recover(&hosts, &session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_failure_serializes_a_fixed_local_message() {
        let value = serde_json::to_value(SessionFailure::from(SessionError::GatewayUnavailable)).unwrap();
        assert_eq!(value["kind"], "gatewayUnavailable");
        assert_eq!(
            value["message"],
            "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。"
        );
    }
    use art_term_core::openab::profiles::HostProfile;
    fn binding() -> SessionBinding {
        SessionBinding {
            profile: HostProfile {
                id: "fixture-host".into(),
                revision: 1,
                name: "Fixture".into(),
                base_url: "https://example.invalid".into(),
                credential_present: true,
                metadata: None,
            },
            name: "fixture-shell".into(),
            generation: 1,
        }
    }
    #[test]
    fn preview_ownership_is_revision_scoped_and_capacity_refuses_untracked_create() {
        let shells = Arc::new(Mutex::new(HashMap::new()));
        let profile = binding().profile;
        let registry = OwnedShells {
            shells: shells.clone(),
            profile_id: ownership_key(&profile),
        };
        registry.mark("mcshot-owned").unwrap();
        let mut changed = profile.clone();
        changed.revision += 1;
        assert_ne!(ownership_key(&profile), ownership_key(&changed));
        assert!(!shells
            .lock()
            .unwrap()
            .contains_key(&ownership_key(&changed)));
        assert!(!shells
            .lock()
            .unwrap()
            .get(&ownership_key(&profile))
            .unwrap()
            .contains("mcshot-foreign"));
        for n in 1..64 {
            registry.mark(&format!("mcshot-{n}")).unwrap();
        }
        assert_eq!(
            registry.mark("mcshot-over-capacity"),
            Err(SessionError::Capacity)
        );
        assert!(!shells
            .lock()
            .unwrap()
            .values()
            .any(|names| names.contains("mcshot-over-capacity")));
        registry.unmark("mcshot-owned");
        registry.mark("mcshot-over-capacity").unwrap();
    }
    struct GuardedPreview;
    impl RuntimePreview for GuardedPreview {
        fn mapped<'a>(
            &'a self,
            origin: &'a str,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>>
                    + Send
                    + 'a,
            >,
        > {
            FakePreview.mapped(origin)
        }
        fn enable<'a>(
            &'a self,
            _: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            panic!("provider must not run without live binding")
        }
        fn screenshot<'a>(
            &'a self,
            _: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            panic!("provider must not run without live binding")
        }
    }
    struct FakePreview;
    impl RuntimePreview for FakePreview {
        fn native_status<'a>(
            &'a self,
            _hosts: &'a RuntimeHosts,
            _conn: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            self.enable("native")
        }
        fn native_screenshot<'a>(
            &'a self,
            _hosts: &'a RuntimeHosts,
            _conn: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            self.screenshot("native")
        }

        fn mapped<'a>(
            &'a self,
            _origin: &'a str,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Ok(None) })
        }
        fn enable<'a>(
            &'a self,
            endpoint: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            let endpoint = endpoint.to_owned();
            Box::pin(async move {
                Ok(DesktopStateDto {
                    enabled: true,
                    message: endpoint,
                })
            })
        }
        fn screenshot<'a>(
            &'a self,
            endpoint: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            let endpoint = endpoint.to_owned();
            Box::pin(async move {
                Ok(ScreenshotDto {
                    desktop_id: None,
                    desktop_revision: None,
                    mime: "image/png".into(),
                    base64: String::new(),
                    source: endpoint,
                    captured_at: 0,
                })
            })
        }
    }
    /// Lifecycle route-seam fixture. Enrollment tests must install a real live handle.
    fn fake_connection(sessions: &RuntimeSessions, route: Option<DesktopRoute>) -> String {
        let id = sessions.id("session");
        let (cancel, _) = watch::channel(false);
        sessions.sessions.lock().unwrap().insert(
            id.clone(),
            Arc::new(Connection {
                handle: Mutex::new(None),
                recovery: Recovery::default(),
                cancel,
                next_generation: AtomicU64::new(2),
                desktop_route: route,
                desktop_enabled: AtomicBool::new(false),
                selected_desktop: Mutex::new(None),
                preview_cancel: watch::channel(0).0,
                preview_operation: Arc::default(),
                owned_shells: Arc::default(),
                closed: AtomicBool::new(false),
                desktop_operation: tokio::sync::Mutex::new(()),
                history_binding: None,
                history: tokio::sync::Mutex::new(None),
                    history_cancel: watch::channel(0).0,
                    native_interaction: tokio::sync::Mutex::new(()),
                    native_enabled: AtomicBool::new(false),
                    native_held: AtomicBool::new(false),
                    native_uncertain: AtomicBool::new(false),
            }),
        );
        id
    }
    #[tokio::test]
    async fn native_uncertain_without_helper_blocks_successive_user_input_but_not_protocol_route() {
        let sessions=RuntimeSessions::default();
        let id=fake_connection(&sessions,None);
        let connection=sessions.connection(&id).unwrap();
        connection.native_enabled.store(true,Ordering::SeqCst);
        connection.native_uncertain.store(true,Ordering::SeqCst);
        for _ in 0..2 {
            let error=sessions.write(&id,b"echo must-not-send\n",false).await.unwrap_err();
            assert_eq!(error.kind,SessionError::InputUncertain);
        }
        // With no handle, reaching the original protocol path yields Busy rather than uncertainty.
        assert_eq!(sessions.write(&id,b"\x1b[1;1R",true).await.unwrap_err().kind,SessionError::Busy);
    }
    #[tokio::test]
    async fn legacy_non_history_session_does_not_enable_native_input_fence() {
        let sessions=RuntimeSessions::default();
        let id=fake_connection(&sessions,None);
        assert!(matches!(sessions.history_request(&fake_hosts(),&id,HistoryRequest::Scroll {lines:0}).await.unwrap(),HistoryReply::Unsupported));
        let connection=sessions.connection(&id).unwrap();
        assert!(!connection.native_enabled.load(Ordering::SeqCst));
        assert!(!connection.native_uncertain.load(Ordering::SeqCst));
        assert_eq!(sessions.write(&id,b"echo legacy\n",false).await.unwrap_err().kind,SessionError::Busy);
    }
    /// Failed-store stub: preview-seam tests never touch it, and a real
    /// native route on it fails closed through the store error.
    fn fake_hosts() -> RuntimeHosts {
        RuntimeHosts {
            store: Err(ApiError::Storage),
            admin: Err(ApiError::Storage),
            connections: None,
            #[cfg(all(debug_assertions, feature = "openab-fixture"))]
            fixture_ca: Ok(None),
        }
    }
    #[test]
    fn repeated_list_reuses_target_without_cumulative_capacity_failure() {
        let sessions = RuntimeSessions::default();
        let initial = sessions.issue(binding(), None, false, None).unwrap().id;
        for generation in 2..=1024 {
            let mut current = binding();
            current.generation = generation;
            assert_eq!(
                sessions.issue(current, None, true, None).unwrap().id,
                initial
            );
        }
        assert_eq!(sessions.targets.lock().unwrap().len(), 1);
    }
    #[test]
    fn replacement_invalidates_old_target_and_missing_sessions_are_pruned() {
        let sessions = RuntimeSessions::default();
        let old = sessions.issue(binding(), None, false, None).unwrap().id;
        let mut replacement = binding();
        replacement.profile.revision += 1;
        let new = sessions.issue(replacement, None, false, None).unwrap().id;
        assert_ne!(old, new);
        assert!(!sessions.targets.lock().unwrap().contains_key(&old));
        sessions.prune_list(&binding().profile, &[]).unwrap();
        assert!(sessions.targets.lock().unwrap().is_empty());
    }
    #[test]
    fn mapped_issue_freezes_desktop_binding_and_unmapped_denies() {
        let sessions = RuntimeSessions::default();
        let mapping = RuntimeMapping {
            route: Some(DesktopRoute::Native),
            coding_cli: Some(art_term_core::connections::CodingCli {
                id: "fixture".into(),
                name: "Fixture CLI".into(),
                command: "printf".into(),
                args: vec!["hello".into()],
            }),
        };
        let dto = sessions
            .issue(binding(), None, false, Some(mapping))
            .unwrap();
        assert!(dto.desktop_available);
        assert_eq!(
            dto.coding_cli.as_ref().map(|c| c.id.as_str()),
            Some("fixture")
        );
        // A re-issue with NO mapping keeps the frozen binding.
        let same = sessions.issue(binding(), None, false, None).unwrap();
        assert_eq!(same.id, dto.id);
        assert!(same.desktop_available);
        assert_eq!(
            same.coding_cli.as_ref().map(|c| c.id.as_str()),
            Some("fixture")
        );
        let unmapped = RuntimeSessions::default();
        let dto = unmapped.issue(binding(), None, false, None).unwrap();
        assert!(!dto.desktop_available);
        assert!(dto.coding_cli.is_none());
    }
    #[tokio::test]
    async fn unmapped_session_desktop_and_screenshot_are_denied() {
        let sessions = RuntimeSessions::default();
        let id = fake_connection(&sessions, None);
        let preview = FakePreview;
        let hosts = fake_hosts();
        assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    }
    #[tokio::test]
    async fn mapped_session_enable_then_close_fences_late_calls() {
        mapped_session_enable_then_close_fences_late_calls_route(DesktopRoute::Native).await;
    }
    #[tokio::test]
    async fn mapped_session_enable_then_close_fences_late_calls_native() {
        mapped_session_enable_then_close_fences_late_calls_route(DesktopRoute::Native).await;
    }
    async fn mapped_session_enable_then_close_fences_late_calls_route(route: DesktopRoute) {
        let sessions = RuntimeSessions::default();
        let id = fake_connection(&sessions, Some(route.clone()));
        let preview = FakePreview;
        let hosts = fake_hosts();
        let state = sessions.desktop(&hosts, &id, true, &preview).await.unwrap();
        assert!(state.enabled);
        // Closed session: enable/disable/screenshot all fail closed,
        // and a late disable never calls the provider again.
        sessions.close(&id).await.unwrap();
        assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
        assert!(sessions
            .desktop(&hosts, &id, false, &preview)
            .await
            .is_err());
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    }
    #[tokio::test]
    async fn disable_is_local_only_and_screenshot_requires_enabled() {
        disable_is_local_only_and_screenshot_requires_enabled_route(DesktopRoute::Native).await;
    }
    #[tokio::test]
    async fn disable_is_local_only_and_screenshot_requires_enabled_native() {
        disable_is_local_only_and_screenshot_requires_enabled_route(DesktopRoute::Native).await;
    }
    async fn disable_is_local_only_and_screenshot_requires_enabled_route(route: DesktopRoute) {
        let sessions = RuntimeSessions::default();
        let id = fake_connection(&sessions, Some(route.clone()));
        let preview = FakePreview;
        let hosts = fake_hosts();
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
        sessions.desktop(&hosts, &id, true, &preview).await.unwrap();
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_ok());
        let state = sessions
            .desktop(&hosts, &id, false, &preview)
            .await
            .unwrap();
        assert!(!state.enabled);
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    }
    #[tokio::test]
    async fn native_route_fails_closed_without_endpoint_fallback() {
        let sessions = RuntimeSessions::default();
        let id = fake_connection(&sessions, Some(DesktopRoute::Native));
        let preview = GuardedPreview;
        let hosts = fake_hosts();
        // The native route binds to the live handle + HostStore only —
        // a failed binding/store check errors without ever consulting
        // the endpoint provider; no SSH fallback exists.
        assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    }
    struct DelayedPreview {
        started: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }
    impl RuntimePreview for DelayedPreview {
        fn native_status<'a>(
            &'a self,
            _hosts: &'a RuntimeHosts,
            _conn: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            self.enable("native")
        }
        fn native_screenshot<'a>(
            &'a self,
            _hosts: &'a RuntimeHosts,
            _conn: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            self.screenshot("native")
        }

        fn mapped<'a>(
            &'a self,
            origin: &'a str,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>>
                    + Send
                    + 'a,
            >,
        > {
            FakePreview.mapped(origin)
        }
        fn enable<'a>(
            &'a self,
            endpoint: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            FakePreview.enable(endpoint)
        }
        fn screenshot<'a>(
            &'a self,
            endpoint: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            Box::pin(async move {
                self.started.notify_one();
                self.resume.notified().await;
                FakePreview.screenshot(endpoint).await
            })
        }
    }
    #[tokio::test]
    async fn in_flight_screenshot_is_discarded_after_disable_or_close() {
        in_flight_screenshot_is_discarded_after_disable_or_close_route(DesktopRoute::Native).await;
    }
    #[tokio::test]
    async fn in_flight_screenshot_is_discarded_after_disable_or_close_native() {
        in_flight_screenshot_is_discarded_after_disable_or_close_route(DesktopRoute::Native).await;
    }
    async fn in_flight_screenshot_is_discarded_after_disable_or_close_route(route: DesktopRoute) {
        for close in [false, true] {
            let sessions = Arc::new(RuntimeSessions::default());
            let id = fake_connection(&sessions, Some(route.clone()));
            let preview = Arc::new(DelayedPreview {
                started: tokio::sync::Notify::new(),
                resume: tokio::sync::Notify::new(),
            });
            let hosts = Arc::new(fake_hosts());
            sessions
                .desktop(&hosts, &id, true, preview.as_ref())
                .await
                .unwrap();
            let task = tokio::spawn({
                let sessions = sessions.clone();
                let preview = preview.clone();
                let hosts = hosts.clone();
                let id = id.clone();
                async move { sessions.screenshot(&hosts, &id, preview.as_ref()).await }
            });
            preview.started.notified().await;
            if close {
                sessions.close(&id).await.unwrap();
            } else {
                sessions
                    .desktop(&hosts, &id, false, preview.as_ref())
                    .await
                    .unwrap();
            }
            preview.resume.notify_one();
            assert!(task.await.unwrap().is_err());
        }
    }
    #[tokio::test]
    async fn shutdown_releases_all_targets() {
        let sessions = RuntimeSessions::default();
        sessions.issue(binding(), None, false, None).unwrap();
        sessions.shutdown().await;
        assert!(sessions.targets.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn both_production_routes_reject_absent_handle_before_io() {
        for route in [DesktopRoute::Native] {
            let sessions = RuntimeSessions::default();
            let id = fake_connection(&sessions, Some(route));
            let hosts = fake_hosts();
            assert!(sessions
                .desktop(&hosts, &id, true, &GuardedPreview)
                .await
                .is_err());
            sessions
                .connection(&id)
                .unwrap()
                .desktop_enabled
                .store(true, Ordering::SeqCst);
            assert!(sessions
                .screenshot(&hosts, &id, &GuardedPreview)
                .await
                .is_err());
        }
    }
    #[test]
    fn production_binding_verifier_rejects_revision_identity_and_generation_drift() {
        let expected = binding();
        assert!(
            RuntimeSessions::verify_preview_binding(&expected, &expected, &expected.profile)
                .is_ok()
        );
        for field in 0..3 {
            let mut changed = expected.clone();
            match field {
                0 => changed.generation += 1,
                1 => changed.profile.revision += 1,
                _ => changed.name = "other".into(),
            }
            assert!(RuntimeSessions::verify_preview_binding(
                &expected,
                &changed,
                &expected.profile
            )
            .is_err());
        }
        let mut profile = expected.profile.clone();
        profile.revision += 1;
        assert!(RuntimeSessions::verify_preview_binding(&expected, &expected, &profile).is_err());
    }
    struct RouteCounters {
        native: AtomicU64,
        endpoint: AtomicU64,
    }
    impl RuntimePreview for RouteCounters {
        fn mapped<'a>(
            &'a self,
            origin: &'a str,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>>
                    + Send
                    + 'a,
            >,
        > {
            FakePreview.mapped(origin)
        }
        fn enable<'a>(
            &'a self,
            _: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            self.endpoint.fetch_add(1, Ordering::SeqCst);
            FakePreview.enable("endpoint")
        }
        fn screenshot<'a>(
            &'a self,
            _: &'a str,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            self.endpoint.fetch_add(1, Ordering::SeqCst);
            FakePreview.screenshot("endpoint")
        }
        fn native_status<'a>(
            &'a self,
            _: &'a RuntimeHosts,
            _: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
        {
            self.native.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { anyhow::bail!("native unavailable") })
        }
        fn native_screenshot<'a>(
            &'a self,
            _: &'a RuntimeHosts,
            _: &'a Arc<Connection>,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>>
        {
            self.native.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { anyhow::bail!("native unavailable") })
        }
    }
    #[tokio::test]
    async fn native_errors_never_call_endpoint_provider() {
        let sessions = RuntimeSessions::default();
        let hosts = fake_hosts();
        let id = fake_connection(&sessions, Some(DesktopRoute::Native));
        let preview = RouteCounters {
            native: AtomicU64::new(0),
            endpoint: AtomicU64::new(0),
        };
        assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
        sessions
            .connection(&id)
            .unwrap()
            .desktop_enabled
            .store(true, Ordering::SeqCst);
        assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
        assert_eq!(preview.native.load(Ordering::SeqCst), 2);
        assert_eq!(preview.endpoint.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn both_route_snapshots_survive_reissue_with_a_different_route() {
        for route in [DesktopRoute::Native] {
            let sessions = RuntimeSessions::default();
            let original = format!("{route:?}");
            let dto = sessions
                .issue(
                    binding(),
                    None,
                    false,
                    Some(RuntimeMapping {
                        route: Some(route),
                        coding_cli: None,
                    }),
                )
                .unwrap();
            sessions
                .issue(
                    binding(),
                    None,
                    false,
                    Some(RuntimeMapping {
                        route: None,
                        coding_cli: None,
                    }),
                )
                .unwrap();
            let targets = sessions.targets.lock().unwrap();
            assert_eq!(
                format!(
                    "{:?}",
                    targets[&dto.id]
                        .desktop
                        .as_ref()
                        .unwrap()
                        .route
                        .as_ref()
                        .unwrap()
                ),
                original
            );
        }
    }
}
