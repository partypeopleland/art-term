use super::*;
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashSet, sync::Mutex, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
#[derive(Default)]
struct Owned(Mutex<HashSet<String>>, tokio::sync::Notify);
impl ShellOwnership for Owned {
    fn mark(&self, n: &str) -> Result<(), SessionError> {
        self.0.lock().unwrap().insert(n.into());
        Ok(())
    }
    fn unmark(&self, n: &str) {
        self.0.lock().unwrap().remove(n);
        self.1.notify_one();
    }
}
struct Fixture {
    api: AdminClient,
    host: HostSnapshot,
    ca: Vec<u8>,
    task: tokio::task::JoinHandle<Vec<String>>,
}
async fn fixture(mode: &'static str) -> Fixture {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = params.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .signed_by(&key, &ca, &ca_key)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone(), ca.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ca = ca.der().to_vec();
    let api = AdminClient::tls_fixture(&ca);
    let host = HostSnapshot {
        profile: super::super::profiles::HostProfile {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: "History fixture".into(),
            base_url: format!("https://{}/", listener.local_addr().unwrap()),
            credential_present: true,
            metadata: None,
        },
        credential: Secret::new("f".repeat(64)).unwrap(),
    };
    let task = tokio::spawn(async move {
        let operation = async move {
            let mut transcript = Vec::new();
            let mut name = String::new();
            for step in 0..if mode == "attachWait" {
                2
            } else if mode == "conflict" {
                1
            } else {
                3
            } {
                let (s, _) = listener.accept().await.unwrap();
                let mut s = acceptor.accept(s).await.unwrap();
                if step == 1 && mode != "attachWait" {
                    let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
                    ws.send(Message::Text("{\"v\":1,\"type\":\"attach-notice\",\"stream_offset\":0,\"ephemeral_workspace\":true,\"teardown_best_effort\":true}".into())).await.unwrap();
                    let mut nonce = String::new();
                    while let Some(Ok(msg)) = ws.next().await {
                        match msg {
                            Message::Binary(b) => {
                                let text = String::from_utf8(b).unwrap();
                                transcript.push(text.clone());
                                if nonce.is_empty() {
                                    nonce = text.split_whitespace().last().unwrap().to_string();
                                    let mut ready = serde_json::json!({"v":1,"nonce":nonce,"requestId":0,"instance":"b".repeat(32),"type":"ready","key":"a".repeat(24)});
                                    if let Some(kind) = mode.strip_prefix("ready-") {
                                        ready["type"] = serde_json::json!("error");
                                        ready["kind"] = serde_json::json!(kind);
                                    }
                                    let output = if mode == "prelimit" {
                                        format!("{}\r\n", "x".repeat(16385))
                                    } else {
                                        format!("$ echo bootstrap\r\n{ready}\r\n")
                                    };
                                    for part in output.as_bytes().chunks(7) {
                                        ws.send(Message::Binary(part.to_vec())).await.unwrap();
                                    }
                                } else {
                                    let req: serde_json::Value =
                                        serde_json::from_str(&text).unwrap();
                                    let op = req["op"].as_str().unwrap();
                                    let mut reply = if op == "viewport" || op == "older" {
                                        serde_json::json!({"v":1,"nonce":nonce,"requestId":req["requestId"],"instance":"b".repeat(32),"type":"page","op":op,"epoch":1,"columns":80,"rows":1,"rowCount":100,"start":99,"end":100,"hasOlder":true,"records":[{"id":100,"text":"藍😀","styles":[[0,3,255,-1,1]]}]})
                                    } else {
                                        serde_json::json!({"v":1,"nonce":nonce,"requestId":req["requestId"],"instance":"b".repeat(32),"type":"ack","op":op,"epoch":req["epoch"]})
                                    };
                                    if mode == "partialViewport" {
                                        reply["rows"] = serde_json::json!(24);
                                        reply["rowCount"] = serde_json::json!(1003);
                                        reply["start"] = serde_json::json!(1000);
                                        reply["end"] = serde_json::json!(1003);
                                        reply["records"] = serde_json::json!((1001..=1003).map(|id|
                                            serde_json::json!({"id":id,"text":"短畫面","styles":[]})).collect::<Vec<_>>());
                                    }
                                    if mode == "viewport300"
                                        || mode == "rowCount6001"
                                        || mode == "rows501"
                                    {
                                        let row_count =
                                            if mode == "rowCount6001" { 6001 } else { 6000 };
                                        let rows = if mode == "rows501" { 501 } else { 300 };
                                        let start = row_count - rows;
                                        reply["rows"] = serde_json::json!(rows);
                                        reply["rowCount"] = serde_json::json!(row_count);
                                        reply["start"] = serde_json::json!(start);
                                        reply["end"] = serde_json::json!(row_count);
                                        reply["records"]=serde_json::json!((start..row_count).map(|i|serde_json::json!({"id":i+1,"text":"藍😀","styles":[[0,3,255,-1,1]]})).collect::<Vec<_>>());
                                    }
                                    if op == "older" {
                                        let end = req["before"].as_u64().unwrap();
                                        let start =
                                            end.saturating_sub(req["limit"].as_u64().unwrap());
                                        reply["start"] = serde_json::json!(start);
                                        reply["end"] = serde_json::json!(end);
                                        reply["hasOlder"] = serde_json::json!(start > 0);
                                        reply["records"]=serde_json::json!((start..end).map(|i|serde_json::json!({"id":i+1,"text":"藍😀","styles":[[0,3,255,-1,1]]})).collect::<Vec<_>>());
                                    }
                                    if mode == "requestId" {
                                        reply["requestId"] = serde_json::json!(999);
                                    }
                                    if mode == "wait" {
                                        continue;
                                    }
                                    if mode == "oversize" {
                                        reply["records"][0]["text"] =
                                            serde_json::json!("x".repeat(256 * 1024));
                                    }
                                    if mode == "nonce" {
                                        reply["nonce"] = serde_json::json!("c".repeat(32));
                                    }
                                    if mode == "style" {
                                        reply["records"][0]["styles"][0][1] = serde_json::json!(2);
                                    }
                                    if mode == "gap" {
                                        ws.send(Message::Text(
                                            "{\"v\":1,\"type\":\"gap\",\"bytes_dropped\":1}".into(),
                                        ))
                                        .await
                                        .unwrap();
                                    }
                                    let output = format!("{reply}\r\n");
                                    for part in output.as_bytes().chunks(if mode == "oversize" {
                                        4096
                                    } else {
                                        17
                                    }) {
                                        if ws.send(Message::Binary(part.to_vec())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                            Message::Text(t) if t.contains("detach") => break,
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    continue;
                }
                let mut bytes = Vec::new();
                let mut block = [0; 2048];
                let (end, len) = loop {
                    let n = s.read(&mut block).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&block[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let h = std::str::from_utf8(&bytes[..i]).unwrap();
                        let l = h
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (i + 4, l);
                    }
                };
                while bytes.len() < end + len {
                    let n = s.read(&mut block).await.unwrap();
                    bytes.extend_from_slice(&block[..n]);
                }
                let head = String::from_utf8_lossy(&bytes[..end]);
                transcript.push(head.lines().next().unwrap().into());
                let body = if step == 0 {
                    let req: serde_json::Value =
                        serde_json::from_slice(&bytes[end..end + len]).unwrap();
                    name = req["name"].as_str().unwrap().into();
                    assert!(name.starts_with("mcshot-"));
                    serde_json::json!({"session":name,"generation":1,"token":"a".repeat(64),"token_expires_in_secs":3600}).to_string()
                } else {
                    assert!(head.starts_with(&format!("DELETE /admin/sessions/{name} ")));
                    "{}".into()
                };
                let status = if mode == "conflict" {
                    409
                } else if step == 2 && mode == "deleteFail" {
                    500
                } else {
                    200
                };
                let response=format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                s.write_all(response.as_bytes()).await.unwrap();
                let _ = s.shutdown().await;
            }
            transcript
        };
        tokio::time::timeout(Duration::from_secs(12), operation)
            .await
            .expect("bounded owned history fixture")
    });
    Fixture {
        api,
        host,
        ca,
        task,
    }
}
async fn open(f: &Fixture, owned: Arc<Owned>) -> Result<HistoryChannel, SessionError> {
    open_version(f, owned, HistoryVersion::Legacy).await
}
async fn open_version(f: &Fixture, owned: Arc<Owned>, version: HistoryVersion) -> Result<HistoryChannel, SessionError> {
    let ca = f.ca.clone();
    HistoryChannel::open(
        &f.api,
        HostSnapshot {
            profile: f.host.profile.clone(),
            credential: Secret::new(f.host.credential.expose().into()).unwrap(),
        },
        &"a".repeat(24),
        version,
        owned,
        move |binding, token, since, generation| async move {
            SessionHandle::tls_fixture_attach_token(binding, token, since, generation, &ca).await
        },
    )
    .await
}
#[tokio::test]
async fn public_history_channel_reads_fragmented_page_ack_and_deletes_only_helper() {
    let f = fixture("valid").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    let reply = ch.request(HistoryRequest::Viewport).await.unwrap();
    match reply {
        HistoryReply::Page {
            records,
            instance,
            epoch,
            ..
        } => {
            assert_eq!(records[0].text, "藍😀");
            assert_eq!(records[0].styles[0], [0, 3, 255, -1, 1]);
            assert!(matches!(
                ch.request(HistoryRequest::Ack { instance, epoch })
                    .await
                    .unwrap(),
                HistoryReply::Ack { .. }
            ));
        }
        _ => panic!("expected page"),
    };
    ch.close().await;
    let t = f.task.await.unwrap();
    assert_eq!(t.iter().filter(|s| s.starts_with("exec \"$HOME/.local/share/art-term/openab-pty/node\" ")).count(), 1);
    assert_eq!(t.iter().filter(|s| s.starts_with("DELETE ")).count(), 1);
    assert!(owned.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn public_history_channel_rejects_nonce_gap_surrogate_and_oversize_without_replay() {
    for mode in ["nonce", "gap", "style", "oversize", "requestId"] {
        let f = fixture(mode).await;
        let owned = Arc::new(Owned::default());
        let mut ch = open(&f, owned.clone()).await.unwrap();
        assert!(
            ch.request(HistoryRequest::Viewport).await.is_err(),
            "accepted {mode}"
        );
        let t = f.task.await.unwrap();
        assert_eq!(
            t.iter()
                .filter(|s| s.contains("\"op\":\"viewport\""))
                .count(),
            1
        );
        assert!(owned.0.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn public_history_channel_rejects_pre_ready_cap_and_deletes_helper() {
    let f = fixture("prelimit").await;
    let owned = Arc::new(Owned::default());
    assert!(matches!(
        open(&f, owned.clone()).await,
        Err(SessionError::Capacity)
    ));
    f.task.await.unwrap();
    assert!(owned.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn public_history_channel_cancel_does_not_replay_and_drop_cleans_helper() {
    let f = fixture("wait").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    assert!(tokio::time::timeout(
        Duration::from_millis(100),
        ch.request(HistoryRequest::Viewport)
    )
    .await
    .is_err());
    assert!(matches!(
        ch.request(HistoryRequest::Viewport).await,
        Err(SessionError::StateChanged)
    ));
    drop(ch);
    let t = f.task.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), owned.1.notified())
        .await
        .unwrap();
    assert_eq!(
        t.iter()
            .filter(|s| s.contains("\"op\":\"viewport\""))
            .count(),
        1
    );
    assert!(owned.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn public_history_channel_cleanup_failure_keeps_owned_registration() {
    let f = fixture("deleteFail").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    ch.close().await;
    f.task.await.unwrap();
    assert_eq!(owned.0.lock().unwrap().len(), 1);
    drop(ch);
}
#[tokio::test]
async fn public_history_channel_foreign_instance_is_rejected_before_input() {
    let f = fixture("valid").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    assert!(matches!(
        ch.request(HistoryRequest::Keep {
            instance: "c".repeat(32),
            epoch: 1
        })
        .await,
        Err(SessionError::Protocol)
    ));
    ch.close().await;
    let t = f.task.await.unwrap();
    assert!(!t.iter().any(|s| s.contains("\"op\"")));
}
#[test]
fn public_history_names_are_strict_routing_candidates() {
    assert_eq!(
        key_from_name(&format!("th2-{}", "a".repeat(24))),
        Some("a".repeat(24))
    );
    assert_eq!(
        key_from_name(&format!("th-{}", "a".repeat(24))),
        Some("a".repeat(24))
    );
    for n in [
        "th-work",
        "th2-work",
        "th2-AAAAAAAAAAAAAAAAAAAAAAAA",
        "th-AAAAAAAAAAAAAAAAAAAAAAAA",
        "th-aaaaaaaaaaaaaaaaaaaaaaaa;",
        "mcshot-aaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(key_from_name(n).is_none());
    }
}

#[tokio::test]
async fn public_history_channel_routes_new_work_to_private_paged_helper() {
    let f = fixture("valid").await;
    let owned = Arc::new(Owned::default());
    let version = HistoryVersion::from_name(&format!("th2-{}", "a".repeat(24))).unwrap();
    let mut channel = open_version(&f, owned.clone(), version).await.unwrap();
    assert!(matches!(channel.request(HistoryRequest::Viewport).await.unwrap(), HistoryReply::Page { .. }));
    channel.close().await;
    let transcript = f.task.await.unwrap();
    assert!(transcript.iter().any(|line| line.starts_with("exec ")
        && line.contains("openab-pty/node") && line.contains("openab-pty/tmux-v2/tmux-history.cjs")));
    assert!(owned.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn public_history_channel_accepts_short_viewport_above_existing_history() {
    let f = fixture("partialViewport").await;
    let mut channel = open_version(&f, Arc::new(Owned::default()), HistoryVersion::Paged).await.unwrap();
    let reply = channel.request(HistoryRequest::Viewport).await.unwrap();
    assert!(matches!(reply, HistoryReply::Page { start: 1000, end: 1003, row_count: 1003, .. }));
    channel.close().await;
    f.task.await.unwrap();
}

#[tokio::test]
async fn public_history_channel_older_is_before_exclusive_and_close_has_no_epoch() {
    let f = fixture("valid").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    let instance = ch.instance().to_string();
    match ch
        .request(HistoryRequest::Older {
            instance,
            epoch: 1,
            before: 99,
            limit: 10,
        })
        .await
        .unwrap()
    {
        HistoryReply::Page {
            start,
            end,
            records,
            ..
        } => {
            assert_eq!((start, end), (89, 99));
            assert_eq!(records.first().unwrap().id, 90);
            assert_eq!(records.last().unwrap().id, 99);
        }
        _ => panic!("expected older page"),
    };
    assert!(matches!(
        ch.request(HistoryRequest::Close).await.unwrap(),
        HistoryReply::Ack { epoch: None, .. }
    ));
    f.task.await.unwrap();
    assert!(owned.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn public_history_channel_ready_errors_are_typed_only_after_valid_framing() {
    for (mode, expected) in [
        ("ready-unavailable", SessionError::Missing),
        ("ready-unsupported", SessionError::UpgradeRequired),
        ("ready-busy", SessionError::Busy),
        ("ready-limit", SessionError::Capacity),
        ("ready-protocol", SessionError::Protocol),
    ] {
        let f = fixture(mode).await;
        let owned = Arc::new(Owned::default());
        let error = match open(&f, owned.clone()).await {
            Err(e) => e,
            Ok(_) => panic!("unexpected ready"),
        };
        assert_eq!(error, expected);
        f.task.await.unwrap();
        assert!(owned.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn public_history_channel_cancel_during_attach_deletes_only_owned_helper() {
    let f = fixture("attachWait").await;
    let owned = Arc::new(Owned::default());
    let (entered, wait) = tokio::sync::oneshot::channel();
    {
        let key = "a".repeat(24);
        let operation = HistoryChannel::open(
            &f.api,
            HostSnapshot {
                profile: f.host.profile.clone(),
                credential: Secret::new(f.host.credential.expose().into()).unwrap(),
            },
            &key,
            HistoryVersion::Legacy,
            owned.clone(),
            move |_, _, _, _| async move {
                let _ = entered.send(());
                std::future::pending().await
            },
        );
        tokio::pin!(operation);
        tokio::select! {r=&mut operation=>panic!("unexpected operation completion {}",r.is_ok()),_=wait=>{}};
    }
    let t = f.task.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), owned.1.notified())
        .await
        .unwrap();
    assert_eq!(t.iter().filter(|s| s.starts_with("DELETE ")).count(), 1);
    assert!(owned.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn public_history_channel_create_conflict_never_attaches_or_deletes() {
    let f = fixture("conflict").await;
    let owned = Arc::new(Owned::default());
    assert!(matches!(
        open(&f, owned.clone()).await,
        Err(SessionError::StateChanged)
    ));
    let t = f.task.await.unwrap();
    assert_eq!(t.len(), 1);
    assert!(owned.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn public_history_r5_viewport300_at_rowcount6000_retains_self_contained_styles() {
    let f = fixture("viewport300").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    match ch.request(HistoryRequest::Viewport).await.unwrap() {
        HistoryReply::Page {
            rows,
            row_count,
            start,
            end,
            records,
            ..
        } => {
            assert_eq!((rows, row_count, start, end), (300, 6000, 5700, 6000));
            assert_eq!(records.len(), 300);
            assert_eq!(records.first().unwrap().id, 5701);
            assert_eq!(records.last().unwrap().id, 6000);
            assert!(records
                .iter()
                .all(|r| r.text == "藍😀" && r.styles == vec![[0, 3, 255, -1, 1]]));
        }
        _ => panic!("expected viewport"),
    };
    ch.close().await;
    f.task.await.unwrap();
    assert!(owned.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn public_history_r5_older257_is_rejected_without_input() {
    let f = fixture("valid").await;
    let owned = Arc::new(Owned::default());
    let mut ch = open(&f, owned.clone()).await.unwrap();
    let instance = ch.instance().to_string();
    assert!(matches!(
        ch.request(HistoryRequest::Older {
            instance,
            epoch: 1,
            before: 99,
            limit: 257
        })
        .await,
        Err(SessionError::Protocol)
    ));
    ch.close().await;
    let transcript = f.task.await.unwrap();
    assert!(!transcript.iter().any(|s| s.contains("\"op\"")));
    assert!(owned.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn public_history_r5_rowcount6001_and_pane501_are_rejected() {
    for mode in ["rowCount6001", "rows501"] {
        let f = fixture(mode).await;
        let owned = Arc::new(Owned::default());
        let mut ch = open(&f, owned.clone()).await.unwrap();
        assert!(matches!(
            ch.request(HistoryRequest::Viewport).await,
            Err(SessionError::Protocol)
        ));
        f.task.await.unwrap();
        assert!(owned.0.lock().unwrap().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn bootstrap_selects_persistent_overlay_with_image_fallback() {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let python = bin.join("python3");
    std::fs::write(&python, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = "0123456789abcdef01234567";
    let run = |version: HistoryVersion| {
        let out = Command::new("bash").args(["-c", &version.bootstrap_command(key).unwrap()])
            .env("HOME", home.path()).env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let base = home.path().join(".local/share/art-term");
    assert_eq!(run(HistoryVersion::Paged), format!("{}/openab-pty/tmux-v2/tmux-shell.py\n{key}\n", base.display()));
    std::fs::create_dir_all(base.join("tools")).unwrap();
    std::fs::write(base.join("tools/tmux-shell.py"), "").unwrap();
    assert_eq!(run(HistoryVersion::Paged), format!("{}/tools/tmux-shell.py\n{key}\n", base.display()));
    assert_eq!(run(HistoryVersion::Legacy), format!("{}/openab-pty/tmux-shell.py\n{key}\n", base.display()));
    for invalid in ["", "0123456789abcdef0123456;", "$(touch /tmp/invalid)"] {
        assert!(HistoryVersion::Paged.bootstrap_command(invalid).is_err());
    }
}
