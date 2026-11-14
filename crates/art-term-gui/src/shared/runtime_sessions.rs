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
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
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
                if let Some(key) = key.as_ref() {
                    active.remove(key);
                }
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
        self.active
            .lock()
            .map_err(|_| SessionError::Internal)?
            .insert(key.clone());
        *self.registered.lock().map_err(|_| SessionError::Internal)? = Some(key.clone());
        if let Err(error) = self.owned.mark(name) {
            if let Ok(mut active) = self.active.lock() {
                active.remove(&key);
            }
            return Err(error);
        }
        Ok(())
    }
    fn unmark(&self, name: &str) {
        self.owned.unmark(name);
        if let Ok(mut active) = self.active.lock() {
            active.remove(&format!("{}:{name}", self.owned.profile_id));
        }
    }
}
impl Drop for HistorySlot {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.active_key);
        }
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
    match (
        history::HistoryVersion::from_name(name),
        history::key_from_name(name),
    ) {
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
            SessionError::GatewayUnavailable => {
                "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。"
            }
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
#[path = "runtime_sessions/instance.rs"]
mod instance;
#[path = "runtime_sessions/lifecycle.rs"]
mod lifecycle;
#[path = "runtime_sessions/preview.rs"]
mod preview;
#[path = "runtime_sessions/history.rs"]
mod session_history;
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
            && !sessions
                .history_owned
                .lock()
                .map_err(|_| ApiError::Storage)?
                .get(&ownership_key(&snapshot.profile))
                .is_some_and(|names| names.contains(&item.name))
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
    let target =
        gui_runtime_create_session(hosts, sessions, preview, id, expected_revision, name).await?;
    let targets = native_sessions
        .targets
        .lock()
        .map_err(|_| ApiError::Storage)?;
    targets
        .get(&target.id)
        .ok_or(ApiError::StateChanged)?
        .history_bootstrap
        .store(true, Ordering::SeqCst);
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
#[path = "runtime_sessions_tests.rs"]
mod tests;
