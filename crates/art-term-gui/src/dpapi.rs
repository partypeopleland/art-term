//! Current-user Windows protection for operator credentials.
//! The app exposes no decrypt/readback command to its WebView.

use art_term_core::openab::{
    api::ApiError,
    secret::{CredentialCipher, CredentialContext, Secret},
};
use std::{
    ffi::c_void,
    ptr,
    sync::atomic::{compiler_fence, Ordering},
};

#[repr(C)]
struct DataBlob {
    size: u32,
    data: *mut u8,
}

#[link(name = "crypt32")]
extern "system" {
    fn CryptProtectData(
        input: *const DataBlob,
        description: *const u16,
        entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: u32,
        output: *mut DataBlob,
    ) -> i32;
    fn CryptUnprotectData(
        input: *const DataBlob,
        description: *mut *mut u16,
        entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: u32,
        output: *mut DataBlob,
    ) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

/// DPAPI's returned allocation is erased before returning it to LocalFree.
struct ProtectedAllocation(DataBlob);
impl Drop for ProtectedAllocation {
    fn drop(&mut self) {
        if self.0.data.is_null() {
            return;
        }
        // SAFETY: Crypt(Un)ProtectData returned this allocation and length.
        // Volatile writes and the fence keep the wipe observable to the OS.
        unsafe {
            for index in 0..self.0.size as usize {
                ptr::write_volatile(self.0.data.add(index), 0);
            }
            compiler_fence(Ordering::SeqCst);
            LocalFree(self.0.data.cast());
        }
    }
}
impl ProtectedAllocation {
    fn bytes(&self) -> Result<&[u8], ApiError> {
        if self.0.data.is_null() || self.0.size == 0 || self.0.size > 65536 {
            return Err(ApiError::Storage);
        }
        // SAFETY: DPAPI owns a valid allocation until this guard is dropped.
        Ok(unsafe { std::slice::from_raw_parts(self.0.data, self.0.size as usize) })
    }
}

fn crypt(
    bytes: &[u8],
    context: &CredentialContext,
    protect: bool,
) -> Result<ProtectedAllocation, ApiError> {
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err(ApiError::Storage);
    }
    let entropy_bytes = context.authenticated_bytes();
    let input = DataBlob {
        size: bytes.len() as u32,
        data: bytes.as_ptr().cast_mut(),
    };
    let entropy = DataBlob {
        size: entropy_bytes.len() as u32,
        data: entropy_bytes.as_ptr().cast_mut(),
    };
    let mut output = ProtectedAllocation(DataBlob {
        size: 0,
        data: ptr::null_mut(),
    });
    // CRYPTPROTECT_UI_FORBIDDEN only; never CRYPTPROTECT_LOCAL_MACHINE.
    // Null description/prompt also avoids descriptions containing user data.
    // SAFETY: input/entropy borrows live for the call; reserved/prompt are null;
    // the output allocation is owned by the guard even on an error path.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                &mut output.0,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                &mut output.0,
            )
        }
    };
    if ok == 0 {
        return Err(ApiError::Storage);
    }
    Ok(output)
}

pub struct WindowsCredentialCipher;
impl CredentialCipher for WindowsCredentialCipher {
    fn seal(&self, secret: &Secret, context: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let protected = crypt(secret.expose().as_bytes(), context, true)?;
        Ok(protected.bytes()?.to_vec())
    }

