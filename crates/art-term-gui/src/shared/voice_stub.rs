//! Non-Windows voice commands: the desktop shell only ships dictation on Windows.

use crate::voice_dto::{VoiceAckDto, VoicePermissionDto, VoiceStatusDto};
use std::sync::Arc;
use tauri::State;

#[derive(Default)]
pub struct VoiceSessions;

fn unavailable_status() -> VoiceStatusDto {
    VoiceStatusDto {
        available: false,
        sdk_int: None,
        permission: "denied".into(),
        installed: Vec::new(),
        supported: Vec::new(),
        language_switch_supported: false,
        detail: Some("語音輸入僅在 Windows 版本提供。".into()),
    }
}

#[tauri::command]
pub fn gui_voice_status() -> VoiceStatusDto {
    unavailable_status()
}

#[tauri::command]
pub fn gui_voice_request_permission() -> VoicePermissionDto {
    VoicePermissionDto { permission: "denied".into() }
}

#[tauri::command]
pub fn gui_voice_start(
    _sessions: State<'_, Arc<VoiceSessions>>,
    _session_id: String,
    _language: String,
    _allow_language_switch: bool,
) -> Result<VoiceAckDto, String> {
    Err("NOT_AVAILABLE".into())
}

#[tauri::command]
pub fn gui_voice_stop(_sessions: State<'_, Arc<VoiceSessions>>, _session_id: String) -> VoiceAckDto {
    VoiceAckDto { ok: true, mechanism: None }
}

#[tauri::command]
pub fn gui_voice_cancel(_sessions: State<'_, Arc<VoiceSessions>>, _session_id: String) -> VoiceAckDto {
    VoiceAckDto { ok: true, mechanism: None }
}

