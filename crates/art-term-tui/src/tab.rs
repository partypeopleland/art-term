use crate::{
    backend::{BackendError, Connector, Registration},
    terminal::Terminal,
};
use art_term_core::openab::{
    api::AdminClient,
    history::{self, HistoryChannel, HistoryReply, HistoryRequest, HistoryVersion},
    profiles::HostStore,
    screenshot::ShellOwnership,
    secret::Secret,
    session::{OutputEvent, Recovery, SessionBinding, SessionError, SessionHandle},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use crossterm::event::KeyEvent;
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};

const QUEUE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TabId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}
impl TerminalSize {
    pub(crate) fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.clamp(1, 240),
            cols: cols.clamp(1, 500),
        }
    }
}
pub(crate) enum TabAction {
    Key(KeyEvent),
    Paste(String),
    Resize(TerminalSize),
    Scroll(i16),
    Follow,
    Recover,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TabStatus {
    Live,
    Recovering,
    InputUncertain,
    Ended(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HistoryStatus {
    Unsupported,
    Following,
    Held { offset: usize },
    Uncertain,
}
#[derive(Clone)]
pub(crate) struct TabView {
    pub screen: Arc<vt100::Screen>,
    pub status: TabStatus,
    pub history: HistoryStatus,
    pub notice: Option<&'static str>,
}

struct Reservation {
    bytes: usize,
    budget: Arc<AtomicUsize>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}
struct Action {
    value: TabAction,
    reservation: Reservation,
}

pub(crate) struct TabClient {
    label: String,
    host: String,
    actions: mpsc::Sender<Action>,
    views: watch::Receiver<Arc<TabView>>,
    sizes: watch::Sender<TerminalSize>,
    cancel: watch::Sender<bool>,
    finished: watch::Receiver<bool>,
    budget: Arc<AtomicUsize>,
}
impl TabClient {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
    pub(crate) fn host_id(&self) -> &str {
        &self.host
    }
    pub(crate) fn view(&self) -> Arc<TabView> {
        self.views.borrow().clone()
    }
    pub(crate) fn try_send(&self, value: TabAction) -> Result<(), BackendError> {
        if let TabAction::Resize(size) = value {
            self.sizes.send_replace(size);
            return Ok(());
        }
        if !matches!(value, TabAction::Recover) && self.views.borrow().status != TabStatus::Live {
            return Err(SessionError::InputUncertain.into());
        }
        let bytes = match &value {
            TabAction::Paste(text) => {
                if text.len() > 65524 {
                    return Err(SessionError::Capacity.into());
                }
                text.len() + 12
            }
            TabAction::Key(_) => 32,
            _ => 0,
        };
        self.budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(bytes).filter(|n| *n <= QUEUE_BYTES)
            })
            .map_err(|_| BackendError::Session(SessionError::Capacity))?;
        let action = Action {
            value,
            reservation: Reservation {
                bytes,
                budget: self.budget.clone(),
            },
        };
        self.actions
            .try_send(action)
            .map_err(|_| BackendError::Busy)
    }
    pub(crate) async fn close(mut self) {
        let _ = self.cancel.send(true);
        while !*self.finished.borrow() && self.finished.changed().await.is_ok() {}
    }
}
impl Drop for TabClient {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

enum HistoryPurpose {
    Control,
    Input {
        tail: Vec<u8>,
        _reservation: Reservation,
    },
    Close,
}
struct HistoryCompletion {
    channel: Option<HistoryChannel>,
    result: Result<HistoryReply, SessionError>,
    purpose: HistoryPurpose,
}
type HistoryFuture = Pin<Box<dyn Future<Output = HistoryCompletion> + Send>>;
type RecoveryFuture = Pin<Box<dyn Future<Output = Result<SessionHandle, SessionError>> + Send>>;

struct Tab {
    handle: Option<SessionHandle>,
    binding: SessionBinding,
    generation: u64,
    terminal: Terminal,
    status: TabStatus,
    history_status: HistoryStatus,
    notice: Option<&'static str>,
    discard_input: bool,
    store: Arc<HostStore>,
    api: AdminClient,
    connector: Connector,
    ownership: Arc<dyn ShellOwnership>,
    history: Option<HistoryChannel>,
    pending_history: Option<HistoryFuture>,
    pending_recovery: Option<RecoveryFuture>,
    actions: VecDeque<Action>,
    input_uncertain: bool,
    recovery_requested: bool,
    recovery_failures: usize,
    last_connected: tokio::time::Instant,
    cancellation: watch::Sender<bool>,
    history_initialized: bool,
    last_history: tokio::time::Instant,
}

pub(super) fn spawn(
    label: String,
    handle: SessionHandle,
    size: TerminalSize,
    store: Arc<HostStore>,
    api: AdminClient,
    connector: Connector,
    ownership: Arc<dyn ShellOwnership>,
) -> (TabClient, Registration) {
    let binding = handle.binding().clone();
    let host = binding.profile.id.clone();
    let terminal = Terminal::new(size);
    let managed = history::key_from_name(&binding.name).is_some();
    let history_status = if managed {
        HistoryStatus::Following
    } else {
        HistoryStatus::Unsupported
    };
    let initial = Arc::new(TabView {
        screen: Arc::new(terminal.screen().clone()),
        status: TabStatus::Live,
        history: history_status.clone(),
        notice: None,
    });
    let (views, receiver) = watch::channel(initial);
    let (actions, input) = mpsc::channel(32);
    let (sizes, resize) = watch::channel(size);
    let (cancel, cancellation) = watch::channel(false);
    let (finished, completion) = watch::channel(false);
    let tab = Tab {
        handle: Some(handle),
        binding,
        generation: 1,
        terminal,
        status: TabStatus::Live,
        history_status,
        notice: None,
        discard_input: false,
        store,
        api,
        connector,
        ownership,
        history: None,
        pending_history: None,
        pending_recovery: None,
        actions: VecDeque::new(),
        input_uncertain: false,
        recovery_requested: false,
        recovery_failures: 0,
        last_connected: tokio::time::Instant::now(),
        cancellation: cancel.clone(),
        history_initialized: !managed,
        last_history: tokio::time::Instant::now(),
    };
    let task = tokio::spawn(async move {
        tab.run(input, resize, cancellation, views).await;
        finished.send_replace(true);
    });
    (
        TabClient {
            label,
            host: host.clone(),
            actions,
            views: receiver,
            sizes,
            cancel: cancel.clone(),
            finished: completion,
            budget: Arc::new(AtomicUsize::new(0)),
        },
        Registration {
            host_id: host,
            cancel,
            task,
        },
    )
}

impl Tab {
    fn publish(&self, views: &watch::Sender<Arc<TabView>>) {
        views.send_replace(Arc::new(TabView {
            screen: Arc::new(self.terminal.screen().clone()),
            status: self.status.clone(),
            history: self.history_status.clone(),
            notice: self.notice,
        }));
    }
    fn current(&self) -> Result<(), SessionError> {
        if self
            .store
            .is_current(&self.binding.profile)
            .map_err(SessionError::from)?
        {
            Ok(())
        } else {
            Err(SessionError::StateChanged)
        }
    }
    fn direct(&self, bytes: &[u8]) -> Result<(), SessionError> {
        self.current()?;
        self.handle.as_ref().ok_or(SessionError::Busy)?.write(bytes)
    }
    fn consume(&mut self) -> Result<(Option<SessionError>, bool), SessionError> {
        let handle = self.handle.as_ref().ok_or(SessionError::Busy)?;
        let batch = handle.poll()?;
        if batch.local_generation != self.generation {
            return Err(SessionError::StateChanged);
        }
        let changed = batch.batch_id != 0
            || batch.ended.is_some()
            || (batch.input_uncertain && !self.input_uncertain);
        if batch.input_uncertain {
            self.input_uncertain = true;
        }
        let mut replies = Vec::new();
        for event in batch.events {
            match event {
                OutputEvent::Bytes {
                    bytes, historical, ..
                } => {
                    let reply = self.terminal.feed(&bytes, historical)?;
                    if replies.len() + reply.len() > 8192 {
                        return Err(SessionError::Capacity);
                    }
                    replies.extend(reply);
                }
                OutputEvent::Gap { .. } => {
                    self.terminal.reset();
                    replies.clear();
                    self.notice = Some("輸出曾有缺段，畫面及未傳送輸入已重置");
                    self.discard_input = true;
                }
                OutputEvent::Notice { .. } | OutputEvent::TtlWarning { .. } => (),
            }
        }
        if batch.batch_id != 0 {
            handle.ack(batch.batch_id, batch.local_generation)?;
        }
        if !replies.is_empty() {
            self.direct(&replies)?;
        }
        if self.input_uncertain && self.status == TabStatus::Live {
            self.status = TabStatus::InputUncertain;
        }
        Ok((batch.ended, changed))
    }
    fn history_request(
        &mut self,
        request: HistoryRequest,
        purpose: HistoryPurpose,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<(), SessionError> {
        self.current()?;
        self.last_history = tokio::time::Instant::now();
        let channel = self.history.take();
        let store = self.store.clone();
        let profile = self.binding.profile.clone();
        let key =
            history::key_from_name(&self.binding.name).ok_or(SessionError::UpgradeRequired)?;
        let version =
            HistoryVersion::from_name(&self.binding.name).ok_or(SessionError::UpgradeRequired)?;
        let api = self.api.clone();
        let connector = self.connector.clone();
        let ownership = self.ownership.clone();
        self.pending_history = Some(Box::pin(async move {
            let mut channel = match channel {
                Some(channel) => channel,
                None => {
                    let opening = async {
                        let snapshot = store.snapshot(&profile.id).map_err(SessionError::from)?;
                        if snapshot.profile != profile {
                            return Err(SessionError::StateChanged);
                        }
                        HistoryChannel::open(
                            &api,
                            snapshot,
                            &key,
                            version,
                            ownership,
                            move |binding, token, since, generation| async move {
                                connector
                                    .attach_token(binding, token, since, generation)
                                    .await
                            },
                        )
                        .await
                    };
                    let result = tokio::select! { biased; _ = cancel.changed() => Err(SessionError::Cancelled), result = opening => result };
                    match result {
                        Ok(channel) => channel,
                        Err(error) => {
                            return HistoryCompletion {
                                channel: None,
                                result: Err(error),
                                purpose,
                            }
                        }
                    }
                }
            };
            let result = tokio::select! { biased; _ = cancel.changed() => Err(SessionError::Cancelled), result = channel.request(request) => result };
            HistoryCompletion {
                channel: Some(channel),
                result,
                purpose,
            }
        }));
        Ok(())
    }
    fn close_history(&mut self) {
        if let Some(mut channel) = self.history.take() {
            self.pending_history = Some(Box::pin(async move {
                let confirmed = channel.close().await;
                HistoryCompletion {
                    channel: None,
                    result: if confirmed {
                        Ok(HistoryReply::Unsupported)
                    } else {
                        Err(SessionError::InputUncertain)
                    },
                    purpose: HistoryPurpose::Close,
                }
            }));
        }
    }
    fn complete_history(&mut self, completion: HistoryCompletion) {
        self.history = completion.channel;
        match completion.purpose {
            HistoryPurpose::Close => {
                if completion.result.is_err() {
                    self.input_uncertain = true;
                    self.history_status = HistoryStatus::Uncertain;
                } else {
                    self.history_status = HistoryStatus::Following;
                }
            }
            purpose => match completion.result {
                Ok(HistoryReply::View {
                    held,
                    offset,
                    available: true,
                    ..
                }) => {
                    self.history_status = if held {
                        HistoryStatus::Held { offset }
                    } else {
                        HistoryStatus::Following
                    };
                    if let HistoryPurpose::Input { tail, .. } = purpose {
                        if held
                            || offset != 0
                            || self.recovery_requested
                            || self.direct(&tail).is_err()
                        {
                            self.input_uncertain = true;
                        }
                    }
                }
                Ok(HistoryReply::View {
                    held: false,
                    offset: 0,
                    available: false,
                    ..
                }) => {
                    self.history_status = HistoryStatus::Unsupported;
                    if matches!(purpose, HistoryPurpose::Input { .. }) {
                        self.input_uncertain = true;
                    }
                }
                Ok(HistoryReply::Unsupported) => {
                    self.history_status = HistoryStatus::Unsupported;
                    if matches!(purpose, HistoryPurpose::Input { .. }) {
                        self.input_uncertain = true;
                    }
                }
                _ => {
                    self.input_uncertain = true;
                    self.history_status = HistoryStatus::Uncertain;
                }
            },
        }
        if self.input_uncertain && !matches!(self.status, TabStatus::Ended(_)) {
            self.status = TabStatus::InputUncertain;
        }
    }
    fn action(
        &mut self,
        action: Action,
        cancel: watch::Receiver<bool>,
    ) -> Result<(), SessionError> {
        self.current()?;
        if matches!(action.value, TabAction::Recover) {
            self.recovery_requested = true;
            self.status = TabStatus::Recovering;
            return Ok(());
        }
        if self.input_uncertain || self.recovery_requested || self.pending_recovery.is_some() {
            return Err(SessionError::InputUncertain);
        }
        let bytes = match action.value {
            TabAction::Key(key) => self.terminal.key(key)?,
            TabAction::Paste(text) => Some(self.terminal.paste(text)?),
            TabAction::Scroll(lines) => {
                if self.terminal.screen().alternate_screen()
                    && history::key_from_name(&self.binding.name).is_none()
                {
                    return Err(SessionError::UpgradeRequired);
                }
                self.history_request(
                    HistoryRequest::Scroll {
                        lines: lines.clamp(-256, 256),
                    },
                    HistoryPurpose::Control,
                    cancel,
                )?;
                return Ok(());
            }
            TabAction::Follow => {
                self.history_request(
                    HistoryRequest::Follow { input: None },
                    HistoryPurpose::Control,
                    cancel,
                )?;
                return Ok(());
            }
            TabAction::Resize(_) | TabAction::Recover => None,
        };
        if let Some(bytes) = bytes {
            if matches!(self.history_status, HistoryStatus::Held { .. }) {
                let head = bytes.len().min(4096);
                let request = HistoryRequest::Follow {
                    input: Some(STANDARD.encode(&bytes[..head])),
                };
                self.history_request(
                    request,
                    HistoryPurpose::Input {
                        tail: bytes[head..].to_vec(),
                        _reservation: action.reservation,
                    },
                    cancel,
                )?;
            } else if matches!(self.history_status, HistoryStatus::Uncertain) {
                return Err(SessionError::InputUncertain);
            } else {
                self.direct(&bytes)?;
            }
        }
        Ok(())
    }
    fn start_recovery(&mut self, cancel: watch::Receiver<bool>) -> Result<(), SessionError> {
        if self.last_connected.elapsed() >= Duration::from_secs(10) {
            self.recovery_failures = 0;
        }
        self.recovery_failures += 1;
        if self.recovery_failures > 3 {
            return Err(SessionError::RetryExhausted);
        }
        let mut old = self.handle.take().ok_or(SessionError::StateChanged)?;
        let cursor = old.rendered_cursor()?;
        let token = old.take_token().ok_or(SessionError::StateChanged)?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(SessionError::StateChanged)?;
        let generation = self.generation;
        let binding = self.binding.clone();
        let store = self.store.clone();
        let connector = self.connector.clone();
        self.status = TabStatus::Recovering;
        self.recovery_requested = false;
        if !self.actions.is_empty() {
            self.input_uncertain = true;
            self.actions.clear();
        }
        self.pending_recovery = Some(Box::pin(async move {
            old.shutdown().await;
            if !store
                .is_current(&binding.profile)
                .map_err(SessionError::from)?
            {
                return Err(SessionError::StateChanged);
            }
            let recovered = Recovery::default()
                .resume(cancel.clone(), || {
                    let retry = Secret::new(token.expose().to_owned()).map_err(SessionError::from);
                    let binding = binding.clone();
                    let connector = connector.clone();
                    async move {
                        connector
                            .attach_token(binding, retry?, cursor, generation)
                            .await
                    }
                })
                .await?;
            if *cancel.borrow()
                || !store
                    .is_current(&binding.profile)
                    .map_err(SessionError::from)?
            {
                recovered.shutdown().await;
                return Err(SessionError::Cancelled);
            }
            Ok(recovered)
        }));
        Ok(())
    }
    async fn run(
        mut self,
        mut input: mpsc::Receiver<Action>,
        mut sizes: watch::Receiver<TerminalSize>,
        mut cancel: watch::Receiver<bool>,
        views: watch::Sender<Arc<TabView>>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_millis(25));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut closing = false;
        loop {
            if !closing && self.current().is_err() {
                closing = true;
                self.status = TabStatus::Ended(SessionError::StateChanged.to_string());
            }
            if closing && !*self.cancellation.borrow() {
                self.cancellation.send_replace(true);
            }
            if closing && self.pending_history.is_none() {
                if self.history.is_some() {
                    self.close_history();
                } else {
                    break;
                }
            }
            if !closing
                && self.recovery_requested
                && self.pending_history.is_none()
                && self.pending_recovery.is_none()
            {
                if self.history.is_some() {
                    self.close_history();
                } else if let Err(error) = self.start_recovery(cancel.clone()) {
                    self.status = TabStatus::Ended(error.to_string());
                    closing = true;
                }
            }
            if !closing
                && !self.recovery_requested
                && self.pending_recovery.is_none()
                && self.pending_history.is_none()
            {
                if !self.history_initialized
                    || (self.history.is_some()
                        && self.last_history.elapsed() >= Duration::from_secs(30))
                {
                    self.history_initialized = true;
                    if let Err(error) = self.history_request(
                        HistoryRequest::Scroll { lines: 0 },
                        HistoryPurpose::Control,
                        cancel.clone(),
                    ) {
                        self.status = TabStatus::Ended(error.to_string());
                        closing = true;
                    }
                }
            }
            if !closing
                && !self.recovery_requested
                && self.pending_recovery.is_none()
                && self.pending_history.is_none()
            {
                if let Some(action) = self.actions.pop_front() {
                    if let Err(error) = self.action(action, cancel.clone()) {
                        if error == SessionError::UpgradeRequired {
                            self.history_status = HistoryStatus::Unsupported;
                        } else {
                            self.input_uncertain = true;
                            self.status = TabStatus::InputUncertain;
                        }
                        self.publish(&views);
                    }
                }
            }
            tokio::select! {
                biased;
                _ = cancel.changed(), if !closing => { closing = true; self.actions.clear(); self.publish(&views); }
                result = async { self.pending_history.as_mut().unwrap().await }, if self.pending_history.is_some() => {
                    self.pending_history = None; self.complete_history(result); self.publish(&views);
                }
                result = async { self.pending_recovery.as_mut().unwrap().await }, if self.pending_recovery.is_some() => {
                    self.pending_recovery = None;
                    match result {
                        Ok(handle) if !closing && !*cancel.borrow() => {
                            let (rows, cols) = self.terminal.screen().size();
                            if let Err(error) = handle.resize(rows, cols) {
                                handle.shutdown().await;
                                self.status = TabStatus::Ended(error.to_string());
                                closing = true;
                            } else {
                                self.handle = Some(handle);
                                self.last_connected = tokio::time::Instant::now();
                                self.history_initialized = history::key_from_name(&self.binding.name).is_none();
                                self.status = if self.input_uncertain { TabStatus::InputUncertain } else { TabStatus::Live };
                            }
                        }
                        Ok(handle) => { handle.shutdown().await; closing = true; }
                        Err(error) => { self.status = TabStatus::Ended(error.to_string()); closing = true; }
                    }
                    self.publish(&views);
                }
                changed = sizes.changed(), if !closing => {
                    if changed.is_ok() { let size = *sizes.borrow_and_update(); self.terminal.resize(size); if let Some(handle) = &self.handle { if handle.resize(size.rows, size.cols).is_err() { self.input_uncertain = true; self.status = TabStatus::InputUncertain; } } self.publish(&views); }
                }
                action = input.recv(), if !closing && self.actions.len() < 32 => {
                    match action { Some(action) => { if self.pending_recovery.is_some() || self.recovery_requested { self.input_uncertain = true; self.publish(&views); } else { self.actions.push_back(action); } }, None => closing = true }
                }
                _ = interval.tick() => {
                    if self.handle.is_some() && self.pending_recovery.is_none() {
                        match self.consume() {
                            Ok((Some(SessionError::Transport | SessionError::Timeout), _)) if !closing => { self.recovery_requested = true; self.status = TabStatus::Recovering; }
                            Ok((Some(error), _)) => { self.status = TabStatus::Ended(error.to_string()); closing = true; }
                            Ok((None, changed)) => { if changed { self.publish(&views); } },
                            Err(SessionError::Busy) => (),
                            Err(error) => { self.status = TabStatus::Ended(error.to_string()); closing = true; }
                        }
                        if self.discard_input {
                            self.actions.clear();
                            while input.try_recv().is_ok() {}
                            self.discard_input = false;
                        }
                        if self.recovery_requested || closing { self.publish(&views); }
                    }
                }
            }
        }
        if let Some(pending) = self.pending_history.take() {
            let completion = pending.await;
            if let Some(mut channel) = completion.channel {
                channel.close().await;
            }
        }
        if let Some(pending) = self.pending_recovery.take() {
            if let Ok(handle) = pending.await {
                handle.shutdown().await;
            }
        }
        if let Some(mut channel) = self.history.take() {
            channel.close().await;
        }
        if let Some(handle) = self.handle.take() {
            handle.shutdown().await;
        }
        if !matches!(self.status, TabStatus::Ended(_)) {
            self.status = TabStatus::Ended("Detached; remote Shell retained".into());
        }
        self.publish(&views);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn client() -> (TabClient, mpsc::Receiver<Action>, watch::Receiver<bool>) {
        let (actions, receiver) = mpsc::channel(32);
        let (cancel, cancellation) = watch::channel(false);
        let (sizes, _resize) = watch::channel(TerminalSize::new(24, 80));
        let (_finished, finished) = watch::channel(false);
        let (_, views) = watch::channel(Arc::new(TabView {
            screen: Arc::new(vt100::Parser::new(24, 80, 0).screen().clone()),
            status: TabStatus::Live,
            history: HistoryStatus::Following,
            notice: None,
        }));
        (
            TabClient {
                label: "Fixture".into(),
                host: "host".into(),
                actions,
                views,
                sizes,
                cancel,
                finished,
                budget: Arc::new(AtomicUsize::new(0)),
            },
            receiver,
            cancellation,
        )
    }

    #[test]
    fn admission_bounds_bytes_items_and_releases_failed_or_cancelled_reservations() {
        let (client, mut receiver, cancellation) = client();
        for _ in 0..4 {
            client
                .try_send(TabAction::Paste("x".repeat(65524)))
                .unwrap();
        }
        assert_eq!(client.budget.load(Ordering::SeqCst), QUEUE_BYTES);
        assert!(client
            .try_send(TabAction::Key(KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::NONE
            )))
            .is_err());
        while receiver.try_recv().is_ok() {}
        assert_eq!(client.budget.load(Ordering::SeqCst), 0);
        for _ in 0..32 {
            client.try_send(TabAction::Follow).unwrap();
        }
        assert!(client
            .try_send(TabAction::Key(KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::NONE
            )))
            .is_err());
        assert_eq!(client.budget.load(Ordering::SeqCst), 0);
        client
            .try_send(TabAction::Resize(TerminalSize::new(30, 100)))
            .unwrap();
        drop(client);
        assert!(*cancellation.borrow());
    }
}
