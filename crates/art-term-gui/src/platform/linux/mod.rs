mod credentials;

use art_term_core::openab::{api::ApiError, profiles::HostStore};
use std::path::PathBuf;

pub(crate) const NAME: &str = "linux";

pub(crate) fn default_config_path(application_id: &str) -> anyhow::Result<PathBuf> {
    credentials::default_config_path(application_id)
        .map_err(|_| anyhow::anyhow!("Unable to open private Linux configuration directory"))
}

pub(crate) fn open_store(path: PathBuf, application_id: &str) -> Result<HostStore, ApiError> {
    credentials::open_store(path, application_id)
}
