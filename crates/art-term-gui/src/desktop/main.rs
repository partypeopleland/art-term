#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "../platform/windows/browser.rs"]
mod browser;
#[cfg(windows)]
#[path = "../platform/windows/dpapi.rs"]
mod dpapi;
#[path = "../shared/instance_desktops.rs"]
mod instance_desktops;
#[path = "../shared/instance_restore.rs"]
mod instance_restore;
#[path = "../shared/runtime_bootstrap.rs"]
mod runtime_bootstrap;
#[path = "../shared/runtime_hosts.rs"]
mod runtime_hosts;
#[path = "../shared/runtime_sessions.rs"]
mod runtime_sessions;
#[path = "../shared/runtime_transfer.rs"]
mod runtime_transfer;
use art_term_core::connections::Connections;
use runtime_hosts::{
    gui_runtime_delete_host, gui_runtime_hosts, gui_runtime_save_host, gui_runtime_test_host,
};
use runtime_sessions::{
    gui_runtime_ack, gui_runtime_create_history_session, gui_runtime_create_session,
    gui_runtime_history, gui_runtime_recover, gui_runtime_sessions, GuiOutput, RuntimePreview,
    RuntimeSessions, SessionFailure,
};

use art_term_core::runtime_preview::RuntimeMapping;
use art_term_core::session_io::{DesktopStateDto, ScreenshotDto, SessionDto};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager, State};

#[derive(serde::Serialize)]
#[serde(untagged)]
enum GuiError {
    /// Fixed/sanitized message — the same plain-string surface the
    /// runtime desktop path already emitted.
    Plain(String),
    Runtime(SessionFailure),
}
impl From<SessionFailure> for GuiError {
    fn from(value: SessionFailure) -> Self {
        Self::Runtime(value)
    }
}
type Reply<T> = Result<T, GuiError>;

fn error(_err: impl std::fmt::Display) -> GuiError {
    GuiError::Plain("此主機桌面尚未可用或連線已結束".into())
}

#[tauri::command]
fn gui_platform() -> &'static str {
    "windows"
}

#[tauri::command]
async fn gui_open(
    hosts: State<'_, Arc<runtime_hosts::RuntimeHosts>>,
    runtime: State<'_, Arc<RuntimeSessions>>,
    target_id: String,
    rows: u16,
    cols: u16,
) -> Reply<SessionDto> {
    // Only runtime-issued targets exist; any other opaque id is stale
    // input and fails closed without reaching a backend.
    if !RuntimeSessions::is_target(&target_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime
        .open(&hosts, &target_id, rows, cols)
        .await
        .map_err(Into::into)
}
#[tauri::command]
async fn gui_poll(
    runtime: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
) -> Reply<GuiOutput> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime.poll(&session_id).map_err(Into::into)
}
#[tauri::command]
async fn gui_write(
    runtime: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
    bytes: Vec<u8>,
    terminal_reply: Option<bool>,
) -> Reply<()> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime
        .write(&session_id, &bytes, terminal_reply.unwrap_or(false))
        .await
        .map_err(Into::into)
}
#[tauri::command]
async fn gui_resize(
    runtime: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
    rows: u16,
    cols: u16,
) -> Reply<()> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime.resize(&session_id, rows, cols).map_err(Into::into)
}
#[tauri::command]
async fn gui_close(runtime: State<'_, Arc<RuntimeSessions>>, session_id: String) -> Reply<()> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime.close(&session_id).await.map_err(Into::into)
}
#[tauri::command]
async fn gui_desktop(
    runtime: State<'_, Arc<RuntimeSessions>>,
    hosts: State<'_, Arc<runtime_hosts::RuntimeHosts>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    session_id: String,
    enabled: bool,
) -> Reply<DesktopStateDto> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime
        .desktop(
            hosts.inner().as_ref(),
            &session_id,
            enabled,
            preview.inner().as_ref(),
        )
        .await
        .map_err(error)
}
#[tauri::command]
async fn gui_screenshot(
    runtime: State<'_, Arc<RuntimeSessions>>,
    hosts: State<'_, Arc<runtime_hosts::RuntimeHosts>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    session_id: String,
) -> Reply<ScreenshotDto> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(GuiError::Runtime(
            art_term_core::openab::session::SessionError::StateChanged.into(),
        ));
    }
    runtime
        .screenshot(
            hosts.inner().as_ref(),
            &session_id,
            preview.inner().as_ref(),
        )
        .await
        .map_err(error)
}

/// Windows native Runtime preview mapping.
struct NativePreview {
    connections: Option<Connections>,
}
impl RuntimePreview for NativePreview {
    fn mapped<'a>(
        &'a self,
        canonical_origin: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    > {
        let origin = canonical_origin.to_owned();
        Box::pin(async move {
            // Canonicalize defensively; an un-canonical origin maps to nothing.
            let Ok(canonical) = art_term_core::openab::profiles::canonical_origin(&origin) else {
                return Ok(None);
            };
            Ok(self
                .connections
                .as_ref()
                .and_then(|c| art_term_core::runtime_preview::mapping_for(c, &canonical)))
        })
    }
}

