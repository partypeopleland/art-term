use super::*;

fn terminal_size(area: Rect) -> Option<TerminalSize> {
    WorkspaceLayout::new(area, Focus::Terminal).terminal_size()
}
async fn test_app() -> (tempfile::TempDir, Backend, App) {
    let directory = tempfile::Builder::new()
        .prefix("art-term-ui-mouse-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let paths =
        crate::vault::VaultPaths::resolve(Some(directory.path().join("connections.toml"))).unwrap();
    let backend = Backend::start(crate::vault::Vault::open_account(paths).unwrap()).unwrap();
    let app = App::new(backend.clone());
    (directory, backend, app)
}
fn draw_test(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    app.resize(Rect::new(0, 0, width, height));
    terminal
        .draw(|frame| {
            app.hits = draw(app, frame);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn click(app: &mut App, x: u16, y: u16) {
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}
fn seed_frame(app: &mut App) -> Arc<CapturedFrame> {
    app.layout = WorkspaceLayout::new(Rect::new(0, 0, 100, 30), app.focus);
    let target = PreviewTarget {
        tab: TabId(1),
        generation: app.generation,
        screen: ScreenSelection {
            id: "fixture".into(),
            revision: 1,
        },
    };
    let frame = Arc::new(CapturedFrame {
        target: target.clone(),
        sequence: 1,
        pixels: image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            800,
            400,
            image::Rgb([20, 120, 240]),
        )),
    });
    app.preview_target = Some(target);
    app.source_frame = Some(frame.clone());
    app.preview_status = Some(PreviewStatus::Live);
    app.preview_image = Some(Protocol::Sixel(
        ratatui_image::protocol::sixel::Sixel::new(
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                31,
                14,
                image::Rgb([20, 120, 240]),
            )),
            app.layout.preview_inner.as_size(),
            false,
        )
        .unwrap(),
    ));
    frame
}
fn draw_without_update(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.hits = draw(app, frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn mouse_event(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}
fn prepared_frame(app: &App, sequence: u64, color: [u8; 3]) -> PreparedSurfaces {
    let plan = app.render_request.as_ref().unwrap().1.clone();
    let thumbnail = Protocol::Sixel(
        ratatui_image::protocol::sixel::Sixel::new(
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(6, 6, image::Rgb(color))),
            plan.thumbnail.as_size(),
            false,
        )
        .unwrap(),
    );
    PreparedSurfaces {
        target: app.preview_target.clone().unwrap(),
        sequence,
        plan,
        thumbnail: Some(thumbnail),
        expanded: None,
        error: None,
    }
}
fn thumbnail_bytes(app: &App) -> &str {
    let Some(Protocol::Sixel(image)) = &app.preview_image else {
        panic!("native thumbnail missing");
    };
    &image.data
}
fn seed_profiles(app: &mut App) {
    app.profiles.hosts.push(HostProfile {
        id: "host-a".into(),
        revision: 1,
        name: "Host A".into(),
        base_url: "https://a.test".into(),
        credential_present: true,
        metadata: None,
    });
    app.profiles.screens.push(DesktopProfile {
        id: "screen-a".into(),
        revision: 1,
        name: "Screen A".into(),
        mcp_url: "http://127.0.0.1:9/mcp".into(),
        token_present: true,
    });
}
fn settings_target(app: &App) -> Option<SettingsTarget> {
    match &app.modal {
        Some(Modal::Settings(settings)) => settings.target.clone(),
        _ => panic!("settings modal expected"),
    }
}

#[path = "ui/tests/preview.rs"]
mod preview;
#[path = "ui/tests/render.rs"]
mod render;
#[path = "ui/tests/settings.rs"]
mod settings;
#[path = "ui/tests/tree.rs"]
mod tree;
