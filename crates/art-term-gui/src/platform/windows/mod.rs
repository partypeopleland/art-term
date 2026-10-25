mod dpapi;

use art_term_core::openab::{api::ApiError, profiles::HostStore};
use std::{path::PathBuf, sync::Arc};

pub(crate) const NAME: &str = "windows";

pub(crate) fn default_config_path(_application_id: &str) -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("executable directory unavailable"))?
        .join("connections.toml"))
}

pub(crate) fn open_store(path: PathBuf, _application_id: &str) -> Result<HostStore, ApiError> {
    HostStore::load(path, Arc::new(dpapi::WindowsCredentialCipher))
}
