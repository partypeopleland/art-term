use super::*;

#[test]
fn chrome_is_not_part_of_remote_dimensions_and_tiny_windows_suspend_resize() {
    assert_eq!(
        terminal_size(Rect::new(0, 0, 100, 30)),
        Some(TerminalSize::new(24, 65))
    );
    assert_eq!(terminal_size(Rect::new(0, 0, 80, 2)), None);
    assert_eq!(terminal_size(Rect::new(0, 0, 0, 10)), None);
}
#[test]
fn visible_labels_never_forward_terminal_control_sequences() {
    assert_eq!(safe("host\u{1b}]52;data\u{7}"), "host�]52;data�");
}

#[tokio::test]
async fn three_panes_render_together_and_modal_input_cannot_navigate_or_quit() {
    let directory = tempfile::Builder::new()
        .prefix("art-term-ui-workspace-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
    let mut app = App::new(backend.clone());
    app.resize(Rect::new(0, 0, 100, 30));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| {
            app.hits = draw(&app, frame);
        })
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Connections"));
    assert!(text.contains("Desktop"));
    assert!(text.contains("Terminal"));
    app.show_modal(Modal::Host(HostForm::new(None)));
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(!app.quit);
    assert!(!app.prefix);
    assert!(matches!(app.modal, Some(Modal::Host(_))));
    app.stop().await;
    backend.shutdown().await;
}

#[test]
fn hit_regions_ignore_empty_cells_and_choose_topmost_visible_control() {
    let mut hits = HitMap::default();
    hits.add(Rect::new(0, 0, 0, 5), HitAction::Confirm);
    hits.add(Rect::new(2, 2, 5, 3), HitAction::Focus(Focus::Terminal));
    hits.add(Rect::new(4, 3, 2, 1), HitAction::Cancel);
    assert!(hits.at(1, 2).is_none());
    assert!(hits.at(7, 2).is_none());
    assert!(matches!(
        hits.at(3, 3),
        Some(HitAction::Focus(Focus::Terminal))
    ));
    assert!(matches!(hits.at(4, 3), Some(HitAction::Cancel)));
}
#[tokio::test]
async fn mouse_focus_and_form_controls_are_local_and_modal_exclusive() {
    let (_directory, backend, mut app) = test_app().await;
    draw_test(&mut app, 100, 30);
    click(&mut app, 80, 10);
    assert_eq!(app.focus, Focus::Terminal);
    draw_test(&mut app, 100, 30);
    click(&mut app, 5, 24);
    assert_eq!(app.focus, Focus::Preview);
    app.show_modal(Modal::Host(HostForm::new(None)));
    draw_test(&mut app, 100, 30);
    click(&mut app, 1, 0);
    assert!(matches!(app.modal, Some(Modal::Host(_))));
    assert!(!app.quit);
    let field = app
        .hits
        .regions
        .iter()
        .find_map(|(rect, action)| matches!(action, HitAction::Field(2)).then_some(*rect))
        .unwrap();
    click(&mut app, field.x, field.y);
    app.paste("private-fixture-token".into());
    let Some(Modal::Host(form)) = &app.modal else {
        panic!("form lost")
    };
    assert_eq!(form.active, 2);
    assert_eq!(&*form.fields[2].value, "private-fixture-token");
    assert!(!draw_test(&mut app, 100, 30).contains("private-fixture-token"));
    let cancel = app
        .hits
        .regions
        .iter()
        .find_map(|(rect, action)| matches!(action, HitAction::Cancel).then_some(*rect))
        .unwrap();
    click(&mut app, cancel.x, cancel.y);
    assert!(app.modal.is_none());
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn mouse_action_menu_reaches_every_command_and_narrow_desktop_remains_visible() {
    let (_directory, backend, mut app) = test_app().await;
    let tiny = draw_test(&mut app, 20, 14);
    assert!(tiny.contains("[Actions]"));
    assert!(tiny.contains("[Desktop]"));
    let text = draw_test(&mut app, 60, 14);
    assert!(text.contains("[Desktop]"));
    assert!(text.contains("[Actions]"));
    for row in 0..COMMANDS.len() {
        app.show_modal(Modal::Actions { row });
        draw_test(&mut app, 60, 14);
        assert!(
            app.hits
                .regions
                .iter()
                .any(|(_, a)| matches!(a,HitAction::ModalRow(i) if *i==row)),
            "action {} is unreachable",
            COMMANDS[row].1
        );
        assert!(app
            .hits
            .regions
            .iter()
            .all(|(r, _)| r.right() <= 60 && r.bottom() <= 14));
    }
    app.show_modal(Modal::Actions {
        row: COMMANDS.len() - 1,
    });
    draw_test(&mut app, 60, 14);
    let quit = app
        .hits
        .regions
        .iter()
        .find_map(|(r, a)| matches!(a,HitAction::ModalRow(i) if *i==COMMANDS.len()-1).then_some(*r))
        .unwrap();
    click(&mut app, quit.x, quit.y);
    assert!(app.quit);
    app.stop().await;
    backend.shutdown().await;
}
#[test]
fn unicode_tab_labels_are_clipped_in_terminal_cells() {
    assert_eq!(elide("中文abc", 4), "中…");
    assert_eq!(elide("abc", 3), "abc");
    assert_eq!(elide("中文", 0), "");
}
#[test]
fn native_cursor_uses_remote_cells_and_respects_hide_history_and_bounds() {
    let area = Rect::new(41, 3, 50, 20);
    let mut parser = vt100::Parser::new(20, 50, 100);
    parser.process(b"abc");
    assert_eq!(screen_cursor(parser.screen(), area), Some((44, 3)));
    parser.process("中文".as_bytes());
    assert_eq!(screen_cursor(parser.screen(), area), Some((48, 3)));
    parser.process(b"\x1b[5;7H");
    assert_eq!(screen_cursor(parser.screen(), area), Some((47, 7)));
    assert_eq!(screen_cursor(parser.screen(), Rect::new(0, 0, 6, 4)), None);
    parser.process(b"\x1b[?25l");
    assert_eq!(screen_cursor(parser.screen(), area), None);
    parser.process(b"\x1b[?25h");
    for _ in 0..30 {
        parser.process(b"line\r\n");
    }
    parser.screen_mut().set_scrollback(3);
    assert_eq!(screen_cursor(parser.screen(), area), None);
}
#[test]
fn form_caret_uses_visible_unicode_or_mask_cells_and_stays_inside_field() {
    let field = Field::new("名稱", "ab中文".into(), false);
    assert_eq!(
        field_cursor(&field, Rect::new(5, 8, 20, 3), true),
        Some((12, 9))
    );
    let masked = Field::new("密碼", "ab中文".into(), true);
    assert_eq!(
        field_cursor(&masked, Rect::new(5, 8, 20, 3), true),
        Some((10, 9))
    );
    assert_eq!(
        field_cursor(&field, Rect::new(5, 8, 10, 1), false),
        Some((14, 8))
    );
    assert_eq!(field_cursor(&field, Rect::new(5, 8, 2, 3), true), None);
    assert_eq!(field_cursor(&field, Rect::new(5, 8, 0, 1), false), None);
}
