//! Android IPC facade — runtime targets only. Runtime credentials and
//! grants never cross the WebView boundary.
use crate::runtime_hosts::RuntimeHosts;
use crate::runtime_sessions::{GuiOutput, RuntimePreview, RuntimeSessions, SessionFailure};
use art_term_core::session_io::{DesktopStateDto, ScreenshotDto, SessionDto};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[derive(serde::Serialize)]
#[serde(untagged)]
enum MobileSessionError {
    Plain(String),
    Runtime(SessionFailure),
}
impl From<String> for MobileSessionError {
    fn from(value: String) -> Self {
        Self::Plain(value)
    }
}
impl From<SessionFailure> for MobileSessionError {
    fn from(value: SessionFailure) -> Self {
        Self::Runtime(value)
    }
}
type Reply<T> = Result<T, MobileSessionError>;

fn stale_id() -> MobileSessionError {
    // Only runtime-issued ids exist; any other opaque id is stale input
    // and fails closed without reaching a backend.
    MobileSessionError::Runtime(art_term_core::openab::session::SessionError::StateChanged.into())
}

#[tauri::command]
fn gui_platform() -> &'static str {
    "android"
}

/// Android Runtime desktop bridge. All I/O uses authenticated native binding.
struct NativePreview {
    connections: Option<art_term_core::connections::Connections>,
}
impl RuntimePreview for NativePreview {
    fn mapped<'a>(
        &'a self,
        canonical_origin: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<Option<art_term_core::runtime_preview::RuntimeMapping>>,
                > + Send
                + 'a,
        >,
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

#[tauri::command]
async fn mobile_desktop(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    session_id: String,
    enabled: bool,
) -> Result<DesktopStateDto, MobileSessionError> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err("此主機桌面尚未可用或連線已結束".to_owned().into());
    }
    sessions
        .desktop(
            hosts.inner().as_ref(),
            &session_id,
            enabled,
            preview.inner().as_ref(),
        )
        .await
        .map_err(|_| MobileSessionError::Plain("此主機桌面尚未可用或連線已結束".into()))
}
#[tauri::command]
async fn mobile_screenshot(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    preview: State<'_, Arc<dyn RuntimePreview>>,
    session_id: String,
) -> Result<ScreenshotDto, String> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err("無法取得此頁桌面圖片".into());
    }
    sessions
        .screenshot(
            hosts.inner().as_ref(),
            &session_id,
            preview.inner().as_ref(),
        )
        .await
        .map_err(|_| "無法取得此頁桌面圖片".into())
}
#[tauri::command]
async fn mobile_open(
    target_id: String,
    rows: u16,
    cols: u16,
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
) -> Result<SessionDto, MobileSessionError> {
    if !RuntimeSessions::is_target(&target_id) {
        return Err(stale_id());
    }
    sessions
        .open(&hosts, &target_id, rows, cols)
        .await
        .map_err(Into::into)
}
#[tauri::command]
async fn mobile_poll(
    session_id: String,
    sessions: State<'_, Arc<RuntimeSessions>>,
) -> Result<GuiOutput, MobileSessionError> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(stale_id());
    }
    sessions.poll(&session_id).map_err(Into::into)
}
#[tauri::command]
async fn mobile_write(
    session_id: String,
    bytes: Vec<u8>,
    terminal_reply: Option<bool>,
    sessions: State<'_, Arc<RuntimeSessions>>,
) -> Result<(), MobileSessionError> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(stale_id());
    }
    sessions.write(&session_id, &bytes, terminal_reply.unwrap_or(false)).await.map_err(Into::into)
}
#[tauri::command]
async fn mobile_resize(
    session_id: String,
    rows: u16,
    cols: u16,
    sessions: State<'_, Arc<RuntimeSessions>>,
) -> Result<(), MobileSessionError> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(stale_id());
    }
    sessions.resize(&session_id, rows, cols).map_err(Into::into)
}
#[tauri::command]
async fn mobile_close(
    session_id: String,
    sessions: State<'_, Arc<RuntimeSessions>>,
) -> Result<(), MobileSessionError> {
    if !RuntimeSessions::is_session(&session_id) {
        return Err(stale_id());
    }
    sessions.close(&session_id).await.map_err(Into::into)
}

pub fn run() {
    let shutting_down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_complete = Arc::new(std::sync::atomic::AtomicBool::new(false));
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(crate::runtime_transfer::RuntimeTransfer::default()))
        .plugin(tauri_plugin_mobile_vault::init())
        .plugin(tauri_plugin_mobile_voice::init())
        .plugin(tauri_plugin_mobile_history::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("mobile-navigation")
                .on_navigation(|_, url| {
                    matches!(url.scheme(), "tauri" | "https")
                        && matches!(url.host_str(), Some("localhost" | "tauri.localhost"))
                })
                .build(),
        )
        .manage(Arc::new(RuntimeSessions::default()))
        .setup(|app| {
            let hosts = Arc::new(crate::android_operator::initialize(app.handle()));
            let preview: Arc<dyn RuntimePreview> = Arc::new(NativePreview {
                connections: hosts.connections.clone(),
            });
            app.manage(hosts);
            app.manage(preview);
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
            gui_platform,
            mobile_desktop,
            mobile_screenshot,
            mobile_open,
            mobile_poll,
            mobile_write,
            mobile_resize,
            mobile_close,
            crate::runtime_transfer::gui_transfer_export,
            crate::runtime_transfer::gui_transfer_pick_import,
            crate::runtime_transfer::gui_transfer_unlock,
            crate::runtime_transfer::gui_transfer_apply,
            crate::runtime_transfer::gui_transfer_cancel,
            crate::runtime_hosts::gui_runtime_hosts,
            crate::runtime_hosts::gui_runtime_save_host,
            crate::runtime_hosts::gui_runtime_delete_host,
            crate::runtime_hosts::gui_runtime_test_host,
            crate::runtime_sessions::gui_runtime_sessions,
            crate::runtime_sessions::gui_runtime_create_session,
            crate::runtime_sessions::gui_runtime_create_history_session,
            crate::runtime_sessions::gui_runtime_history,
            crate::runtime_sessions::gui_runtime_ack,
            crate::runtime_sessions::gui_runtime_recover,
        ])
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                window
                    .state::<Arc<crate::runtime_transfer::RuntimeTransfer>>()
                    .foreground(window.label());
            }
            if matches!(
                event,
                tauri::WindowEvent::Focused(false)
                    | tauri::WindowEvent::CloseRequested { .. }
                    | tauri::WindowEvent::Destroyed
            ) {
                window
                    .state::<Arc<crate::runtime_transfer::RuntimeTransfer>>()
                    .background(
                        window.label(),
                        !matches!(event, tauri::WindowEvent::Focused(false)),
                    );
            }
        })
        .build(tauri::generate_context!())
        .expect("Android application startup failed")
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if shutdown_complete.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                api.prevent_exit();
                if !shutting_down.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let app: AppHandle = app.clone();
                    let shutdown_complete = shutdown_complete.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(8),
                            app.state::<Arc<RuntimeSessions>>().shutdown(),
                        )
                        .await;
                        shutdown_complete.store(true, std::sync::atomic::Ordering::SeqCst);
                        app.exit(0);
                    });
                }
            }
        });
}
