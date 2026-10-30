//! Strict, bounded portable connection files. Sensitive owners never implement IPC Serialize.
use super::{api::ApiError, profiles::canonical_origin, secret::Secret};
use crate::connections::{Category, CodingCli, ConnectionMetadata};
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashSet;
use zeroize::Zeroizing;

pub const MAX_FILE: usize = 2 * 1024 * 1024;
pub const MAX_PAYLOAD: usize = 1024 * 1024;
const FORMAT: &str = "art-term-connections";
const DOMAIN_V2: &[u8] = b"art-term-connections:v2:encrypted:argon2id:v19:m65536:t3:p4:aes-256-gcm";
const DOMAIN: &[u8] = b"art-term-connections:v1:encrypted:argon2id:v19:m65536:t3:p4:aes-256-gcm";

// Dedicated transfer DTOs keep strictness independent of configuration migrations.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    category: Category,
    desktop_transport: Option<String>,
    coding_cli: Option<Cli>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cli {
    id: String,
    name: String,
    command: String,
    args: Vec<String>,
}
impl From<&ConnectionMetadata> for Metadata {
    fn from(m: &ConnectionMetadata) -> Self {
        Self {
            display_name: m.display_name.clone(),
            category: m.category,
            desktop_transport: m.desktop_transport.clone(),
            coding_cli: m.coding_cli.as_ref().map(|c| Cli {
                id: c.id.clone(),
                name: c.name.clone(),
                command: c.command.clone(),
                args: c.args.clone(),
            }),
        }
    }
}
impl From<Metadata> for ConnectionMetadata {
    fn from(m: Metadata) -> Self {
        Self {
            display_name: m.display_name,
            category: m.category,
            desktop_transport: m.desktop_transport,
            coding_cli: m.coding_cli.map(|c| CodingCli {
                id: c.id,
                name: c.name,
                command: c.command,
                args: c.args,
            }),
        }
    }
}

pub struct Entry {
    pub name: String,
    pub origin: String,
    pub metadata: ConnectionMetadata,
    pub credential: Option<Secret>,
}
pub struct ScreenEntry {
    pub name: String,
    pub mcp_url: String,
    pub token: Option<Secret>,
}
pub struct Payload {
    pub entries: Vec<Entry>,
    pub desktops: Vec<ScreenEntry>,
}
impl Payload {
    fn version(&self) -> u32 {
        if self.desktops.is_empty() {
            1
        } else {
            2
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicScreen {
    name: String,
    mcp_url: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicEntry {
    name: String,
    openab_pty_url: String,
    metadata: Metadata,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicFile {
    format: String,
    version: u32,
    mode: String,
    connections: Vec<PublicEntry>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_screens"
    )]
    desktops: Option<Vec<PublicScreen>>,
}

// An explicitly present null collection is invalid, including on legacy v1.
fn present_screens<'de, D, T>(d: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Vec::<T>::deserialize(d).map(Some)
}

// Parse strings directly into a zeroizing owner. No serde_json::Value secret tree.
struct Credential(Secret);
impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Credential;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("credential")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Secret::new(v.to_owned())
                    .map(Credential)
                    .map_err(|_| E::custom("invalid credential"))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Secret::new(v)
                    .map(Credential)
                    .map_err(|_| E::custom("invalid credential"))
            }
        }
        d.deserialize_str(Visitor)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateEntry {
    name: String,
    openab_pty_url: String,
    metadata: Metadata,
    credential: Credential,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateScreen {
    name: String,
    mcp_url: String,
    token: Credential,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateFile {
    format: String,
    version: u32,
    connections: Vec<PrivateEntry>,
    #[serde(default, deserialize_with = "present_screens")]
    desktops: Option<Vec<PrivateScreen>>,
}
// Serialization exists only for a private borrowed wrapper, never the public sensitive owner.
struct PrivateOutput<'a>(&'a Payload);
impl Serialize for PrivateOutput<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeSeq, SerializeStruct};
        struct Entries<'a>(&'a [Entry]);
        struct Item<'a>(&'a Entry);
        impl Serialize for Item<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut o = s.serialize_struct("Connection", 4)?;
                o.serialize_field("name", &self.0.name)?;
                o.serialize_field("openab_pty_url", &self.0.origin)?;
                o.serialize_field("metadata", &Metadata::from(&self.0.metadata))?;
                o.serialize_field(
                    "credential",
                    self.0
                        .credential
                        .as_ref()
                        .ok_or_else(|| serde::ser::Error::custom("credential required"))?
                        .expose(),
                )?;
                o.end()
            }
        }
        impl Serialize for Entries<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut o = s.serialize_seq(Some(self.0.len()))?;
                for e in self.0 {
                    o.serialize_element(&Item(e))?;
                }
                o.end()
            }
        }
        struct Screens<'a>(&'a [ScreenEntry]);
        struct Screen<'a>(&'a ScreenEntry);
        impl Serialize for Screen<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut o = s.serialize_struct("Screen", 3)?;
                o.serialize_field("name", &self.0.name)?;
                o.serialize_field("mcp_url", &self.0.mcp_url)?;
                o.serialize_field(
                    "token",
                    self.0
                        .token
                        .as_ref()
                        .ok_or_else(|| serde::ser::Error::custom("token required"))?
                        .expose(),
                )?;
                o.end()
            }
        }
        impl Serialize for Screens<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut o = s.serialize_seq(Some(self.0.len()))?;
                for e in self.0 {
                    o.serialize_element(&Screen(e))?;
                }
                o.end()
            }
        }
        let version = self.0.version();
        let mut o = s.serialize_struct("Payload", if version == 1 { 3 } else { 4 })?;
        o.serialize_field("format", FORMAT)?;
        o.serialize_field("version", &version)?;
        o.serialize_field("connections", &Entries(&self.0.entries))?;
        if version == 2 {
            o.serialize_field("desktops", &Screens(&self.0.desktops))?;
        }
        o.end()
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Kdf {
    name: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}
