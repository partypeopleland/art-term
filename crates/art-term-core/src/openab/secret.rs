use std::fmt;
use zeroize::Zeroizing;

/// Non-serializable, zeroizing native-only credential or session token.
///
/// ```compile_fail
/// use art_term_core::openab::secret::Secret;
/// let secret = Secret::new("one-shot-credential".into()).unwrap();
/// let _ = serde_json::to_string(&secret);
/// ```
pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn new(value: String) -> Result<Self, super::api::ApiError> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 1024 || value.bytes().any(|b| b <= 32 || b == 127) {
            return Err(super::api::ApiError::Protocol);
        }
        Ok(Self(value))
    }
    /// Available only to native transport/cipher code, never an IPC readback.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Platforms must authenticate this entire context (AEAD/DPAPI entropy).
#[derive(Clone, Debug)]
pub struct CredentialContext {
    pub schema: u32,
    pub host_id: String,
    pub origin: String,
    pub revision: u64,
}
impl CredentialContext {
    pub fn authenticated_bytes(&self) -> Vec<u8> {
        // Length-prefixed JSON tuple is unambiguous and shared across platforms.
        serde_json::to_vec(&(self.schema, &self.host_id, &self.origin, self.revision))
            .expect("string serialization")
    }
}
pub trait CredentialCipher: Send + Sync {
    fn seal(
        &self,
        secret: &Secret,
        context: &CredentialContext,
    ) -> Result<Vec<u8>, super::api::ApiError>;
    fn unseal(
        &self,
        bytes: &[u8],
        context: &CredentialContext,
    ) -> Result<Secret, super::api::ApiError>;
}
