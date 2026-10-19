//! Desktop catalog and explicit per-tab selection; no IPC token readback.
use crate::{
    runtime_hosts::RuntimeHosts,
    runtime_sessions::{InstanceSelection, RuntimeSessions},
};
use art_term_core::{instance_mcp::DesktopProfile, openab::secret::Secret};
use serde::Deserialize;
use std::sync::Arc;
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopInput {
    id: Option<String>,
    expected_revision: Option<u64>,
    name: String,
    mcp_url: String,
    token: String,
}
#[tauri::command]
pub async fn gui_instance_desktops(
    hosts: State<'_, Arc<RuntimeHosts>>,
) -> Result<Vec<DesktopProfile>, String> {
    hosts
        .store()
        .and_then(|s| s.desktops())
        .map_err(|_| "無法讀取桌面設定".into())
}
#[tauri::command]
pub async fn gui_instance_save(
    hosts: State<'_, Arc<RuntimeHosts>>,
    mut input: DesktopInput,
) -> Result<DesktopProfile, String> {
    let token = std::mem::take(&mut input.token);
    let token = if token.is_empty() {
        None
    } else {
        Some(Secret::new(token).map_err(|_| "桌面 token 格式不符")?)
    };
    let hosts = hosts.inner().clone();
    tokio::task::spawn_blocking(move || {
        hosts.store()?.save_desktop(
            input.id.as_deref(),
            input.expected_revision,
            input.name,
            input.mcp_url,
            token,
        )
    })
    .await
    .map_err(|_| "桌面保存失敗")?
    .map_err(|_| "無法保存桌面；請確認網址、token 與設定版本".into())
}
#[tauri::command]
pub async fn gui_instance_delete(
    hosts: State<'_, Arc<RuntimeHosts>>,
    id: String,
    expected_revision: u64,
) -> Result<(), String> {
    let hosts = hosts.inner().clone();
    tokio::task::spawn_blocking(move || hosts.store()?.delete_desktop(&id, expected_revision))
        .await
        .map_err(|_| "桌面刪除失敗")?
        .map_err(|_| "桌面已變更，請重新整理".into())
}
#[tauri::command]
pub async fn gui_instance_test(
    hosts: State<'_, Arc<RuntimeHosts>>,
    id: String,
    expected_revision: u64,
) -> Result<(), String> {
    let snapshot = hosts
        .store()
        .and_then(|s| s.desktop(&id))
        .map_err(|_| "桌面設定不可用")?;
    if snapshot.profile.revision != expected_revision {
        return Err("桌面已變更".into());
    }
    art_term_core::instance_mcp::screenshot(snapshot)
        .await
        .map_err(|_| "桌面連線測試失敗；請確認 Tailnet、MCP 網址與 token")?;
    if hosts
        .store()
        .and_then(|s| s.desktop(&id))
        .map_err(|_| "桌面已變更")?
        .profile
        .revision
        != expected_revision
    {
        return Err("桌面已變更".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn gui_instance_select(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
    desktop_id: String,
    expected_revision: u64,
) -> Result<InstanceSelection, String> {
    sessions
        .select_instance(&hosts, &session_id, &desktop_id, expected_revision)
        .await
        .map_err(|_| "桌面切換未確認成功；請重新選擇，並確認受控端工具入口已安裝".into())
}
#[tauri::command]
pub async fn gui_instance_restore(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
) -> Result<Option<InstanceSelection>, String> {
    sessions.restore_instance(&hosts, &session_id).await.map_err(|error| {
        let stage = match error.to_string().as_str() {
            "Shell broker 查詢失敗" => "Shell broker 查詢",
            "桌面清單讀取失敗" => "本機桌面清單讀取",
            "過期指派清除失敗" => "過期指派清除",
            "桌面憑證讀取失敗" => "本機桌面憑證讀取",
            "桌面工具重新驗證失敗" => "桌面工具重新驗證",
            "Shell 指派來自其他裝置，受控端需更新後才能核對" => "受控端缺少跨裝置身分資訊",
            "Shell 指派的桌面在本機不存在或網址不唯一" => "本機 Screen 網址核對",
            _ => "Screen 身分核對",
        };
        format!("無法恢復 Shell 的既有 Screen（{stage}）；請檢查連線後重試")
    })
}
#[tauri::command]
pub async fn gui_instance_clear(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
) -> Result<(), String> {
    sessions
        .clear_instance(&hosts, &session_id)
        .await
        .map_err(|_| "預覽已停止，遠端工具取消未確認；請重試或結束 CLI".into())
}
#[tauri::command]
pub async fn gui_instance_launch(
    hosts: State<'_, Arc<RuntimeHosts>>,
    sessions: State<'_, Arc<RuntimeSessions>>,
    session_id: String,
) -> Result<InstanceSelection, String> {
    sessions
        .launch_instance(&hosts, &session_id)
        .await
        .map_err(|_| "工具入口未確認啟動；本輪支援 Codex，請查看終端狀態後重新選擇桌面".into())
}
