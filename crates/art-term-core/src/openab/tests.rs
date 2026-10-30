use super::{
    api::{AdminClient, ApiError},
    profiles::{canonical_origin, HostStore},
    secret::{CredentialCipher, CredentialContext, Secret},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Deliberately test-only reversible cipher, never compiled into production.
struct FixtureCipher;
impl CredentialCipher for FixtureCipher {
    fn seal(&self, secret: &Secret, ctx: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let mut tag = Sha256::new();
        tag.update(ctx.authenticated_bytes());
        tag.update(secret.expose());
        let mut bytes = tag.finalize().to_vec();
        bytes.extend(secret.expose().bytes().map(|b| b ^ 0xa5));
        Ok(bytes)
    }
    fn unseal(&self, bytes: &[u8], ctx: &CredentialContext) -> Result<Secret, ApiError> {
        if bytes.len() < 32 {
            return Err(ApiError::Storage);
        }
        let secret = Secret::new(
            String::from_utf8(bytes[32..].iter().map(|b| b ^ 0xa5).collect())
                .map_err(|_| ApiError::Storage)?,
        )?;
        if self.seal(&secret, ctx)? != bytes {
            return Err(ApiError::Storage);
        }
        Ok(secret)
    }
}
fn secret() -> Secret {
    Secret::new("fixture-operator-credential".into()).unwrap()
}
#[test]
fn hostile_origins_and_redaction() {
    for invalid in [
        "http://host",
        "wss://host",
        "https://user@host",
        "https://@host",
        "https://host?a=b",
        "https://host#x",
        "https://host/a",
        "https://host/a/..",
        "https://host/%2e/",
        "https://host\\",
        " https://host",
        "https://host\n",
    ] {
        assert!(canonical_origin(invalid).is_err(), "accepted {invalid:?}");
    }
    assert_eq!(
        canonical_origin("https://EXAMPLE.test:443").unwrap(),
        "https://example.test/"
    );
    assert_eq!(
        canonical_origin("https://[::1]:8443/").unwrap(),
        "https://[::1]:8443/"
    );
    assert_eq!(format!("{:?}", secret()), "Secret([REDACTED])");
}
#[test]
fn crud_reload_origin_and_revision_binding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.json");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let p = store
        .add("Fixture".into(), "https://example.test".into(), secret())
        .unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(secret().expose()));
    let reloaded = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(
        reloaded.snapshot(&p.id).unwrap().credential.expose(),
        secret().expose()
    );
    assert!(store
        .update(
            &p.id,
            p.revision,
            "Fixture".into(),
            "https://elsewhere.test".into(),
            None
        )
        .is_err());
    let next = store
        .update(
            &p.id,
            p.revision,
            "Renamed".into(),
            "https://EXAMPLE.test:443/".into(),
            None,
        )
        .unwrap();
    assert_eq!(next.revision, 2);
    assert!(!store.is_current(&p).unwrap());
    assert!(store.delete(&p.id, 1).is_err());
    assert!(store
        .update(&p.id, 1, "Late".into(), p.base_url.clone(), Some(secret()))
        .is_err());
    let changed = store
        .update(
            &p.id,
            2,
            "Fresh".into(),
            "https://elsewhere.test".into(),
            Some(secret()),
        )
        .unwrap();
    store.delete(&p.id, changed.revision).unwrap();
    assert!(store.snapshot(&p.id).is_err());
    assert!(HostStore::load(path, Arc::new(FixtureCipher))
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
}
#[test]
fn metadata_restart_legacy_credential_and_revision_continuity() {
    use crate::{connections::*, runtime_preview::*};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.json");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let p = store
        .add(
            "Keep local name".into(),
            "https://example.test/".into(),
            secret(),
        )
        .unwrap();
    // Saving default metadata retains the original credential context and ciphertext.
    let legacy = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&legacy).contains("metadata"));
    let reopened = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(
        reopened.snapshot(&p.id).unwrap().credential.expose(),
        secret().expose()
    );
    let metadata = ConnectionMetadata {
        display_name: None,
        category: Category::Agent,
        desktop_transport: Some("runtime-mcp".into()),
        coding_cli: Some(CodingCli {
            id: "assistant".into(),
            name: "Assistant".into(),
            command: "another-cli".into(),
            args: vec!["--model".into(), "some model".into()],
        }),
    };
    let next = store
        .update_with_metadata(
            &p.id,
            1,
            p.name.clone(),
            p.base_url.clone(),
            None,
            Some(metadata.clone()),
        )
        .unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(next.id, p.id);
    assert!(!store.is_current(&p).unwrap());
    assert!(store
        .update_with_metadata(
            &p.id,
            1,
            p.name.clone(),
            p.base_url.clone(),
            None,
            Some(metadata.clone())
        )
        .is_err());
    let restarted = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let snapshot = restarted.snapshot(&p.id).unwrap();
    assert_eq!(snapshot.profile, next);
    assert_eq!(snapshot.credential.expose(), secret().expose());
    let catalog = crate::connections::parse(crate::connections::tests::UNIFIED).unwrap();
    let resolved = resolve_metadata(
        snapshot.profile.metadata.as_ref(),
        Some(&catalog),
        &p.base_url,
    );
    assert_eq!(resolved, metadata); // catalog Host overridden by persisted Agent.
    let without_catalog = resolve_metadata(snapshot.profile.metadata.as_ref(), None, &p.base_url);
    let frozen = mapping_from_metadata(&without_catalog);
    assert!(matches!(frozen.route, Some(DesktopRoute::Native)));
    assert_eq!(
        next.metadata
            .as_ref()
            .unwrap()
            .coding_cli
            .as_ref()
            .unwrap()
            .command,
        "another-cli"
    );
    let disabled = restarted
        .update_with_metadata(
            &p.id,
            2,
            p.name.clone(),
            p.base_url.clone(),
            None,
            Some(ConnectionMetadata::default()),
        )
        .unwrap();
    assert!(mapping_from_metadata(&resolve_metadata(
        disabled.metadata.as_ref(),
        Some(&catalog),
        &p.base_url
    ))
    .route
    .is_none());
    assert!(matches!(frozen.route, Some(DesktopRoute::Native))); // snapshot stays frozen.
    let raw = std::fs::read(&path).unwrap();
    let mut invalid = metadata;
    invalid
        .coding_cli
        .as_mut()
        .unwrap()
        .args
        .push("bad\narg".into());
    assert!(restarted
        .update_with_metadata(&p.id, 3, p.name, p.base_url, None, Some(invalid))
        .is_err());
    assert_eq!(std::fs::read(path).unwrap(), raw);
    assert_eq!(restarted.list().unwrap(), vec![disabled]);
}
#[test]
fn corruption_and_wrong_context_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.json");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    store
        .add("Fixture".into(), "https://example.test".into(), secret())
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    for field in ["revision", "openab_pty_url", "id"] {
        let mut envelope: toml::Value =
            toml::from_str(std::str::from_utf8(&original).unwrap()).unwrap();
        envelope["connections"][0][field] = match field {
            "revision" => toml::Value::Integer(2),
            "openab_pty_url" => toml::Value::String("https://elsewhere.test/".into()),
            _ => toml::Value::String(uuid::Uuid::new_v4().to_string()),
        };
        std::fs::write(&path, toml::to_string(&envelope).unwrap()).unwrap();
        assert!(HostStore::load(path.clone(), Arc::new(FixtureCipher)).is_err());
    }
    std::fs::write(&path, b"{broken").unwrap();
    assert!(HostStore::load(path, Arc::new(FixtureCipher)).is_err());
}
#[cfg(windows)]
#[test]
fn failed_windows_replace_preserves_disk_and_memory() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.json");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let p = store
        .add("Fixture".into(), "https://example.test".into(), secret())
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(store
        .update_with_metadata(
            &p.id,
            p.revision,
            "Changed".into(),
            p.base_url.clone(),
            None,
            Some(crate::connections::ConnectionMetadata {
                display_name: None,
                category: crate::connections::Category::Agent,
                ..Default::default()
            })
        )
        .is_err());
    assert_eq!(store.list().unwrap(), vec![p]);
    drop(held);
    assert_eq!(std::fs::read(path).unwrap(), original);
}
#[test]
fn concurrent_mutations_cannot_accept_two_old_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(HostStore::load(dir.path().join("hosts.json"), Arc::new(FixtureCipher)).unwrap());
    let p = store
        .add("Fixture".into(), "https://example.test".into(), secret())
        .unwrap();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let p = p.clone();
        workers.push(std::thread::spawn(move || {
            store
                .update(&p.id, 1, "Updated".into(), p.base_url, None)
                .is_ok()
        }));
    }
    assert_eq!(
        workers
            .into_iter()
            .map(|w| usize::from(w.join().unwrap()))
            .sum::<usize>(),
        1
    );
}

