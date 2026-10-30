use super::*;
use crate::openab::{
    profiles::{ExportSelection, HostStore, ImportAction, ImportSelection},
    secret::{CredentialCipher, CredentialContext},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
// Independently authenticated with Argon2id and AES-GCM, outside this codec.
const LEGACY_V1_VECTOR: &str = r#"{"format":"art-term-connections","version":1,"mode":"encrypted","kdf":{"name":"argon2id","version":19,"memory_kib":65536,"iterations":3,"parallelism":4},"cipher":"aes-256-gcm","salt":"BwcHBwcHBwcHBwcHBwcHBw==","nonce":"CQkJCQkJCQkJCQkJ","ciphertext":"bHNskqUfv5U2gYtBZpPexx//ToWWpFgTEM+rKMC+6MedmZ6kK6KPzqaoNL1XEQsPsO/HoObEQ0cMgPghNnVZdkLryPQtlZJZTuZ6bj35wX5C9jGyW9/naj2OHP3mah8hV/2DChXMQ0q7iip1XqKeutgKUcdmxTuxzPUQT9eCjl+YEk8JgwzVKET0Cv/ek2BNzWtcHlVIosX/uM9LNwuR5VUalMWvPU94gEDQPzpP3Z5v+VxB+ONBX+ChxZ0qWpWrZ8f60v8Ew0Cv2BfpoNHyDk++mstEVKqJvXAQbl7F4K3/ys++r3/XvW7Rq87PbDTORibgBKqtuF5/eehTuDSoNEx+cigDmxED+f5N1317YZqqUdGLZ8Q3B6W5k4lvbuvrcQAP8ACiZEtGO43zNVVUfqZMQB73oNFnIvKqAHgarX4DwWctZB+ir5c7ohJSXlcyHFI8RvssxdLSYeLrO1BlBNA41FD/0ThGTE9Y/xY8"}"#;

// Stale my-cli ciphertext under an art-term envelope must never authenticate.
const WRONG_DOMAIN_V1_VECTOR: &str = r#"{"format":"art-term-connections","version":1,"mode":"encrypted","kdf":{"name":"argon2id","version":19,"memory_kib":65536,"iterations":3,"parallelism":4},"cipher":"aes-256-gcm","salt":"BwcHBwcHBwcHBwcHBwcHBw==","nonce":"CQkJCQkJCQkJCQkJ","ciphertext":"bHNskqUfv5U2gYtNbcqQ3xOgQMebpVMeAcWwL9zyt8fH3pqyML6Ig/K7Iq4YXAYOu+LWqv3DXwtF+blYIzZacg20yIhmq49USPYoSnrdyG8OrjHxCdX5bjGwDtbvQRMMTq3VCkeaX068wHZgEOqas8lKQJFi0Cqw2bcBBNCT3weYUwgQhxzVOESiUabGiyNayXhWC0MY4cWk/ctCJkfJ5R1dg8uoOUtIhG3FIyhRwZx0qRJBsLRdXue4yd0iFIjqO8e1n/gCyU6e3CTj7oKrFl340ZVED/ubp3ANfETQrPX/iIy9qzCIvRWAmdTPcSbUU2q4BOXgtlxzeu0f5jTzZV5kcjURgQRA7r9HmTh7LMqsUJTCHtxBCOX7moNnKavrMUsX/AamegoNO477dB5ZcucBNEHX8Y4ocvS8AHILqmsGgjEtOFSnsp0gpRxQElhtGkQ8TOor0NeRcJ3rRKlvJ6gM50FOI9vAzyiOwg=="}"#;

fn payload(private: bool) -> Payload {
    Payload {
        desktops: Vec::new(),
        entries: vec![Entry {
            name: "Fixture Agent".into(),
            origin: "https://agent.example.test".into(),
            metadata: ConnectionMetadata {
                display_name: None,
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
    assert_eq!(b, LEGACY_V1_VECTOR.as_bytes());
    let p = decoded(LEGACY_V1_VECTOR.as_bytes(), " 密碼 pass ");
    assert_eq!(
        p.entries[0].credential.as_ref().unwrap().expose(),
        "fictional-credential"
    );
    // A fixed public vector hash guards cross-platform format/KDF/AAD compatibility.
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&b));
    assert_eq!(
        digest,
        "cb4f9307d3590401d730ff637bd5daad25d0bc7053d4a7b49735b330a0d979ec"
    );
}
#[test]
fn legacy_v1_fixed_file_decodes_full_payload() {
    let Decoded::Encrypted(file) = decode(LEGACY_V1_VECTOR.as_bytes()).unwrap() else {
        panic!("legacy v1 fixture must require its password")
    };
    let p = unlock(file, Zeroizing::new(" 密碼 pass ".into())).unwrap();
    assert!(p.desktops.is_empty());
    assert_eq!(p.entries.len(), 1);
    let entry = &p.entries[0];
    assert_eq!(entry.name, "Fixture Agent");
    assert_eq!(entry.origin, "https://agent.example.test/");
    assert_eq!(entry.metadata, payload(false).entries[0].metadata);
    assert_eq!(
        entry.credential.as_ref().unwrap().expose(),
        "fictional-credential"
    );
}

#[test]
fn legacy_wrong_domain_ciphertext_is_rejected() {
    let Decoded::Encrypted(file) = decode(WRONG_DOMAIN_V1_VECTOR.as_bytes()).unwrap() else {
        panic!("wrong-domain fixture must remain an encrypted envelope")
    };
    assert!(matches!(
        unlock(file, Zeroizing::new(" 密碼 pass ".into())),
        Err(ApiError::Auth)
    ));
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
        let result = decode(&serde_json::to_vec(&v).unwrap());
        if field == "version" {
            let Decoded::Encrypted(f) = result.unwrap() else {
                panic!()
            };
            assert!(matches!(
                unlock(f, Zeroizing::new("fixture password".into())),
                Err(ApiError::Auth)
            ));
        } else {
            assert!(result.is_err());
        }
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
                kind: crate::openab::profiles::TransferKind::Host,
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
            kind: crate::openab::profiles::TransferKind::Host,
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
        kind: crate::openab::profiles::TransferKind::Host,
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

#[test]
fn screen_only_public_file_is_supported_without_a_secret() {
    let bytes = br#"{"format":"art-term-connections","version":2,"mode":"without-credentials","connections":[],"desktops":[{"name":"Fixture Screen","mcp_url":"https://screen.example.test/mcp"}]}"#;
    assert!(
        decode(bytes).is_ok(),
        "Screen-only portable settings should be importable"
    );
}

fn screen_payload(private: bool, mixed: bool) -> Payload {
    let mut p = payload(private);
    if !mixed {
        p.entries.clear();
    }
    p.desktops.push(ScreenEntry {
        name: "Fixture Screen".into(),
        mcp_url: "https://screen.example.test/mcp".into(),
        token: private.then(|| Secret::new("fictional-screen-token".into()).unwrap()),
    });
    p
}

#[test]
fn screen_codec_public_token_fence_and_v2_authenticated_versions() {
    let public = encode(screen_payload(false, true), None).unwrap();
    let text = String::from_utf8(public.clone()).unwrap();
    assert!(text.contains("\"version\":2"));
    assert!(!text.contains("\"token\"") && !text.contains("\"credential\""));
    let p = decoded(&public, "");
    assert_eq!(p.entries.len(), 1);
    assert!(p.desktops[0].token.is_none());
    for field in [
        "\"token\":null",
        "\"token\":\"private\"",
        "\"token_ciphertext\":[1]",
        "\"unknown\":true",
    ] {
        let bad = text.replace(
            "\"name\":\"Fixture Screen\"",
            &format!("\"name\":\"Fixture Screen\",{field}"),
        );
        assert!(decode(bad.as_bytes()).is_err());
    }
    for bad in [
        text.replace("\"version\":2", "\"version\":1"),
        text.replace("\"desktops\":[", "\"desktops\":null,\"extra\":["),
        text.replace("/mcp", "/mcp?secret=bad"),
    ] {
        assert!(decode(bad.as_bytes()).is_err());
    }
    let b = encrypt(
        screen_payload(true, true),
        Zeroizing::new("fixture password".into()),
        [3; 16],
        [4; 12],
    )
    .unwrap();
    let encrypted = String::from_utf8_lossy(&b);
    for plain in [
        "Fixture",
        "screen.example",
        "fictional-screen-token",
        "fictional-credential",
    ] {
        assert!(!encrypted.contains(plain));
    }
    let p = decoded(&b, "fixture password");
    assert_eq!(
        p.desktops[0].token.as_ref().unwrap().expose(),
        "fictional-screen-token"
    );
    // A valid alternate envelope version still cannot authenticate another version's domain.
    let mut envelope: serde_json::Value = serde_json::from_slice(&b).unwrap();
    envelope["version"] = 1.into();
    let Decoded::Encrypted(f) = decode(&serde_json::to_vec(&envelope).unwrap()).unwrap() else {
        panic!()
    };
    assert!(matches!(
        unlock(f, Zeroizing::new("fixture password".into())),
        Err(ApiError::Auth)
    ));
    // Correct v2 AEAD, malicious inner v1: both versions must agree after authentication.
    let salt = [5; 16];
    let nonce = [6; 12];
    let mut inner = Zeroizing::new(br#"{"format":"art-term-connections","version":1,"connections":[],"desktops":[{"name":"Screen","mcp_url":"https://screen.test/mcp","token":"fictional"}]}"#.to_vec());
    let key = derive("fixture password", &salt).unwrap();
    Aes256Gcm::new_from_slice(key.as_ref())
        .unwrap()
        .encrypt_in_place(
            Nonce::from_slice(&nonce),
            &aad(2, &salt, &nonce),
            &mut *inner,
        )
        .unwrap();
    assert!(matches!(
        unlock(
            LockedFile {
                version: 2,
                salt,
                nonce,
                ciphertext: inner.to_vec()
            },
            Zeroizing::new("fixture password".into())
        ),
        Err(ApiError::Protocol)
    ));
}

struct ProbeCipher {
    inner: Cipher,
    unseals: AtomicUsize,
    key: u8,
}
impl ProbeCipher {
    fn new(key: u8) -> Arc<Self> {
        Arc::new(Self {
            inner: Cipher {
                seals: AtomicUsize::new(0),
                fail_at: AtomicUsize::new(usize::MAX),
            },
            unseals: AtomicUsize::new(0),
            key,
        })
    }
}
impl CredentialCipher for ProbeCipher {
    fn seal(&self, secret: &Secret, context: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let mut result = vec![self.key];
        result.extend(self.inner.seal(secret, context)?);
        Ok(result)
    }
    fn unseal(&self, bytes: &[u8], context: &CredentialContext) -> Result<Secret, ApiError> {
        self.unseals.fetch_add(1, Ordering::SeqCst);
        if bytes.first() != Some(&self.key) {
            return Err(ApiError::Auth);
        }
        self.inner.unseal(&bytes[1..], context)
    }
}
#[test]
fn mixed_screen_import_atomic_reseal_public_export_and_full_source_cas() {
    use crate::openab::profiles::TransferKind;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let cipher = ProbeCipher::new(17);
    let store = HostStore::load(path.clone(), cipher.clone()).unwrap();
    // The second (Screen) seal fails after the Host candidate is built: no partial Host file.
    cipher.inner.fail_at.store(2, Ordering::SeqCst);
    let p = store.preview_import(screen_payload(true, true)).unwrap();
    let s = choices(&p, ImportAction::Add);
    assert!(store.apply_import(p, s).is_err());
    assert!(!path.exists());
    assert!(store.list().unwrap().is_empty());
    assert!(store.desktops().unwrap().is_empty());
    cipher.inner.fail_at.store(usize::MAX, Ordering::SeqCst);
    let p = store.preview_import(screen_payload(true, true)).unwrap();
    let s = choices(&p, ImportAction::Add);
    let result = store.apply_import(p, s).unwrap();
    assert_eq!(result.added, 2);
    let host = result.profiles[0].clone();
    let screen = result.desktops[0].clone();
    let selected = [
        ExportSelection {
            kind: TransferKind::Host,
            id: host.id.clone(),
            expected_revision: host.revision,
        },
        ExportSelection {
            kind: TransferKind::Screen,
            id: screen.id.clone(),
            expected_revision: screen.revision,
        },
    ];
    cipher.unseals.store(0, Ordering::SeqCst);
    let p = store.export_snapshot(&selected, false, None).unwrap();
    assert_eq!(cipher.unseals.load(Ordering::SeqCst), 0);
    assert!(p.desktops[0].token.is_none());
    let public = encode(p, None).unwrap();
    assert!(!String::from_utf8_lossy(&public).contains("fictional"));
    // Destination has a different cipher key and cannot read source ciphertext directly.
    let destination_cipher = ProbeCipher::new(42);
    let destination_path = dir.path().join("destination.toml");
    let destination =
        HostStore::load(destination_path.clone(), destination_cipher.clone()).unwrap();
    let p = store.export_snapshot(&selected, true, None).unwrap();
    let p = destination.preview_import(p).unwrap();
    let s = choices(&p, ImportAction::Add);
    let imported = destination.apply_import(p, s).unwrap();
    assert_ne!(imported.desktops[0].id, screen.id);
    assert_eq!(imported.desktops[0].revision, 1);
    let reloaded = HostStore::load(destination_path, destination_cipher).unwrap();
    assert_eq!(
        reloaded
            .desktop(&imported.desktops[0].id)
            .unwrap()
            .token
            .expose(),
        "fictional-screen-token"
    );
    assert_eq!(
        reloaded
            .snapshot(&imported.profiles[0].id)
            .unwrap()
            .credential
            .expose(),
        "fictional-credential"
    );
    // Screen-only mutation invalidates even a Host-only import preview.
    let p = store.preview_import(payload(true)).unwrap();
    let s = choices(&p, ImportAction::Update);
    let screen = store
        .save_desktop(
            Some(&screen.id),
            Some(screen.revision),
            "Renamed".into(),
            screen.mcp_url.clone(),
            None,
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        store.apply_import(p, s),
        Err(ApiError::StateChanged)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // Public updates preserve the local token but re-seal under incremented revision.
    let p = store.preview_import(screen_payload(false, false)).unwrap();
    let s = choices(&p, ImportAction::Update);
    let updated = store.apply_import(p, s).unwrap().desktops.remove(0);
    assert_eq!(updated.id, screen.id);
    assert_eq!(updated.revision, screen.revision + 1);
    assert_eq!(
        store.desktop(&updated.id).unwrap().token.expose(),
        "fictional-screen-token"
    );
    // Existing-file second-seal failure preserves both catalogs and exact file bytes.
    let before = std::fs::read(&path).unwrap();
    let hosts = store.list().unwrap();
    let screens = store.desktops().unwrap();
    cipher.inner.fail_at.store(
        cipher.inner.seals.load(Ordering::SeqCst) + 2,
        Ordering::SeqCst,
    );
    let p = store.preview_import(screen_payload(true, true)).unwrap();
    let s = choices(&p, ImportAction::Update);
    assert!(store.apply_import(p, s).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(store.list().unwrap(), hosts);
    assert_eq!(store.desktops().unwrap(), screens);
}

#[test]
fn public_screen_add_requires_token_and_duplicate_screen_matching_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let store = HostStore::load(dir.path().join("connections.toml"), ProbeCipher::new(9)).unwrap();
    let p = store.preview_import(screen_payload(false, false)).unwrap();
    let s = choices(&p, ImportAction::Add);
    assert!(store.apply_import(p, s).is_err());
    assert!(store.desktops().unwrap().is_empty());
    let p = store.preview_import(screen_payload(false, false)).unwrap();
    let mut s = choices(&p, ImportAction::Add);
    s[0].credential = Some(Secret::new("supplement-token".into()).unwrap());
    let first = store.apply_import(p, s).unwrap().desktops.remove(0);
    assert_eq!(
        store.desktop(&first.id).unwrap().token.expose(),
        "supplement-token"
    );
    store
        .save_desktop(
            None,
            None,
            "Other name".into(),
            first.mcp_url.clone(),
            Some(Secret::new("other-token".into()).unwrap()),
        )
        .unwrap();
    let p = store.preview_import(screen_payload(true, false)).unwrap();
    assert_eq!(p.entries[0].match_status, "ambiguous");
    let s = choices(&p, ImportAction::Update);
    assert!(store.apply_import(p, s).is_err());
    assert_eq!(store.desktops().unwrap().len(), 2);
    let mut p = screen_payload(false, false);
    p.desktops.push(ScreenEntry {
        name: "Duplicate".into(),
        mcp_url: "https://SCREEN.example.test:443/mcp".into(),
        token: None,
    });
    assert!(encode(p, None).is_err());
}

#[test]
fn legacy_v1_output_retains_original_codec_bytes() {
    use sha2::{Digest, Sha256};
    // Independently reproduced from HEAD e249f20 codec + fixture before implementation.
    let b = encrypt(
        payload(true),
        Zeroizing::new(" 密碼 pass ".into()),
        [7; 16],
        [9; 12],
    )
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&b)),
        "cb4f9307d3590401d730ff637bd5daad25d0bc7053d4a7b49735b330a0d979ec"
    );
    assert_eq!(
        decoded(&b, " 密碼 pass ").entries[0]
            .credential
            .as_ref()
            .unwrap()
            .expose(),
        "fictional-credential"
    );
}
