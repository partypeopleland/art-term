#[path = "connection_order.rs"]
mod connection_order;
pub use connection_order::ConnectionKind;
#[path = "instance_profiles.rs"]
mod instance_profiles;
use super::{
    api::ApiError,
    secret::{CredentialCipher, CredentialContext, Secret},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use url::Url;

const SCHEMA: u32 = 1;
const MAX_FILE: u64 = 1024 * 1024;
const MAX_HOSTS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostProfile {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub base_url: String,
    pub credential_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<crate::connections::ConnectionMetadata>,
}
/// Immutable endpoint and native credential captured under the store lock.
#[derive(Debug)]
pub struct HostSnapshot {
    pub profile: HostProfile,
    pub credential: Secret,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    profile: HostProfile,
    ciphertext: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    connections: Vec<DiskConnection>,
    // Preserve newer desktop settings without enabling or decrypting them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktops: Option<Vec<toml::Value>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskConnection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    id: String,
    revision: u64,
    name: String,
    openab_pty_url: String,
    category: crate::connections::Category,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop_transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coding_cli: Option<crate::connections::CodingCli>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_ciphertext: Option<Vec<u8>>,
}
fn decode(bytes: &[u8]) -> Result<Vec<Record>, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ApiError::Protocol)?;
    let value: toml::Value = toml::from_str(text).map_err(|_| ApiError::Protocol)?;
    let table = value.as_table().ok_or(ApiError::Protocol)?;
    if table.is_empty()
        || (table.len() == 1
            && table
                .get("connections")
                .and_then(toml::Value::as_array)
                .is_some_and(Vec::is_empty))
    {
        return Ok(Vec::new());
    }
    if table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .is_some_and(|version| version == 5 || version == 6)
    {
        let doc: Document = toml::from_str(text).map_err(|_| ApiError::Protocol)?;
        return doc
            .connections
            .into_iter()
            .map(|c| {
                if c.credential_ciphertext.as_ref().is_some_and(Vec::is_empty) {
                    return Err(ApiError::Protocol);
                }
                Ok(Record {
                    profile: HostProfile {
                        id: c.id,
                        revision: c.revision,
                        name: c.name,
                        base_url: c.openab_pty_url,
                        credential_present: c.credential_ciphertext.is_some(),
                        metadata: Some(crate::connections::ConnectionMetadata {
                            display_name: c.display_name,
                            category: c.category,
                            desktop_transport: c.desktop_transport,
                            coding_cli: c.coding_cli,
                        }),
                    },
                    ciphertext: c.credential_ciphertext.unwrap_or_default(),
                })
            })
            .collect();
    }
    let legacy = crate::connections::parse(text).map_err(|_| ApiError::Protocol)?;
    Ok(legacy
        .connections
        .into_iter()
        .map(|c| Record {
            profile: HostProfile {
                id: c.id,
                revision: 1,
                name: c.name,
                base_url: c.openab_pty_url,
                credential_present: false,
                metadata: Some(c.metadata),
            },
            ciphertext: Vec::new(),
        })
        .collect())
}
fn encode(records: &[Record], source: Option<&[u8]>) -> Result<Vec<u8>, ApiError> {
    let original = source
        .map(|bytes| std::str::from_utf8(bytes).map_err(|_| ApiError::Protocol))
        .transpose()?
        .map(toml::from_str::<Document>)
        .transpose();
    // Legacy schemas still migrate normally; schema 5/6 opaque data survives
    // every save, including deleting the final Host connection.
    let (schema_version, desktops) = match original {
        Ok(Some(doc)) if matches!(doc.schema_version, 5 | 6) => (doc.schema_version, doc.desktops),
        _ => (5, None),
    };
    let connections = records
        .iter()
        .map(|r| {
            let m = r.profile.metadata.clone().unwrap_or_default();
            DiskConnection {
                id: r.profile.id.clone(),
                revision: r.profile.revision,
                name: r.profile.name.clone(),
                openab_pty_url: r.profile.base_url.clone(),
                display_name: m.display_name,
                category: m.category,
                desktop_transport: m.desktop_transport,
                coding_cli: m.coding_cli,
                credential_ciphertext: (!r.ciphertext.is_empty()).then(|| r.ciphertext.clone()),
            }
        })
        .collect();
    toml::to_string(&Document {
        schema_version,
        connections,
        desktops,
    })
    .map(String::into_bytes)
    .map_err(|_| ApiError::Protocol)
}
fn validate_records(records: &[Record]) -> Result<(), ApiError> {
    if records.len() > MAX_HOSTS {
        return Err(ApiError::Capacity);
    }
    let mut ids = std::collections::HashSet::new();
    let mut origins = std::collections::HashSet::new();
    for r in records {
        validate_name(&r.profile.name)?;
        if let Some(m) = &r.profile.metadata {
            m.validate().map_err(|_| ApiError::Protocol)?;
        }
        let id = &r.profile.id;
        let slug = !id.is_empty()
            && id.len() <= 32
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b == b'-'));
        let uuid = uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == *id);
        if (!slug && !uuid)
            || !ids.insert(id)
            || r.profile.revision == 0
            || canonical_origin(&r.profile.base_url)? != r.profile.base_url
            || !origins.insert(&r.profile.base_url)
            || r.ciphertext.len() > 32768
            || r.profile.credential_present == r.ciphertext.is_empty()
        {
            return Err(ApiError::Protocol);
        }
    }
    Ok(())
}
fn read_source(path: &Path) -> Result<Option<Vec<u8>>, ApiError> {
    check_path(path)?;
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ApiError::Storage),
        Ok(meta) => {
            if !meta.is_file() || meta.len() > MAX_FILE {
                return Err(ApiError::Protocol);
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(|_| ApiError::Storage)?
                .take(MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ApiError::Storage)?;
            if bytes.len() as u64 > MAX_FILE {
                return Err(ApiError::Protocol);
            }
            Ok(Some(bytes))
        }
    }
}
fn fingerprint(source: Option<&[u8]>) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};
    source.map(|bytes| Sha256::digest(bytes).to_vec())
}

