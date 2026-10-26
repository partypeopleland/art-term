use crate::backend::{BackendError, Connector, Result, ScreenSelection};
use art_term_core::{
    instance_mcp::{DesktopProfile, DesktopSnapshot, Mcp},
    instance_restore,
    openab::{
        api::{AdminClient, ApiError},
        profiles::HostStore,
        screenshot::{self, ShellOwnership},
        session::{SessionBinding, SessionError},
    },
};
use serde_json::{json, Value};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DesktopRequest {
    Select(ScreenSelection),
    Restore,
    Clear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolReadiness {
    Ready,
    Unconfirmed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DesktopState {
    Unknown,
    Unselected,
    Selected {
        screen: ScreenSelection,
        tools: ToolReadiness,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopView {
    pub state: DesktopState,
    pub busy: bool,
    pub notice: Option<&'static str>,
}

pub(super) struct Desktop {
    requests: mpsc::Sender<DesktopRequest>,
    view: watch::Sender<DesktopView>,
    busy: Arc<AtomicBool>,
    closing: watch::Receiver<bool>,
    store: Arc<HostStore>,
    binding: SessionBinding,
    task: Mutex<Option<JoinHandle<()>>>,
}
struct Context {
    binding: SessionBinding,
    store: Arc<HostStore>,
    api: AdminClient,
    connector: Connector,
    ownership: Arc<dyn ShellOwnership>,
    closing: watch::Receiver<bool>,
    cancellation: watch::Receiver<u64>,
}
impl Desktop {
    pub(super) fn start(
        binding: SessionBinding,
        store: Arc<HostStore>,
        api: AdminClient,
        connector: Connector,
        ownership: Arc<dyn ShellOwnership>,
        closing: watch::Receiver<bool>,
    ) -> Arc<Self> {
        let (requests, receiver) = mpsc::channel(1);
        let (view, _) = watch::channel(DesktopView {
            state: DesktopState::Unknown,
            busy: false,
            notice: None,
        });
        let busy = Arc::new(AtomicBool::new(false));
        let (cancel, cancellation) = watch::channel(0);
        let context = Context {
            binding: binding.clone(),
            store: store.clone(),
            api,
            connector,
            ownership,
            closing: closing.clone(),
            cancellation,
        };
        let task = tokio::spawn(run(
            context,
            closing.clone(),
            receiver,
            view.clone(),
            busy.clone(),
            cancel,
        ));
        Arc::new(Self {
            requests,
            view,
            busy,
            closing,
            store,
            binding,
            task: Mutex::new(Some(task)),
        })
    }
    pub(super) fn request(&self, request: DesktopRequest) -> Result<()> {
        if *self.closing.borrow() || !self.store.is_current(&self.binding.profile)? {
            return Err(ApiError::StateChanged.into());
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(BackendError::Busy);
        }
        self.view.send_modify(|v| {
            v.busy = true;
            v.notice = None;
        });
        if self.requests.try_send(request).is_err() {
            self.busy.store(false, Ordering::SeqCst);
            self.view.send_modify(|v| v.busy = false);
            return Err(BackendError::Busy);
        }
        Ok(())
    }
    pub(super) fn view(&self) -> Result<DesktopView> {
        let mut view = self.view.borrow().clone();
        if *self.closing.borrow() || !self.store.is_current(&self.binding.profile)? {
            view.state = DesktopState::Unknown;
            view.notice = Some("連線已變更，請重新連線後設定桌面。");
        } else if let DesktopState::Selected { screen, .. } = &view.state {
            if !self
                .store
                .desktops()?
                .iter()
                .any(|p| p.id == screen.id && p.revision == screen.revision)
            {
                view.state = DesktopState::Unknown;
                view.notice = Some("桌面設定已變更，請重新選擇桌面。");
            }
        }
        Ok(view)
    }
    pub(super) async fn stop(&self) {
        let task = self.task.lock().ok().and_then(|mut task| task.take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

trait Source: Send {
    fn execute(
        &self,
        request: DesktopRequest,
    ) -> impl Future<Output = Result<(DesktopState, Option<&'static str>)>> + Send;
}
impl Source for Context {
    async fn execute(
        &self,
        request: DesktopRequest,
    ) -> Result<(DesktopState, Option<&'static str>)> {
        Context::execute(self, request).await
    }
}
async fn run(
    context: impl Source,
    mut closing: watch::Receiver<bool>,
    mut requests: mpsc::Receiver<DesktopRequest>,
    view: watch::Sender<DesktopView>,
    busy: Arc<AtomicBool>,
    cancel: watch::Sender<u64>,
) {
    loop {
        if *closing.borrow() {
            break;
        }
        let request = tokio::select! {
            _ = closing.changed() => break,
            request = requests.recv() => match request { Some(request) => request, None => break },
        };
        let operation = context.execute(request);
        tokio::pin!(operation);
        let result = tokio::select! {
            result = &mut operation => Some(result),
            _ = wait_closed(closing.clone()) => {
                cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                // The task owns the core future until its management Shell cleanup finishes.
                let _ = operation.await;
                None
            },
        };
        if *closing.borrow() {
            break;
        }
        let Some(result) = result else {
            break;
        };
        let (state, notice) = result.unwrap_or((
            DesktopState::Unknown,
            Some("桌面工具狀態未確認；請重試，並確認此新 Shell 已安裝通用桌面工具。"),
        ));
        view.send_replace(DesktopView {
            state,
            busy: false,
            notice,
        });
        busy.store(false, Ordering::SeqCst);
    }
    busy.store(false, Ordering::SeqCst);
    view.send_modify(|v| v.busy = false);
}
async fn wait_closed(mut closing: watch::Receiver<bool>) {
    while !*closing.borrow() && closing.changed().await.is_ok() {}
}
impl Context {
    fn check(&self) -> Result<()> {
        if *self.closing.borrow() || !self.store.is_current(&self.binding.profile)? {
            return Err(ApiError::StateChanged.into());
        }
        Ok(())
    }

    async fn validate_source(&self, snapshot: DesktopSnapshot) -> Result<()> {
        let mut mcp = Mcp::connect(snapshot)
            .await
            .map_err(|_| ApiError::Protocol)?;
        mcp.close().await;
        Ok(())
    }

    async fn current_binding(&self) -> Result<()> {
        self.check()?;
        let snapshot = self.store.snapshot(&self.binding.profile.id)?;
        let list = self.api.list(&snapshot).await?;
        self.binding.verify_list(&list)?;
        self.check()
    }
    fn screen(&self, selected: &ScreenSelection) -> Result<DesktopSnapshot> {
        self.check()?;
        let snapshot = self.store.desktop(&selected.id)?;
        if snapshot.profile.revision != selected.revision {
            return Err(ApiError::StateChanged.into());
        }
        Ok(snapshot)
    }
    async fn control(&self, payload: Zeroizing<String>) -> Result<Value> {
        self.current_binding().await?;
        let snapshot = self.store.snapshot(&self.binding.profile.id)?;
        let connector = self.connector.clone();
        let result = screenshot::configure(
            &self.api,
            snapshot,
            &self.binding.name,
            payload,
            self.ownership.clone(),
            self.cancellation.clone(),
            move |binding, token, since, generation| async move {
                connector
                    .attach_token(binding, token, since, generation)
                    .await
            },
        )
        .await?;
        self.current_binding().await?;
        if result["ok"] != true {
            return Err(SessionError::Protocol.into());
        }
        Ok(result)
    }
    async fn execute(
        &self,
        request: DesktopRequest,
    ) -> Result<(DesktopState, Option<&'static str>)> {
        self.current_binding().await?;
        match request {
            DesktopRequest::Select(selected) => {
                let snapshot = self.screen(&selected)?;
                let profile = snapshot.profile.clone();
                self.validate_source(snapshot).await?;
                let snapshot = self.screen(&selected)?;
                let payload = Zeroizing::new(json!({"op":"configure","id":profile.id,"revision":profile.revision,"name":profile.name,"url":profile.mcp_url,"token":snapshot.token.expose()}).to_string());
                let result = self.control(payload).await;
                self.screen(&selected)?;
                let ready = result
                    .as_ref()
                    .is_ok_and(|result| tools_confirmed(result, &profile));
                Ok((
                    DesktopState::Selected {
                        screen: selected,
                        tools: if ready {
                            ToolReadiness::Ready
                        } else {
                            ToolReadiness::Unconfirmed
                        },
                    },
                    if ready {
                        None
                    } else {
                        Some("已選擇桌面；桌面工具未確認。請確認通用工具已安裝，並使用新建的 Shell。")
                    },
                ))
            }
            DesktopRequest::Restore => {
                let status = self
                    .control(Zeroizing::new("{\"op\":\"current\"}".into()))
                    .await?;
                let profiles = self.store.desktops()?;
                let selected = instance_restore::resolve(&status, &profiles)
                    .map_err(|_| ApiError::StateChanged)?;
                let Some(selected) = selected else {
                    return Ok((
                        DesktopState::Unselected,
                        Some("此 Shell 尚未設定桌面，請點選「桌面」選擇 Screen。"),
                    ));
                };
                let screen = ScreenSelection {
                    id: selected.profile.id.clone(),
                    revision: selected.profile.revision,
                };
                self.validate_source(self.screen(&screen)?).await?;
                let verified = self
                    .control(Zeroizing::new(
                        json!({"op":"verify","expected_targets":selected.broker_targets})
                            .to_string(),
                    ))
                    .await;
                let ready = verified
                    .as_ref()
                    .is_ok_and(|value| verified_targets(value, &status));
                let current = self
                    .control(Zeroizing::new("{\"op\":\"current\"}".into()))
                    .await?;
                let latest = self.screen(&screen)?.profile;
                selected
                    .confirm(&current, &latest, 0, 0, *self.closing.borrow())
                    .map_err(|_| ApiError::StateChanged)?;
                Ok((
                    DesktopState::Selected {
                        screen,
                        tools: if ready {
                            ToolReadiness::Ready
                        } else {
                            ToolReadiness::Unconfirmed
                        },
                    },
                    if ready {
                        None
                    } else {
                        Some("已找到既有桌面指派；桌面工具尚未確認，請重新選擇桌面或檢查受控端。")
                    },
                ))
            }
            DesktopRequest::Clear => {
                let result = self
                    .control(Zeroizing::new("{\"op\":\"clear\"}".into()))
                    .await?;
                if !cleared(&result) {
                    return Err(SessionError::Protocol.into());
                }
                Ok((
                    DesktopState::Unselected,
                    Some("已取消此 Shell 的桌面工具；共享桌面仍在執行。"),
                ))
            }
        }
    }
}
fn tools_confirmed(result: &Value, profile: &DesktopProfile) -> bool {
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
fn verified_targets(result: &Value, expected: &Value) -> bool {
    result["ok"] == true
        && result["adapters"].as_u64().is_some_and(|n| n > 0)
        && result["adapters"] == expected["adapters"]
        && result["targets"] == expected["targets"]
}
fn cleared(result: &Value) -> bool {
    result["ok"] == true
        && result["targets"].as_array().is_some_and(|targets| {
            result["adapters"].as_u64() == Some(targets.len() as u64)
                && targets.iter().all(Value::is_null)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> DesktopProfile {
        DesktopProfile {
            id: "fixture".into(),
            revision: 3,
            name: "Fixture".into(),
            mcp_url: "https://screen.example/mcp".into(),
            token_present: true,
        }
    }
    #[test]
    fn preview_success_and_zero_adapters_never_mean_tools_ready() {
        let p = profile();
        assert!(!tools_confirmed(
            &json!({"ok":true,"adapters":0,"targets":[]}),
            &p
        ));
        for target in [
            json!({"id":"other","revision":3,"generation":1}),
            json!({"id":"fixture","revision":2,"generation":1}),
            json!({"id":"fixture","revision":3,"generation":0}),
        ] {
            assert!(!tools_confirmed(
                &json!({"ok":true,"adapters":1,"targets":[target]}),
                &p
            ));
        }
        assert!(tools_confirmed(
            &json!({"ok":true,"adapters":1,"targets":[{"id":"fixture","revision":3,"generation":4}]}),
            &p
        ));
        assert!(!tools_confirmed(
            &json!({"ok":true,"adapters":2,"targets":[{"id":"fixture","revision":3,"generation":4}]}),
            &p
        ));
    }
    #[test]
    fn verify_requires_same_positive_generation_and_clear_requires_all_targets_empty() {
        let expected = json!({"ok":true,"adapters":1,"targets":[{"id":"fixture","revision":3,"generation":4}]});
        assert!(verified_targets(&expected, &expected));
        let newer = json!({"ok":true,"adapters":1,"targets":[{"id":"fixture","revision":3,"generation":5}]});
        assert!(!verified_targets(&newer, &expected));
        assert!(!cleared(&expected));
        assert!(cleared(&json!({"ok":true,"adapters":1,"targets":[null]})));
        assert!(!cleared(&json!({"ok":true,"adapters":2,"targets":[null]})));
    }
    #[tokio::test]
    async fn close_signals_cancellation_and_joins_inflight_cleanup() {
        struct Delayed {
            cancelled: watch::Receiver<u64>,
            started: Arc<tokio::sync::Notify>,
            cleaned: Arc<AtomicBool>,
        }
        impl Source for Delayed {
            async fn execute(
                &self,
                _: DesktopRequest,
            ) -> Result<(DesktopState, Option<&'static str>)> {
                let mut cancelled = self.cancelled.clone();
                self.started.notify_one();
                cancelled.changed().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                self.cleaned.store(true, Ordering::SeqCst);
                Err(SessionError::Cancelled.into())
            }
        }
        let (close, closing) = watch::channel(false);
        let (cancel, cancelled) = watch::channel(0);
        let (requests, receiver) = mpsc::channel(1);
        let (view, _) = watch::channel(DesktopView {
            state: DesktopState::Unknown,
            busy: true,
            notice: None,
        });
        let started = Arc::new(tokio::sync::Notify::new());
        let cleaned = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(run(
            Delayed {
                cancelled,
                started: started.clone(),
                cleaned: cleaned.clone(),
            },
            closing,
            receiver,
            view.clone(),
            busy.clone(),
            cancel,
        ));
        requests.send(DesktopRequest::Restore).await.unwrap();
        started.notified().await;
        close.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            cleaned.load(Ordering::SeqCst),
            "in-flight cleanup must finish before the owner exits"
        );
        assert!(!busy.load(Ordering::SeqCst));
        assert!(!view.borrow().busy);
    }
    #[tokio::test]
    async fn admission_is_single_flight_and_stale_or_closed_requests_fail_before_transport() {
        use crate::vault::{Vault, VaultPaths};
        use art_term_core::openab::secret::Secret;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::Builder::new()
            .prefix("art-term-desktop-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths = VaultPaths::resolve(Some(root.path().join("connections.toml"))).unwrap();
        let vault = Vault::open_account(paths).unwrap();
        let store = vault.store();
        let profile = store
            .add(
                "Fixture".into(),
                "https://fixture.example".into(),
                Secret::new("fixture-only-token".into()).unwrap(),
            )
            .unwrap();
        let (requests, mut receiver) = mpsc::channel(1);
        let (view, _) = watch::channel(DesktopView {
            state: DesktopState::Unknown,
            busy: false,
            notice: None,
        });
        let (close, closing) = watch::channel(false);
        let desktop = Desktop {
            requests,
            view,
            busy: Arc::new(AtomicBool::new(false)),
            closing,
            store,
            binding: SessionBinding {
                profile,
                name: "th2-aaaaaaaaaaaaaaaaaaaaaaaa".into(),
                generation: 1,
            },
            task: Mutex::new(None),
        };
        desktop.request(DesktopRequest::Restore).unwrap();
        assert!(desktop.view().unwrap().busy);
        assert!(matches!(
            desktop.request(DesktopRequest::Clear),
            Err(BackendError::Busy)
        ));
        assert_eq!(receiver.recv().await, Some(DesktopRequest::Restore));
        assert!(
            receiver.try_recv().is_err(),
            "second action must not be queued"
        );
        desktop.busy.store(false, Ordering::SeqCst);
        close.send_replace(true);
        assert!(desktop.request(DesktopRequest::Clear).is_err());
        assert!(receiver.try_recv().is_err());
        close.send_replace(false);
        let screen = desktop
            .store
            .save_desktop(
                None,
                None,
                "Screen".into(),
                "https://screen.example/mcp".into(),
                Some(Secret::new("fixture-screen-token".into()).unwrap()),
            )
            .unwrap();
        desktop.view.send_modify(|view| {
            view.state = DesktopState::Selected {
                screen: ScreenSelection {
                    id: screen.id.clone(),
                    revision: screen.revision,
                },
                tools: ToolReadiness::Ready,
            }
        });
        assert!(matches!(
            desktop.view().unwrap().state,
            DesktopState::Selected {
                tools: ToolReadiness::Ready,
                ..
            }
        ));
        desktop
            .store
            .delete_desktop(&screen.id, screen.revision)
            .unwrap();
        assert_eq!(
            desktop.view().unwrap().state,
            DesktopState::Unknown,
            "stale local Screen must not claim tools are ready"
        );
        assert!(
            receiver.try_recv().is_err(),
            "profile deletion must not implicitly clear the broker"
        );
        desktop
            .store
            .delete(
                &desktop.binding.profile.id,
                desktop.binding.profile.revision,
            )
            .unwrap();
        assert!(desktop.request(DesktopRequest::Restore).is_err());
        assert!(receiver.try_recv().is_err());
    }
}
