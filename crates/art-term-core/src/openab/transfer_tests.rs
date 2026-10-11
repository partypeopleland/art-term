use super::*;
use crate::openab::{
    profiles::{ExportSelection, HostStore, ImportAction, ImportSelection},
    secret::{CredentialCipher, CredentialContext},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn payload(private: bool) -> Payload {
    Payload {
        entries: vec![Entry {
            name: "Fixture Agent".into(),
            origin: "https://agent.example.test".into(),
            metadata: ConnectionMetadata {
                category: Category::Agent,
                desktop_transport: Some("runtime-mcp".into()),
                coding_cli: Some(CodingCli {
                    id: "assistant".into(),
                    name: "Assistant".into(),
                    command: "assistant-cli".into(),
                    args: vec!["--model".into(), "literal ' $(echo)".into()],
                }),
            },
            credential: private.then(|| Secret::new("fictional-credential".into()).unwrap()),
        }],
    }
}
fn decoded(bytes: &[u8], password: &str) -> Payload {
    match decode(bytes).unwrap() {
        Decoded::Public(p) => p,
        Decoded::Encrypted(f) => unlock(f, Zeroizing::new(password.into())).unwrap(),
    }
}
#[test]
fn public_strict_roundtrip_and_private_vector() {
    let b = encode(payload(false), None).unwrap();
    assert!(!String::from_utf8_lossy(&b).contains("credential\""));
    let p = decoded(&b, "");
    assert_eq!(p.entries[0].metadata, payload(false).entries[0].metadata);
    let b = encrypt(
        payload(true),
        Zeroizing::new(" 密碼 pass ".into()),
        [7; 16],
        [9; 12],
    )
    .unwrap();
    let text = String::from_utf8_lossy(&b);
    for plain in [
        "Fixture",
        "agent.example",
        "fictional",
        "runtime-mcp",
        "assistant-cli",
    ] {
        assert!(!text.contains(plain));
    }
    const VECTOR: &str = r#"{"format":"art-term-connections","version":1,"mode":"encrypted","kdf":{"name":"argon2id","version":19,"memory_kib":65536,"iterations":3,"parallelism":4},"cipher":"aes-256-gcm","salt":"BwcHBwcHBwcHBwcHBwcHBw==","nonce":"CQkJCQkJCQkJCQkJ","ciphertext":"bHNskqUfv5U2gYtNbcqQ3xOgQMebpVMeAcWwL9zyt8fH3pqyML6Ig/K7Iq4YXAYOu+LWqv3DXwtF+blYIzZacg20yIhmq49USPYoSnrdyG8OrjHxCdX5bjGwDtbvQRMMTq3VCkeaX068wHZgEOqas8lKQJFi0Cqw2bcBBNCT3weYUwgQhxzVOESiUabGiyNayXhWC0MY4cWk/ctCJkfJ5R1dg8uoOUtIhG3FIyhRwZx0qRJBsLRdXue4yd0iFIjqO8e1n/gCyU6e3CTj7oKrFl340ZVED/ubp3ANfETQrPX/iIy9qzCIvRWAmdTPcSbUU2q4BOXgtlxzeu0f5jTzZV5kcjURgQRA7r9HmTh7LMqsUJTCHtxBCOX7moNnKavrMUsX/AamegoNO477dB5ZcucBNEHX8Y4ocvS8AHILqmsGgjEtOFSnsp0gpRxQElhtGkQ8TOor0NeRcJ3rRKlvJ6gM50FOI9vAzyiOwg=="}"#;
    assert_eq!(b, VECTOR.as_bytes());
    let p = decoded(VECTOR.as_bytes(), " 密碼 pass ");
    assert_eq!(
        p.entries[0].credential.as_ref().unwrap().expose(),
        "fictional-credential"
    );
    // A fixed public vector hash guards cross-platform format/KDF/AAD compatibility.
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&b));
    assert_eq!(
        digest,
        "55506b0d8f844fcdb8300c5edc08decfeb624e18729a7fa8f9f0444774f14ec1"
    );
}
#[test]
fn reject_untrusted_files_and_authenticated_changes() {
    let public = encode(payload(false), None).unwrap();
    let text = String::from_utf8(public).unwrap();
    for hostile in [
        text.replace("\"version\":1", "\"version\":2"),
        text.replace("\"version\":1", "\"version\":1,\"version\":1"),
        text.replace(
            "\"name\":\"Fixture Agent\"",
            "\"name\":\"Fixture Agent\",\"credential\":null",
        ),
        text.replace("\"category\":\"agent\"", "\"category\":\"other\""),
        text.replace(
            "\"command\":\"assistant-cli\"",
            "\"command\":\"assistant-cli\",\"unknown\":true",
        ),
        text[..text.len() - 4].into(),
    ] {
        assert!(decode(hostile.as_bytes()).is_err());
    }
    assert!(decode(&vec![b' '; MAX_FILE + 1]).is_err());
    let b = encrypt(
        payload(true),
        Zeroizing::new("fixture password".into()),
        [7; 16],
        [9; 12],
    )
    .unwrap();
    if let Decoded::Encrypted(f) = decode(&b).unwrap() {
        assert!(matches!(
            unlock(f, Zeroizing::new("wrong".into())),
            Err(ApiError::Auth)
        ));
    }
    for (field, value) in [
        ("cipher", serde_json::json!("aes-128-gcm")),
        ("version", serde_json::json!(2)),
        ("nonce", serde_json::json!("x")),
        ("extra", serde_json::json!(true)),
        (
            "kdf",
            serde_json::json!({"name":"argon2id","version":19,"memory_kib":1,"iterations":1,"parallelism":1}),
        ),
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        v[field] = value;
        assert!(decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    let mut v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let mut c = STANDARD.decode(v["ciphertext"].as_str().unwrap()).unwrap();
    c[0] ^= 1;
    v["ciphertext"] = STANDARD.encode(c).into();
    if let Decoded::Encrypted(f) = decode(&serde_json::to_vec(&v).unwrap()).unwrap() {
        assert!(matches!(
            unlock(f, Zeroizing::new("fixture password".into())),
            Err(ApiError::Auth)
        ));
    }
    for kind in 0..7 {
        let mut p = payload(false);
        match kind {
            0 => p.entries[0].name = "x".repeat(129),
            1 => p.entries[0].origin = "http://example.test".into(),
            2 => p.entries[0].metadata.desktop_transport = Some("ssh".into()),
            3 => p.entries[0].metadata.coding_cli.as_mut().unwrap().args = vec!["a".repeat(1025)],
            4 => {
                let mut other = payload(false).entries.remove(0);
                other.origin = "https://AGENT.example.test:443/".into();
                p.entries.push(other);
            }
            5 => {
                p.entries = (0..65)
                    .map(|i| {
                        let mut e = payload(false).entries.remove(0);
                        e.origin = format!("https://h{i}.test");
                        e
                    })
                    .collect()
            }
            _ => p.entries[0].metadata.coding_cli.as_mut().unwrap().args = vec!["x".into(); 33],
        }
        assert!(encode(p, None).is_err());
    }
    assert!(Secret::new("x".repeat(1025)).is_err());
}
struct Cipher {
    seals: AtomicUsize,
    fail_at: AtomicUsize,
}
impl CredentialCipher for Cipher {
    fn seal(&self, s: &Secret, c: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let count = self.seals.fetch_add(1, Ordering::SeqCst) + 1;
        if count == self.fail_at.load(Ordering::SeqCst) {
            return Err(ApiError::Storage);
        }
        let mut b = c.authenticated_bytes();
        b.push(0);
        b.extend_from_slice(s.expose().as_bytes());
        Ok(b)
    }
    fn unseal(&self, b: &[u8], c: &CredentialContext) -> Result<Secret, ApiError> {
        let mut prefix = c.authenticated_bytes();
        prefix.push(0);
        let b = b.strip_prefix(prefix.as_slice()).ok_or(ApiError::Auth)?;
        Secret::new(String::from_utf8(b.to_vec()).map_err(|_| ApiError::Auth)?)
    }
}
fn choices(
    p: &crate::openab::profiles::ImportPreview,
    action: ImportAction,
) -> Vec<ImportSelection> {
    p.entries
        .iter()
        .map(|e| ImportSelection {
            key: e.key.clone(),
            action,
            credential: None,
        })
        .collect()
}
#[test]
fn atomic_batch_metadata_reload_stale_and_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let cipher = Arc::new(Cipher {
        seals: AtomicUsize::new(0),
        fail_at: AtomicUsize::new(usize::MAX),
    });
    let store = HostStore::load(path.clone(), cipher.clone()).unwrap();
    let preview = store.preview_import(payload(true)).unwrap();
    let result = store.apply_import(preview, vec![]).unwrap();
    assert_eq!(result.skipped, 1);
    assert!(store.list().unwrap().is_empty());
    let preview = store.preview_import(payload(false)).unwrap();
    let selection = choices(&preview, ImportAction::Add);
    assert!(store.apply_import(preview, selection).is_err());
    assert!(store.list().unwrap().is_empty());
    let preview = store.preview_import(payload(true)).unwrap();
    let selection = choices(&preview, ImportAction::Add);
    let added = store
        .apply_import(preview, selection)
        .unwrap()
        .profiles
        .remove(0);
    assert_eq!(added.revision, 1);
    let preview = store.preview_import(payload(false)).unwrap();
    assert_eq!(preview.entries[0].match_status, "duplicate");
    let selection = choices(&preview, ImportAction::Update);
    let result = store.apply_import(preview, selection).unwrap();
    assert_eq!(result.profiles[0].id, added.id);
    assert_eq!(result.profiles[0].revision, 2);
    assert_eq!(
        store.snapshot(&added.id).unwrap().credential.expose(),
        "fictional-credential"
    );
    assert_eq!(
        HostStore::load(path.clone(), cipher.clone())
            .unwrap()
            .list()
            .unwrap(),
        store.list().unwrap()
    );
    for operation in 0..3 {
        let preview = store.preview_import(payload(true)).unwrap();
        let selection = choices(&preview, ImportAction::Update);
        let current = store.list().unwrap()[0].clone();
        match operation {
            0 => {
                store
                    .add(
                        "Other".into(),
                        "https://other.test".into(),
                        Secret::new("fake".into()).unwrap(),
                    )
                    .unwrap();
            }
            1 => {
                store
                    .update(
                        &current.id,
                        current.revision,
                        "Changed".into(),
                        current.base_url,
                        None,
                    )
                    .unwrap();
            }
            _ => {
                let other = store
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|p| p.name == "Other")
                    .unwrap();
                store.delete(&other.id, other.revision).unwrap();
            }
        }
        assert!(matches!(
            store.apply_import(preview, selection),
            Err(ApiError::StateChanged)
        ));
    }
    let before = std::fs::read(&path).unwrap();
    let profiles = store.list().unwrap();
    let mut p = payload(true);
    let mut other = payload(true).entries.remove(0);
    other.origin = "https://new.test".into();
    p.entries.push(other);
    let preview = store.preview_import(p).unwrap();
    let selection = preview
        .entries
        .iter()
        .map(|e| ImportSelection {
            key: e.key.clone(),
            action: if e.match_status == "new" {
                ImportAction::Add
            } else {
                ImportAction::Update
            },
            credential: None,
        })
        .collect();
    cipher
        .fail_at
        .store(cipher.seals.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    assert!(matches!(
        store.apply_import(preview, selection),
        Err(ApiError::Storage)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(store.list().unwrap(), profiles);
    cipher.fail_at.store(usize::MAX, Ordering::SeqCst);
    // Make persistence fail without modifying the existing file: replace its parent with a file is
    // unnecessary; a missing parent in another isolated store exercises the exact save boundary.
    let missing = dir.path().join("missing").join("store.json");
    let failed = HostStore::load(missing, cipher).unwrap();
    let p = failed.preview_import(payload(true)).unwrap();
    let selection = choices(&p, ImportAction::Add);
    assert!(failed.apply_import(p, selection).is_err());
    assert!(failed.list().unwrap().is_empty());
}
#[test]
fn unique_origin_capacity_and_supplemental_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let cipher = Arc::new(Cipher {
        seals: AtomicUsize::new(0),
        fail_at: AtomicUsize::new(usize::MAX),
    });
    let store = HostStore::load(dir.path().join("store.json"), cipher).unwrap();
    let p = store.preview_import(payload(false)).unwrap();
    let mut selected = choices(&p, ImportAction::Add);
    selected[0].credential = Some(Secret::new("supplement".into()).unwrap());
    store.apply_import(p, selected).unwrap();
    assert!(store
        .add(
            "Same origin".into(),
            "https://agent.example.test".into(),
            Secret::new("fake".into()).unwrap()
        )
        .is_err());
    let p = store.preview_import(payload(true)).unwrap();
    assert_eq!(p.entries[0].match_status, "duplicate");
    for i in 1..64 {
        store
            .add(
                "Fixture Agent".into(),
                format!("https://host{i}.test"),
                Secret::new("fake".into()).unwrap(),
            )
            .unwrap();
    }
    let mut data = payload(true);
    data.entries[0].origin = "https://new.test".into();
    let p = store.preview_import(data).unwrap();
    let selected = choices(&p, ImportAction::Add);
    let before = store.list().unwrap();
    assert!(matches!(
        store.apply_import(p, selected),
        Err(ApiError::Capacity)
    ));
    assert_eq!(before, store.list().unwrap());
    assert!(store
        .export_snapshot(
            &[ExportSelection {
                id: before[0].id.clone(),
                expected_revision: 999
            }],
            false,
            None
        )
        .is_err());
}

#[test]
fn mixed_batch_metadata_none_and_existing_file_save_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let cipher = Arc::new(Cipher {
        seals: AtomicUsize::new(0),
        fail_at: AtomicUsize::new(usize::MAX),
    });
    let store = HostStore::load(path.clone(), cipher.clone()).unwrap();
    let p = store.preview_import(payload(true)).unwrap();
    let selected = choices(&p, ImportAction::Add);
    let original = store.apply_import(p, selected).unwrap().profiles.remove(0);
    let mut data = payload(true);
    data.entries[0].name = "Changed".into();
    data.entries[0].metadata = ConnectionMetadata::default();
    data.entries[0].credential = Some(Secret::new("replacement".into()).unwrap());
    let mut new = payload(true).entries.remove(0);
    new.origin = "https://new.test".into();
    let mut skipped = payload(true).entries.remove(0);
    skipped.origin = "https://skip.test".into();
    data.entries.extend([new, skipped]);
    let p = store.preview_import(data).unwrap();
    let selected = p
        .entries
        .iter()
        .zip([ImportAction::Update, ImportAction::Add, ImportAction::Skip])
        .map(|(e, action)| ImportSelection {
            key: e.key.clone(),
            action,
            credential: None,
        })
        .collect();
    let result = store.apply_import(p, selected).unwrap();
    assert_eq!((result.added, result.updated, result.skipped), (1, 1, 1));
    assert_eq!(result.profiles[0].id, original.id);
    assert_eq!(result.profiles[0].revision, 2);
    assert_eq!(
        result.profiles[0].metadata,
        Some(ConnectionMetadata::default())
    );
    assert_eq!(
        store.snapshot(&original.id).unwrap().credential.expose(),
        "replacement"
    );
    let restored = HostStore::load(path.clone(), cipher).unwrap();
    assert_eq!(restored.list().unwrap(), store.list().unwrap());
    let select: Vec<_> = result
        .profiles
        .iter()
        .map(|p| ExportSelection {
            id: p.id.clone(),
            expected_revision: p.revision,
        })
        .collect();
    let b = encode(
        restored.export_snapshot(&select, false, None).unwrap(),
        None,
    )
    .unwrap();
    let public = decoded(&b, "");
    assert_eq!(public.entries.len(), 2);
    assert_eq!(public.entries[0].metadata, ConnectionMetadata::default());
    assert!(public.entries.iter().all(|e| e.credential.is_none()));
    #[cfg(windows)]
    {
        let before = std::fs::read(&path).unwrap();
        let profiles = restored.list().unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();
        let p = restored.preview_import(payload(true)).unwrap();
        let selected = choices(&p, ImportAction::Update);
        let failure = restored.apply_import(p, selected);
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert!(failure.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(restored.list().unwrap(), profiles);
        let p = restored.preview_import(payload(true)).unwrap();
        let selected = choices(&p, ImportAction::Update);
        restored.apply_import(p, selected).unwrap();
    }
}
#[test]
fn public_export_never_unseals_and_ignores_captured_catalog() {
    struct NoUnseal;
    impl CredentialCipher for NoUnseal {
        fn seal(&self, _: &Secret, _: &CredentialContext) -> Result<Vec<u8>, ApiError> {
            Ok(vec![1])
        }
        fn unseal(&self, _: &[u8], _: &CredentialContext) -> Result<Secret, ApiError> {
            Err(ApiError::Auth)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = HostStore::load(dir.path().join("store.json"), Arc::new(NoUnseal)).unwrap();
    let profile = store
        .add(
            "Legacy".into(),
            "https://agent.example.test".into(),
            Secret::new("fictional".into()).unwrap(),
        )
        .unwrap();
    let metadata = payload(false).entries.remove(0).metadata;
    let catalog = crate::connections::Connections {
        generation: "fixture".into(),
        migration_warnings: vec![],
        connections: vec![crate::connections::Connection {
            id: "fixture".into(),
            name: "Fixture".into(),
            openab_pty_url: profile.base_url.clone(),
            metadata: metadata.clone(),
        }],
    };
    let selected = [ExportSelection {
        id: profile.id,
        expected_revision: profile.revision,
    }];
    let p = store
        .export_snapshot(&selected, false, Some(&catalog))
        .unwrap();
    assert_eq!(p.entries[0].metadata, ConnectionMetadata::default());
    assert!(p.entries[0].credential.is_none());
    assert!(store
        .export_snapshot(&selected, true, Some(&catalog))
        .is_err());
}