pub fn canonical_origin(value: &str) -> Result<String, ApiError> {
    if value.len() > 2048
        || value.trim() != value
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(ApiError::Protocol);
    }
    // Reject raw path/userinfo before URL normalization can erase hostile forms.
    let authority = value.split_once("://").ok_or(ApiError::Protocol)?.1;
    let (authority, path) = authority.split_once('/').unwrap_or((authority, ""));
    if authority.contains('@') || !path.is_empty() {
        return Err(ApiError::Protocol);
    }
    let url = Url::parse(value).map_err(|_| ApiError::Protocol)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(ApiError::Protocol);
    }
    Ok(url.to_string())
}
fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty()
        || name.len() > 128
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        Err(ApiError::Protocol)
    } else {
        Ok(())
    }
}
fn context(profile: &HostProfile) -> CredentialContext {
    CredentialContext {
        schema: SCHEMA,
        host_id: profile.id.clone(),
        origin: profile.base_url.clone(),
        revision: profile.revision,
    }
}
fn check_path(path: &Path) -> Result<(), ApiError> {
    // Reject symlinks at every existing ancestor and final component.
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(ApiError::StateChanged),
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(ApiError::Storage); // junction/reparse point
                    }
                }
                #[cfg(not(windows))]
                let _ = meta;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(ApiError::Storage),
        }
    }
    Ok(())
}
pub struct HostStore {
    path: PathBuf,
    cipher: Arc<dyn CredentialCipher>,
    records: Mutex<Vec<Record>>,
    source: Mutex<Option<Vec<u8>>>,
}
impl HostStore {
    pub fn load(path: PathBuf, cipher: Arc<dyn CredentialCipher>) -> Result<Self, ApiError> {
        if !path.is_absolute() {
            return Err(ApiError::Protocol);
        }
        let bytes = read_source(&path)?;
        let records = match &bytes {
            Some(bytes) => decode(bytes)?,
            None => Vec::new(),
        };
        validate_records(&records)?;
        for r in &records {
            if !r.ciphertext.is_empty() {
                cipher.unseal(&r.ciphertext, &context(&r.profile))?;
            }
        }
        Ok(Self {
            path,
            cipher,
            records: Mutex::new(records),
            source: Mutex::new(fingerprint(bytes.as_deref())),
        })
    }
    fn ensure_source(&self) -> Result<(), ApiError> {
        let current = read_source(&self.path).map_err(|_| ApiError::StateChanged)?;
        if fingerprint(current.as_deref())
            != *self.source.lock().map_err(|_| ApiError::StateChanged)?
        {
            return Err(ApiError::StateChanged);
        }
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<HostProfile>, ApiError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| ApiError::StateChanged)?
            .iter()
            .map(|r| r.profile.clone())
            .collect())
    }
    pub fn snapshot(&self, id: &str) -> Result<HostSnapshot, ApiError> {
        let guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        let r = guard
            .iter()
            .find(|r| r.profile.id == id)
            .ok_or(ApiError::StateChanged)?;
        if r.ciphertext.is_empty() {
            return Err(ApiError::Protocol);
        }
        Ok(HostSnapshot {
            profile: r.profile.clone(),
            credential: self.cipher.unseal(&r.ciphertext, &context(&r.profile))?,
        })
    }
    pub fn is_current(&self, profile: &HostProfile) -> Result<bool, ApiError> {
        self.ensure_source()?;
        Ok(self
            .records
            .lock()
            .map_err(|_| ApiError::StateChanged)?
            .iter()
            .any(|r| r.profile == *profile))
    }
    pub fn add(
        &self,
        name: String,
        base_url: String,
        credential: Secret,
    ) -> Result<HostProfile, ApiError> {
        self.add_with_metadata(name, base_url, credential, None)
    }
    pub fn add_with_metadata(
        &self,
        name: String,
        base_url: String,
        credential: Secret,
        mut metadata: Option<crate::connections::ConnectionMetadata>,
    ) -> Result<HostProfile, ApiError> {
        if let Some(m) = &mut metadata {
            if m.display_name
                .as_ref()
                .is_some_and(|v| v.chars().any(char::is_control))
            {
                return Err(ApiError::Protocol);
            }
            m.normalize_display_name();
        }
        validate_name(&name)?;
        if let Some(m) = &metadata {
            m.validate().map_err(|_| ApiError::Protocol)?;
        }
        let profile = HostProfile {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name,
            base_url: canonical_origin(&base_url)?,
            credential_present: true,
            metadata: Some(metadata.unwrap_or_default()),
        };
        let mut guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        if guard.len() >= MAX_HOSTS {
            return Err(ApiError::Capacity);
        }
        let mut candidate = guard.clone();
        candidate.push(Record {
            ciphertext: self.cipher.seal(&credential, &context(&profile))?,
            profile: profile.clone(),
        });
        self.save(&candidate)?;
        *guard = candidate;
        Ok(profile)
    }
    /// Missing credential retains the old secret only at the same canonical origin.
    pub fn update(
        &self,
        id: &str,
        revision: u64,
        name: String,
        base_url: String,
        credential: Option<Secret>,
    ) -> Result<HostProfile, ApiError> {
        self.update_with_metadata(id, revision, name, base_url, credential, None)
    }
    /// Omitted metadata preserves the prior value; explicit metadata can disable capabilities.
    pub fn update_with_metadata(
        &self,
        id: &str,
        revision: u64,
        name: String,
        base_url: String,
        credential: Option<Secret>,
        mut metadata: Option<crate::connections::ConnectionMetadata>,
    ) -> Result<HostProfile, ApiError> {
        if let Some(m) = &mut metadata {
            if m.display_name
                .as_ref()
                .is_some_and(|v| v.chars().any(char::is_control))
            {
                return Err(ApiError::Protocol);
            }
            m.normalize_display_name();
        }
        validate_name(&name)?;
        if let Some(m) = &metadata {
            m.validate().map_err(|_| ApiError::Protocol)?;
        }
        let origin = canonical_origin(&base_url)?;
        let mut guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        let index = guard
            .iter()
            .position(|r| r.profile.id == id && r.profile.revision == revision)
            .ok_or(ApiError::StateChanged)?;
        let old = &guard[index];
        let secret = match credential {
            Some(secret) => Some(secret),
            None if origin == old.profile.base_url => {
                if old.ciphertext.is_empty() {
                    None
                } else {
                    Some(
                        self.cipher
                            .unseal(&old.ciphertext, &context(&old.profile))?,
                    )
                }
            }
            None => return Err(ApiError::StateChanged),
        };
        let profile = HostProfile {
            id: id.into(),
            revision: revision.checked_add(1).ok_or(ApiError::StateChanged)?,
            name,
            base_url: origin,
            credential_present: secret.is_some(),
            metadata: metadata.or_else(|| old.profile.metadata.clone()),
        };
        let mut candidate = guard.clone();
        candidate[index] = Record {
            ciphertext: secret
                .as_ref()
                .map(|s| self.cipher.seal(s, &context(&profile)))
                .transpose()?
                .unwrap_or_default(),
            profile: profile.clone(),
        };
        self.save(&candidate)?;
        *guard = candidate;
        Ok(profile)
    }
    /// Local removal only; does not issue DELETE or terminate any remote PTY.
    pub fn delete(&self, id: &str, revision: u64) -> Result<(), ApiError> {
        let mut guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        let index = guard
            .iter()
            .position(|r| r.profile.id == id && r.profile.revision == revision)
            .ok_or(ApiError::StateChanged)?;
        let mut candidate = guard.clone();
        candidate.remove(index);
        self.save(&candidate)?;
        *guard = candidate;
        Ok(())
    }
    fn save(&self, records: &[Record]) -> Result<(), ApiError> {
        check_path(&self.path)?;
        self.ensure_source()?;
        validate_records(records)?;
        let source = read_source(&self.path)?;
        // Recheck after reading so externally changed settings cannot be
        // silently replaced while preserving a different file's extensions.
        self.ensure_source()?;
        let bytes = encode(records, source.as_deref())?;
        self.persist_bytes(&bytes)
    }
    fn persist_bytes(&self, bytes: &[u8]) -> Result<(), ApiError> {
        if bytes.len() as u64 > MAX_FILE {
            return Err(ApiError::Capacity);
        }
        let parent = self.path.parent().ok_or(ApiError::StateChanged)?;
        // Caller creates a private application data directory; never change ACLs here.
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| ApiError::StateChanged)?;
        temp.write_all(&bytes).map_err(|_| ApiError::StateChanged)?;
        temp.as_file()
            .sync_all()
            .map_err(|_| ApiError::StateChanged)?;
        check_path(&self.path)?;
        self.ensure_source()?;
        // tempfile persist uses atomic replace (MoveFileExW on Windows), never remove-first.
        temp.persist(&self.path)
            .map_err(|_| ApiError::StateChanged)?;
        *self.source.lock().map_err(|_| ApiError::StateChanged)? = fingerprint(Some(&bytes));
        Ok(())
    }
}

