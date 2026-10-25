//! Native transfer orchestration: opaque, window-bound owners; no file path or secret readback.
use crate::runtime_hosts::RuntimeHosts;
use art_term_core::openab::{
    api::ApiError,
    profiles::{
        ExportSelection, ImportAction, ImportEntry, ImportPreview, ImportResult, ImportSelection,
    },
    secret::Secret,
    transfer::{self, Decoded, LockedFile},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
use zeroize::Zeroizing;
const TTL: std::time::Duration = std::time::Duration::from_secs(120);
#[derive(Default)]
pub struct RuntimeTransfer {
    inner: Mutex<Inner>,
    busy: Arc<AtomicBool>,
}
#[derive(Default)]
struct Inner {
    generation: u64,
    window: String,
    phase: Phase,
    pending: Option<Pending>,
    expiry: Option<tauri::async_runtime::JoinHandle<()>>,
    unfocused: bool,
}
#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Working,
    Picker,
    Preview,
    Committing,
}
enum Pending {
    Locked(String, LockedFile),
    Preview(String, ImportPreview),
}
impl Pending {
    fn token(&self) -> &str {
        match self {
            Self::Locked(t, _) | Self::Preview(t, _) => t,
        }
    }
}
struct Lease(Arc<AtomicBool>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferError {
    kind: &'static str,
    message: &'static str,
}
impl From<ApiError> for TransferError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Auth => Self {
                kind: "auth",
                message: "密碼錯誤或檔案已損壞，請重新選檔。",
            },
            ApiError::Capacity => Self {
                kind: "capacity",
                message: "檔案或連線數量超過上限。",
            },
            ApiError::StateChanged => Self {
                kind: "stateChanged",
                message: "操作已取消、逾時或本機設定已變更，請重新選檔。",
            },
            ApiError::Storage => Self {
                kind: "storage",
                message: "檔案讀寫失敗。輸出檔可能不完整，請刪除後重新匯出；本機設定未變更。",
            },
            _ => Self {
                kind: "protocol",
                message: "檔案格式或選擇無效，未匯入任何設定。",
            },
        }
    }
}
type Reply<T> = Result<T, TransferError>;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickReply {
    status: &'static str,
    token: Option<String>,
    entries: Vec<ImportEntry>,
}
#[derive(Serialize)]
pub struct Status {
    status: &'static str,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportInput {
    selected: Vec<ExportSelection>,
    include_credentials: bool,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyChoice {
    key: String,
    action: ImportAction,
    #[serde(default)]
    credential: String,
}

impl Inner {
    fn clear(&mut self) {
        self.pending = None;
        if let Some(task) = self.expiry.take() {
            task.abort();
        }
    }
}
impl RuntimeTransfer {
    fn begin(&self, window: &str) -> Reply<(u64, Lease)> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TransferError {
                kind: "busy",
                message: "另一個檔案操作正在進行，請稍候。",
            });
        }
        let lease = Lease(self.busy.clone());
        let mut i = self.inner.lock().map_err(|_| ApiError::StateChanged)?;
        i.clear();
        i.generation += 1;
        i.window = window.into();
        i.phase = Phase::Working;
        Ok((i.generation, lease))
    }
    fn check(i: &Inner, window: &str, generation: u64) -> Result<(), ApiError> {
        if i.window != window || i.generation != generation {
            Err(ApiError::StateChanged)
        } else {
            Ok(())
        }
    }
    fn phase(&self, window: &str, generation: u64, phase: Phase) -> Result<(), ApiError> {
        let mut i = self.inner.lock().map_err(|_| ApiError::StateChanged)?;
        Self::check(&i, window, generation)?;
        i.phase = phase;
        Ok(())
    }
    pub fn foreground(&self, window: &str) {
        if let Ok(mut i) = self.inner.lock() {
            if i.window == window {
                i.unfocused = false;
            }
        }
    }
    /// Native focus loss clears secret work; only the non-secret picker can survive.
    pub fn background(&self, window: &str, closing: bool) {
        if let Ok(mut i) = self.inner.lock() {
            if i.window == window {
                i.unfocused = true;
            }
            if i.window == window
                && i.phase != Phase::Committing
                && (closing || i.phase != Phase::Picker)
            {
                i.generation += 1;
                i.clear();
                i.phase = Phase::Idle;
            }
        }
    }
    fn cancel(&self, window: &str) -> Status {
        if let Ok(mut i) = self.inner.lock() {
            if i.window == window {
                if i.phase == Phase::Committing {
                    return Status {
                        status: "committing",
                    };
                }
                i.generation += 1;
                i.clear();
                i.phase = Phase::Idle;
            }
        }
        Status {
            status: "cancelled",
        }
    }
    fn install(
        self: &Arc<Self>,
        window: &str,
        generation: u64,
        pending: Pending,
    ) -> Result<(), ApiError> {
        self.install_for(window, generation, pending, TTL)
    }
    fn install_for(
        self: &Arc<Self>,
        window: &str,
        generation: u64,
        pending: Pending,
        ttl: std::time::Duration,
    ) -> Result<(), ApiError> {
        let mut i = self.inner.lock().map_err(|_| ApiError::StateChanged)?;
        Self::check(&i, window, generation)?;
        if i.unfocused
            && matches!(&pending, Pending::Preview(_, p) if p.entries.iter().any(|e| e.has_secret()))
        {
            return Err(ApiError::StateChanged);
        }
        i.clear();
        let token = pending.token().to_owned();
        i.pending = Some(pending);
        i.phase = Phase::Preview;
        let weak: Weak<Self> = Arc::downgrade(self);
        i.expiry = Some(tauri::async_runtime::spawn(async move {
            tokio::time::sleep(ttl).await;
            if let Some(state) = weak.upgrade() {
                if let Ok(mut i) = state.inner.lock() {
                    if i.generation == generation
                        && i.pending.as_ref().is_some_and(|p| p.token() == token)
                    {
                        i.pending = None;
                        i.phase = Phase::Idle;
                        i.generation += 1;
                    }
                };
            }
        }));
        Ok(())
    }
    fn take(&self, window: &str, token: &str, committing: bool) -> Reply<(u64, Pending, Lease)> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TransferError {
                kind: "busy",
                message: "另一個檔案操作正在進行，請稍候。",
            });
        }
        let lease = Lease(self.busy.clone());
        let mut i = self.inner.lock().map_err(|_| ApiError::StateChanged)?;
        if i.unfocused
            || i.window != window
            || !i.pending.as_ref().is_some_and(|p| p.token() == token)
        {
            return Err(ApiError::StateChanged.into());
        }
        let pending = i.pending.take().ok_or(ApiError::StateChanged)?;
        i.clear();
        // This lock is the commit grant linearization point. Cancel after this cannot revoke it.
        i.phase = if committing {
            Phase::Committing
        } else {
            Phase::Working
        };
        Ok((i.generation, pending, lease))
    }
}
fn read_bounded<R: Read>(reader: R) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::new();
    reader
        .take(transfer::MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ApiError::Storage)?;
    if bytes.len() > transfer::MAX_FILE {
        return Err(ApiError::Capacity);
    }
    Ok(bytes)
}
fn write_selected(app: &tauri::AppHandle, path: FilePath, bytes: &[u8]) -> Result<(), ApiError> {
    #[cfg(not(target_os = "android"))]
    {
        let path = path.into_path().map_err(|_| ApiError::Storage)?;
        let parent = path.parent().ok_or(ApiError::Storage)?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| ApiError::Storage)?;
        file.write_all(bytes).map_err(|_| ApiError::Storage)?;
        file.flush().map_err(|_| ApiError::Storage)?;
        file.as_file().sync_all().map_err(|_| ApiError::Storage)?;
        file.persist(path).map_err(|_| ApiError::Storage)?;
        let _ = app;
    }
    #[cfg(target_os = "android")]
    {
        let mut options = OpenOptions::new();
        options.write(true).truncate(true);
        let mut file = app
            .fs()
            .open(path, options)
            .map_err(|_| ApiError::Storage)?;
        file.write_all(bytes).map_err(|_| ApiError::Storage)?;
        file.flush().map_err(|_| ApiError::Storage)?;
        drop(file);
    }
    Ok(())
}
#[tauri::command]
pub async fn gui_transfer_export(
    window: tauri::WebviewWindow,
    hosts: State<'_, Arc<RuntimeHosts>>,
    transfer: State<'_, Arc<RuntimeTransfer>>,
    mut input: ExportInput,
) -> Reply<Status> {
    let password = Zeroizing::new(std::mem::take(&mut input.password));
    let state = transfer.inner().clone();
    let hosts = hosts.inner().clone();
    let label = window.label().to_owned();
    let app = window.app_handle().clone();
    let (generation, lease) = state.begin(&label)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let result = (|| -> Result<Status, ApiError> {
            if !input.include_credentials && !password.is_empty() {
                return Err(ApiError::Protocol);
            }
            let payload = hosts.store()?.export_snapshot(
                &input.selected,
                input.include_credentials,
                hosts.connections.as_ref(),
            )?;
            // All plaintext owners are dropped by encode before opening the system picker.
            let bytes = transfer::encode(
                payload,
                if input.include_credentials {
                    Some(password)
                } else {
                    None
                },
            )?;
            state.phase(&label, generation, Phase::Picker)?;
            let dialog = app.dialog().file();
            #[cfg(windows)]
            let dialog = dialog.set_parent(&window);
            let path = dialog
                .set_file_name("art-term-connections.json")
                .add_filter("連線設定", &["json"])
                .blocking_save_file();
            state.phase(&label, generation, Phase::Working)?;
            let Some(path) = path else {
                return Ok(Status {
                    status: "cancelled",
                });
            };
            write_selected(&app, path, &bytes)?;
            Ok(Status { status: "saved" })
        })();
        let _ = state.phase(&label, generation, Phase::Idle);
        result.map_err(Into::into)
    })
    .await
    .map_err(|_| TransferError::from(ApiError::StateChanged))?
}
#[tauri::command]
pub async fn gui_transfer_pick_import(
    window: tauri::WebviewWindow,
    hosts: State<'_, Arc<RuntimeHosts>>,
    transfer: State<'_, Arc<RuntimeTransfer>>,
) -> Reply<PickReply> {
    let state = transfer.inner().clone();
    let hosts = hosts.inner().clone();
    let label = window.label().to_owned();
    let app = window.app_handle().clone();
    let (generation, lease) = state.begin(&label)?;
    state.phase(&label, generation, Phase::Picker)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let result = (|| -> Result<PickReply, ApiError> {
            let dialog = app.dialog().file();
            #[cfg(windows)]
            let dialog = dialog.set_parent(&window);
            let path = dialog
                .add_filter("連線設定", &["json"])
                .blocking_pick_file();
            state.phase(&label, generation, Phase::Working)?;
            let Some(path) = path else {
                state.phase(&label, generation, Phase::Idle)?;
                return Ok(PickReply {
                    status: "cancelled",
                    token: None,
                    entries: vec![],
                });
            };
            let mut options = OpenOptions::new();
            options.read(true);
            let bytes = read_bounded(
                app.fs()
                    .open(path, options)
                    .map_err(|_| ApiError::Storage)?,
            )?;
            let token = uuid::Uuid::new_v4().to_string();
            let (pending, entries, status) = match transfer::decode(&bytes)? {
                Decoded::Public(p) => {
                    let p = hosts.store()?.preview_import(p)?;
                    let entries = p.entries.clone();
                    (Pending::Preview(token.clone(), p), entries, "preview")
                }
                Decoded::Encrypted(p) => (
                    Pending::Locked(token.clone(), p),
                    vec![],
                    "requiresPassword",
                ),
            };
            state.install(&label, generation, pending)?;
            Ok(PickReply {
                status,
                token: Some(token),
                entries,
            })
        })();
        if result.is_err() {
            let _ = state.phase(&label, generation, Phase::Idle);
        }
        result.map_err(Into::into)
    })
    .await
    .map_err(|_| TransferError::from(ApiError::StateChanged))?
}
#[tauri::command]
pub async fn gui_transfer_unlock(
    window: tauri::WebviewWindow,
    hosts: State<'_, Arc<RuntimeHosts>>,
    transfer: State<'_, Arc<RuntimeTransfer>>,
    token: String,
    password: String,
) -> Reply<PickReply> {
    let password = Zeroizing::new(password);
    let state = transfer.inner().clone();
    let hosts = hosts.inner().clone();
    let label = window.label().to_owned();
    let (generation, pending, lease) = state.take(&label, &token, false)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let result = (|| -> Result<PickReply, ApiError> {
            let Pending::Locked(_, file) = pending else {
                return Err(ApiError::Protocol);
            };
            let p = hosts
                .store()?
                .preview_import(transfer::unlock(file, password)?)?;
            let token = uuid::Uuid::new_v4().to_string();
            let entries = p.entries.clone();
            state.install(&label, generation, Pending::Preview(token.clone(), p))?;
            Ok(PickReply {
                status: "preview",
                token: Some(token),
                entries,
            })
        })();
        if result.is_err() {
            let _ = state.phase(&label, generation, Phase::Idle);
        }
        result.map_err(Into::into)
    })
    .await
    .map_err(|_| TransferError::from(ApiError::StateChanged))?
}
#[tauri::command]
pub async fn gui_transfer_apply(
    window: tauri::WebviewWindow,
    hosts: State<'_, Arc<RuntimeHosts>>,
    transfer: State<'_, Arc<RuntimeTransfer>>,
    token: String,
    choices: Vec<ApplyChoice>,
) -> Reply<ImportResult> {
    let state = transfer.inner().clone();
    let hosts = hosts.inner().clone();
    let label = window.label().to_owned();
    // Zeroize every input, including later inputs when any validation fails.
    let choices: Vec<_> = choices
        .into_iter()
        .map(|mut c| {
            let v = Zeroizing::new(std::mem::take(&mut c.credential));
            (c.key, c.action, v)
        })
        .collect();
    let (generation, pending, lease) = state.take(&label, &token, false)?;
    let mut selected = Vec::new();
    if choices.len() > 128 {
        let _ = state.phase(&label, generation, Phase::Idle);
        return Err(ApiError::Capacity.into());
    }
    for (key, action, mut credential) in choices {
        let credential = if credential.is_empty() {
            None
        } else {
            Some(Secret::new(std::mem::take(&mut *credential)).map_err(|e| {
                let _ = state.phase(&label, generation, Phase::Idle);
                TransferError::from(e)
            })?)
        };
        selected.push(ImportSelection {
            key,
            action,
            credential,
        });
    }

    // Validation consumed the preview, but cancellation may still revoke commit until here.
    state.phase(&label, generation, Phase::Committing)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let result = match pending {
            Pending::Preview(_, p) => hosts.store().and_then(|s| s.apply_import(p, selected)),
            _ => Err(ApiError::Protocol),
        };
        let _ = state.phase(&label, generation, Phase::Idle);
        result.map_err(Into::into)
    })
    .await
    .map_err(|_| TransferError::from(ApiError::StateChanged))?
}
#[tauri::command]
pub fn gui_transfer_cancel(
    window: tauri::WebviewWindow,
    transfer: State<'_, Arc<RuntimeTransfer>>,
) -> Status {
    transfer.cancel(window.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use art_term_core::openab::{
        profiles::HostStore,
        secret::{CredentialCipher, CredentialContext},
    };
    struct Cipher;
    impl CredentialCipher for Cipher {
        fn seal(&self, _: &Secret, _: &CredentialContext) -> Result<Vec<u8>, ApiError> {
            Ok(vec![1])
        }
        fn unseal(&self, _: &[u8], _: &CredentialContext) -> Result<Secret, ApiError> {
            Secret::new("fictional".into())
        }
    }
    fn pending() -> Pending {
        let dir = tempfile::tempdir().unwrap();
        let store = HostStore::load(dir.path().join("store.json"), Arc::new(Cipher)).unwrap();
        Pending::Preview(
            "opaque".into(),
            store
                .preview_import(transfer::Payload {
                    desktops: Vec::new(),
                    entries: vec![transfer::Entry {
                        name: "Fixture".into(),
                        origin: "https://fixture.test".into(),
                        metadata: Default::default(),
                        credential: Some(Secret::new("fictional".into()).unwrap()),
                    }],
                })
                .unwrap(),
        )
    }
    #[tokio::test]
    async fn bounded_ttl_window_binding_busy_cancel_late_and_commit_grant() {
        let state = Arc::new(RuntimeTransfer::default());
        let (g, lease) = state.begin("main").unwrap_or_else(|_| panic!("begin"));
        assert!(state.begin("main").is_err());
        state
            .install_for("main", g, pending(), std::time::Duration::from_millis(10))
            .unwrap();
        drop(lease);
        assert!(state.take("other", "opaque", false).is_err());
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        assert!(state.inner.lock().unwrap().pending.is_none());
        assert!(state.take("main", "opaque", false).is_err());
        let (g, lease) = state.begin("main").unwrap_or_else(|_| panic!("begin"));
        state.cancel("main");
        assert!(state.install("main", g, pending()).is_err());
        // Cancellation cannot queue another KDF while the cancelled calculation is alive.
        assert!(state.begin("main").is_err());
        drop(lease);
        let (g, lease) = state.begin("main").unwrap_or_else(|_| panic!("begin"));
        state.phase("main", g, Phase::Picker).unwrap();
        state.background("main", false);
        assert_eq!(state.inner.lock().unwrap().generation, g);
        assert!(state.install("main", g, pending()).is_err());
        state.foreground("main");
        state.install("main", g, pending()).unwrap();
        drop(lease);
        state.background("main", false);
        assert!(state.inner.lock().unwrap().pending.is_none());
        state.foreground("main");
        let (g, lease) = state.begin("main").unwrap_or_else(|_| panic!("begin"));
        state.install("main", g, pending()).unwrap();
        drop(lease);
        let (_g, owner, lease) = state
            .take("main", "opaque", true)
            .unwrap_or_else(|_| panic!("take"));
        assert_eq!(state.cancel("main").status, "committing");
        assert!(state.take("main", "opaque", true).is_err());
        drop(owner);
        drop(lease);
    }
    #[tokio::test]
    async fn screen_token_preview_is_sensitive_without_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let store = HostStore::load(dir.path().join("connections.toml"), Arc::new(Cipher)).unwrap();
        let preview = store
            .preview_import(transfer::Payload {
                entries: Vec::new(),
                desktops: vec![transfer::ScreenEntry {
                    name: "Screen".into(),
                    mcp_url: "https://screen.test/mcp".into(),
                    token: Some(Secret::new("fictional-screen-secret".into()).unwrap()),
                }],
            })
            .unwrap();
        let json = serde_json::to_string(&preview.entries).unwrap();
        assert!(json.contains("hasToken"));
        assert!(!json.contains("fictional-screen-secret"));
        let state = Arc::new(RuntimeTransfer::default());
        let (generation, lease) = state.begin("main").unwrap_or_else(|_| panic!("begin"));
        state.phase("main", generation, Phase::Picker).unwrap();
        state.background("main", false);
        assert!(state
            .install(
                "main",
                generation,
                Pending::Preview("opaque".into(), preview)
            )
            .is_err());
        drop(lease);
    }
    #[test]
    fn bounded_read_and_dto_never_contains_credentials() {
        assert!(read_bounded(std::io::repeat(b'x')).is_err());
        assert_eq!(read_bounded(&b"fixture"[..]).unwrap(), b"fixture");
        let Pending::Preview(_, p) = pending() else {
            panic!("preview");
        };
        let json = serde_json::to_string(&PickReply {
            status: "preview",
            token: Some("opaque".into()),
            entries: p.entries.clone(),
        })
        .unwrap();
        assert!(!json.contains("fictional"));
        assert!(!json.contains("ciphertext"));
        assert!(json.contains("hasCredential"));
        for kind in [
            ApiError::Auth,
            ApiError::Protocol,
            ApiError::Storage,
            ApiError::Capacity,
            ApiError::StateChanged,
        ] {
            let json = serde_json::to_string(&TransferError::from(kind)).unwrap();
            assert!(!json.contains("fictional"));
        }
    }
}
