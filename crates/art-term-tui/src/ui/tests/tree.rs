use super::*;

#[tokio::test]
async fn joined_local_detach_refreshes_current_host_only_after_work_has_closed() {
    let directory = tempfile::Builder::new()
        .prefix("art-term-tree-detach-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
    let mut app = App::new(backend.clone());
    app.profiles.hosts.push(HostProfile {
        id: "host".into(),
        revision: 2,
        name: "Host".into(),
        base_url: "https://fixture.example".into(),
        credential_present: false,
        metadata: None,
    });
    app.tree_load = Some(TreeLoad {
        host: HostSelection {
            id: "host".into(),
            revision: 2,
        },
        task: tokio::spawn(std::future::pending()),
    });
    let (release, done) = tokio::sync::oneshot::channel();
    app.closing_tabs.push(ClosingTab {
        host: HostSelection {
            id: "host".into(),
            revision: 1,
        },
        task: tokio::spawn(async move {
            let _ = done.await;
        }),
    });
    app.complete().await;
    assert!(app.load_queue.is_empty());
    release.send(()).unwrap();
    while !app.closing_tabs[0].task.is_finished() {
        tokio::task::yield_now().await;
    }
    app.complete().await;
    assert_eq!(app.load_queue.len(), 1);
    assert_eq!(app.load_queue[0].revision, 2);
    app.stop().await;
    backend.shutdown().await;
}

#[tokio::test]
async fn host_edit_queues_latest_revision_behind_stale_inflight_tree_read() {
    let directory = tempfile::Builder::new()
        .prefix("art-term-tree-revision-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
    let mut app = App::new(backend.clone());
    app.tree_load = Some(TreeLoad {
        host: HostSelection {
            id: "host".into(),
            revision: 1,
        },
        task: tokio::spawn(std::future::pending()),
    });
    app.queue_host(HostSelection {
        id: "host".into(),
        revision: 2,
    });
    app.queue_host(HostSelection {
        id: "host".into(),
        revision: 3,
    });
    assert_eq!(app.load_queue.len(), 1);
    assert_eq!(app.load_queue[0].revision, 3);
    app.stop().await;
    backend.shutdown().await;
}

#[tokio::test]
async fn slow_import_preview_does_not_replace_a_newer_host_draft() {
    let directory = tempfile::Builder::new()
        .prefix("art-term-ui-draft-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let vault = crate::vault::Vault::open_account(paths).unwrap();
    let backend = Backend::start(vault).unwrap();
    let mut app = App::new(backend.clone());
    app.job(async {
        JobResult::Preview(Ok(ImportView {
            ticket: 999,
            entries: Vec::new(),
        }))
    });
    app.show_modal(Modal::Host(HostForm::new(None)));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !app.pending.as_ref().unwrap().task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.complete().await;
    assert!(app.pending.is_none());
    assert!(matches!(app.modal, Some(Modal::Host(_))));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn connection_tree_preserves_saved_type_order_and_chinese_aliases() {
    let (_directory, _backend, mut app) = test_app().await;
    for (id, category, display_name) in [
        (
            "agent-b",
            art_term_core::connections::Category::Agent,
            Some("開發助手"),
        ),
        ("host-a", art_term_core::connections::Category::Host, None),
        ("agent-a", art_term_core::connections::Category::Agent, None),
    ] {
        app.profiles.hosts.push(HostProfile {
            id: id.into(),
            revision: 1,
            name: id.into(),
            base_url: format!("https://{id}.test/"),
            credential_present: true,
            metadata: Some(art_term_core::connections::ConnectionMetadata {
                category,
                display_name: display_name.map(String::from),
                ..Default::default()
            }),
        });
    }
    let rows = app.rows();
    let hosts: Vec<_> = rows.iter().filter(|r| r.key.starts_with("host:")).collect();
    assert_eq!(
        hosts.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        vec!["host:host-a", "host:agent-b", "host:agent-a"]
    );
    assert_eq!(hosts[1].label, "  agent-b / 開發助手");
    assert_eq!(hosts[2].label, "  agent-a");
    app.stop().await;
}

#[tokio::test]
async fn tree_is_permanently_expanded_and_selection_survives_inserted_children() {
    let (_directory, backend, mut app) = test_app().await;
    app.profiles.screens.push(DesktopProfile {
        id: "screen-a".into(),
        revision: 1,
        name: "Screen A".into(),
        mcp_url: "http://127.0.0.1:9/mcp".into(),
        token_present: true,
    });
    app.tree_selected = Some("screen:screen-a".into());
    let before = app
        .rows()
        .iter()
        .position(|r| r.key == "screen:screen-a")
        .unwrap();
    app.profiles.hosts.push(HostProfile {
        id: "host-a".into(),
        revision: 1,
        name: "Host A".into(),
        base_url: "https://fixture.invalid".into(),
        credential_present: true,
        metadata: None,
    });
    let rows = app.rows();
    assert!(
        rows.iter()
            .position(|r| r.key == "screen:screen-a")
            .unwrap()
            > before
    );
    assert_eq!(app.tree_selected.as_deref(), Some("screen:screen-a"));
    for header in ["Computer", "Agent", "Screen"] {
        assert!(rows.iter().any(|r| r.label == header && r.action.is_none()));
    }
    assert!(rows.iter().any(|r| r.key == "new-shell:host-a"));
    assert!(rows.iter().any(|r| r.key == "refresh:host-a"));
    let mut keys = HashSet::new();
    assert!(rows.iter().all(|row| keys.insert(row.key.clone())));
    app.tree_selected = Some("new-host".into());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.modal, Some(Modal::Host(_))));
    app.stop().await;
    backend.shutdown().await;
}