/// Opaque native owner of validated imported data and the complete store snapshot.
pub struct ImportPreview {
    payload: super::transfer::Payload,
    fingerprint: Option<Vec<u8>>,
    pub entries: Vec<ImportEntry>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    #[default]
    Host,
    Screen,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub key: String,
    pub name: String,
    pub match_status: &'static str,
    #[serde(flatten)]
    pub details: ImportDetails,
}
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ImportDetails {
    Host {
        origin: String,
        metadata: crate::connections::ConnectionMetadata,
        has_credential: bool,
        target: Option<HostProfile>,
    },
    Screen {
        mcp_url: String,
        has_token: bool,
        target: Option<crate::instance_mcp::DesktopProfile>,
    },
}
impl ImportEntry {
    pub fn has_secret(&self) -> bool {
        match &self.details {
            ImportDetails::Host { has_credential, .. } => *has_credential,
            ImportDetails::Screen { has_token, .. } => *has_token,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportAction {
    Add,
    Update,
    Skip,
}
pub struct ImportSelection {
    pub key: String,
    pub action: ImportAction,
    pub credential: Option<Secret>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportSelection {
    #[serde(default)]
    pub kind: TransferKind,
    pub id: String,
    pub expected_revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub profiles: Vec<HostProfile>,
    pub desktops: Vec<crate::instance_mcp::DesktopProfile>,
}
impl HostStore {
    /// One lock, expected revisions, and no unseal at all for the public mode.
    pub fn export_snapshot(
        &self,
        selected: &[ExportSelection],
        include_credentials: bool,
        _catalog: Option<&crate::connections::Connections>,
    ) -> Result<super::transfer::Payload, ApiError> {
        if selected.is_empty() || selected.len() > MAX_HOSTS * 2 {
            return Err(ApiError::Capacity);
        }
        let guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::new();
        let mut desktops = Vec::new();
        let desktop_records = if selected.iter().any(|s| s.kind == TransferKind::Screen) {
            self.desktop_records()?
        } else {
            Vec::new()
        };
        let mut selections: Vec<_> = selected.iter().collect();
        selections.sort_by_key(|s| match s.kind {
            TransferKind::Host => (0, guard.iter().position(|r| r.profile.id == s.id)),
            TransferKind::Screen => (1, desktop_records.iter().position(|r| r.profile.id == s.id)),
        });
        for s in selections {
            if !seen.insert((s.kind as u8, &s.id)) {
                return Err(ApiError::Protocol);
            }
            if s.kind == TransferKind::Screen {
                let r = desktop_records
                    .iter()
                    .find(|r| r.profile.id == s.id && r.profile.revision == s.expected_revision)
                    .ok_or(ApiError::StateChanged)?;
                desktops.push(super::transfer::ScreenEntry {
                    name: r.profile.name.clone(),
                    mcp_url: r.profile.mcp_url.clone(),
                    token: if include_credentials {
                        Some(self.cipher.unseal(
                            &r.token_ciphertext,
                            &instance_profiles::desktop_context(&r.profile),
                        )?)
                    } else {
                        None
                    },
                });
                continue;
            }
            let r = guard
                .iter()
                .find(|r| r.profile.id == s.id && r.profile.revision == s.expected_revision)
                .ok_or(ApiError::StateChanged)?;
            entries.push(super::transfer::Entry {
                name: r.profile.name.clone(),
                origin: r.profile.base_url.clone(),
                metadata: crate::runtime_preview::resolve_metadata(
                    r.profile.metadata.as_ref(),
                    None,
                    &r.profile.base_url,
                ),
                credential: if include_credentials && !r.ciphertext.is_empty() {
                    Some(self.cipher.unseal(&r.ciphertext, &context(&r.profile))?)
                } else {
                    None
                },
            });
        }
        let mut payload = super::transfer::Payload { entries, desktops };
        payload.validate()?;
        Ok(payload)
    }
    pub fn preview_import(
        &self,
        mut payload: super::transfer::Payload,
    ) -> Result<ImportPreview, ApiError> {
        payload.validate()?;
        let guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        let hosts: Vec<_> = guard.iter().map(|r| r.profile.clone()).collect();
        let fingerprint = self
            .source
            .lock()
            .map_err(|_| ApiError::StateChanged)?
            .clone();
        let mut entries: Vec<_> = payload
            .entries
            .iter()
            .map(|e| {
                let matches: Vec<_> = hosts.iter().filter(|p| p.base_url == e.origin).collect();
                ImportEntry {
                    key: uuid::Uuid::new_v4().to_string(),
                    name: e.name.clone(),

                    match_status: match matches.len() {
                        0 => "new",
                        1 => "duplicate",
                        _ => "ambiguous",
                    },
                    details: ImportDetails::Host {
                        origin: e.origin.clone(),
                        metadata: e.metadata.clone(),
                        has_credential: e.credential.is_some(),
                        target: if matches.len() == 1 {
                            Some(matches[0].clone())
                        } else {
                            None
                        },
                    },
                }
            })
            .collect();
        if !payload.desktops.is_empty() {
            let local = self.desktop_records()?;
            for e in &payload.desktops {
                let matches: Vec<_> = local
                    .iter()
                    .filter(|r| r.profile.mcp_url == e.mcp_url)
                    .collect();
                entries.push(ImportEntry {
                    key: uuid::Uuid::new_v4().to_string(),
                    name: e.name.clone(),
                    match_status: match matches.len() {
                        0 => "new",
                        1 => "duplicate",
                        _ => "ambiguous",
                    },
                    details: ImportDetails::Screen {
                        mcp_url: e.mcp_url.clone(),
                        has_token: e.token.is_some(),
                        target: if matches.len() == 1 {
                            Some(matches[0].profile.clone())
                        } else {
                            None
                        },
                    },
                });
            }
        }
        Ok(ImportPreview {
            payload,
            fingerprint,
            entries,
        })
    }
    /// Entire validation/sealing/save is a single records critical section.
    /// Caller consumes the native preview exactly once and grants commit before entering here.
    pub fn apply_import(
        &self,
        preview: ImportPreview,
        selected: Vec<ImportSelection>,
    ) -> Result<ImportResult, ApiError> {
        if selected.len() > MAX_HOSTS * 2 {
            return Err(ApiError::Capacity);
        }
        let mut guard = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        if *self.source.lock().map_err(|_| ApiError::StateChanged)? != preview.fingerprint {
            return Err(ApiError::StateChanged);
        }
        let mut choices = std::collections::HashMap::new();
        for s in selected {
            if !preview.entries.iter().any(|e| e.key == s.key)
                || choices.insert(s.key.clone(), s).is_some()
            {
                return Err(ApiError::Protocol);
            }
        }
        let mut candidate = guard.clone();
        let mut result = ImportResult {
            added: 0,
            updated: 0,
            skipped: 0,
            profiles: Vec::new(),
            desktops: Vec::new(),
        };
        for (mut entry, info) in preview.payload.entries.into_iter().zip(&preview.entries) {
            let Some(choice) = choices.remove(&info.key) else {
                result.skipped += 1;
                continue;
            };
            if choice.action == ImportAction::Skip {
                result.skipped += 1;
                continue;
            }
            if info.match_status == "ambiguous" {
                return Err(ApiError::Protocol);
            }
            let ImportDetails::Host { target, .. } = &info.details else {
                return Err(ApiError::Protocol);
            };
            let (mut profile, index, old_secret) = match (choice.action, target.clone()) {
                (ImportAction::Add, None) if info.match_status == "new" => (
                    HostProfile {
                        id: uuid::Uuid::new_v4().to_string(),
                        revision: 1,
                        name: entry.name,
                        base_url: entry.origin,
                        credential_present: true,
                        metadata: Some(entry.metadata),
                    },
                    None,
                    None,
                ),
                (ImportAction::Update, Some(old)) if info.match_status == "duplicate" => {
                    let index = candidate
                        .iter()
                        .position(|r| r.profile == old)
                        .ok_or(ApiError::StateChanged)?;
                    let secret =
                        if entry.credential.is_none() && !candidate[index].ciphertext.is_empty() {
                            Some(
                                self.cipher
                                    .unseal(&candidate[index].ciphertext, &context(&old))?,
                            )
                        } else {
                            None
                        };
                    (
                        HostProfile {
                            id: old.id,
                            revision: old.revision.checked_add(1).ok_or(ApiError::StateChanged)?,
                            name: entry.name,
                            base_url: entry.origin,
                            credential_present: true,
                            metadata: Some(entry.metadata),
                        },
                        Some(index),
                        secret,
                    )
                }
                _ => return Err(ApiError::Protocol),
            };
            // Supplemental credentials are only meaningful for public new entries.
            if choice.credential.is_some() && (index.is_some() || entry.credential.is_some()) {
                return Err(ApiError::Protocol);
            }
            let secret = entry.credential.take().or(choice.credential).or(old_secret);
            if secret.is_none() && index.is_none() {
                return Err(ApiError::Protocol);
            }
            profile.credential_present = secret.is_some();
            let record = Record {
                ciphertext: secret
                    .as_ref()
                    .map(|s| self.cipher.seal(s, &context(&profile)))
                    .transpose()?
                    .unwrap_or_default(),
                profile: profile.clone(),
            };
            if let Some(i) = index {
                candidate[i] = record;
                result.updated += 1;
            } else {
                candidate.push(record);
                result.added += 1;
            }
            result.profiles.push(profile);
        }
        if candidate.len() > MAX_HOSTS {
            return Err(ApiError::Capacity);
        }
        let has_screens = !preview.payload.desktops.is_empty();
        if has_screens {
            let mut desktops = self.desktop_records()?;
            let had_desktops = !desktops.is_empty();
            for (mut entry, info) in preview.payload.desktops.into_iter().zip(
                preview
                    .entries
                    .iter()
                    .filter(|e| matches!(e.details, ImportDetails::Screen { .. })),
            ) {
                let Some(choice) = choices.remove(&info.key) else {
                    result.skipped += 1;
                    continue;
                };
                if choice.action == ImportAction::Skip {
                    result.skipped += 1;
                    continue;
                }
                let ImportDetails::Screen { target, .. } = &info.details else {
                    return Err(ApiError::Protocol);
                };
                let (profile, index) = match (choice.action, target) {
                    (ImportAction::Add, None) if info.match_status == "new" => (
                        crate::instance_mcp::DesktopProfile {
                            id: uuid::Uuid::new_v4().to_string(),
                            revision: 1,
                            name: entry.name,
                            mcp_url: entry.mcp_url,
                            token_present: true,
                        },
                        None,
                    ),
                    (ImportAction::Update, Some(old)) if info.match_status == "duplicate" => {
                        let i = desktops
                            .iter()
                            .position(|r| r.profile == *old)
                            .ok_or(ApiError::StateChanged)?;
                        (
                            crate::instance_mcp::DesktopProfile {
                                id: old.id.clone(),
                                revision: old
                                    .revision
                                    .checked_add(1)
                                    .ok_or(ApiError::StateChanged)?,
                                name: entry.name,
                                mcp_url: entry.mcp_url,
                                token_present: true,
                            },
                            Some(i),
                        )
                    }
                    _ => return Err(ApiError::Protocol),
                };
                if choice.credential.is_some() && (index.is_some() || entry.token.is_some()) {
                    return Err(ApiError::Protocol);
                }
                let token = entry
                    .token
                    .take()
                    .or(choice.credential)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        let i = index.ok_or(ApiError::Protocol)?;
                        self.cipher.unseal(
                            &desktops[i].token_ciphertext,
                            &instance_profiles::desktop_context(&desktops[i].profile),
                        )
                    })?;
                if !token.expose().is_ascii() {
                    return Err(ApiError::Protocol);
                }
                let record = instance_profiles::DesktopRecord {
                    token_ciphertext: self
                        .cipher
                        .seal(&token, &instance_profiles::desktop_context(&profile))?,
                    profile: profile.clone(),
                };
                if let Some(i) = index {
                    desktops[i] = record;
                    result.updated += 1;
                } else {
                    desktops.push(record);
                    result.added += 1;
                }
                result.desktops.push(profile);
            }
            if result.added + result.updated > 0 {
                let bytes = self.encode_desktop_records(&candidate, &desktops)?;
                if !had_desktops && !desktops.is_empty() {
                    self.backup_before_desktops()?;
                }
                self.persist_bytes(&bytes)?;
                *guard = candidate;
            }
        } else if result.added + result.updated > 0 {
            self.save(&candidate)?;
            *guard = candidate;
        }
        Ok(result)
    }
}