async fn fixture(
    status: u16,
    body: String,
    extra: &str,
) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let extra = extra.to_string();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut block = [0; 1024];
        while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut block).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&block[..n]);
            assert!(bytes.len() < 16384);
        }
        let response=format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",body.len());
        let _ = socket.write_all(response.as_bytes()).await;
        String::from_utf8(bytes).unwrap()
    });
    (origin, task)
}
#[tokio::test]
async fn http_status_redirect_and_body_bounds() {
    let api = AdminClient::fixture();
    for (status, expected) in [
        (401, ApiError::Auth),
        (404, ApiError::SessionMissing),
        (409, ApiError::StateChanged),
        (429, ApiError::Throttled),
        (502, ApiError::GatewayUnavailable),
        (503, ApiError::Capacity),
        (504, ApiError::GatewayUnavailable),
        (500, ApiError::Protocol),
        (302, ApiError::Protocol),
    ] {
        let (origin, task) = fixture(
            status,
            "never-return-this-sensitive-upstream-body".into(),
            "Location: http://127.0.0.1:9/stolen\r\n",
        )
        .await;
        assert_eq!(
            api.fixture_request(&origin, &secret()).await.unwrap_err(),
            expected
        );
        let request = task.await.unwrap();
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fixture-operator-credential"));
    }
    let (origin, task) = fixture(200, "x".repeat(1024 * 1024 + 1), "").await;
    assert_eq!(
        api.fixture_request(&origin, &secret()).await.unwrap_err(),
        ApiError::Protocol
    );
    // Server write may fail when bounded client closes early; no detached fixture remains.
    let _ = task.await;
}

struct TlsIdentity {
    ca_der: Vec<u8>,
    chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
}
fn tls_identity(hostname: &str, expired: bool) -> TlsIdentity {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    ca_params.not_after = rcgen::date_time_ymd(2100, 1, 1);
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let mut leaf = CertificateParams::new(vec![hostname.into()]).unwrap();
    leaf.not_before = rcgen::date_time_ymd(2000, 1, 1);
    leaf.not_after = rcgen::date_time_ymd(if expired { 2001 } else { 2100 }, 1, 1);
    leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = leaf.signed_by(&leaf_key, &ca, &ca_key).unwrap();
    TlsIdentity {
        ca_der: ca.der().to_vec(),
        chain: vec![leaf.der().clone(), ca.der().clone()],
        key: rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
    }
}
/// Owns one loopback listener/task. Returns decrypted application bytes only;
/// a rejected handshake cannot populate these bytes with Authorization.
async fn tls_server(
    identity: TlsIdentity,
    response: Vec<u8>,
) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}/", listener.local_addr().unwrap());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(identity.chain, identity.key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let task = tokio::spawn(async move {
        let operation = async move {
            let (socket, _) = listener.accept().await.unwrap();
            let Ok(mut socket) = acceptor.accept(socket).await else {
                return Vec::new();
            };
            let mut request = Vec::new();
            let mut block = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let Ok(n) = socket.read(&mut block).await else {
                    return request;
                };
                if n == 0 {
                    return request;
                };
                request.extend_from_slice(&block[..n]);
                assert!(request.len() < 16384);
            }
            let _ = socket.write_all(&response).await;
            let _ = socket.shutdown().await;
            request
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), operation)
            .await
            .expect("owned TLS fixture deadline")
    });
    (origin, task)
}
fn tls_response(status: u16, body: &[u8], extra: &str) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}
fn tls_snapshot(origin: String) -> super::profiles::HostSnapshot {
    super::profiles::HostSnapshot {
        profile: super::profiles::HostProfile {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: "TLS Fixture".into(),
            base_url: origin,
            credential_present: true,
            metadata: None,
        },
        credential: secret(),
    }
}
#[tokio::test]
async fn native_tls_validates_ca_hostname_expiry_before_authorization() {
    let identity = tls_identity("localhost", false);
    let client = AdminClient::tls_fixture(&identity.ca_der);
    let body = br#"{"sessions":[],"draining":false}"#;
    let (origin, task) = tls_server(identity, tls_response(200, body, "")).await;
    let list = client
        .list(&tls_snapshot(origin.replace("127.0.0.1", "localhost")))
        .await
        .unwrap();
    assert!(list.sessions.is_empty());
    let request = String::from_utf8(task.await.unwrap()).unwrap();
    assert!(request.starts_with("GET /admin/sessions HTTP/1.1"));
    assert!(request
        .to_lowercase()
        .contains("authorization: bearer fixture-operator-credential"));

    for (host, expired, trust) in [
        ("localhost", false, false),
        ("wrong.invalid", false, true),
        ("localhost", true, true),
    ] {
        let identity = tls_identity(host, expired);
        let client = if trust {
            AdminClient::tls_fixture(&identity.ca_der)
        } else {
            AdminClient::new().unwrap()
        };
        let (origin, task) = tls_server(identity, tls_response(200, body, "")).await;
        let error = client
            .list(&tls_snapshot(origin.replace("127.0.0.1", "localhost")))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ApiError::Tls,
            "host={host} expired={expired} trust={trust}"
        );
        assert_eq!(error.to_string(), "TLS verification failed");
        assert!(
            task.await.unwrap().is_empty(),
            "Authorization crossed rejected TLS handshake"
        );
    }
}
#[tokio::test]
async fn authenticated_tls_redirect_never_contacts_target_and_errors_are_sanitized() {
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let identity = tls_identity("127.0.0.1", false);
    let client = AdminClient::tls_fixture(&identity.ca_der);
    let location = format!(
        "Location: https://{}/stolen\r\n",
        sink.local_addr().unwrap()
    );
    let (origin, task) = tls_server(
        identity,
        tls_response(302, b"private-upstream-body", &location),
    )
    .await;
    let error = client.list(&tls_snapshot(origin)).await.unwrap_err();
    assert_eq!(error, ApiError::Protocol);
    assert!(!task.await.unwrap().is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), sink.accept())
            .await
            .is_err()
    );
    assert!(!format!("{error:?} {error}").contains("private-upstream-body"));
    assert!(!format!("{error:?} {error}").contains(secret().expose()));
}
#[tokio::test]
async fn authenticated_tls_streamed_body_bound() {
    let identity = tls_identity("127.0.0.1", false);
    let client = AdminClient::tls_fixture(&identity.ca_der);
    // Chunked has no Content-Length, exercising the incremental body cap.
    let body = vec![b'x'; 1024 * 1024 + 1];
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let (origin, task) = tls_server(identity, response).await;
    assert_eq!(
        client.list(&tls_snapshot(origin)).await.unwrap_err(),
        ApiError::Protocol
    );
    assert!(!task.await.unwrap().is_empty());
}