fn initialize() -> anyhow::Result<std::path::PathBuf> {
    let mut config = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(value) if value.starts_with("--config=") => {
                let path = &value[9..];
                anyhow::ensure!(!path.is_empty(), "--config needs a path");
                config = Some(std::path::PathBuf::from(path));
            }
            Some("--config") => {
                config = Some(std::path::PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--config needs a path"))?,
                ))
            }
            _ => anyhow::bail!("Unknown argument. Supported: --config <path>"),
        }
    }
    let config = match config {
        Some(path) => path,
        None => std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("executable directory unavailable"))?
            .join("connections.toml"),
    };
    Ok(if config.is_absolute() {
        config
    } else {
        std::env::current_dir()?.join(config)
    })
}
#[cfg(windows)]
fn startup_error(message: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            flags: u32,
        ) -> i32;
    }
    let text = format!("art-term 無法啟動\n\n{message}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let caption = "art-term".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}
#[cfg(not(windows))]
fn startup_error(message: &str) {
    eprintln!("{message}");
}

#[tauri::command]
fn gui_open_web_url(url: String) -> Result<(), String> {
    browser::open_web_url(&url).map_err(str::to_owned)
}

fn main() {
    // Resolve the sole writable source; missing files load as an empty store.
    let runtime_connections = match initialize() {
        Ok(connections) => connections,
        Err(err) => {
            startup_error(&err.to_string());
            return;
        }
    };
    let shutting_down = Arc::new(AtomicBool::new(false));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(runtime_transfer::RuntimeTransfer::default()))
        .manage(Arc::new(RuntimeSessions::default()))
        .setup(move |app| {
            #[cfg(windows)]
            {
                // A corrupt operator vault fails closed in Host commands.
                let store = art_term_core::openab::profiles::HostStore::load(
                    runtime_connections.clone(),
                    Arc::new(dpapi::WindowsCredentialCipher),
                );
                #[cfg(all(debug_assertions, feature = "openab-fixture"))]
                let fixture_ca = runtime_hosts::load_fixture_ca();
                #[cfg(all(debug_assertions, feature = "openab-fixture"))]
                let admin = match &fixture_ca {
                    Ok(Some(ca)) => art_term_core::openab::api::AdminClient::fixture_ca(ca),
                    Ok(None) => art_term_core::openab::api::AdminClient::new(),
                    Err(e) => Err(*e),
                };
                #[cfg(not(all(debug_assertions, feature = "openab-fixture")))]
                let admin = art_term_core::openab::api::AdminClient::new();
                app.manage(Arc::new(runtime_hosts::RuntimeHosts {
                    store,
                    admin,
                    connections: None,
                    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
                    fixture_ca,
                }));
                let preview: Arc<dyn RuntimePreview> =
                    Arc::new(NativePreview { connections: None });
                app.manage(preview);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::instance_desktops::gui_instance_desktops,
            crate::instance_desktops::gui_instance_save,
            crate::instance_desktops::gui_instance_delete,
            crate::instance_desktops::gui_instance_test,
            crate::instance_desktops::gui_instance_select,
            crate::instance_desktops::gui_instance_clear,
            crate::instance_desktops::gui_instance_restore,
            crate::instance_desktops::gui_instance_launch,
            gui_open_web_url,
            gui_platform,
            gui_open,
            gui_poll,
            gui_write,
            gui_resize,
            gui_close,
            gui_desktop,
            gui_screenshot,
            runtime_transfer::gui_transfer_export,
            runtime_transfer::gui_transfer_pick_import,
            runtime_transfer::gui_transfer_unlock,
            runtime_transfer::gui_transfer_apply,
            runtime_transfer::gui_transfer_cancel,
            gui_runtime_hosts,
            gui_runtime_save_host,
            gui_runtime_delete_host,
            gui_runtime_test_host,
            gui_runtime_sessions,
            gui_runtime_create_session,
            gui_runtime_create_history_session,
            gui_runtime_history,
            gui_runtime_ack,
            gui_runtime_recover
        ])
        .on_window_event(move |window, event| {
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                window
                    .state::<Arc<runtime_transfer::RuntimeTransfer>>()
                    .foreground(window.label());
            }
            if matches!(
                event,
                tauri::WindowEvent::Focused(false)
                    | tauri::WindowEvent::CloseRequested { .. }
                    | tauri::WindowEvent::Destroyed
            ) {
                window
                    .state::<Arc<runtime_transfer::RuntimeTransfer>>()
                    .background(
                        window.label(),
                        !matches!(event, tauri::WindowEvent::Focused(false)),
                    );
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if !shutting_down.swap(true, Ordering::SeqCst) {
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(8),
                            app.state::<Arc<RuntimeSessions>>().shutdown(),
                        )
                        .await;
                        app.exit(0);
                    });
                }
            }
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(err) => startup_error(&err.to_string()),
    }
}
