use super::*;

#[tokio::test]
async fn unavailable_native_preview_has_no_workers_and_keeps_screen_controls_accessible() {
    let (_directory, backend, mut previous) = test_app().await;
    previous.stop().await;
    let notice = "原生桌面預覽未啟用；桌面工具仍可使用。終端未宣告支援 Sixel。";
    let mut app = App::with_display(
        backend.clone(),
        Bindings::default(),
        DisplaySelection::Unavailable(notice.into()),
    );
    app.resize(Rect::new(0, 0, 100, 30));
    assert!(app.preview.is_none());
    assert!(app.renderer.is_none());
    assert!(app.preview_target.is_none());
    app.toggle_viewer();
    assert!(!app.expanded);
    assert_eq!(app.status, notice);
    let text: String = draw_without_update(&mut app).split_whitespace().collect();
    assert!(text.contains("桌面工具仍可使用"));
    assert!(text.contains("終端未宣告支援Sixel"));
    app.local(LocalCommand::Screens);
    assert!(matches!(app.modal, Some(Modal::Screens { .. })));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn completing_native_probe_preserves_screen_selection_ui_and_existing_workers() {
    let (_directory, backend, mut previous) = test_app().await;
    previous.stop().await;
    let mut app = App::with_display(
        backend.clone(),
        Bindings::default(),
        DisplaySelection::Unavailable("正在偵測原生圖片能力".into()),
    );
    app.local(LocalCommand::Screens);
    assert!(matches!(app.modal, Some(Modal::Screens { .. })));
    app.set_display(DisplaySelection::Available(
        CellPixels::new(10, 20).unwrap(),
    ))
    .await;
    assert!(app.preview.is_some());
    assert!(app.renderer.is_some());
    assert!(matches!(app.modal, Some(Modal::Screens { .. })));
    app.modal = None;
    let original = seed_frame(&mut app);
    let generation = app.generation;
    app.set_display(DisplaySelection::Available(
        CellPixels::new(10, 20).unwrap(),
    ))
    .await;
    assert_eq!(app.generation, generation);
    assert!(Arc::ptr_eq(app.source_frame.as_ref().unwrap(), &original));
    app.set_display(DisplaySelection::Unavailable(
        "偵測失敗；桌面工具仍可使用".into(),
    ))
    .await;
    assert!(app.preview.is_none());
    assert!(app.renderer.is_none());
    assert!(app.source_frame.is_none());
    app.local(LocalCommand::Screens);
    assert!(matches!(app.modal, Some(Modal::Screens { .. })));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn completed_frames_stay_monotonic_while_capture_and_presentation_advance() {
    let (_directory, backend, mut app) = test_app().await;
    seed_frame(&mut app);
    let plan = app.surface_plan();
    app.render_request = Some((10, plan.clone()));
    app.apply_surfaces(prepared_frame(&app, 8, [255, 0, 0]));
    assert_eq!(app.last_rendered_sequence, Some(8));
    let latest = thumbnail_bytes(&app).to_owned();
    app.apply_surfaces(prepared_frame(&app, 7, [0, 255, 0]));
    assert_eq!(thumbnail_bytes(&app), latest);
    assert_eq!(app.last_rendered_sequence, Some(8));

    let mut invalid = prepared_frame(&app, 9, [0, 255, 0]);
    invalid.target.generation += 1;
    app.apply_surfaces(invalid);
    let mut invalid = prepared_frame(&app, 9, [0, 255, 0]);
    invalid.plan.thumbnail.x += 1;
    app.apply_surfaces(invalid);
    app.apply_surfaces(prepared_frame(&app, 11, [0, 255, 0]));
    assert_eq!(thumbnail_bytes(&app), latest);
    assert_eq!(app.last_rendered_sequence, Some(8));

    let mut changed = plan;
    changed.thumbnail.x += 1;
    app.render_request = Some((10, changed));
    app.apply_surfaces(prepared_frame(&app, 7, [0, 255, 0]));
    assert_eq!(thumbnail_bytes(&app), latest);
    app.apply_surfaces(prepared_frame(&app, 8, [0, 255, 0]));
    assert_ne!(thumbnail_bytes(&app), latest);
    assert_eq!(app.last_rendered_sequence, Some(8));
    app.apply_surfaces(prepared_frame(&app, 9, [0, 0, 255]));
    assert_eq!(app.last_rendered_sequence, Some(9));

    let mut old_error = prepared_frame(&app, 8, [0, 0, 0]);
    old_error.error = Some(crate::preview::PreviewError::InvalidImage);
    app.apply_surfaces(old_error);
    assert!(app.source_frame.is_some());
    assert!(app.preview_image.is_some());
    let mut current_error = prepared_frame(&app, 9, [0, 0, 0]);
    current_error.error = Some(crate::preview::PreviewError::InvalidImage);
    app.apply_surfaces(current_error);
    assert!(app.source_frame.is_none());
    assert!(app.preview_image.is_none());
    assert_eq!(app.last_rendered_sequence, Some(9));
    app.invalidate_preview();
    assert_eq!(app.last_rendered_sequence, None);
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn cached_thumbnail_survives_focus_and_viewer_gestures_without_instruction_flash() {
    let (_directory, backend, mut app) = test_app().await;
    let source = seed_frame(&mut app);
    draw_without_update(&mut app);
    let generation = app.generation;
    app.hit(HitAction::Focus(Focus::Terminal));
    assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
    assert!(app.preview_image.is_some());
    assert_eq!(app.generation, generation);
    let text = draw_without_update(&mut app);
    assert!(!text.contains("選擇此 Shell 使用的桌面"));
    click(&mut app, 4, 23);
    assert!(app.expanded);
    assert_eq!(app.focus, Focus::Terminal);
    let text = draw_without_update(&mut app);
    assert!(text.contains("[X]"));
    assert!(text.contains("[Fit]"));
    assert!(!text.contains("選擇此 Shell 使用的桌面"));
    let canvas = app.layout.viewer_inner();
    let point = (canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
    let before = app.viewport.clone();
    mouse_event(&mut app, MouseEventKind::ScrollUp, point.0, point.1);
    assert_ne!(app.viewport, before);
    let zoomed = app.viewport.clone();
    click(&mut app, point.0, point.1);
    assert_eq!(app.drag, Some(point));
    mouse_event(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        point.0 + 3,
        point.1 + 2,
    );
    assert_ne!(app.viewport, zoomed);
    mouse_event(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        point.0 + 3,
        point.1 + 2,
    );
    assert!(app.drag.is_none());
    assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
    assert_eq!(app.generation, generation);
    app.hit(HitAction::ViewerFit);
    assert_eq!(app.viewport, ImageViewport::default());
    let close = app
        .hits
        .regions
        .iter()
        .find_map(|(r, a)| matches!(a, HitAction::ViewerClose).then_some(*r))
        .unwrap();
    click(&mut app, close.x, close.y);
    assert!(!app.expanded);
    assert!(app.preview_image.is_some());
    click(&mut app, 4, 23);
    assert!(app.expanded);
    click(&mut app, 4, 23);
    assert!(!app.expanded);
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn viewer_closes_on_modal_help_source_stop_and_narrow_layout_without_resizing_remote() {
    let (_directory, backend, mut app) = test_app().await;
    let source = seed_frame(&mut app);
    let size = app.layout.terminal_size();
    app.toggle_viewer();
    assert!(app.expanded);
    assert_eq!(app.layout.terminal_size(), size);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.expanded);
    app.toggle_viewer();
    app.show_modal(Modal::Host(HostForm::new(None)));
    assert!(!app.expanded);
    assert!(app.drag.is_none());
    assert!(Arc::ptr_eq(&source, app.source_frame.as_ref().unwrap()));
    app.dismiss_modal();
    app.toggle_viewer();
    app.navigate(Page::Help);
    assert!(!app.expanded);
    assert!(app.source_frame.is_some());
    app.navigate(Page::Terminal);
    app.toggle_viewer();
    app.invalidate_preview();
    assert!(!app.expanded);
    assert!(app.source_frame.is_none());
    assert!(app.preview_image.is_none());
    seed_frame(&mut app);
    app.layout = WorkspaceLayout::new(Rect::new(0, 0, 50, 12), Focus::Preview);
    app.toggle_viewer();
    assert!(!app.expanded);
    assert!(app.status.contains("放大"));
    app.stop().await;
    backend.shutdown().await;
}
#[tokio::test]
async fn native_damage_tracks_visibility_geometry_and_source_without_clearing_focus() {
    let (_directory, backend, mut app) = test_app().await;
    seed_frame(&mut app);
    app.display = DisplaySelection::Available(CellPixels::new(8, 16).unwrap());
    app.image_damage = false;
    app.hit(HitAction::Focus(Focus::Terminal));
    assert!(!app.image_damage);
    app.toggle_viewer();
    assert!(std::mem::take(&mut app.image_damage));
    app.close_viewer();
    assert!(std::mem::take(&mut app.image_damage));
    app.show_modal(Modal::Host(HostForm::new(None)));
    assert!(std::mem::take(&mut app.image_damage));
    app.invalidate_preview();
    assert!(app.image_damage);
    app.stop().await;
    backend.shutdown().await;
}
