//! Private opt-in runtime bootstrap (build-time seed).
//!
//! A private operator build may embed exactly one declared runtime-agent
//! credential — see `build.rs` (`ART_TERM_ENABLE_PRIVATE_RUNTIME_BOOTSTRAP=1`
//! + `ART_TERM_PRIVATE_RUNTIME_BOOTSTRAP_FILE`). On the first native start
//! after a valid `HostStore` load, the seed is imported once into the
//! platform-protected operator store (Windows DPAPI / Android vault)
//! and a secret-free once-marker records the decision, so a later user
//! deletion is never resurrected.
//!
//! Public builds embed `None` — this module is a no-op for them. The
//! seed never crosses IPC, is never returned in a DTO, and has no
//! `Debug` impl; note the private binary inherently contains the raw
//! credential bytes (extractable by someone who already holds the
//! private binary) — documented residual risk, root scans releases.

use art_term_core::{
    connections::Connections,
    openab::{api::ApiError, profiles::HostStore, secret::Secret},
};
use std::path::{Path, PathBuf};

/// Build-time embedded seed — `&'static str` slices inside the binary.
/// Deliberately no `Debug`: the credential must never be logged.
pub(crate) struct Seed {
    pub name: &'static str,
    pub origin: &'static str,
    pub credential: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/runtime_bootstrap.rs"));

/// Marker directory inside the protected native operator data dir
/// (same directory that already hosts `runtime-hosts-v1.json`).
fn marker_dir(dir: &Path) -> PathBuf {
    dir.join("runtime-bootstrap")
}

/// Deterministic marker name derived from safe configured values only —
/// the declared agent id and canonical origin, never the credential.
/// A changed origin or id produces a different marker, so a rebound
/// private build still performs its own one-time decision.
fn marker_path(dir: &Path, agent_id: &str, origin: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let key =
        Sha256::digest(format!("art-term-runtime-bootstrap-v1\0{agent_id}\0{origin}").as_bytes());
    marker_dir(dir).join(format!(
        "imported-{}",
        key[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

/// Bounded marker payload — compared byte-for-byte, never parsed.
fn marker_content(agent_id: &str, origin: &str) -> String {
    format!("{{\"v\":1,\"agent\":\"{agent_id}\",\"origin\":\"{origin}\"}}\n")
}

/// Reject symlinks/reparse points at every existing ancestor and the
/// final component — the same rule the Host store applies to its file.
fn check_path(path: &Path) -> Result<(), ApiError> {
    for part in path.ancestors() {
        match std::fs::symlink_metadata(part) {
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

/// One-time private import, invoked only after a valid `HostStore` load.
/// Fail-closed: any inconsistency — no loaded catalog, no matching
/// declared runtime agent, duplicate exact origins, tampered marker —
/// surfaces as `Err` so the caller leaves the runtime store failed.
/// Returns `Ok(())` when there is nothing to do.
pub(crate) fn initialize(
    store: &HostStore,
    dir: &Path,
    connections: Option<&Connections>,
) -> Result<(), ApiError> {
    let Some(seed) = PRIVATE_RUNTIME_BOOTSTRAP else {
        return Ok(());
    };
    // Re-validate against the actually loaded runtime_agents catalog —
    // an opt-in binary whose embedded origin is no longer declared
    // fails closed rather than importing an unbound credential.
    let connections = connections.ok_or(ApiError::Storage)?;
    let agent = connections
        .connections
        .iter()
        .find(|agent| {
            agent.openab_pty_url == seed.origin
                && agent.metadata.category == art_term_core::connections::Category::Agent
        })
        .ok_or(ApiError::Storage)?;
    let profiles = store.list()?;
    let matching = profiles
        .iter()
        .filter(|p| p.base_url == agent.openab_pty_url)
        .count();
    if matching > 1 {
        return Err(ApiError::Storage);
    }
    let marker = marker_path(dir, &agent.id, &agent.openab_pty_url);
    let expected = marker_content(&agent.id, &agent.openab_pty_url);
    match std::fs::symlink_metadata(&marker) {
        Ok(meta) => {
            check_path(&marker)?;
            if !meta.is_file() || meta.len() != expected.len() as u64 {
                return Err(ApiError::Storage);
            }
            let bytes = std::fs::read(&marker).map_err(|_| ApiError::Storage)?;
            if bytes != expected.as_bytes() {
                return Err(ApiError::Storage);
            }
            // One-time decision already recorded: a later user deletion
            // stays deleted — the marker is never used to re-import.
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ApiError::Storage),
    }
    // Interrupted imports recover through unique exact-origin
    // recognition: an already-saved origin is reused unchanged and never
    // overwritten; duplicate exact origins are ambiguous — fail closed.
    match matching {
        0 => {
            store.add(
                seed.name.to_owned(),
                agent.openab_pty_url.clone(),
                Secret::new(seed.credential.to_owned())?,
            )?;
        }
        1 => {}
        _ => return Err(ApiError::Storage),
    }
    // The marker is written only after the store mutation succeeded.
    check_path(&marker)?;
    std::fs::create_dir_all(marker_dir(dir)).map_err(|_| ApiError::Storage)?;
    check_path(&marker)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .map_err(|_| ApiError::StateChanged)?;
    use std::io::Write;
    file.write_all(expected.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| ApiError::Storage)?;
    Ok(())
}
