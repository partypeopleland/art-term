#[path = "../../art-term-gui/src/instance_restore.rs"]
mod instance_restore;

use art_term_core::{
    instance_mcp::DesktopProfile,
    openab::{
        api::ApiError,
        profiles::{ExportSelection, HostStore, ImportAction, ImportSelection, TransferKind},
        secret::{CredentialCipher, CredentialContext, Secret},
    },
};
use serde_json::{json, Value};
use std::sync::Arc;

struct Cipher;
impl CredentialCipher for Cipher {
    fn seal(&self, secret: &Secret, _: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        Ok(secret.expose().as_bytes().to_vec())
    }
    fn unseal(&self, bytes: &[u8], _: &CredentialContext) -> Result<Secret, ApiError> {
        Secret::new(String::from_utf8(bytes.to_vec()).map_err(|_| ApiError::Storage)?)
    }
}
fn profile(id: &str, revision: u64, url: &str) -> DesktopProfile {
    DesktopProfile {
        id: id.into(),
        revision,
        name: "Fixture Screen".into(),
        mcp_url: url.into(),
        token_present: true,
    }
}
fn status(profile: &DesktopProfile, url: Option<&str>) -> Value {
    json!({"ok":true,"adapters":1,"targets":[{"id":profile.id,"revision":profile.revision,"generation":7}],"urls":[url]})
}
#[test]
fn actual_export_import_restores_both_directions_without_adopting_foreign_identity() {
    let root = tempfile::tempdir().unwrap();
    let source = HostStore::load(root.path().join("source.toml"), Arc::new(Cipher)).unwrap();
    let destination =
        HostStore::load(root.path().join("destination.toml"), Arc::new(Cipher)).unwrap();
    let original = source
        .save_desktop(
            None,
            None,
            "Fixture Screen".into(),
            "https://fixture.example.invalid/mcp".into(),
            Some(Secret::new("fictional-token".into()).unwrap()),
        )
        .unwrap();
    let payload = source
        .export_snapshot(
            &[ExportSelection {
                kind: TransferKind::Screen,
                id: original.id.clone(),
                expected_revision: original.revision,
            }],
            true,
            None,
        )
        .unwrap();
    let preview = destination.preview_import(payload).unwrap();
    let choices = preview
        .entries
        .iter()
        .map(|entry| ImportSelection {
            key: entry.key.clone(),
            action: ImportAction::Add,
            credential: None,
        })
        .collect();
    let imported = destination
        .apply_import(preview, choices)
        .unwrap()
        .desktops
        .remove(0);
    assert_ne!(imported.id, original.id);
    for (remote, local) in [(&original, &imported), (&imported, &original)] {
        let current = status(remote, Some(&remote.mcp_url));
        let resolved = instance_restore::resolve(&current, &[local.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(resolved.profile, *local);
        assert_eq!(
            resolved.broker_targets,
            current["targets"].as_array().unwrap().clone()
        );
    }
}
#[test]
fn legacy_exact_identity_restores_but_unknown_identity_requires_explicit_selection() {
    let local = profile("local", 3, "https://fixture.example.invalid/mcp");
    assert!(
        instance_restore::resolve(&status(&local, None), &[local.clone()])
            .unwrap()
            .is_some()
    );
    let foreign = profile("foreign", 3, &local.mcp_url);
    assert!(instance_restore::resolve(&status(&foreign, None), &[local]).is_err());
}
#[test]
fn stale_revision_is_rejected_instead_of_clearing_live_broker() {
    let local = profile("local", 3, "https://fixture.example.invalid/mcp");
    let remote = profile("local", 2, &local.mcp_url);
    assert!(instance_restore::resolve(&status(&remote, Some(&remote.mcp_url)), &[local]).is_err());
}

#[test]
fn restore_completion_rejects_cancel_close_changed_profile_and_changed_broker() {
    let local = profile("local", 3, "https://fixture.example.invalid/mcp");
    let original = status(&local, Some(&local.mcp_url));
    let resolve = || {
        instance_restore::resolve(&original, &[local.clone()])
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        resolve().confirm(&original, &local, 8, 8, false).unwrap(),
        local
    );
    assert!(resolve().confirm(&original, &local, 8, 9, false).is_err());
    assert!(resolve().confirm(&original, &local, 8, 8, true).is_err());
    for latest in [
        profile("local", 4, &local.mcp_url),
        profile("local", 3, "https://other.example.invalid/mcp"),
    ] {
        assert!(resolve().confirm(&original, &latest, 8, 8, false).is_err());
    }
    let mut changed = original.clone();
    changed["targets"][0]["generation"] = json!(8);
    assert!(resolve().confirm(&changed, &local, 8, 8, false).is_err());
    changed = original.clone();
    changed["targets"][0] = Value::Null;
    assert!(resolve().confirm(&changed, &local, 8, 8, false).is_err());
    changed = original.clone();
    changed["urls"][0] = json!("https://other.example.invalid/mcp");
    assert!(resolve().confirm(&changed, &local, 8, 8, false).is_err());
}
#[test]
fn unknown_ambiguous_mismatched_and_malformed_assignments_are_rejected() {
    let local = profile("local", 3, "https://fixture.example.invalid/mcp");
    let foreign = profile("foreign", 1, &local.mcp_url);
    let duplicate = profile("duplicate", 1, &local.mcp_url);
    assert!(instance_restore::resolve(&status(&foreign, Some(&foreign.mcp_url)), &[]).is_err());
    assert!(instance_restore::resolve(
        &status(&foreign, Some(&foreign.mcp_url)),
        &[local.clone(), duplicate]
    )
    .is_err());
    assert!(instance_restore::resolve(
        &status(&local, Some("https://other.example.invalid/mcp")),
        &[local.clone()]
    )
    .is_err());
    let valid = status(&local, Some(&local.mcp_url));
    for (key, value) in [
        ("adapters", json!(0)),
        ("targets", json!([null, valid["targets"][0]])),
        ("urls", json!([])),
        ("urls", json!([3])),
        (
            "targets",
            json!([{"id":"local","revision":3,"generation":0}]),
        ),
    ] {
        let mut broken = valid.clone();
        broken[key] = value;
        assert!(instance_restore::resolve(&broken, &[local.clone()]).is_err());
    }
    assert!(instance_restore::resolve(
        &json!({"ok":true,"adapters":1,"targets":[null],"urls":[null]}),
        &[local]
    )
    .unwrap()
    .is_none());
}
