//! One native WSS owner per terminal. Byte cursors commit only after renderer ACK.
use super::{
    api::{ApiError, SessionGrant, SessionList},
    profiles::{HostProfile, HostSnapshot},
    secret::Secret,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    tungstenite::{
        self,
        client::IntoClientRequest,
        protocol::{Message, WebSocketConfig},
    },
    MaybeTlsStream, WebSocketStream,
};

const OUTPUT_BYTES: usize = 512 * 1024;
const SOCKET_BYTES: usize = 256 * 1024;
const OUTPUT_EVENTS: usize = 128;
const POLL_BYTES: usize = 64 * 1024;
const INPUT_BYTES: usize = 256 * 1024;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Local binding snapshot for one runtime Shell. Officially the session
/// name IS the identity — there is no runtime-instance or session-identity
/// field on the wire — so a binding pins only the profile revision, the
/// name, and the generation observed at issue time. It is never claimed
/// to prove continuity across a runtime incarnation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionBinding {
    pub profile: HostProfile,
    pub name: String,
    pub generation: u64,
}
impl SessionBinding {
    pub fn from_list(
        profile: HostProfile,
        list: &SessionList,
        name: &str,
    ) -> Result<Self, ApiError> {
        let item = list
            .sessions
            .iter()
            .find(|s| s.name == name)
            .ok_or(ApiError::SessionMissing)?;
        if item.generation == 0 {
            return Err(ApiError::Protocol);
        }
        if !item.alive {
            return Err(ApiError::SessionDead);
        }
        Ok(Self {
            profile,
            name: name.into(),
            generation: item.generation,
        })
    }
    pub fn from_grant(profile: HostProfile, grant: &SessionGrant) -> Result<Self, ApiError> {
        super::api::validate_session_name(&grant.session)?;
        if grant.generation == 0 {
            return Err(ApiError::Protocol);
        }
        Ok(Self {
            profile,
            name: grant.session.clone(),
            generation: grant.generation,
        })
    }
    /// Diagnostic list check: the name must still be present and alive,
    /// and a generation drift means the token family this binding was
    /// issued under is dead — recovery must stop, never rebind silently.
    pub fn verify_list(&self, list: &SessionList) -> Result<(), ApiError> {
        let other = Self::from_list(self.profile.clone(), list, &self.name)?;
        if self.generation != other.generation {
            return Err(ApiError::StateChanged);
        }
        Ok(())
    }
    pub fn verify_grant(
        &self,
        grant: &SessionGrant,
        exact_generation: bool,
    ) -> Result<(), ApiError> {
        let other = Self::from_grant(self.profile.clone(), grant)?;
        if self.name != other.name {
            return Err(ApiError::Replacement);
        }
        if exact_generation && self.generation != other.generation {
            return Err(ApiError::StateChanged);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionError {
    Auth,
    Tls,
    Transport,
    Timeout,
    Protocol,
    UpgradeRequired,
    Replacement,
    Missing,
    Exited,
    TtlExpired,
    TakenOver,
    Renewed,
    SlowConsumer,
    Capacity,
    Throttled,
    OperatorKilled,
    Internal,
    Cancelled,
    InputUncertain,
    RetryExhausted,
    Busy,
    StateChanged,
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auth => "Authentication rejected",
            Self::Tls => "TLS verification failed",
            Self::Transport => "Connection lost",
            Self::Timeout => "Operation timed out",
            Self::Protocol => "Invalid runtime protocol",
            Self::UpgradeRequired => "Runtime upgrade required",
            Self::Replacement => "Runtime or session replaced",
            Self::Missing => "Session missing",
            Self::Exited => "Shell exited",
            Self::TtlExpired => "Session lifetime expired",
            Self::TakenOver => "Another connection took over",
            Self::Renewed => "Token renewed",
            Self::SlowConsumer => "Output backlog exceeded",
            Self::Capacity => "Capacity unavailable",
            Self::Throttled => "Operation throttled",
            Self::OperatorKilled => "Operator terminated session",
            Self::Internal => "Runtime internal failure",
            Self::Cancelled => "Locally detached",
            Self::InputUncertain => "Input delivery uncertain",
            Self::RetryExhausted => "Recovery attempts exhausted",
            Self::Busy => "Operation already pending",
            Self::StateChanged => "Host or session state changed",
        })
    }
}
impl std::error::Error for SessionError {}
impl From<ApiError> for SessionError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Auth => Self::Auth,
            ApiError::Tls => Self::Tls,
            ApiError::Timeout => Self::Timeout,
            ApiError::Protocol => Self::Protocol,
            ApiError::UpgradeRequired => Self::UpgradeRequired,
            ApiError::Replacement => Self::Replacement,
            ApiError::SessionMissing => Self::Missing,
            ApiError::SessionDead => Self::Exited,
            ApiError::Capacity => Self::Capacity,
            ApiError::Throttled => Self::Throttled,
            ApiError::StateChanged => Self::StateChanged,
            _ => Self::Transport,
        }
    }
}
pub fn close_error(code: u16) -> SessionError {
    match code {
        4001 => SessionError::TtlExpired,
        4002 => SessionError::TakenOver,
        4003 => SessionError::Renewed,
        4004 => SessionError::Exited,
        4005 => SessionError::SlowConsumer,
        4006 => SessionError::Replacement,
        4007 => SessionError::Capacity,
        4008 => SessionError::OperatorKilled,
        4009 => SessionError::Internal,
        _ => SessionError::Transport,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutputEvent {
    Bytes {
        bytes: Vec<u8>,
        start: u64,
        end: u64,
        historical: bool,
    },
    #[serde(rename_all = "camelCase")]
    Gap {
        bytes_dropped: u64,
        resume_offset: u64,
    },
    #[serde(rename_all = "camelCase")]
    Notice {
        ephemeral_workspace: bool,
        teardown_best_effort: bool,
    },
    #[serde(rename_all = "camelCase")]
    TtlWarning {
        seconds_remaining: u64,
        absolute: bool,
    },
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPoll {
    pub batch_id: u64,
    pub local_generation: u64,
    pub events: Vec<OutputEvent>,
    pub ended: Option<SessionError>,
    pub input_uncertain: bool,
}
struct Shared {
    events: VecDeque<OutputEvent>,
    bytes: usize,
    acknowledged: u64,
    pending: Option<(u64, u64)>,
    batch: u64,
    ended: Option<SessionError>,
    input_uncertain: bool,
}
impl Shared {
    fn push(&mut self, event: OutputEvent) -> Result<(), SessionError> {
        let bytes = match &event {
            OutputEvent::Bytes { bytes, .. } => bytes.len(),
            _ => 0,
        };
        if self.events.len() >= OUTPUT_EVENTS || self.bytes.saturating_add(bytes) > OUTPUT_BYTES {
            return Err(SessionError::SlowConsumer);
        }
        self.bytes += bytes;
        self.events.push_back(event);
        Ok(())
    }
}
struct Input {
    bytes: Vec<u8>,
    reserved: Arc<AtomicUsize>,
}
impl Drop for Input {
    fn drop(&mut self) {
        self.reserved.fetch_sub(self.bytes.len(), Ordering::SeqCst);
    }
}
enum Command {
    Input(Input),
    Resize(u16, u16),
    Ping,
}
pub struct SessionHandle {
    binding: SessionBinding,
    /// Retained in native memory only — the same token is reused for a
    /// bounded same-session reattach after a transient drop. Never
    /// serialized into a DTO, a log, or disk.
    token: Option<Secret>,
    preview_flow: Arc<std::sync::atomic::AtomicBool>,
    output_ready: Arc<tokio::sync::Notify>,
    output_drained: Arc<tokio::sync::Notify>,
    local_generation: u64,
    shared: Arc<Mutex<Shared>>,
    commands: mpsc::Sender<Command>,
    cancel: watch::Sender<bool>,
    input_bytes: Arc<AtomicUsize>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl SessionHandle {
    #[cfg(any(test, all(debug_assertions, feature = "openab-fixture")))]
    fn fixture_connector(ca_der: &[u8]) -> Result<tokio_tungstenite::Connector, SessionError> {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls::pki_types::CertificateDer::from(ca_der.to_vec()))
            .map_err(|_| SessionError::Protocol)?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| SessionError::Protocol)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(tokio_tungstenite::Connector::Rustls(Arc::new(config)))
    }
    #[cfg(any(test, all(debug_assertions, feature = "openab-fixture")))]
    async fn attach_with_fixture_ca(
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        local_generation: u64,
        ca_der: &[u8],
    ) -> Result<Self, SessionError> {
        let connector = Self::fixture_connector(ca_der)?;
        Self::attach_with(host, grant, since, local_generation, Some(connector)).await
    }
    #[cfg(test)]
    pub(super) async fn tls_fixture_attach(
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        local_generation: u64,
        ca_der: &[u8],
    ) -> Result<Self, SessionError> {
        Self::attach_with_fixture_ca(host, grant, since, local_generation, ca_der).await
    }
    #[cfg(test)]
    pub(super) async fn tls_fixture_attach_token(
        binding: SessionBinding,
        token: Secret,
        since: u64,
        local_generation: u64,
        ca_der: &[u8],
    ) -> Result<Self, SessionError> {
        let connector = Self::fixture_connector(ca_der)?;
        Self::attach_bound(binding, token, since, local_generation, Some(connector)).await
    }
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    pub async fn attach_fixture(
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        local_generation: u64,
        ca_der: &[u8],
    ) -> Result<Self, SessionError> {
        Self::attach_with_fixture_ca(host, grant, since, local_generation, ca_der).await
    }
    /// Fixture-CA attach by binding+token — used by the GUI fixture path
    /// for same-token reattach and the dedicated screenshot Shell.
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    pub async fn attach_token_fixture(
        binding: SessionBinding,
        token: Secret,
        since: u64,
        local_generation: u64,
        ca_der: &[u8],
    ) -> Result<Self, SessionError> {
        let connector = Self::fixture_connector(ca_der)?;
        Self::attach_bound(binding, token, since, local_generation, Some(connector)).await
    }
    pub async fn attach(
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        local_generation: u64,
    ) -> Result<Self, SessionError> {
        Self::attach_with(host, grant, since, local_generation, None).await
    }
    /// Reattach with the SAME retained token — official `?since=` resume.
    /// No admin POST happens here; minting a new token (renew) is an
    /// explicit user-directed action elsewhere.
    pub async fn attach_token(
        binding: SessionBinding,
        token: Secret,
        since: u64,
        local_generation: u64,
    ) -> Result<Self, SessionError> {
        Self::attach_bound(binding, token, since, local_generation, None).await
    }
    async fn attach_with(
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        local_generation: u64,
        connector: Option<tokio_tungstenite::Connector>,
    ) -> Result<Self, SessionError> {
        let binding = SessionBinding::from_grant(host.profile, &grant)?;
        let token = grant.token;
        // Host credential is a one-shot snapshot; it is never retained.
        drop(host.credential);
        Self::attach_bound(binding, token, since, local_generation, connector).await
    }
    async fn attach_bound(
        binding: SessionBinding,
        token: Secret,
        since: u64,
        local_generation: u64,
        connector: Option<tokio_tungstenite::Connector>,
    ) -> Result<Self, SessionError> {
        if local_generation == 0 {
            return Err(SessionError::Protocol);
        }
        let mut socket = connect(&binding, &token, since, connector).await?;
        let mut tracker = Tracker::new(since);
        let shared = Arc::new(Mutex::new(Shared {
            events: VecDeque::new(),
            bytes: 0,
            acknowledged: since,
            pending: None,
            batch: 0,
            ended: None,
            input_uncertain: false,
        }));
        let notice = tokio::time::timeout(Duration::from_secs(5), async {
            for _ in 0..9 {
                let message = socket
                    .next()
                    .await
                    .ok_or(SessionError::Transport)?
                    .map_err(ws_error)?;
                match message {
                    Message::Text(text) => {
                        if text.len() > 8192 {
                            return Err(SessionError::Protocol);
                        }
                        let control: Control =
                            serde_json::from_str(&text).map_err(|_| SessionError::Protocol)?;
                        if control.v != 1 {
                            return Err(SessionError::Protocol);
                        }
                        if control.kind == "gap" {
                            tracker
                                .pre_gap(control.bytes_dropped.ok_or(SessionError::Protocol)?)?;
                            continue;
                        }
                        if control.kind != "attach-notice" {
                            return Err(SessionError::Protocol);
                        }
                        // Official attach-notice carries only v, type,
                        // stream_offset and advisory flags — no runtime or
                        // session identity is asserted on the wire.
                        tracker.notice(control.stream_offset.ok_or(SessionError::Protocol)?)?;
                        let mut q = shared.lock().map_err(|_| SessionError::Internal)?;
                        if tracker.next != since {
                            q.push(OutputEvent::Gap {
                                bytes_dropped: tracker.next - since,
                                resume_offset: tracker.next,
                            })?;
                        }
                        q.push(OutputEvent::Notice {
                            ephemeral_workspace: control
                                .ephemeral_workspace
                                .ok_or(SessionError::Protocol)?,
                            teardown_best_effort: control
                                .teardown_best_effort
                                .ok_or(SessionError::Protocol)?,
                        })?;
                        return Ok(());
                    }
                    Message::Close(frame) => {
                        return Err(frame
                            .map(|f| close_error(f.code.into()))
                            .unwrap_or(SessionError::Transport))
                    }
                    _ => return Err(SessionError::Protocol),
                }
            }
            Err(SessionError::Protocol)
        })
        .await
        .map_err(|_| SessionError::Timeout)?;
        notice?;
        let (commands, rx) = mpsc::channel(32);
        let (cancel, cancel_rx) = watch::channel(false);
        let input_bytes = Arc::new(AtomicUsize::new(0));
        let preview_flow = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let output_ready = Arc::new(tokio::sync::Notify::new());
        let output_drained = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn(owner(
            socket,
            tracker,
            shared.clone(),
            rx,
            cancel_rx,
            preview_flow.clone(),
            output_ready.clone(),
            output_drained.clone(),
        ));
        Ok(Self {
            binding,
            token: Some(token),
            preview_flow,
            output_ready,
            output_drained,
            local_generation,
            shared,
            commands,
            cancel,
            input_bytes,
            task: Some(task),
        })
    }
    /// Compatibility switch retained for callers; every terminal now uses
    /// bounded output backpressure, including work terminals.
    pub fn preview_flow(&self) {
        self.preview_flow.store(true, Ordering::SeqCst);
    }
    pub async fn wait_output(&self) {
        if self
            .shared
            .lock()
            .map(|q| !q.events.is_empty() || q.ended.is_some())
            .unwrap_or(true)
        {
            return;
        }
        self.output_ready.notified().await;
    }
    pub fn binding(&self) -> &SessionBinding {
        &self.binding
    }
    /// Take the retained attach token out of a handle being torn down —
    /// the only way recovery gets the credential material for a
    /// same-token `?since=` reattach. Returns `None` once consumed.
    pub fn take_token(&mut self) -> Option<Secret> {
        self.token.take()
    }
    /// Pure local observation, not a guarantee of remote liveness. Never
    /// consumes events or changes batch/ACK/input state and sends no probe.
    pub fn is_live(&self) -> Result<bool, SessionError> {
        let shared = self.shared.lock().map_err(|_| SessionError::Internal)?;
        Ok(shared.ended.is_none()
            && !*self.cancel.borrow()
            && !self.commands.is_closed()
            && self.task.as_ref().is_some_and(|task| !task.is_finished()))
    }
    pub fn rendered_cursor(&self) -> Result<u64, SessionError> {
        Ok(self
            .shared
            .lock()
            .map_err(|_| SessionError::Internal)?
            .acknowledged)
    }
    pub fn poll(&self) -> Result<SessionPoll, SessionError> {
        let mut q = self.shared.lock().map_err(|_| SessionError::Internal)?;
        if q.pending.is_some() {
            return Err(SessionError::Busy);
        }
        let mut events = Vec::new();
        let mut bytes = 0;
        let mut cursor = q.acknowledged;
        while events.len() < 64 {
            let Some(mut event) = q.events.pop_front() else {
                break;
            };
            if let OutputEvent::Bytes {
                bytes: payload,
                start,
                end,
                historical,
            } = &mut event
            {
                let n = (POLL_BYTES - bytes).min(payload.len());
                if n == 0 {
                    q.events.push_front(event);
                    break;
                }
                if n < payload.len() {
                    let tail = payload.split_off(n);
                    let tail_start = start.checked_add(n as u64).ok_or(SessionError::Protocol)?;
                    q.events.push_front(OutputEvent::Bytes {
                        bytes: tail,
                        start: tail_start,
                        end: *end,
                        historical: *historical,
                    });
                    *end = tail_start;
                }
                bytes += payload.len();
                q.bytes -= payload.len();
                cursor = *end;
            }
            if let OutputEvent::Gap { resume_offset, .. } = event {
                cursor = resume_offset;
            }
            events.push(event);
        }
        let batch_id = if events.is_empty() {
            0
        } else {
            q.batch = q.batch.checked_add(1).ok_or(SessionError::Protocol)?;
            q.pending = Some((q.batch, cursor));
            q.batch
        };
        let ended = if q.events.is_empty() && events.is_empty() {
            q.ended
        } else {
            None
        };
        Ok(SessionPoll {
            batch_id,
            local_generation: self.local_generation,
            events,
            ended,
            input_uncertain: q.input_uncertain,
        })
    }
    pub fn ack(&self, batch_id: u64, local_generation: u64) -> Result<(), SessionError> {
        if local_generation != self.local_generation {
            return Err(SessionError::StateChanged);
        }
        let mut q = self.shared.lock().map_err(|_| SessionError::Internal)?;
        let Some((expected, cursor)) = q.pending else {
            return Err(SessionError::StateChanged);
        };
        if batch_id != expected {
            return Err(SessionError::StateChanged);
        }
        q.acknowledged = cursor;
        q.pending = None;
        drop(q);
        self.output_drained.notify_one();
        Ok(())
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), SessionError> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > 64 * 1024 {
            return Err(SessionError::Capacity);
        }
        // Keep admission and nonblocking enqueue atomic with owner teardown.
        let q = self.shared.lock().map_err(|_| SessionError::Internal)?;
        if q.ended.is_some() || *self.cancel.borrow() {
            return Err(SessionError::StateChanged);
        }
        #[allow(deprecated)] // Keep compatibility with Rust before Atomic::try_update.
        self.input_bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(bytes.len()).filter(|n| *n <= INPUT_BYTES)
            })
            .map_err(|_| SessionError::Capacity)?;
        let input = Input {
            bytes: bytes.to_vec(),
            reserved: self.input_bytes.clone(),
        };
        let result = self
            .commands
            .try_send(Command::Input(input))
            .map_err(|_| SessionError::Capacity);
        drop(q);
        result
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), SessionError> {
        if !(1..=1000).contains(&rows) || !(1..=1000).contains(&cols) {
            return Err(SessionError::Protocol);
        }
        self.commands
            .try_send(Command::Resize(rows, cols))
            .map_err(|_| SessionError::Capacity)
    }
    pub fn ping(&self) -> Result<(), SessionError> {
        self.commands
            .try_send(Command::Ping)
            .map_err(|_| SessionError::Capacity)
    }
    pub fn close(&self) {
        let _ = self.cancel.send(true);
    }
    pub async fn shutdown(mut self) {
        self.close();
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.close();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Deserialize)]
struct Control {
    v: u64,
    #[serde(rename = "type")]
    kind: String,
    stream_offset: Option<u64>,
    bytes_dropped: Option<u64>,
    ephemeral_workspace: Option<bool>,
    teardown_best_effort: Option<bool>,
    seconds_remaining: Option<u64>,
    absolute: Option<bool>,
}
struct Tracker {
    requested: u64,
    next: u64,
    replay_end: Option<u64>,
}
impl Tracker {
    fn new(since: u64) -> Self {
        Self {
            requested: since,
            next: since,
            replay_end: None,
        }
    }
    fn pre_gap(&mut self, dropped: u64) -> Result<(), SessionError> {
        self.next = self
            .next
            .checked_add(dropped)
            .ok_or(SessionError::Protocol)?;
        Ok(())
    }
    fn notice(&mut self, end: u64) -> Result<(), SessionError> {
        if end < self.requested {
            return Err(SessionError::Protocol);
        }
        self.replay_end = Some(end);
        Ok(())
    }
    fn bytes(&mut self, bytes: Vec<u8>) -> Result<Vec<OutputEvent>, SessionError> {
        let replay_end = self.replay_end.ok_or(SessionError::Protocol)?;
        let mut events = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let historic = self.next < replay_end;
            let n = if historic {
                (replay_end - self.next).min((bytes.len() - offset) as u64) as usize
            } else {
                bytes.len() - offset
            };
            let start = self.next;
            self.next = self
                .next
                .checked_add(n as u64)
                .ok_or(SessionError::Protocol)?;
            events.push(OutputEvent::Bytes {
                bytes: bytes[offset..offset + n].to_vec(),
                start,
                end: self.next,
                historical: historic,
            });
            offset += n;
        }
        Ok(events)
    }
}
fn ws_error(error: tungstenite::Error) -> SessionError {
    if super::api::tls_cause(&error) {
        return SessionError::Tls;
    }
    match error {
        tungstenite::Error::Tls(_) => SessionError::Tls,
        tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => SessionError::Auth,
            429 | 503 => SessionError::Capacity,
            _ => SessionError::Protocol,
        },
        tungstenite::Error::Capacity(_) => SessionError::Capacity,
        tungstenite::Error::Protocol(_) => SessionError::Protocol,
        _ => SessionError::Transport,
    }
}
async fn connect(
    binding: &SessionBinding,
    token: &super::secret::Secret,
    since: u64,
    connector: Option<tokio_tungstenite::Connector>,
) -> Result<Socket, SessionError> {
    super::profiles::canonical_origin(&binding.profile.base_url)?;
    let mut url = url::Url::parse(&binding.profile.base_url).map_err(|_| SessionError::Protocol)?;
    url.set_scheme("wss").map_err(|_| SessionError::Protocol)?;
    url.set_path(&format!("/pty/{}", binding.name));
    url.query_pairs_mut()
        .append_pair("since", &since.to_string());
    let mut request = url.as_str().into_client_request().map_err(ws_error)?;
    let auth = zeroize::Zeroizing::new(format!("Bearer {}", token.expose()));
    let mut header =
        tungstenite::http::HeaderValue::from_str(&auth).map_err(|_| SessionError::Protocol)?;
    header.set_sensitive(true);
    request
        .headers_mut()
        .insert(tungstenite::http::header::AUTHORIZATION, header);
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(SOCKET_BYTES);
    config.max_frame_size = Some(SOCKET_BYTES);
    config.max_write_buffer_size = 512 * 1024;
    let (socket, response) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_tls_with_config(request, Some(config), true, connector),
    )
    .await
    .map_err(|_| SessionError::Timeout)?
    .map_err(ws_error)?;
    if response.headers().contains_key("sec-websocket-protocol") {
        return Err(SessionError::Protocol);
    }
    Ok(socket)
}
async fn owner(
    mut socket: Socket,
    mut tracker: Tracker,
    shared: Arc<Mutex<Shared>>,
    mut commands: mpsc::Receiver<Command>,
    mut cancel: watch::Receiver<bool>,
    _preview_flow: Arc<std::sync::atomic::AtomicBool>,
    output_ready: Arc<tokio::sync::Notify>,
    output_drained: Arc<tokio::sync::Notify>,
) {
    let mut sent_input = false;
    let result:Result<(),SessionError>=async {
        loop {
            // Reserve capacity for a whole allowed WebSocket message (which
            // can split into historical/live events). Keep commands and cancel
            // responsive while renderer ACKs release bounded output capacity.
            let paused = {
                let q = shared.lock().map_err(|_| SessionError::Internal)?;
                q.pending.is_some() || q.bytes > OUTPUT_BYTES - SOCKET_BYTES || q.events.len() > OUTPUT_EVENTS - 2
            };
            tokio::select!{ biased;
                _=cancel.changed()=>{let _=tokio::time::timeout(Duration::from_millis(250),socket.send(Message::Text("{\"v\":1,\"type\":\"detach\"}".into()))).await;let _=tokio::time::timeout(Duration::from_millis(250),socket.close(None)).await;return Err(SessionError::Cancelled)}
                command=commands.recv()=>{
                    let Some(command)=command else{return Err(SessionError::Cancelled)};
                    let message=match &command {Command::Input(input)=>Message::Binary(input.bytes.clone()),Command::Resize(rows,cols)=>Message::Text(serde_json::json!({"v":1,"type":"resize","rows":rows,"cols":cols}).to_string()),Command::Ping=>Message::Text("{\"v\":1,\"type\":\"ping\"}".into())};
                    let result=tokio::time::timeout(Duration::from_secs(2),socket.send(message)).await;
                    if matches!(command,Command::Input(_)) {if !matches!(&result,Ok(Ok(()))) {if let Ok(mut q)=shared.lock(){q.input_uncertain=true;}return Err(SessionError::InputUncertain)}}
                    result.map_err(|_|SessionError::Timeout)?.map_err(ws_error)?;
                    if matches!(command,Command::Input(_)){sent_input=true;}
                }
                _=output_drained.notified(), if paused => {}
                message=socket.next(), if !paused =>{
                    let message=message.ok_or(SessionError::Transport)?.map_err(ws_error)?;
                    let events=match message {
                        Message::Binary(bytes)=>tracker.bytes(bytes)?,
                        Message::Text(text)=>{if text.len()>8192{return Err(SessionError::Protocol)}let control:Control=serde_json::from_str(&text).map_err(|_|SessionError::Protocol)?;if control.v!=1{return Err(SessionError::Protocol)}
                            match control.kind.as_str(){"gap"=>{let dropped=control.bytes_dropped.ok_or(SessionError::Protocol)?;tracker.pre_gap(dropped)?;vec![OutputEvent::Gap{bytes_dropped:dropped,resume_offset:tracker.next}]},"ttl-warning"=>vec![OutputEvent::TtlWarning{seconds_remaining:control.seconds_remaining.ok_or(SessionError::Protocol)?,absolute:control.absolute.ok_or(SessionError::Protocol)?}],"pong"=>Vec::new(),"shutting-down"=>return Err(SessionError::Replacement),_=>return Err(SessionError::Protocol)}},
                        Message::Close(frame)=>return Err(frame.map(|f|close_error(f.code.into())).unwrap_or(SessionError::Transport)),
                        Message::Ping(payload)=>{tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Pong(payload))).await.map_err(|_|SessionError::Timeout)?.map_err(ws_error)?;Vec::new()},Message::Pong(_)=>Vec::new(),_=>return Err(SessionError::Protocol)};
                    for event in events {
                        shared.lock().map_err(|_|SessionError::Internal)?.push(event)?;
                        output_ready.notify_one();
                    }
                    tokio::task::yield_now().await;
                }

            }
        }
    }.await;
    if let Ok(mut q) = shared.lock() {
        q.ended = result.err();
        output_ready.notify_one();
        if sent_input && q.ended != Some(SessionError::Cancelled) {
            q.input_uncertain = true;
        }
        while let Ok(command) = commands.try_recv() {
            if matches!(command, Command::Input(_)) {
                q.input_uncertain = true;
            }
        }
    }
    // Queued/in-flight input is dropped forever. Recovery never owns this channel.
    drop(commands);
    drop(socket);
}