fn wss_grant() -> super::api::SessionGrant {
    super::api::SessionGrant {
        session: "fixture".into(),
        generation: 2,
        token: Secret::new("a".repeat(64)).unwrap(),
        token_expires_in_secs: 100,
    }
}
fn wss_notice(offset: u64) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({"v":1,"type":"attach-notice","stream_offset":offset,"ephemeral_workspace":true,"teardown_best_effort":true}).to_string())
}
#[tokio::test]
async fn preview_explicit_create_conflict_disarms_cleanup_and_never_attaches() {
    use super::{
        screenshot::{self, Mode, ShellOwnership},
        session::SessionError,
    };
    #[derive(Default)]
    struct Owned(std::sync::atomic::AtomicUsize);
    impl ShellOwnership for Owned {
        fn mark(&self, _: &str) -> Result<(), SessionError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn unmark(&self, _: &str) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let identity = tls_identity("127.0.0.1", false);
    let api = AdminClient::tls_fixture(&identity.ca_der);
    let (origin, server) = tls_server(identity, tls_response(409, b"{}", "")).await;
    let ownership = Arc::new(Owned::default());
    let (_cancel, rx) = tokio::sync::watch::channel(0u64);
    let result = screenshot::run(
        &api,
        tls_snapshot(origin),
        Mode::Screenshot,
        ownership.clone(),
        rx,
        |_, _, _, _| async { panic!("conflicting Shell must not attach") },
    )
    .await;
    assert!(matches!(result, Err(SessionError::StateChanged)));
    assert_eq!(ownership.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    let request = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(request.starts_with("POST /admin/sessions HTTP/1.1"));
}
#[tokio::test]
async fn preview_cancel_during_attach_deletes_only_created_owned_shell() {
    use super::{
        screenshot::{self, Mode, ShellOwnership},
        session::SessionError,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[derive(Default)]
    struct Owned(std::sync::Mutex<std::collections::HashSet<String>>);
    impl ShellOwnership for Owned {
        fn mark(&self, name: &str) -> Result<(), SessionError> {
            self.0.lock().unwrap().insert(name.into());
            Ok(())
        }
        fn unmark(&self, name: &str) {
            self.0.lock().unwrap().remove(name);
        }
    }
    let identity = tls_identity("127.0.0.1", false);
    let api = AdminClient::tls_fixture(&identity.ca_der);
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(identity.chain, identity.key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = tls_snapshot(format!("https://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let mut created = String::new();
        for index in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = acceptor.accept(socket).await.unwrap();
            let mut bytes = Vec::new();
            let mut block = [0; 2048];
            let (end, length) = loop {
                let n = socket.read(&mut block).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let n = socket.read(&mut block).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
            }
            let request = std::str::from_utf8(&bytes).unwrap();
            let body = if index == 0 {
                assert!(request.starts_with("POST /admin/sessions HTTP/1.1"));
                let value: serde_json::Value = serde_json::from_slice(&bytes[end..]).unwrap();
                assert_eq!(value.as_object().unwrap().len(), 1);
                created = value["name"].as_str().unwrap().to_owned();
                assert!(created.starts_with("mcshot-"));
                serde_json::to_vec(&serde_json::json!({"session":created,"generation":1,"token":"a".repeat(64),"token_expires_in_secs":3600})).unwrap()
            } else {
                assert!(request.starts_with(&format!("DELETE /admin/sessions/{created} HTTP/1.1")));
                b"{}".to_vec()
            };
            socket
                .write_all(&tls_response(200, &body, ""))
                .await
                .unwrap();
            let _ = socket.shutdown().await;
        }
    });
    let ownership = Arc::new(Owned::default());
    let (cancel, rx) = tokio::sync::watch::channel(0u64);
    let (entered, wait) = tokio::sync::oneshot::channel();
    let operation = screenshot::run(
        &api,
        host,
        Mode::Screenshot,
        ownership.clone(),
        rx,
        |_, _, _, _| async {
            let _ = entered.send(());
            std::future::pending().await
        },
    );
    let cancel_operation = async {
        wait.await.unwrap();
        cancel.send_replace(1);
    };
    let (result, _) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(operation, cancel_operation)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(SessionError::Cancelled)));
    server.await.unwrap();
    assert!(ownership.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn preview_burst_larger_than_terminal_queue_is_acknowledged_without_slow_consumer() {
    use super::session::{OutputEvent, SessionHandle};
    use tokio_tungstenite::tungstenite::Message;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let bytes = vec![b'x'; 2 * 1024 * 1024];
    let mut frames = vec![wss_notice(0)];
    frames.extend(bytes.chunks(32768).map(|b| Message::Binary(b.to_vec())));
    let (origin, server) = wss_fixture(identity, frames, 0).await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
        .await
        .unwrap();
    handle.preview_flow();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut received = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while received.len() < bytes.len() {
            let poll = handle.poll().unwrap();
            assert!(poll.ended.is_none(), "{:?}", poll.ended);
            for event in poll.events {
                if let OutputEvent::Bytes {
                    bytes, start, end, ..
                } = event
                {
                    assert_eq!(start, received.len() as u64);
                    assert_eq!(end, start + bytes.len() as u64);
                    received.extend(bytes);
                }
            }
            if poll.batch_id != 0 {
                handle.ack(poll.batch_id, 1).unwrap();
            }
            if received.len() < bytes.len() {
                handle.wait_output().await;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(received, bytes);
    handle.shutdown().await;
    let input = server.await.unwrap();
    assert!(!input.iter().any(|m| matches!(m, Message::Binary(_))));
}

#[tokio::test]
async fn native_same_retained_token_resumes_from_ack_without_admin_renew() {
    use super::session::{OutputEvent, SessionHandle};
    use tokio_tungstenite::tungstenite::Message;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let (origin, server) = wss_fixture(
        identity,
        vec![wss_notice(0), Message::Binary(b"abc".to_vec())],
        0,
    )
    .await;
    let mut handle =
        SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
            .await
            .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let poll = handle.poll().unwrap();
            let received = poll
                .events
                .iter()
                .any(|e| matches!(e, OutputEvent::Bytes { .. }));
            if poll.batch_id != 0 {
                handle.ack(poll.batch_id, 1).unwrap();
            }
            if received {
                break;
            }
            handle.wait_output().await;
        }
    })
    .await
    .unwrap();
    let cursor = handle.rendered_cursor().unwrap();
    assert_eq!(cursor, 3);
    let mut binding = handle.binding().clone();
    let token = handle.take_token().unwrap();
    handle.shutdown().await;
    server.await.unwrap();
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let (origin, server) = wss_fixture(
        identity,
        vec![wss_notice(3), Message::Binary(b"def".to_vec())],
        cursor,
    )
    .await;
    binding.profile.base_url = origin;
    let handle = SessionHandle::tls_fixture_attach_token(binding, token, cursor, 2, &ca)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let poll = handle.poll().unwrap();
            let bytes: Vec<_> = poll
                .events
                .iter()
                .filter_map(|e| {
                    if let OutputEvent::Bytes {
                        bytes, historical, ..
                    } = e
                    {
                        assert!(!historical);
                        Some(bytes.clone())
                    } else {
                        None
                    }
                })
                .flatten()
                .collect();
            if poll.batch_id != 0 {
                handle.ack(poll.batch_id, 2).unwrap();
            }
            if !bytes.is_empty() {
                assert_eq!(bytes, b"def");
                break;
            }
            handle.wait_output().await;
        }
    })
    .await
    .unwrap();
    handle.shutdown().await;
    server.await.unwrap();
}
async fn wss_fixture(
    identity: TlsIdentity,
    messages: Vec<tokio_tungstenite::tungstenite::Message>,
    expected_since: u64,
) -> (
    String,
    tokio::task::JoinHandle<Vec<tokio_tungstenite::tungstenite::Message>>,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}/", listener.local_addr().unwrap());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(identity.chain, identity.key)
    .unwrap();
    let task = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(8),async move {
            let (socket,_)=listener.accept().await.unwrap();let socket=tokio_rustls::TlsAcceptor::from(Arc::new(config)).accept(socket).await.unwrap();
            let mut socket=tokio_tungstenite::accept_hdr_async(socket,move|request:&Request,response:Response| {
                assert_eq!(request.uri().to_string(),format!("/pty/fixture?since={expected_since}"));
                assert_eq!(request.headers().get("authorization").unwrap().to_str().unwrap(),format!("Bearer {}","a".repeat(64)));
                assert!(!request.headers().contains_key("sec-websocket-protocol"));Ok(response)
            }).await.unwrap();
            for message in messages {if socket.send(message).await.is_err(){return Vec::new();}}
            let mut input=Vec::new();
            while let Some(Ok(message))=socket.next().await {
                let detach=matches!(&message,tokio_tungstenite::tungstenite::Message::Text(text) if text.contains("detach"));
                input.push(message);if detach {let _=socket.close(None).await;break;}
            }input
        }).await.expect("owned WSS fixture deadline")
    });
    (origin, task)
}
#[tokio::test]
async fn native_wss_historical_gap_ack_and_live_input_are_byte_exact() {
    use super::session::{OutputEvent, SessionError, SessionHandle};
    use tokio_tungstenite::tungstenite::Message;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let replay = b"\x1b[6n\xe4\xbd".to_vec();
    let (origin, task) = wss_fixture(
        identity,
        vec![
            Message::Text("{\"v\":1,\"type\":\"gap\",\"bytes_dropped\":2}".into()),
            wss_notice(9),
            Message::Binary(replay.clone()),
            Message::Binary(vec![0xa0, b'!']),
        ],
        2,
    )
    .await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 2, 7, &ca)
        .await
        .unwrap();
    let mut all: Vec<u8> = Vec::new();
    let mut historical: Vec<u8> = Vec::new();
    let mut live: Vec<u8> = Vec::new();
    let mut saw_gap = false;
    for _ in 0..100 {
        let poll = handle.poll().unwrap();
        for event in &poll.events {
            match event {
                OutputEvent::Gap {
                    bytes_dropped,
                    resume_offset,
                } => {
                    assert_eq!((*bytes_dropped, *resume_offset), (2, 4));
                    saw_gap = true
                }
                OutputEvent::Bytes {
                    bytes,
                    historical: is_history,
                    ..
                } => {
                    all.extend(bytes);
                    if *is_history {
                        historical.extend(bytes)
                    } else {
                        live.extend(bytes)
                    }
                }
                _ => {}
            }
        }
        if poll.batch_id != 0 {
            assert!(matches!(handle.poll(), Err(SessionError::Busy)));
            assert!(handle.ack(poll.batch_id, 6).is_err());
            handle.ack(poll.batch_id, 7).unwrap();
        }
        if all.len() == replay.len() + 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(saw_gap);
    assert_eq!(all, [replay, vec![0xa0, b'!']].concat());
    assert_eq!(historical.len(), 5);
    assert_eq!(live.len(), 3);
    assert_eq!(handle.rendered_cursor().unwrap(), 12);
    // Renderer sees historical CPR and must suppress its automatic reply. Only
    // deliberate live input is admitted; no backend-generated replay replies.
    handle.write(b"live-only").unwrap();
    handle.resize(40, 120).unwrap();
    handle.ping().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    handle.shutdown().await;
    let input = task.await.unwrap();
    let binary: Vec<_> = input
        .iter()
        .filter_map(|m| {
            if let Message::Binary(b) = m {
                Some(b.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(binary, vec![b"live-only".to_vec()]);
    assert!(input
        .iter()
        .any(|m| matches!(m,Message::Text(t) if t.contains("resize"))));
    assert!(input
        .iter()
        .any(|m| matches!(m,Message::Text(t) if t.contains("detach"))));
}
#[tokio::test]
async fn native_wss_official_notice_needs_no_custom_identity() {
    use super::session::SessionHandle;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let (origin, task) = wss_fixture(identity, vec![wss_notice(0)], 0).await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
        .await
        .unwrap();
    handle.shutdown().await;
    let _ = task.await;
}
#[tokio::test]
async fn native_wss_tls_rejection_and_redirect_never_leak_token() {
    use super::session::{SessionError, SessionHandle};
    for (hostname, expired, trust) in [
        ("wrong.invalid", false, true),
        ("127.0.0.1", true, true),
        ("127.0.0.1", false, false),
    ] {
        let identity = tls_identity(hostname, expired);
        let ca = identity.ca_der.clone();
        let (origin, task) = tls_server(identity, tls_response(200, b"unused", "")).await;
        let result = if trust {
            SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca).await
        } else {
            SessionHandle::attach(tls_snapshot(origin), wss_grant(), 0, 1).await
        };
        assert!(matches!(result, Err(SessionError::Tls)));
        assert!(task.await.unwrap().is_empty());
    }
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let location = format!("Location: wss://{}/stolen\r\n", sink.local_addr().unwrap());
    let (origin, task) = tls_server(
        identity,
        tls_response(302, b"private-upstream-body", &location),
    )
    .await;
    let result =
        SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca).await;
    assert!(matches!(result, Err(SessionError::Protocol)));
    let request = String::from_utf8(task.await.unwrap()).unwrap();
    assert!(request.starts_with("GET /pty/fixture?since=0 HTTP/1.1"));
    assert!(!request.contains("token="));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), sink.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn native_wss_operator_close_is_local_and_never_replays_input() {
    use super::session::{SessionError, SessionHandle};
    use tokio_tungstenite::tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    };
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let (origin, task) = wss_fixture(
        identity,
        vec![
            wss_notice(0),
            Message::Close(Some(CloseFrame {
                code: CloseCode::Library(4008),
                reason: "fixture".into(),
            })),
        ],
        0,
    )
    .await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
        .await
        .unwrap();
    let mut ended = None;
    for _ in 0..100 {
        let poll = handle.poll().unwrap();
        if poll.batch_id != 0 {
            handle.ack(poll.batch_id, 1).unwrap();
        }
        if poll.ended.is_some() {
            ended = poll.ended;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(ended, Some(SessionError::OperatorKilled));
    assert!(!handle.is_live().unwrap()); // Actual ended WSS owner, no poll probe.
    assert!(handle.write(b"must-not-replay").is_err());
    handle.shutdown().await;
    let messages = task.await.unwrap();
    assert!(!messages.iter().any(|m| matches!(m, Message::Binary(_))));
}

#[tokio::test]
async fn recovery_same_token_attach_is_three_attempts_and_cancellation_releases_flight() {
    use super::session::{Recovery, SessionError};
    let recovery = Recovery::default();
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let result = recovery
        .resume(rx, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err(SessionError::Transport) }
        })
        .await;
    assert!(matches!(result, Err(SessionError::RetryExhausted)));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    for _ in 0..2 {
        let (_cancel, rx) = tokio::sync::watch::channel(true);
        let result = recovery
            .resume(rx, || async {
                panic!("cancelled recovery must not attach")
            })
            .await;
        assert!(matches!(result, Err(SessionError::Cancelled)));
    }
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let result = recovery
        .resume(rx, || async { Err(SessionError::Auth) })
        .await;
    assert!(matches!(result, Err(SessionError::Auth)));
}
#[tokio::test]
async fn native_https_official_list_does_not_require_custom_fields() {
    let identity = tls_identity("127.0.0.1", false);
    let client = AdminClient::tls_fixture(&identity.ca_der);
    let body=serde_json::to_vec(&serde_json::json!({"sessions":[{"name":"fixture","generation":1,"alive":true,"attached":false,"bytes_written":0,"tier":"tier1","teardown_best_effort":true,"absolute_ttl_best_effort":true}],"draining":false})).unwrap();
    let (origin, task) = tls_server(identity, tls_response(200, &body, "")).await;
    let list = client.list(&tls_snapshot(origin)).await.unwrap();
    assert_eq!(list.sessions[0].name, "fixture");
    let request = String::from_utf8(task.await.unwrap()).unwrap();
    assert!(request.starts_with("GET /admin/sessions HTTP/1.1"));
    assert!(!request.contains("POST"));
}
#[tokio::test]
async fn native_wss_oversize_frame_is_bounded_and_closes_without_input_replay() {
    use super::session::{SessionError, SessionHandle};
    use tokio_tungstenite::tungstenite::Message;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let (origin, task) = wss_fixture(
        identity,
        vec![wss_notice(0), Message::Binary(vec![42; 256 * 1024 + 1])],
        0,
    )
    .await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
        .await
        .unwrap();
    let mut ended = None;
    for _ in 0..100 {
        let poll = handle.poll().unwrap();
        if poll.batch_id != 0 {
            handle.ack(poll.batch_id, 1).unwrap();
        }
        if poll.ended.is_some() {
            ended = poll.ended;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(ended, Some(SessionError::Capacity));
    handle.shutdown().await;
    let messages = task.await.unwrap();
    assert!(!messages.iter().any(|m| matches!(m, Message::Binary(_))));
}

#[tokio::test]
async fn explicit_official_renew_is_one_post_without_custom_conditions() {
    let identity = tls_identity("127.0.0.1", false);
    let client = AdminClient::tls_fixture(&identity.ca_der);
    let body=serde_json::to_vec(&serde_json::json!({"session":"fixture","generation":3,"token":"a".repeat(64),"token_expires_in_secs":3600})).unwrap();
    let (origin, task) = tls_server(identity, tls_response(200, &body, "")).await;
    let grant = client
        .renew(&tls_snapshot(origin), "fixture")
        .await
        .unwrap();
    assert_eq!(grant.generation, 3);
    let request = String::from_utf8(task.await.unwrap()).unwrap();
    assert!(request.starts_with("POST /admin/sessions/fixture/renew HTTP/1.1"));
    assert!(!request.contains("expected_"));
}
#[test]
fn single_toml_empty_sources_and_no_json_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    std::fs::write(
        dir.path().join("runtime-hosts-v1.json"),
        b"private old store ignored",
    )
    .unwrap();
    assert!(HostStore::load(path.clone(), Arc::new(FixtureCipher))
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
    assert!(!path.exists());
    for source in [
        "",
        " \n# comment\n",
        "connections = []",
        "schema_version = 4\nconnections = []",
        "schema_version = 5\nconnections = []",
    ] {
        std::fs::write(&path, source).unwrap();
        assert!(HostStore::load(path.clone(), Arc::new(FixtureCipher))
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }
    for source in ["schema_version = 5", "password = 'secret'", "{invalid"] {
        std::fs::write(&path, source).unwrap();
        assert!(HostStore::load(path.clone(), Arc::new(FixtureCipher)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }
    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(HostStore::load(path, Arc::new(FixtureCipher)).is_err());
}
fn legacy_toml() -> &'static str {
    "schema_version = 4\n[[connections]]\nid = 'legacy-host'\nname = 'Legacy'\nopenab_pty_url = 'https://example.test/'\ncategory = 'host'\n"
}
#[test]
fn single_toml_missing_credential_edit_export_import_and_fill() {
    use super::profiles::{ExportSelection, ImportAction, ImportSelection};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    std::fs::write(&path, legacy_toml()).unwrap();
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let p = store.list().unwrap().remove(0);
    assert!(!p.credential_present);
    assert!(store.snapshot(&p.id).is_err());
    let p = store
        .update(&p.id, p.revision, "Edited".into(), p.base_url.clone(), None)
        .unwrap();
    assert!(!p.credential_present);
    let payload = store
        .export_snapshot(
            &[ExportSelection {
                kind: crate::openab::profiles::TransferKind::Host,
                id: p.id.clone(),
                expected_revision: p.revision,
            }],
            true,
            None,
        )
        .unwrap();
    assert!(payload.entries[0].credential.is_none());
    let encrypted_payload = store
        .export_snapshot(
            &[ExportSelection {
                kind: crate::openab::profiles::TransferKind::Host,
                id: p.id.clone(),
                expected_revision: p.revision,
            }],
            true,
            None,
        )
        .unwrap();
    assert!(super::transfer::encode(
        encrypted_payload,
        Some(zeroize::Zeroizing::new("fixture-password".into()))
    )
    .is_err());
    let preview = store.preview_import(payload).unwrap();
    let key = preview.entries[0].key.clone();
    let result = store
        .apply_import(
            preview,
            vec![ImportSelection {
                key,
                action: ImportAction::Update,
                credential: None,
            }],
        )
        .unwrap();
    let p = &result.profiles[0];
    assert!(!p.credential_present);
    let p = store
        .update(
            &p.id,
            p.revision,
            p.name.clone(),
            p.base_url.clone(),
            Some(secret()),
        )
        .unwrap();
    assert_eq!(p.id, "legacy-host");
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(bytes.contains("schema_version = 5"));
    assert!(bytes.contains("credential_ciphertext"));
    assert!(!bytes.contains(secret().expose()));
    let reopened = HostStore::load(path, Arc::new(FixtureCipher)).unwrap();
    assert_eq!(reopened.list().unwrap(), vec![p.clone()]);
    assert_eq!(
        reopened.snapshot(&p.id).unwrap().credential.expose(),
        secret().expose()
    );
}
#[test]
fn single_toml_external_changes_refuse_stale_operations() {
    for replacement in [
        None,
        Some(""),
        Some("# changed\n"),
        Some("connections = []"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.toml");
        let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
        let p = store
            .add("Fixture".into(), "https://example.test/".into(), secret())
            .unwrap();
        match replacement {
            Some(text) => std::fs::write(&path, text).unwrap(),
            None => std::fs::remove_file(&path).unwrap(),
        }
        assert!(matches!(store.snapshot(&p.id), Err(ApiError::StateChanged)));
        assert!(matches!(store.is_current(&p), Err(ApiError::StateChanged)));
        assert!(matches!(
            store.delete(&p.id, p.revision),
            Err(ApiError::StateChanged)
        ));
        assert!(matches!(
            store.add("Fresh".into(), "https://fresh.test/".into(), secret()),
            Err(ApiError::StateChanged)
        ));
        assert!(HostStore::load(path.clone(), Arc::new(FixtureCipher))
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        match replacement {
            Some(text) => assert_eq!(std::fs::read_to_string(path).unwrap(), text),
            None => assert!(!path.exists()),
        }
    }
}

#[tokio::test]
async fn native_wss_large_history_replay_waits_for_renderer_without_losing_bytes() {
    large_output_waits_for_renderer(true).await;
}
#[tokio::test]
async fn native_wss_live_resume_burst_waits_for_renderer_without_losing_bytes() {
    large_output_waits_for_renderer(false).await;
}
async fn large_output_waits_for_renderer(historic: bool) {
    use super::session::SessionHandle;
    use tokio_tungstenite::tungstenite::Message;
    let identity = tls_identity("127.0.0.1", false);
    let ca = identity.ca_der.clone();
    let chunk = vec![b'x'; 64 * 1024];
    let total = chunk.len() * 64;
    let mut messages = vec![wss_notice(if historic { total as u64 } else { 0 })];
    messages.extend((0..64).map(|_| Message::Binary(chunk.clone())));
    let (origin, task) = wss_fixture(identity, messages, 0).await;
    let handle = SessionHandle::tls_fixture_attach(tls_snapshot(origin), wss_grant(), 0, 1, &ca)
        .await
        .unwrap();
    // The WebView is not mounted yet. Replay must wait instead of ending.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    handle.write(b"explicit-live-input").unwrap();
    let mut received = 0;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while received < total {
            let batch = handle.poll().unwrap();
            assert!(
                batch.ended.is_none(),
                "history replay ended: {:?}",
                batch.ended
            );
            for event in batch.events {
                if let super::session::OutputEvent::Bytes {
                    bytes,
                    start,
                    end,
                    historical,
                } = event
                {
                    assert_eq!(historical, historic);
                    assert_eq!(start, received as u64);
                    assert!(bytes.iter().all(|b| *b == b'x'));
                    received += bytes.len();
                    assert_eq!(end, received as u64);
                }
            }
            if batch.batch_id != 0 {
                handle.ack(batch.batch_id, batch.local_generation).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("bounded replay must finish");
    assert_eq!(received, total);
    assert_eq!(handle.rendered_cursor().unwrap(), total as u64);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    handle.shutdown().await;
    let input = task.await.unwrap();
    let sent: Vec<_> = input
        .iter()
        .filter_map(|m| {
            if let Message::Binary(b) = m {
                Some(b.as_slice())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        sent,
        vec![b"explicit-live-input".as_slice()],
        "only current explicit input may be sent while output is paused"
    );
}

#[test]
fn newer_connection_document_preserves_inactive_desktops_through_host_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let host = store
        .add("Existing".into(), "https://example.test/".into(), secret())
        .unwrap();
    drop(store);
    let mut document: toml::Value =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let desktops: toml::Value = toml::from_str("desktops = [{ id = 'saved-desktop', revision = 7, name = 'Saved', mcpUrl = 'https://desktop.test/mcp', tokenPresent = true, token_ciphertext = [77, 88], extra = { keep = 'untouched' } }]").unwrap();
    document["schema_version"] = toml::Value::Integer(6);
    document
        .as_table_mut()
        .unwrap()
        .insert("desktops".into(), desktops["desktops"].clone());
    std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(store.list().unwrap(), vec![host.clone()]);
    assert_eq!(
        store.snapshot(&host.id).unwrap().credential.expose(),
        secret().expose()
    );
    let edited = store
        .update(
            &host.id,
            host.revision,
            "Renamed".into(),
            host.base_url.clone(),
            None,
        )
        .unwrap();
    let added = store
        .add("Second".into(), "https://second.test/".into(), secret())
        .unwrap();
    store.delete(&edited.id, edited.revision).unwrap();
    store.delete(&added.id, added.revision).unwrap();
    let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["schema_version"].as_integer(), Some(6));
    assert_eq!(saved["desktops"], desktops["desktops"]);
    assert!(HostStore::load(path, Arc::new(FixtureCipher))
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
}

#[test]
fn standalone_desktop_rejects_plaintext_or_malformed_schema6_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    std::fs::write(&path, "schema_version = 6\nconnections = []\n[[desktops]]\nid = 'broken'\ntoken = 'must-not-be-accepted'\n").unwrap();
    let store = HostStore::load(path, Arc::new(FixtureCipher)).unwrap();
    assert!(
        store.desktops().is_err(),
        "active desktop catalog must reject unvalidated plaintext/unknown records"
    );
}

#[test]
fn standalone_desktop_roundtrip_backup_and_host_mutations_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let host = store
        .add("Host".into(), "https://host.example/".into(), secret())
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    let desktop = store
        .save_desktop(
            None,
            None,
            "Screen A".into(),
            "http://127.0.0.1:8765/mcp".into(),
            Some(Secret::new("desktop-private-token".into()).unwrap()),
        )
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("connections.pre-desktops.toml")).unwrap(),
        original
    );
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("schema_version = 6"));
    assert!(!raw.contains("desktop-private-token"));
    assert_eq!(
        store.snapshot(&host.id).unwrap().credential.expose(),
        secret().expose()
    );
    let updated = store
        .save_desktop(
            Some(&desktop.id),
            Some(desktop.revision),
            "Renamed".into(),
            desktop.mcp_url.clone(),
            None,
        )
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        store.desktop(&updated.id).unwrap().token.expose(),
        "desktop-private-token"
    );
    assert!(store
        .save_desktop(
            Some(&updated.id),
            Some(1),
            "Stale".into(),
            updated.mcp_url.clone(),
            None
        )
        .is_err());
    assert!(store
        .save_desktop(
            Some(&updated.id),
            Some(2),
            "Changed URL".into(),
            "http://127.0.0.1:9999/mcp".into(),
            None
        )
        .is_err());
    store.delete(&host.id, host.revision).unwrap();
    drop(store);
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(store.desktops().unwrap(), [updated.clone()]);
    assert_eq!(
        store.desktop(&updated.id).unwrap().token.expose(),
        "desktop-private-token"
    );
    assert!(store.list().unwrap().is_empty());
    store.delete_desktop(&updated.id, updated.revision).unwrap();
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("schema_version = 5"));
    assert_eq!(
        std::fs::read(dir.path().join("connections.pre-desktops.toml")).unwrap(),
        original
    );
}

#[test]
fn standalone_desktop_token_context_and_host_import_preserve_the_catalog() {
    use super::profiles::{ImportAction, ImportSelection};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let desktop = store
        .save_desktop(
            None,
            None,
            "Screen".into(),
            "http://localhost:9876/mcp".into(),
            Some(secret()),
        )
        .unwrap();
    let preview = store
        .preview_import(super::transfer::Payload {
            desktops: Vec::new(),
            entries: vec![super::transfer::Entry {
                name: "Imported host".into(),
                origin: "https://import.example/".into(),
                metadata: Default::default(),
                credential: Some(secret()),
            }],
        })
        .unwrap();
    let key = preview.entries[0].key.clone();
    store
        .apply_import(
            preview,
            vec![ImportSelection {
                key,
                action: ImportAction::Add,
                credential: None,
            }],
        )
        .unwrap();
    assert_eq!(store.desktops().unwrap(), [desktop.clone()]);
    assert_eq!(
        store.desktop(&desktop.id).unwrap().token.expose(),
        secret().expose()
    );
    drop(store);
    let mut document: toml::Value =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["desktops"][0]["revision"] = toml::Value::Integer(2);
    std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
    let tampered = HostStore::load(path, Arc::new(FixtureCipher)).unwrap();
    assert!(
        tampered.desktop(&desktop.id).is_err(),
        "desktop revision must authenticate the token"
    );
    assert_eq!(
        tampered.list().unwrap().len(),
        1,
        "invalid desktop data must not destroy Hosts"
    );
}

#[test]
fn connection_display_name_accepts_chinese_and_bounds_characters() {
    let metadata: crate::connections::ConnectionMetadata =
        serde_json::from_value(serde_json::json!({
            "category": "agent", "display_name": "開發助手"
        }))
        .expect("Chinese display names must be accepted independently of CLI commands");
    metadata.validate().unwrap();
    let long: crate::connections::ConnectionMetadata = serde_json::from_value(serde_json::json!({
        "category": "agent", "display_name": "字".repeat(21)
    }))
    .unwrap();
    assert!(long.validate().is_err());
    let boundary: crate::connections::ConnectionMetadata =
        serde_json::from_value(serde_json::json!({
            "category": "agent", "display_name": "字".repeat(20)
        }))
        .unwrap();
    boundary.validate().unwrap();
}

#[test]
fn connection_order_is_persistent_bounded_and_keeps_credentials() {
    use super::profiles::ConnectionKind;
    use crate::connections::{Category, ConnectionMetadata};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let a = store
        .add_with_metadata(
            "A".into(),
            "https://a.test".into(),
            secret(),
            Some(ConnectionMetadata {
                category: Category::Agent,
                ..Default::default()
            }),
        )
        .unwrap();
    let h = store
        .add("Host".into(), "https://h.test".into(), secret())
        .unwrap();
    let b = store
        .add_with_metadata(
            "B".into(),
            "https://b.test".into(),
            secret(),
            Some(ConnectionMetadata {
                category: Category::Agent,
                ..Default::default()
            }),
        )
        .unwrap();
    let before = [a.id.clone(), b.id.clone()];
    let after = [b.id.clone(), a.id.clone()];
    store
        .reorder_connections(ConnectionKind::Agent, &before, &after)
        .expect("same-kind order must save");
    assert_eq!(store.list().unwrap(), vec![b.clone(), h.clone(), a.clone()]);
    assert_eq!(
        store.snapshot(&a.id).unwrap().credential.expose(),
        secret().expose()
    );
    let loaded = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(
        loaded.list().unwrap(),
        vec![b.clone(), h.clone(), a.clone()]
    );
    assert!(store
        .reorder_connections(ConnectionKind::Agent, &before, &after)
        .is_err());
    for invalid in [
        vec![a.id.clone(), h.id.clone()],
        vec![a.id.clone()],
        vec![a.id.clone(), a.id.clone()],
    ] {
        assert!(store
            .reorder_connections(ConnectionKind::Agent, &after, &invalid)
            .is_err());
    }
    assert_eq!(store.list().unwrap(), vec![b, h, a]);
}

#[test]
fn connection_alias_roundtrip_defaults_and_transfer_order() {
    use super::profiles::{ExportSelection, ImportAction, ImportSelection, TransferKind};
    use crate::connections::ConnectionMetadata;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let a = store
        .add_with_metadata(
            "A".into(),
            "https://a.test".into(),
            secret(),
            Some(ConnectionMetadata {
                display_name: Some("  開發助手  ".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    assert_eq!(
        a.metadata.as_ref().unwrap().display_name.as_deref(),
        Some("開發助手")
    );
    let b = store
        .add("B".into(), "https://b.test".into(), secret())
        .unwrap();
    let reloaded = HostStore::load(path, Arc::new(FixtureCipher)).unwrap();
    assert_eq!(reloaded.list().unwrap(), vec![a.clone(), b.clone()]);
    let payload = store
        .export_snapshot(
            &[
                ExportSelection {
                    kind: TransferKind::Host,
                    id: b.id.clone(),
                    expected_revision: b.revision,
                },
                ExportSelection {
                    kind: TransferKind::Host,
                    id: a.id.clone(),
                    expected_revision: a.revision,
                },
            ],
            true,
            None,
        )
        .unwrap();
    assert_eq!(
        payload
            .entries
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
    let other = HostStore::load(dir.path().join("other.toml"), Arc::new(FixtureCipher)).unwrap();
    let preview = other.preview_import(payload).unwrap();
    let choices = preview
        .entries
        .iter()
        .map(|e| ImportSelection {
            key: e.key.clone(),
            action: ImportAction::Add,
            credential: None,
        })
        .collect();
    other.apply_import(preview, choices).unwrap();
    assert_eq!(
        other
            .list()
            .unwrap()
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
    assert_eq!(
        other.list().unwrap()[0]
            .metadata
            .as_ref()
            .unwrap()
            .display_name
            .as_deref(),
        Some("開發助手")
    );
    let cleared = store
        .update_with_metadata(
            &a.id,
            a.revision,
            a.name.clone(),
            a.base_url.clone(),
            None,
            Some(ConnectionMetadata {
                display_name: Some("   ".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    assert!(cleared.metadata.unwrap().display_name.is_none());
}

#[test]
fn screen_order_preserves_tokens_and_export_order() {
    use super::profiles::{ConnectionKind, ExportSelection, TransferKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let store = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    let host = store
        .add("Host".into(), "https://host.test".into(), secret())
        .unwrap();
    let a = store
        .save_desktop(
            None,
            None,
            "A".into(),
            "https://screen-a.test/mcp".into(),
            Some(secret()),
        )
        .unwrap();
    let b = store
        .save_desktop(
            None,
            None,
            "B".into(),
            "https://screen-b.test/mcp".into(),
            Some(secret()),
        )
        .unwrap();
    let before = [a.id.clone(), b.id.clone()];
    let after = [b.id.clone(), a.id.clone()];
    store
        .reorder_connections(ConnectionKind::Screen, &before, &after)
        .unwrap();
    assert_eq!(store.list().unwrap(), vec![host]);
    assert_eq!(store.desktops().unwrap(), vec![b.clone(), a.clone()]);
    assert_eq!(
        store.desktop(&a.id).unwrap().token.expose(),
        secret().expose()
    );
    let loaded = HostStore::load(path.clone(), Arc::new(FixtureCipher)).unwrap();
    assert_eq!(loaded.desktops().unwrap(), vec![b.clone(), a.clone()]);
    let payload = store
        .export_snapshot(
            &[
                ExportSelection {
                    kind: TransferKind::Screen,
                    id: a.id.clone(),
                    expected_revision: a.revision,
                },
                ExportSelection {
                    kind: TransferKind::Screen,
                    id: b.id.clone(),
                    expected_revision: b.revision,
                },
            ],
            true,
            None,
        )
        .unwrap();
    assert_eq!(
        payload
            .desktops
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "A"]
    );
    assert!(store
        .reorder_connections(ConnectionKind::Screen, &before, &after)
        .is_err());
    std::fs::write(path, "connections = []").unwrap();
    assert!(store
        .reorder_connections(ConnectionKind::Screen, &after, &before)
        .is_err());
}
