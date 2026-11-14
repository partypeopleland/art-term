use super::*;

#[tokio::test]
async fn every_narrow_form_and_confirmation_has_a_visible_mouse_cancel() {
    let directory = tempfile::Builder::new()
        .prefix("art-term-modal-cancel-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
    let mut app = App::new(backend.clone());
    app.resize(Rect::new(0, 0, 20, 14));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 14)).unwrap();
    for modal in [
        Modal::Host(HostForm::new(None)),
        Modal::Screen(ScreenForm::new(None)),
        Modal::Transfer(TransferForm::new(true)),
        Modal::Confirm(Confirm::DeleteHost(HostSelection {
            id: "host".into(),
            revision: 1,
        })),
        Modal::Import {
            preview: ImportView {
                ticket: 1,
                entries: Vec::new(),
            },
            replace: false,
            scroll: 0,
        },
    ] {
        app.show_modal(modal);
        terminal.draw(|frame| app.hits = draw(&app, frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("[Cancel]"), "{text}");
        assert!(app
            .hits
            .regions
            .iter()
            .any(|(_, action)| matches!(action, HitAction::Cancel)));
    }
    app.stop().await;
    backend.shutdown().await;
}

#[test]
fn secret_fields_mask_unicode_and_clear_without_returning_plaintext() {
    let mut field = Field::new("secret", "秘密-value".into(), true);
    assert_eq!(field.visible(), "********");
    field.input(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert!(field.value.is_empty());
    field.paste("token\n\u{1b}value");
    assert_eq!(&*field.value, "tokenvalue");
}
#[tokio::test]
async fn settings_target_is_typed_explicit_and_children_return() {
    let (_directory, backend, mut app) = test_app().await;
    seed_profiles(&mut app);
    // No saved-record selection: CRUD must stay disabled, never fall back.
    app.open_settings();
    assert_eq!(settings_target(&app), None);
    app.settings_action(SettingsAction::Delete, None, 0);
    assert!(matches!(app.modal, Some(Modal::Settings(_))));
    assert!(!matches!(app.modal, Some(Modal::Confirm(_))));
    assert!(app.status.contains("請先選擇"));
    // Screen row resolves a Screen target; delete confirms the record only.
    app.tree_selected = Some("screen:screen-a".into());
    app.open_settings();
    let target = settings_target(&app);
    assert!(matches!(target, Some(SettingsTarget::Screen(_))));
    app.settings_action(SettingsAction::Delete, target, 0);
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm(Confirm::DeleteScreen(_)))
    ));
    assert!(!matches!(
        app.modal,
        Some(Modal::Confirm(Confirm::DeleteShell(_)))
    ));
    // Declining restores the same settings context and target.
    app.modal_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(matches!(
        settings_target(&app),
        Some(SettingsTarget::Screen(_))
    ));
    // Host row resolves a Host target; Test uses the typed selection.
    app.tree_selected = Some("host:host-a".into());
    app.open_settings();
    let target = settings_target(&app);
    assert!(matches!(target, Some(SettingsTarget::Host(_))));
    app.settings_action(SettingsAction::Test, target, 0);
    assert!(app.pending.is_some());
    assert!(matches!(
        app.modal_origin,
        Some(ModalOrigin::Settings {
            target: Some(SettingsTarget::Host(_)),
            ..
        })
    ));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn settings_target_revalidates_revision_and_clears_when_stale() {
    let (_directory, backend, mut app) = test_app().await;
    seed_profiles(&mut app);
    app.tree_selected = Some("host:host-a".into());
    app.open_settings();
    let stale = settings_target(&app);
    app.profiles.hosts[0].revision = 9;
    assert_eq!(app.resolve_settings_target(&stale), None);
    app.refresh_settings_target();
    assert_eq!(settings_target(&app), None);
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn settings_edit_cancel_restores_settings_and_leader_esc_cancels() {
    let (_directory, backend, mut app) = test_app().await;
    seed_profiles(&mut app);
    app.tree_selected = Some("host:host-a".into());
    app.open_settings();
    let target = settings_target(&app);
    app.settings_action(SettingsAction::Edit, target, 0);
    assert!(matches!(app.modal, Some(Modal::Host(_))));
    app.modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.modal, Some(Modal::Settings(_))));
    assert!(matches!(
        settings_target(&app),
        Some(SettingsTarget::Host(_))
    ));
    app.modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.modal.is_none());
    app.prefix = true;
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.prefix);
    assert!(!app.status.contains("未知快捷鍵"));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn settings_modal_draws_target_rows_actions_and_back() {
    let (_directory, backend, mut app) = test_app().await;
    seed_profiles(&mut app);
    app.open_settings();
    let text = draw_test(&mut app, 100, 30);
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("連線設定/Connectionsettings"), "{text}");
    assert!(compact.contains("Host：HostA"), "{text}");
    assert!(compact.contains("Screen：ScreenA"), "{text}");
    assert!(text.contains("Delete target"), "{text}");
    assert!(text.contains("[Back]"), "{text}");
    assert!(compact.contains("管理目標：未選擇"), "{text}");
    // Settings rows must not advertise global shortcuts the modal cannot
    // receive; Enter/click is the advertised activation instead.
    assert!(!compact.contains("未綁定"), "{text}");
    assert!(compact.contains("Enter/點擊選擇或執行·Esc返回"), "{text}");
    app.stop().await;
    backend.shutdown().await;
}
