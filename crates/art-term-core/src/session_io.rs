//! Shared session IPC DTOs between the frontends and the native runtime
//! session layer.
//!
//! The DTOs serialized across the IPC boundary live here so the Windows
//! and Android facades share one definition. Runtime output ordering is
//! handled by the openab session contract (batch ACK/generation); the
//! wire payloads below carry no credentials or raw remote addresses.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub id: String,
    pub target_id: String,
    pub label: String,
    pub remote_session: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDto {
    pub bytes: Vec<u8>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub id: String,
    pub host_id: String,
    pub label: String,
    /// Small secondary line — the configured tool. Pure display text,
    /// never interpreted.
    pub subtitle: String,
    /// Reserved safe public account metadata; runtime targets never
    /// carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    pub kind: String,
    pub running: bool,
    pub phase: String,
    pub desktop_available: bool,
    /// Configured coding CLI hint (`"codex"`) — explicit config only,
    /// never inferred from display text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_cli: Option<crate::connections::CodingCli>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDto {
    pub id: String,
    pub label: String,
    pub desktop_available: bool,
    pub targets: Vec<TargetDto>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStateDto {
    pub enabled: bool,
    pub message: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotDto {
    pub mime: String,
    pub base64: String,
    pub source: String,
    pub captured_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_revision: Option<u64>,
}