    fn unseal(&self, bytes: &[u8], context: &CredentialContext) -> Result<Secret, ApiError> {
        let plain = crypt(bytes, context, false)?;
        let text = std::str::from_utf8(plain.bytes()?).map_err(|_| ApiError::Storage)?;
        Secret::new(text.to_owned()).map_err(|_| ApiError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> CredentialContext {
        CredentialContext {
            schema: 1,
            host_id: "fixture-host".into(),
            origin: "https://fixture.example:8443".into(),
            revision: 1,
        }
    }

    #[test]
    fn native_protection_roundtrip_and_context_binding() {
        let cipher = WindowsCredentialCipher;
        let context = context();
        let secret = Secret::new("fixture-only-admin-credential".into()).unwrap();
        let encrypted = cipher.seal(&secret, &context).unwrap();
        assert!(!encrypted
            .windows(secret.expose().len())
            .any(|part| part == secret.expose().as_bytes()));
        assert_eq!(
            cipher.unseal(&encrypted, &context).unwrap().expose(),
            secret.expose()
        );
        for drift in [
            CredentialContext {
                schema: context.schema + 1,
                ..context.clone()
            },
            CredentialContext {
                host_id: "other-host".into(),
                ..context.clone()
            },
            CredentialContext {
                origin: "https://different.example:8443".into(),
                ..context.clone()
            },
            CredentialContext {
                revision: context.revision + 1,
                ..context.clone()
            },
        ] {
            assert!(cipher.unseal(&encrypted, &drift).is_err());
        }
        let mut corrupted = encrypted;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        assert!(cipher.unseal(&corrupted, &context).is_err());
    }

    #[test]
    fn native_host_store_reload_edit_delete_and_corruption() {
        use art_term_core::{
            connections::{Category, CodingCli, ConnectionMetadata},
            openab::profiles::HostStore,
        };
        use std::{
            fs,
            path::Path,
            sync::Arc,
            time::{SystemTime, UNIX_EPOCH},
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap();
        let base = workspace.join("target/windows-regression");
        fs::create_dir_all(&base).unwrap();
        let directory = base.join(format!("dpapi-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("connections.toml");
        let cipher = Arc::new(WindowsCredentialCipher);
        // A missing file loads as an empty store without writing anything.
        let store = HostStore::load(path.clone(), cipher.clone()).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!path.exists());
        let secret = "fixture-only-persisted-admin-credential";
        let metadata = ConnectionMetadata {
            category: Category::Agent,
            desktop_transport: Some("runtime-mcp".into()),
            coding_cli: Some(CodingCli {
                id: "fixture-cli".into(),
                name: "Fixture CLI".into(),
                command: "fixture-cli".into(),
                args: vec!["--flag \"quoted words\"".into(), "".into()],
            }),
        };
        let host = store
            .add_with_metadata(
                "Fixture".into(),
                "https://fixture.example:8443".into(),
                Secret::new(secret.into()).unwrap(),
                Some(metadata.clone()),
            )
            .unwrap();
        let original = fs::read(&path).unwrap();
        let text = String::from_utf8(original.clone()).unwrap();
        assert!(text.contains("schema_version = 5"));
        assert!(!original
            .windows(secret.len())
            .any(|part| part == secret.as_bytes()));
        // Reloading the real schema5 TOML unseals under the same user/context
        // and preserves Agent + runtime-mcp + CLI metadata.
        let reopened = HostStore::load(path.clone(), cipher.clone()).unwrap();
        let snapshot = reopened.snapshot(&host.id).unwrap();
        assert_eq!(snapshot.credential.expose(), secret);
        assert_eq!(snapshot.profile.metadata.as_ref(), Some(&metadata));
        // Blank-secret rename bumps the revision, re-seals under the new
        // context, and preserves the omitted metadata.
        let renamed = reopened
            .update(
                &host.id,
                host.revision,
                "Renamed".into(),
                host.base_url.clone(),
                None,
            )
            .unwrap();
        assert_eq!(renamed.revision, host.revision + 1);
        assert_eq!(renamed.metadata.as_ref(), Some(&metadata));
        // Explicit category edit under a blank secret applies the new
        // metadata while the DPAPI credential survives the revision bump.
        let trimmed = ConnectionMetadata {
            category: Category::Host,
            desktop_transport: None,
            coding_cli: None,
        };
        let recategorized = reopened
            .update_with_metadata(
                &renamed.id,
                renamed.revision,
                renamed.name.clone(),
                renamed.base_url.clone(),
                None,
                Some(trimmed.clone()),
            )
            .unwrap();
        assert_eq!(recategorized.revision, renamed.revision + 1);
        let reopened = HostStore::load(path.clone(), cipher.clone()).unwrap();
        let snapshot = reopened.snapshot(&host.id).unwrap();
        assert_eq!(snapshot.credential.expose(), secret);
        assert_eq!(snapshot.profile.metadata.as_ref(), Some(&trimmed));
        // A different origin without a new credential fails closed and the
        // on-disk file is untouched.
        let before_refusal = fs::read(&path).unwrap();
        assert!(reopened
            .update(
                &host.id,
                recategorized.revision,
                "Renamed".into(),
                "https://other.example:8443".into(),
                None
            )
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before_refusal);
        reopened
            .delete(&recategorized.id, recategorized.revision)
            .unwrap();
        assert!(HostStore::load(path.clone(), cipher.clone())
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        fs::write(&path, b"corrupt fixture data").unwrap();
        assert!(HostStore::load(path.clone(), cipher).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"corrupt fixture data");
        // Only the exact task-owned objects; no recursive cleanup.
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }
}