/// Explicit recovery coordinator: one flight, max 3 attach attempts, no
/// automatic create/renew/restart and no admin POST at all. Recovery is a
/// same-token `?since=` reattach of the existing session — the retained
/// attach token and the renderer ACK cursor are supplied by the caller's
/// closure, which is invoked once per attempt. Terminal errors
/// (Auth/4xxx close codes/StateChanged) stop immediately; only a
/// transient transport/timeout is retried. Caller closes the prior
/// handle before recovery and controls mobile foreground policy.
pub struct Recovery {
    busy: std::sync::atomic::AtomicBool,
}
impl Default for Recovery {
    fn default() -> Self {
        Self {
            busy: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
impl Recovery {
    pub async fn resume<F, Fut>(
        &self,
        mut cancel: watch::Receiver<bool>,
        mut attempt: F,
    ) -> Result<SessionHandle, SessionError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<SessionHandle, SessionError>>,
    {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(SessionError::Busy);
        }
        struct Release<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _release = Release(&self.busy);
        tokio::time::timeout(Duration::from_secs(30),async {
        for attempt_n in 0..3 {
            if *cancel.borrow() {
                return Err(SessionError::Cancelled);
            }
            let result = tokio::select! {_ = cancel.changed()=>return Err(SessionError::Cancelled),result=attempt()=>result};
            match result {
                Err(SessionError::Transport | SessionError::Timeout) if attempt_n < 2 => {
                    let jitter = (uuid::Uuid::new_v4().as_u128() % 100) as u64;
                    tokio::select! {_=cancel.changed()=>return Err(SessionError::Cancelled),_=tokio::time::sleep(Duration::from_millis(200*(1<<attempt_n)+jitter))=>{}}
                }
                Err(SessionError::Transport | SessionError::Timeout) => {
                    return Err(SessionError::RetryExhausted)
                }
                other => return other,
            }
        }
        Err(SessionError::RetryExhausted)
        }).await.map_err(|_|SessionError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> HostProfile {
        HostProfile {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: "Fixture".into(),
            base_url: "https://example.test/".into(),
            credential_present: true,
            metadata: None,
        }
    }
    fn handle() -> (
        SessionHandle,
        mpsc::Receiver<Command>,
        watch::Receiver<bool>,
    ) {
        let (commands, rx) = mpsc::channel(32);
        let (cancel, cancel_rx) = watch::channel(false);
        (
            SessionHandle {
                binding: SessionBinding {
                    profile: profile(),
                    name: "fixture".into(),
                    generation: 1,
                },
                token: Some(Secret::new("a".repeat(64)).unwrap()),
                preview_flow: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                output_ready: Arc::new(tokio::sync::Notify::new()),
                output_drained: Arc::new(tokio::sync::Notify::new()),
                local_generation: 7,
                shared: Arc::new(Mutex::new(Shared {
                    events: VecDeque::new(),
                    bytes: 0,
                    acknowledged: 4,
                    pending: None,
                    batch: 0,
                    ended: None,
                    input_uncertain: false,
                })),
                commands,
                cancel,
                input_bytes: Arc::new(AtomicUsize::new(0)),
                task: None,
            },
            rx,
            cancel_rx,
        )
    }
    #[tokio::test]
    async fn liveness_observation_has_no_queue_ack_input_or_wire_effects() {
        let (mut handle, mut rx, _cancel) = handle();
        assert!(!handle.is_live().unwrap()); // No owner task is not proof of life.
        handle.task = Some(tokio::spawn(std::future::pending()));
        {
            let mut q = handle.shared.lock().unwrap();
            q.events.push_back(OutputEvent::Gap {
                bytes_dropped: 2,
                resume_offset: 6,
            });
            q.batch = 9;
            q.pending = Some((9, 6));
        }
        let snapshot = || {
            let q = handle.shared.lock().unwrap();
            (
                q.events.len(),
                q.bytes,
                q.batch,
                q.pending,
                q.acknowledged,
                handle.input_bytes.load(Ordering::SeqCst),
            )
        };
        let before = snapshot();
        for _ in 0..10 {
            assert!(handle.is_live().unwrap());
        }
        assert_eq!(snapshot(), before);
        assert!(rx.try_recv().is_err());
        handle.close();
        assert!(!handle.is_live().unwrap());
        assert_eq!(snapshot(), before);
        let task = handle.task.take().unwrap();
        task.abort();
        let _ = task.await;
    }
    #[tokio::test]
    async fn liveness_closed_channel_finished_owner_and_poison_fail_closed() {
        let (mut channel_handle, channel_rx, _cancel_rx) = handle();
        channel_handle.task = Some(tokio::spawn(std::future::pending()));
        assert!(channel_handle.is_live().unwrap());
        drop(channel_rx);
        assert!(!channel_handle.is_live().unwrap());
        let task = channel_handle.task.take().unwrap();
        task.abort();
        let _ = task.await;
        let (mut handle, rx, _cancel) = handle();
        handle.task = Some(tokio::spawn(async {}));
        tokio::task::yield_now().await;
        assert!(!handle.is_live().unwrap());
        drop(rx);
        assert!(!handle.is_live().unwrap());
        let shared = handle.shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = shared.lock().unwrap();
            panic!("fixture poison")
        })
        .join();
        assert_eq!(handle.is_live(), Err(SessionError::Internal));
    }
    #[test]
    fn rendered_ack_is_exact_bounded_and_fenced() {
        let (handle, _rx, _cancel) = handle();
        {
            let mut q = handle.shared.lock().unwrap();
            q.push(OutputEvent::Bytes {
                bytes: vec![42; 100000],
                start: 4,
                end: 100004,
                historical: true,
            })
            .unwrap();
        }
        let first = handle.poll().unwrap();
        assert_eq!(handle.rendered_cursor().unwrap(), 4);
        assert!(matches!(
            first.events[0],
            OutputEvent::Bytes { end: 65540, .. }
        ));
        assert!(handle.ack(first.batch_id + 1, 7).is_err());
        assert!(handle.ack(first.batch_id, 6).is_err());
        handle.ack(first.batch_id, 7).unwrap();
        assert_eq!(handle.rendered_cursor().unwrap(), 65540);
        assert!(handle.ack(first.batch_id, 7).is_err());
        let second = handle.poll().unwrap();
        handle.ack(second.batch_id, 7).unwrap();
        assert_eq!(handle.rendered_cursor().unwrap(), 100004);
        handle
            .shared
            .lock()
            .unwrap()
            .push(OutputEvent::Gap {
                bytes_dropped: 10,
                resume_offset: 100014,
            })
            .unwrap();
        let gap = handle.poll().unwrap();
        assert_eq!(handle.rendered_cursor().unwrap(), 100004);
        handle.ack(gap.batch_id, 7).unwrap();
        assert_eq!(handle.rendered_cursor().unwrap(), 100014);
    }
    #[test]
    fn output_and_input_pressure_terminate_or_reject_without_replay() {
        let (handle, mut rx, _cancel) = handle();
        for _ in 0..4 {
            handle.write(&vec![1; 64 * 1024]).unwrap();
        }
        assert_eq!(handle.write(&[2]), Err(SessionError::Capacity));
        assert_eq!(handle.input_bytes.load(Ordering::SeqCst), INPUT_BYTES);
        let command = rx.try_recv().unwrap();
        drop(command);
        assert_eq!(
            handle.input_bytes.load(Ordering::SeqCst),
            INPUT_BYTES - 64 * 1024
        );
        handle.close();
        assert_eq!(handle.write(b"late"), Err(SessionError::StateChanged));
        drop(rx);
        assert_eq!(handle.input_bytes.load(Ordering::SeqCst), 0);
        let mut q = handle.shared.lock().unwrap();
        for index in 0..8 {
            q.push(OutputEvent::Bytes {
                bytes: vec![3; 64 * 1024],
                start: index * 65536,
                end: (index + 1) * 65536,
                historical: false,
            })
            .unwrap();
        }
        assert_eq!(
            q.push(OutputEvent::Bytes {
                bytes: vec![4],
                start: OUTPUT_BYTES as u64,
                end: OUTPUT_BYTES as u64 + 1,
                historical: false
            }),
            Err(SessionError::SlowConsumer)
        );
        assert_eq!(q.bytes, OUTPUT_BYTES);
    }
    #[test]
    fn cursor_gap_overlap_and_offset_overflow_remain_byte_exact() {
        let mut tracker = Tracker::new(8);
        tracker.pre_gap(10).unwrap();
        tracker.notice(12).unwrap();
        let events = tracker.bytes(vec![0xe4, 0xbd, 0xa0]).unwrap();
        assert!(matches!(
            events[0],
            OutputEvent::Bytes {
                start: 18,
                end: 21,
                historical: false,
                ..
            }
        ));
        let mut tracker = Tracker::new(8);
        assert_eq!(tracker.notice(7), Err(SessionError::Protocol));
        let mut tracker = Tracker::new(u64::MAX);
        tracker.notice(u64::MAX).unwrap();
        assert!(tracker.bytes(vec![1]).is_err());
    }
    /// Official lists carry no custom identity fields; bindings key on
    /// the name plus observed generation. A dead/missing session or a
    /// generation drift (renew/restart killed the token family) stops
    /// recovery — it is never silently rebound.
    #[test]
    fn list_binding_needs_alive_name_and_stable_generation() {
        let list:SessionList=serde_json::from_value(serde_json::json!({"sessions":[{"name":"fixture","generation":1,"alive":true,"attached":false,"bytes_written":0,"tier":"tier1","teardown_best_effort":true,"absolute_ttl_best_effort":true}],"draining":false})).unwrap();
        let binding = SessionBinding::from_list(profile(), &list, "fixture").unwrap();
        let mut changed = list.clone();
        changed.sessions[0].alive = false;
        assert_eq!(binding.verify_list(&changed), Err(ApiError::SessionDead));
        changed = list.clone();
        changed.sessions.clear();
        assert_eq!(binding.verify_list(&changed), Err(ApiError::SessionMissing));
        changed = list.clone();
        changed.sessions[0].generation = 2;
        assert_eq!(binding.verify_list(&changed), Err(ApiError::StateChanged));
        assert_eq!(binding.verify_list(&list), Ok(()));
    }
    #[test]
    fn close_codes_do_not_conflate_exit_replacement_takeover_or_operator_kill() {
        for (code, expected) in [
            (4001, SessionError::TtlExpired),
            (4002, SessionError::TakenOver),
            (4003, SessionError::Renewed),
            (4004, SessionError::Exited),
            (4005, SessionError::SlowConsumer),
            (4006, SessionError::Replacement),
            (4007, SessionError::Capacity),
            (4008, SessionError::OperatorKilled),
            (4009, SessionError::Internal),
            (1006, SessionError::Transport),
        ] {
            assert_eq!(close_error(code), expected);
        }
    }
}
