//! Independent Android operator AEAD; never exposes vault plaintext through IPC.
use crate::runtime_hosts::RuntimeHosts;
use art_term_core::openab::{
    api::{AdminClient, ApiError},
    profiles::HostStore,
    secret::{CredentialCipher, CredentialContext, Secret},
};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

struct AndroidOperator(AppHandle);
impl CredentialCipher for AndroidOperator {
    fn seal(&self, secret: &Secret, context: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        self.0
            .state::<tauri_plugin_mobile_vault::Vault<tauri::Wry>>()
            .operator_seal(&context.authenticated_bytes(), secret.expose().as_bytes())
            .map_err(|_| ApiError::Storage)
    }
    fn unseal(&self, data: &[u8], context: &CredentialContext) -> Result<Secret, ApiError> {
        let plain = self
            .0
            .state::<tauri_plugin_mobile_vault::Vault<tauri::Wry>>()
            .operator_unseal(&context.authenticated_bytes(), data)
            .map_err(|_| ApiError::Storage)?;
        let value = std::str::from_utf8(&plain).map_err(|_| ApiError::Storage)?;
        Secret::new(value.to_owned()).map_err(|_| ApiError::Storage)
    }
}

#[cfg(all(debug_assertions, feature = "openab-fixture"))]
fn fixture_ca(root: &std::path::Path) -> Result<Option<Vec<u8>>, ApiError> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let path = root.join("openab-fixture-ca.der");
    let metadata = match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ApiError::Storage),
        Ok(value) => value,
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || !(1..=65536).contains(&metadata.len())
    {
        return Err(ApiError::Storage);
    }
    // Android O_NOFOLLOW is architecture-specific (libc 0.2.189 definitions).
    #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
    const NO_FOLLOW: i32 = 0x8000;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    const NO_FOLLOW: i32 = 0x20000;
    #[cfg(target_arch = "riscv64")]
    const NO_FOLLOW: i32 = 0x20000;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW)
        .open(&path)
        .map_err(|_| ApiError::Storage)?;
    let opened = file.metadata().map_err(|_| ApiError::Storage)?;
    if !opened.is_file() || opened.ino() != metadata.ino() || opened.dev() != metadata.dev() {
        return Err(ApiError::Storage);
    }
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| ApiError::Storage)?;
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err(ApiError::Storage);
    }
    Ok(Some(bytes))
}

pub fn initialize(app: &AppHandle) -> RuntimeHosts {
    let root = app
        .state::<tauri_plugin_mobile_vault::Vault<tauri::Wry>>()
        .operator_root()
        .map_err(|_| ApiError::Storage);
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    let fixture_ca = root
        .as_ref()
        .map_err(|_| ApiError::Storage)
        .and_then(|root| fixture_ca(root));
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    let admin = match &fixture_ca {
        Ok(Some(ca)) => AdminClient::fixture_ca(ca),
        Ok(None) => AdminClient::new(),
        Err(_) => Err(ApiError::Storage),
    };
    #[cfg(not(all(debug_assertions, feature = "openab-fixture")))]
    let admin = AdminClient::new();
    let connections = None;
    let store = root.and_then(|root| {
        HostStore::load(
            root.join("connections.toml"),
            Arc::new(AndroidOperator(app.clone())),
        )
    });
    RuntimeHosts {
        store,
        admin,
        connections,
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        fixture_ca,
    }
}
