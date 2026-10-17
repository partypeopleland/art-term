//! Standalone encrypted desktop catalog. Host-only operations keep the opaque
//! desktop extension byte-for-value, including data from a newer app version.
use super::*;
use crate::instance_mcp::{DesktopProfile, DesktopSnapshot};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DesktopRecord {
    #[serde(flatten)]
    pub(super) profile: DesktopProfile,
    pub(super) token_ciphertext: Vec<u8>,
}
pub(super) fn desktop_context(profile: &DesktopProfile) -> CredentialContext {
    CredentialContext {
        schema: 2,
        host_id: format!("instance-mcp:{}", profile.id),
        origin: profile.mcp_url.clone(),
        revision: profile.revision,
    }
}
fn validate(items: &[DesktopRecord]) -> Result<(), ApiError> {
    if items.len() > 64 { return Err(ApiError::Capacity); }
    let mut ids = std::collections::HashSet::new();
    for item in items {
        let p = &item.profile;
        validate_name(&p.name)?;
        if uuid::Uuid::parse_str(&p.id).is_err() || !ids.insert(&p.id)
            || p.revision == 0 || !p.token_present || item.token_ciphertext.is_empty()
            || item.token_ciphertext.len() > 32768
            || crate::instance_mcp::canonical_url(&p.mcp_url)? != p.mcp_url {
            return Err(ApiError::Protocol);
        }
    }
    Ok(())
}
impl HostStore {
    pub(super) fn desktop_records(&self) -> Result<Vec<DesktopRecord>, ApiError> {
        self.ensure_source()?;
        let Some(bytes) = read_source(&self.path)? else { return Ok(Vec::new()); };
        let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes).map_err(|_| ApiError::Protocol)?).map_err(|_| ApiError::Protocol)?;
        let Some(raw) = value.get("desktops") else { return Ok(Vec::new()); };
        if value.get("schema_version").and_then(toml::Value::as_integer) != Some(6) {
            return Err(ApiError::Protocol);
        }
        let records: Vec<DesktopRecord> = raw.clone().try_into().map_err(|_| ApiError::Protocol)?;
        validate(&records)?;
        Ok(records)
    }
    pub fn desktops(&self) -> Result<Vec<DesktopProfile>, ApiError> {
        let _guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        Ok(self.desktop_records()?.into_iter().map(|item| item.profile).collect())
    }
    pub fn desktop(&self, id: &str) -> Result<DesktopSnapshot, ApiError> {
        let _guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        let item = self.desktop_records()?.into_iter().find(|item| item.profile.id == id).ok_or(ApiError::StateChanged)?;
        let token = self.cipher.unseal(&item.token_ciphertext, &desktop_context(&item.profile))?;
        if !token.expose().is_ascii() { return Err(ApiError::Protocol); }
        Ok(DesktopSnapshot { profile: item.profile, token })
    }
    pub fn save_desktop(&self, id: Option<&str>, expected_revision: Option<u64>, name: String,
        url: String, token: Option<Secret>) -> Result<DesktopProfile, ApiError> {
        validate_name(&name)?;
        let url = crate::instance_mcp::canonical_url(&url)?;
        let records = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        let mut desktops = self.desktop_records()?;
        let index = match (id, expected_revision) {
            (None, None) => None,
            (Some(id), Some(revision)) => Some(desktops.iter().position(|d| d.profile.id == id && d.profile.revision == revision).ok_or(ApiError::StateChanged)?),
            _ => return Err(ApiError::StateChanged),
        };
        let secret = match (token, index) {
            (Some(token), _) => token,
            (None, Some(i)) if desktops[i].profile.mcp_url == url => self.cipher.unseal(&desktops[i].token_ciphertext, &desktop_context(&desktops[i].profile))?,
            _ => return Err(ApiError::Protocol),
        };
        if !secret.expose().is_ascii() { return Err(ApiError::Protocol); }
        let profile = DesktopProfile {
            id: index.map(|i| desktops[i].profile.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            revision: index.map(|i| desktops[i].profile.revision.checked_add(1).ok_or(ApiError::StateChanged)).transpose()?.unwrap_or(1),
            name, mcp_url: url, token_present: true,
        };
        let item = DesktopRecord { token_ciphertext: self.cipher.seal(&secret, &desktop_context(&profile))?, profile: profile.clone() };
        if desktops.is_empty() { self.backup_before_desktops()?; }
        if let Some(i) = index { desktops[i] = item; } else { desktops.push(item); }
        self.save_desktop_records(&records, &desktops)?;
        Ok(profile)
    }
    pub fn delete_desktop(&self, id: &str, expected_revision: u64) -> Result<(), ApiError> {
        let records = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        let mut desktops = self.desktop_records()?;
        let index = desktops.iter().position(|item| item.profile.id == id && item.profile.revision == expected_revision).ok_or(ApiError::StateChanged)?;
        desktops.remove(index);
        self.save_desktop_records(&records, &desktops)
    }
    pub(super) fn save_desktop_records(&self, records: &[Record], desktops: &[DesktopRecord]) -> Result<(), ApiError> {
        let bytes = self.encode_desktop_records(records, desktops)?;
        self.persist_bytes(&bytes)
    }
    pub(super) fn encode_desktop_records(&self, records: &[Record], desktops: &[DesktopRecord]) -> Result<Vec<u8>, ApiError> {
        check_path(&self.path)?;
        self.ensure_source()?;
        validate_records(records)?;
        validate(desktops)?;
        let source = read_source(&self.path)?;
        let mut document: Document = toml::from_str(std::str::from_utf8(&encode(records, source.as_deref())?).map_err(|_| ApiError::Protocol)?).map_err(|_| ApiError::Protocol)?;
        document.schema_version = if desktops.is_empty() { 5 } else { 6 };
        document.desktops = if desktops.is_empty() { None } else {
            Some(desktops.iter().map(toml::Value::try_from).collect::<Result<_,_>>().map_err(|_| ApiError::Protocol)?)
        };
        let bytes = toml::to_string(&document).map_err(|_| ApiError::Protocol)?.into_bytes();
        if bytes.len() as u64 > MAX_FILE { return Err(ApiError::Capacity); }
        Ok(bytes)
    }
    pub(super) fn backup_before_desktops(&self) -> Result<(), ApiError> {
        let Some(bytes) = read_source(&self.path)? else { return Ok(()); };
        let backup = self.path.with_file_name("connections.pre-desktops.toml");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        match options.open(&backup) {
            Ok(mut file) => { file.write_all(&bytes).map_err(|_| ApiError::Storage)?; file.sync_all().map_err(|_| ApiError::Storage)?; }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(backup).map_err(|_| ApiError::Storage)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() { return Err(ApiError::Storage); }
                #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; if metadata.permissions().mode() & 0o077 != 0 { return Err(ApiError::Storage); } }
            }
            Err(_) => return Err(ApiError::Storage),
        }
        Ok(())
    }
}