impl Default for Kdf {
    fn default() -> Self {
        Self {
            name: "argon2id".into(),
            version: 19,
            memory_kib: 65536,
            iterations: 3,
            parallelism: 4,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptedFile {
    format: String,
    version: u32,
    mode: String,
    kdf: Kdf,
    cipher: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}
pub struct LockedFile {
    version: u32,
    salt: [u8; 16],
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}
pub enum Decoded {
    Public(Payload),
    Encrypted(LockedFile),
}
impl Payload {
    pub fn validate(&mut self) -> Result<(), ApiError> {
        if (self.entries.is_empty() && self.desktops.is_empty())
            || self.entries.len() > 64
            || self.desktops.len() > 64
        {
            return Err(ApiError::Capacity);
        }
        let mut origins = HashSet::new();
        let mut size = 0usize;
        for e in &mut self.entries {
            if e.name.is_empty()
                || e.name.len() > 128
                || e.name.trim() != e.name
                || e.name.chars().any(char::is_control)
            {
                return Err(ApiError::Protocol);
            }
            e.origin = canonical_origin(&e.origin)?;
            if !origins.insert(e.origin.clone()) {
                return Err(ApiError::Protocol);
            }
            e.metadata.validate().map_err(|_| ApiError::Protocol)?;
            e.metadata.normalize_display_name();
            size += e.name.len()
                + e.origin.len()
                + serde_json::to_vec(&Metadata::from(&e.metadata))
                    .map_err(|_| ApiError::Protocol)?
                    .len()
                + e.credential.as_ref().map_or(0, |s| s.expose().len())
                + 128;
        }
        let mut urls = HashSet::new();
        for e in &mut self.desktops {
            if e.name.is_empty()
                || e.name.len() > 128
                || e.name.trim() != e.name
                || e.name.chars().any(char::is_control)
            {
                return Err(ApiError::Protocol);
            }
            e.mcp_url = crate::instance_mcp::canonical_url(&e.mcp_url)?;
            if !urls.insert(e.mcp_url.clone())
                || e.token.as_ref().is_some_and(|t| !t.expose().is_ascii())
            {
                return Err(ApiError::Protocol);
            }
            size += e.name.len()
                + e.mcp_url.len()
                + e.token.as_ref().map_or(0, |t| t.expose().len())
                + 128;
        }
        if size > MAX_PAYLOAD {
            return Err(ApiError::Capacity);
        }
        Ok(())
    }
}
fn check_header(format: &str, version: u32) -> Result<(), ApiError> {
    if format != FORMAT || !matches!(version, 1 | 2) {
        Err(ApiError::Protocol)
    } else {
        Ok(())
    }
}
fn decode64(s: &str, max: usize) -> Result<Vec<u8>, ApiError> {
    if s.len() > max.div_ceil(3) * 4 {
        return Err(ApiError::Capacity);
    }
    let b = STANDARD.decode(s).map_err(|_| ApiError::Protocol)?;
    if b.len() > max || STANDARD.encode(&b) != s {
        return Err(ApiError::Protocol);
    }
    Ok(b)
}
pub fn decode(bytes: &[u8]) -> Result<Decoded, ApiError> {
    if bytes.len() > MAX_FILE {
        return Err(ApiError::Capacity);
    }
    // Untagged deserialization validates the complete concrete schema, including duplicate fields.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum File {
        Public(PublicFile),
        Encrypted(EncryptedFile),
    }
    match serde_json::from_slice::<File>(bytes).map_err(|_| ApiError::Protocol)? {
        File::Public(f) => {
            check_header(&f.format, f.version)?;
            if f.mode != "without-credentials" || bytes.len() > MAX_PAYLOAD {
                return Err(ApiError::Protocol);
            }
            if (f.version == 1) != f.desktops.is_none() {
                return Err(ApiError::Protocol);
            }
            let mut p = Payload {
                desktops: f
                    .desktops
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| ScreenEntry {
                        name: e.name,
                        mcp_url: e.mcp_url,
                        token: None,
                    })
                    .collect(),
                entries: f
                    .connections
                    .into_iter()
                    .map(|e| Entry {
                        name: e.name,
                        origin: e.openab_pty_url,
                        metadata: e.metadata.into(),
                        credential: None,
                    })
                    .collect(),
            };
            p.validate()?;
            Ok(Decoded::Public(p))
        }
        File::Encrypted(f) => {
            check_header(&f.format, f.version)?;
            if f.mode != "encrypted" || f.kdf != Kdf::default() || f.cipher != "aes-256-gcm" {
                return Err(ApiError::Protocol);
            }
            let salt = decode64(&f.salt, 16)?
                .try_into()
                .map_err(|_| ApiError::Protocol)?;
            let nonce = decode64(&f.nonce, 12)?
                .try_into()
                .map_err(|_| ApiError::Protocol)?;
            let ciphertext = decode64(&f.ciphertext, MAX_PAYLOAD + 16)?;
            if ciphertext.len() < 16 {
                return Err(ApiError::Protocol);
            }
            Ok(Decoded::Encrypted(LockedFile {
                version: f.version,
                salt,
                nonce,
                ciphertext,
            }))
        }
    }
}
fn derive(password: &str, salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, ApiError> {
    if password.is_empty() || password.len() > 1024 {
        return Err(ApiError::Protocol);
    }
    let params = Params::new(65536, 3, 4, Some(32)).map_err(|_| ApiError::Protocol)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut scratch = Zeroizing::new(vec![Block::default(); 65536]);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into_with_memory(
            password.as_bytes(),
            salt,
            key.as_mut(),
            scratch.as_mut_slice(),
        )
        .map_err(|_| ApiError::Protocol)?;
    Ok(key)
}
fn aad(version: u32, salt: &[u8; 16], nonce: &[u8; 12]) -> Vec<u8> {
    [
        if version == 1 { DOMAIN } else { DOMAIN_V2 },
        &[0],
        salt,
        nonce,
    ]
    .concat()
}
pub fn unlock(file: LockedFile, password: Zeroizing<String>) -> Result<Payload, ApiError> {
    let key = derive(&password, &file.salt)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| ApiError::Protocol)?;
    let mut bytes = Zeroizing::new(file.ciphertext);
    cipher
        .decrypt_in_place(
            Nonce::from_slice(&file.nonce),
            &aad(file.version, &file.salt, &file.nonce),
            &mut *bytes,
        )
        .map_err(|_| ApiError::Auth)?;
    if bytes.len() > MAX_PAYLOAD {
        return Err(ApiError::Capacity);
    }
    let f: PrivateFile = serde_json::from_slice(&bytes).map_err(|_| ApiError::Protocol)?;
    check_header(&f.format, f.version)?;
    if f.version != file.version || (f.version == 1) != f.desktops.is_none() {
        return Err(ApiError::Protocol);
    }
    let mut p = Payload {
        desktops: f
            .desktops
            .unwrap_or_default()
            .into_iter()
            .map(|e| ScreenEntry {
                name: e.name,
                mcp_url: e.mcp_url,
                token: Some(e.token.0),
            })
            .collect(),
        entries: f
            .connections
            .into_iter()
            .map(|e| Entry {
                name: e.name,
                origin: e.openab_pty_url,
                metadata: e.metadata.into(),
                credential: Some(e.credential.0),
            })
            .collect(),
    };
    p.validate()?;
    Ok(p)
}
pub fn encode(
    mut payload: Payload,
    password: Option<Zeroizing<String>>,
) -> Result<Vec<u8>, ApiError> {
    payload.validate()?;
    let version = payload.version();
    match password {
        None => {
            let f = PublicFile {
                format: FORMAT.into(),
                version,
                mode: "without-credentials".into(),
                desktops: if version == 2 {
                    Some(
                        payload
                            .desktops
                            .into_iter()
                            .map(|e| PublicScreen {
                                name: e.name,
                                mcp_url: e.mcp_url,
                            })
                            .collect(),
                    )
                } else {
                    None
                },
                connections: payload
                    .entries
                    .into_iter()
                    .map(|e| PublicEntry {
                        name: e.name,
                        openab_pty_url: e.origin,
                        metadata: Metadata::from(&e.metadata),
                    })
                    .collect(),
            };
            let bytes = serde_json::to_vec(&f).map_err(|_| ApiError::Protocol)?;
            if bytes.len() > MAX_PAYLOAD {
                return Err(ApiError::Capacity);
            }
            Ok(bytes)
        }
        Some(password) => {
            let mut salt = [0; 16];
            let mut nonce = [0; 12];
            getrandom::fill(&mut salt).map_err(|_| ApiError::Storage)?;
            getrandom::fill(&mut nonce).map_err(|_| ApiError::Storage)?;
            encrypt(payload, password, salt, nonce)
        }
    }
}
fn encrypt(
    payload: Payload,
    password: Zeroizing<String>,
    salt: [u8; 16],
    nonce: [u8; 12],
) -> Result<Vec<u8>, ApiError> {
    // Preallocate and bound writes: serde cannot reallocate a secret-bearing buffer.
    struct Bounded<'a>(&'a mut Vec<u8>);
    impl std::io::Write for Bounded<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + bytes.len() > MAX_PAYLOAD {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let version = payload.version();
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_PAYLOAD + 16));
    serde_json::to_writer(Bounded(&mut bytes), &PrivateOutput(&payload))
        .map_err(|_| ApiError::Capacity)?;
    if bytes.len() > MAX_PAYLOAD {
        return Err(ApiError::Capacity);
    }
    let key = derive(&password, &salt)?;
    Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| ApiError::Protocol)?
        .encrypt_in_place(
            Nonce::from_slice(&nonce),
            &aad(version, &salt, &nonce),
            &mut *bytes,
        )
        .map_err(|_| ApiError::Protocol)?;
    let f = EncryptedFile {
        format: FORMAT.into(),
        version,
        mode: "encrypted".into(),
        kdf: Kdf::default(),
        cipher: "aes-256-gcm".into(),
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(&*bytes),
    };
    let result = serde_json::to_vec(&f).map_err(|_| ApiError::Protocol)?;
    if result.len() > MAX_FILE {
        return Err(ApiError::Capacity);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
