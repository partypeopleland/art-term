use super::*;

pub(super) fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
pub(super) fn body(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(3),
    )
}
pub(super) fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray))
}

pub(super) fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}
pub(super) fn elide(text: &str, width: u16) -> String {
    let mut text = safe(text);
    if Span::raw(&text).width() <= width as usize {
        return text;
    }
    while !text.is_empty() && Span::raw(&text).width() + 1 > width as usize {
        text.pop();
    }
    if width > 0 {
        text.push('…');
    }
    text
}
pub(super) fn action_hint(action: BindingAction) -> String {
    match action {
        BindingAction::Hosts => "連線".into(),
        BindingAction::Terminal => "終端".into(),
        BindingAction::Preview => "預覽".into(),
        BindingAction::Screens => "桌面".into(),
        BindingAction::Shells => "Shells".into(),
        BindingAction::Create => "新Shell".into(),
        BindingAction::Previous => "上分頁".into(),
        BindingAction::Next => "下分頁".into(),
        BindingAction::Close => "關分頁".into(),
        BindingAction::Up => "上捲".into(),
        BindingAction::Down => "下捲".into(),
        BindingAction::Follow => "跟隨".into(),
        BindingAction::Recover => "復原".into(),
        BindingAction::Import => "匯入".into(),
        BindingAction::Export => "匯出".into(),
        BindingAction::Help => "說明".into(),
        BindingAction::Quit => "離開".into(),
        BindingAction::Actions => "選單".into(),
        BindingAction::Settings => "連線設定".into(),
        BindingAction::Edit => "編輯".into(),
        BindingAction::Delete => "刪除".into(),
        BindingAction::Test => "測試".into(),
        BindingAction::Refresh => "重整".into(),
        BindingAction::NewHost => "新Host".into(),
        BindingAction::NewAgent => "新Agent".into(),
        BindingAction::NewScreen => "新Screen".into(),
        BindingAction::ViewerToggle => "預覽開關".into(),
        BindingAction::ViewerFit => "預覽還原".into(),
        BindingAction::FocusNext => "切焦點".into(),
        BindingAction::TreeUp => "樹上".into(),
        BindingAction::TreeDown => "樹下".into(),
        BindingAction::TreeActivate => "樹執行".into(),
        BindingAction::Tab1 => "分頁1".into(),
        BindingAction::Tab2 => "分頁2".into(),
        BindingAction::Tab3 => "分頁3".into(),
        BindingAction::Tab4 => "分頁4".into(),
        BindingAction::Tab5 => "分頁5".into(),
        BindingAction::Tab6 => "分頁6".into(),
        BindingAction::Tab7 => "分頁7".into(),
        BindingAction::Tab8 => "分頁8".into(),
        BindingAction::Tab9 => "分頁9".into(),
    }
}
pub(super) fn leader_hint(app: &App, width: u16) -> String {
    let mut text = format!("{} 命令模式：Esc 取消", app.bindings.leader_label());
    let mut used = Span::raw(&text).width();
    for (key, action) in app.bindings.leader_entries() {
        let entry = format!(" · {key} {}", action_hint(action));
        let entry_width = Span::raw(&entry).width();
        if used + entry_width + 2 > width as usize {
            text.push_str(" …");
            break;
        }
        text.push_str(&entry);
        used += entry_width;
    }
    text
}
pub(super) fn bound_hint(bindings: &Bindings, action: BindingAction) -> Option<String> {
    let label = bindings.label(action);
    (label != "未綁定").then_some(label)
}
pub(super) fn context_hint(app: &App) -> String {
    if matches!(app.page, Page::Help) {
        return "↑↓ 捲動說明 · Esc 返回".into();
    }
    if app.focus == Focus::Preview {
        return "Enter 選擇此 Shell 桌面 · 點左下圖片展開預覽".into();
    }
    let mut parts = if app.focus == Focus::Terminal && matches!(app.page, Page::Terminal) {
        vec!["按鍵與貼上送往遠端".to_string()]
    } else {
        vec!["↑↓/Enter 連線樹".to_string()]
    };
    match bound_hint(&app.bindings, LocalCommand::Settings) {
        Some(label) => parts.push(format!("{label} 連線設定")),
        None => parts.push("[連線設定] 管理連線".into()),
    }
    for (action, name) in [
        (LocalCommand::Help, "說明"),
        (LocalCommand::Actions, "操作選單"),
    ] {
        if let Some(label) = bound_hint(&app.bindings, action) {
            parts.push(format!("{label} {name}"));
        }
    }
    parts.join(" · ")
}
pub(super) fn button(
    frame: &mut Frame,
    hits: &mut HitMap,
    area: Rect,
    label: &str,
    action: HitAction,
) {
    frame.render_widget(
        Paragraph::new(elide(label, area.width)).style(Style::default().fg(Color::Cyan)),
        area,
    );
    hits.add(area, action);
}
pub(super) fn draw(app: &App, frame: &mut Frame) -> HitMap {
    let area = frame.area();
    let mut hits = HitMap::default();
    if area.height < 4 || area.width < 20 {
        frame.render_widget(Paragraph::new("終端太小，請放大視窗。"), area);
        return hits;
    }
    let layout = WorkspaceLayout::new(area, app.focus);
    let mut x = area.x;
    let desktop_width = 9.min(area.width);
    for (label, command) in [
        ("[Actions]", LocalCommand::Actions),
        ("[連線設定]", LocalCommand::Settings),
        ("[Connections]", LocalCommand::Hosts),
        ("[Preview]", LocalCommand::Preview),
        ("[Terminal]", LocalCommand::Terminal),
        ("[Help]", LocalCommand::Help),
    ] {
        let width = Span::raw(label).width() as u16 + 1;
        if x + width > area.right() - desktop_width {
            break;
        }
        button(
            frame,
            &mut hits,
            Rect::new(x, area.y, width, 1),
            label,
            HitAction::Command(command),
        );
        x += width;
    }
    button(
        frame,
        &mut hits,
        Rect::new(area.right() - desktop_width, area.y, desktop_width, 1),
        "[Desktop]",
        HitAction::Command(LocalCommand::Screens),
    );
    let border = |title: String, focus: Focus| {
        panel(title).border_style(Style::default().fg(if app.focus == focus {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
    };
    if layout.connections.width > 0 {
        let block = border("Connections / 連線".into(), Focus::Connections);
        let inner = block.inner(layout.connections);
        frame.render_widget(block, layout.connections);
        hits.add(layout.connections, HitAction::Focus(Focus::Connections));
        let rows = app.rows();
        let selected = rows
            .iter()
            .position(|r| app.tree_selected.as_ref() == Some(&r.key));
        let offset = selected
            .map(|i| i.saturating_sub(inner.height.saturating_sub(1) as usize))
            .unwrap_or(0)
            .max(
                app.tree_offset
                    .min(rows.len().saturating_sub(inner.height as usize)),
            );
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(offset)
            .take(inner.height as usize)
        {
            let rect = Rect::new(inner.x, inner.y + (index - offset) as u16, inner.width, 1);
            let chosen = Some(index) == selected;
            let text = format!("{}{}", if chosen { "▶" } else { " " }, row.label);
            frame.render_widget(
                Paragraph::new(elide(&text, inner.width)).style(if chosen {
                    Style::default().bg(Color::DarkGray)
                } else if row.action.is_none() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                }),
                rect,
            );
            if row.action.is_some() {
                hits.add(rect, HitAction::Tree(row.key.clone()));
            }
        }
    }
    if layout.preview.width > 0 {
        let name = app
            .selected_screen()
            .and_then(|b| app.profiles.screens.iter().find(|p| p.id == b.id))
            .map(|p| safe(&p.name))
            .unwrap_or_else(|| "停止".into());
        frame.render_widget(
            border(format!("Desktop / {name}"), Focus::Preview),
            layout.preview,
        );
        hits.add(layout.preview, HitAction::ViewerToggle);
        if let Some(image) = app
            .preview_image
            .as_ref()
            .filter(|_| app.modal.is_none() && !matches!(app.page, Page::Help))
        {
            frame.render_widget(Image::new(image), layout.preview_inner);
        } else {
            let message = if let Some(notice) = app.display.notice() {
                format!("{notice}\n{}", app.desktop_status())
            } else if app.source_frame.is_some() {
                "準備圖片…".into()
            } else {
                match app.preview_status {
                    Some(PreviewStatus::Capturing) => "擷取中…".into(),
                    Some(PreviewStatus::Failed(error)) => error.to_string(),
                    _ => format!(
                        "點擊右上 [Desktop]\n選擇此 Shell 使用的桌面。\n{}",
                        app.desktop_status()
                    ),
                }
            };
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }),
                layout.preview_inner,
            );
        }
    }
    if layout.tabs.width > 0 {
        let slots = (layout.tabs.width / 24).max(1) as usize;
        let start = app.tab / slots * slots;
        let slot_width = (layout.tabs.width / slots as u16).max(1);
        for (slot, (index, tab)) in app
            .tabs
            .iter()
            .enumerate()
            .skip(start)
            .take(slots)
            .enumerate()
        {
            let rect = Rect::new(
                layout.tabs.x + slot as u16 * slot_width,
                layout.tabs.y,
                slot_width,
                1,
            );
            let close_width = 3.min(rect.width);
            let title_width = rect.width.saturating_sub(close_width);
            let label = elide(&format!("{}:{}", index + 1, tab.label()), title_width);
            frame.render_widget(
                Paragraph::new(label).style(if index == app.tab {
                    Style::default().bg(Color::Cyan).fg(Color::Black)
                } else {
                    Style::default().fg(Color::Gray)
                }),
                Rect::new(rect.x, rect.y, title_width, 1),
            );
            hits.add(
                Rect::new(rect.x, rect.y, title_width, 1),
                HitAction::Tab(index),
            );
            button(
                frame,
                &mut hits,
                Rect::new(rect.right() - close_width, rect.y, close_width, 1),
                "[x]",
                HitAction::CloseTab(index),
            );
        }
    }
    if layout.terminal.width > 0 {
        let title = elide(
            &if app.expanded {
                "Desktop | 拖曳平移 · 滾輪縮放".into()
            } else {
                format!("Terminal | {}", app.desktop_status())
            },
            layout.terminal.width.saturating_sub(2),
        );
        frame.render_widget(border(title, Focus::Terminal), layout.terminal);
        hits.add(layout.terminal, HitAction::Focus(Focus::Terminal));
        if app.expanded {
            let canvas = layout.viewer_inner();
            hits.add(canvas, HitAction::ViewerCanvas);
            if let Some(image) = &app.expanded_image {
                frame.render_widget(Image::new(image), canvas);
            } else {
                frame.render_widget(Paragraph::new("準備圖片…"), canvas);
            }
            let x = layout.terminal.right().saturating_sub(4);
            button(
                frame,
                &mut hits,
                Rect::new(x, layout.terminal.y, 3, 1),
                "[X]",
                HitAction::ViewerClose,
            );
            if layout.terminal.width >= 14 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(x - 6, layout.terminal.y, 5, 1),
                    "[Fit]",
                    HitAction::ViewerFit,
                );
            }
        } else if let Some(tab) = app.tabs.get(app.tab) {
            frame.render_widget(
                PseudoTerminal::new(tab.view().screen.as_ref())
                    .cursor(Cursor::default().visibility(false)),
                layout.terminal_inner,
            );
        } else {
            frame.render_widget(Paragraph::new("左側選擇 New Shell 或現有 Shell，按 Enter 或滑鼠點選。\n桌面設定在右上 [Desktop]。"),layout.terminal_inner);
        }
    }
    if matches!(app.page, Page::Help) {
        let content = body(area);
        frame.render_widget(Clear, content);
        hits = HitMap::default();
        let mut lines = vec![
            Line::from("滑鼠點選區塊與分頁；連線樹固定展開，↑↓移動、Enter執行。"),
            Line::from("[Actions] 包含新增／編輯／刪除、匯入匯出及歷史操作。"),
            Line::from(
                "點擊左下桌面圖片切換右側預覽；拖曳平移、滾輪縮放、[Fit] 還原、[X] 或 Esc 關閉。",
            ),
            Line::from("本機與 SSH 都偵測 Sixel 與實際像素；失敗只停預覽，桌面工具仍可使用。"),
            Line::from("display.toml 的舊 compatible 值視為 auto，不再使用字格圖片。"),
            Line::from(format!(
                "快捷鍵可在 keybindings.toml 自訂；目前前綴 {}。",
                app.bindings.leader_label()
            )),
        ];
        lines.extend(COMMANDS.iter().map(|(command, label)| {
            Line::from(format!("{}  {}", app.bindings.label(*command), label))
        }));
        lines.extend([
            Line::from("[連線設定] 集中管理已保存 Host／Agent／Screen 並提供加密匯入／匯出；刪除只作用於所選連線，不會刪除遠端 Shell。"),
            Line::from("連線設定預設沒有全域快捷鍵；可在 keybindings.toml 設定 settings = [...] 啟用。"),
            Line::from("前導鍵待命時，底部列出目前生效的第二鍵與 Esc 取消；連按兩次前導鍵則送入遠端。"),
            Line::from("桌面預覽與遠端工具狀態分開顯示；Tools ready 才代表 broker 配置已確認。"),
            Line::from(
                "Stop 只停止預覽；Clear tools 取消此 Shell 的桌面工具。關頁不刪除遠端工作。",
            ),
            Line::from(
                "只有 Terminal 焦點會送普通按鍵與貼上；Shift+滑鼠可使用終端本身的文字選取。",
            ),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel("Help / 說明"))
                .scroll((app.help_offset, 0))
                .wrap(Wrap { trim: false }),
            content,
        );
        let close = Rect::new(
            content.right().saturating_sub(10),
            content.y,
            10.min(content.width),
            1,
        );
        button(
            frame,
            &mut hits,
            close,
            "[Close]",
            HitAction::Command(if app.tabs.is_empty() {
                LocalCommand::Hosts
            } else {
                LocalCommand::Terminal
            }),
        );
    }
    let state = app
        .tabs
        .get(app.tab)
        .map(|t| {
            let v = t.view();
            format!(
                "{} | {}{}",
                status(&v.status),
                history(&v.history),
                v.notice
                    .map(|n| format!(" | {}", safe(n)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| {
            format!(
                "{} Hosts | {} Screens",
                app.profiles.hosts.len(),
                app.profiles.screens.len()
            )
        });
    frame.render_widget(
        Paragraph::new(format!(
            "{}  {}{}",
            state,
            if app.pending.is_some() {
                "Working… "
            } else {
                ""
            },
            safe(&app.status)
        ))
        .style(Style::default().fg(Color::Gray)),
        Rect::new(area.x, area.bottom() - 2, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(if app.prefix {
            leader_hint(app, area.width)
        } else {
            context_hint(app)
        })
        .style(Style::default().fg(Color::Cyan)),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
    if let Some(modal) = &app.modal {
        hits = draw_modal(app, modal, frame);
    } else if let Some(position) = terminal_cursor(app, layout.terminal_inner) {
        frame.set_cursor_position(position);
    }
    hits
}

pub(super) fn terminal_cursor(app: &App, area: Rect) -> Option<(u16, u16)> {
    if app.focus != Focus::Terminal
        || !matches!(app.page, Page::Terminal)
        || app.expanded
        || app.modal.is_some()
    {
        return None;
    }
    let view = app.tabs.get(app.tab)?.view();
    if !matches!(view.status, TabStatus::Live)
        || matches!(
            view.history,
            HistoryStatus::Held { .. } | HistoryStatus::Uncertain
        )
        || view.screen.hide_cursor()
        || view.screen.scrollback() > 0
    {
        return None;
    }
    screen_cursor(&view.screen, area)
}
pub(super) fn screen_cursor(screen: &vt100::Screen, area: Rect) -> Option<(u16, u16)> {
    let (row, column) = screen.cursor_position();
    (!screen.hide_cursor() && screen.scrollback() == 0 && row < area.height && column < area.width)
        .then_some((area.x + column, area.y + row))
}
pub(super) fn field_cursor(field: &Field, region: Rect, bordered: bool) -> Option<(u16, u16)> {
    let area = if bordered {
        panel("").inner(region)
    } else {
        region
    };
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let width = Span::raw(field.visible()).width()
        + if bordered {
            0
        } else {
            Span::raw(format!("{}: ", field.label)).width()
        };
    Some((area.x + (width as u16).min(area.width - 1), area.y))
}

pub(super) fn status(value: &TabStatus) -> String {
    match value {
        TabStatus::Live => "Live".into(),
        TabStatus::Recovering => "恢復連線中".into(),
        TabStatus::InputUncertain => "輸入結果不確定，已停止送入".into(),
        TabStatus::Ended(error) => format!("已斷線：{}", safe(error)),
    }
}
pub(super) fn history(value: &HistoryStatus) -> String {
    match value {
        HistoryStatus::Unsupported => "無歷史控制".into(),
        HistoryStatus::Following => "即時輸出".into(),
        HistoryStatus::Held { offset } => format!("歷史閱讀 -{offset}"),
        HistoryStatus::Uncertain => "歷史控制狀態不確定".into(),
    }
}
pub(super) fn modal_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(88);
    let height = area.height.saturating_sub(2).min(24);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
pub(super) fn draw_modal(app: &App, modal: &Modal, frame: &mut Frame) -> HitMap {
    let rect = modal_rect(frame.area());
    let mut hits = HitMap::default();
    frame.render_widget(Clear, rect);
    match modal {
        Modal::Host(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if matches!(form.category, art_term_core::connections::Category::Agent) {
                    if form.previous.is_some() {
                        "編輯 Agent / Edit"
                    } else {
                        "新增 Agent / New Agent"
                    }
                } else if form.previous.is_some() {
                    "編輯 Host / Edit"
                } else {
                    "新增 Host / Add"
                },
                rect,
                frame,
            )
        }
        Modal::Screen(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if form.previous.is_some() {
                    "編輯 Screen"
                } else {
                    "新增 Screen"
                },
                rect,
                frame,
            )
        }
        Modal::Transfer(form) => {
            return draw_fields(
                &form.fields,
                form.active,
                if form.export {
                    "加密匯出全部連線（含憑證；不覆寫既有檔案）"
                } else {
                    "匯入 Host / Screen（不含密碼可留白）"
                },
                rect,
                frame,
            )
        }
        Modal::Settings(settings) => {
            let rows = app.settings_rows();
            let block = panel("連線設定 / Connection settings");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let resolved = app.resolve_settings_target(&settings.target);
            let detail_rows = if inner.height >= 5 { 1 } else { 0 };
            let height = inner.height.saturating_sub(1 + detail_rows) as usize;
            let offset = settings.row.saturating_sub(height.saturating_sub(1));
            for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
                let region = Rect::new(inner.x, inner.y + (i - offset) as u16, inner.width, 1);
                let (text, disabled) = app.settings_row_text(row, &resolved);
                frame.render_widget(
                    Paragraph::new(elide(&text, inner.width)).style(if i == settings.row {
                        Style::default().bg(Color::DarkGray)
                    } else if disabled {
                        Style::default().fg(Color::DarkGray)
                    } else if matches!(row, SettingsRow::Target(_)) {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    }),
                    region,
                );
                hits.add(region, HitAction::ModalRow(i));
            }
            if detail_rows > 0 {
                let detail = match &resolved {
                    Some(target) => format!("管理目標：{}", app.settings_target_label(target)),
                    None => "管理目標：未選擇 — 編輯／刪除／測試已停用".to_string(),
                };
                frame.render_widget(
                    Paragraph::new(elide(&detail, inner.width))
                        .style(Style::default().fg(Color::Yellow)),
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                );
            }
            if inner.height > 0 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(inner.x, inner.bottom() - 1, inner.width.min(10), 1),
                    "[Back]",
                    HitAction::Cancel,
                );
                let hint = "Enter/點擊 選擇或執行 · Esc 返回";
                let hint_width = Span::raw(hint).width() as u16;
                if inner.width >= hint_width + 12 {
                    frame.render_widget(
                        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                        Rect::new(
                            inner.x + 11,
                            inner.bottom() - 1,
                            (inner.width - 11).min(hint_width + 1),
                            1,
                        ),
                    );
                }
            }
        }
        Modal::Actions { row } | Modal::Screens { row } => {
            let actions = matches!(modal, Modal::Actions { .. });
            let labels = if actions {
                COMMANDS
                    .iter()
                    .map(|(command, label)| format!("{}  {}", label, app.bindings.label(*command)))
                    .collect::<Vec<_>>()
            } else {
                std::iter::once("停止預覽 / Stop".into())
                    .chain(app.profiles.screens.iter().map(|p| safe(&p.name)))
                    .chain([
                        "取消桌面工具 / Clear tools".into(),
                        "恢復預覽 / Resume preview".into(),
                        "新增 Screen / New Screen".into(),
                    ])
                    .collect::<Vec<String>>()
            };
            let block = panel(if actions {
                "Actions / 所有操作"
            } else {
                "Screen：選擇此 Shell 的桌面"
            });
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let detail_rows = if !actions && inner.height >= 5 { 2 } else { 0 };
            let height = inner.height.saturating_sub(1 + detail_rows) as usize;
            let offset = row.saturating_sub(height.saturating_sub(1));
            for (i, label) in labels.iter().enumerate().skip(offset).take(height) {
                let region = Rect::new(inner.x, inner.y + (i - offset) as u16, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(elide(label, inner.width)).style(if i == *row {
                        Style::default().bg(Color::DarkGray)
                    } else {
                        Style::default()
                    }),
                    region,
                );
                hits.add(region, HitAction::ModalRow(i));
            }
            if detail_rows > 0 {
                let notice = app
                    .tabs
                    .get(app.tab)
                    .and_then(|t| app.backend.desktop_view(t.id()).ok())
                    .and_then(|view| view.notice)
                    .unwrap_or("Stop 只停止預覽；Clear tools 才會取消工具設定。");
                frame.render_widget(
                    Paragraph::new(elide(&app.desktop_status(), inner.width)),
                    Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
                );
                frame.render_widget(
                    Paragraph::new(elide(notice, inner.width)),
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                );
            }
            if inner.height > 0 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(inner.x, inner.bottom() - 1, inner.width.min(10), 1),
                    "[Cancel]",
                    HitAction::Cancel,
                );
            }
        }
        Modal::Confirm(action) => {
            let message = match action {
                Confirm::DeleteScreen(_) => {
                    "刪除此 Screen？本機預覽將停止；不會清除遠端 Shell 的現有工具設定。"
                }
                Confirm::DeleteHost(_) => "刪除此 Host 設定？本機分頁會失效，遠端工作不會被刪除。",
                Confirm::Attach(shell) => {
                    if shell.attached {
                        "接管此 Shell？目前其他連線可能中斷。"
                    } else {
                        "連線至選定的 Shell？此操作會重新取得連線權限。"
                    }
                }
                Confirm::DeleteShell(_) => "永久終止並刪除此遠端 Shell？這與關閉本機分頁不同。",
                Confirm::ClearTools(_) => {
                    "取消此 Shell 的桌面工具？其他 Shell 與共享桌面不受影響。"
                }
            };
            let block = panel("確認 / Confirm");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                ),
            );
            if inner.height > 1 {
                frame.render_widget(
                    Paragraph::new(if app.confirm_yes {
                        "↑↓ 選擇 [Yes]；Enter 確認"
                    } else {
                        "↑↓ 選擇 [Cancel]；Enter 取消"
                    })
                    .style(Style::default().fg(Color::Yellow)),
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                );
            }
            if inner.height > 0 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(
                        inner.x,
                        inner.bottom() - 1,
                        (if inner.width >= 18 {
                            8
                        } else {
                            inner.width.saturating_sub(8)
                        })
                        .min(inner.width),
                        1,
                    ),
                    "[Yes]",
                    HitAction::Confirm,
                );
                if inner.width >= 14 {
                    button(
                        frame,
                        &mut hits,
                        Rect::new(
                            if inner.width >= 18 {
                                inner.x + 9
                            } else {
                                inner.right() - 8
                            },
                            inner.bottom() - 1,
                            8,
                            1,
                        ),
                        "[Cancel]",
                        HitAction::Cancel,
                    );
                }
            }
        }
        Modal::Import {
            preview,
            replace,
            scroll,
        } => {
            let block = panel("匯入預覽 / Import preview");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut lines = vec![Line::from(
                "新項若不含憑證會略過；Replace 更新可能需要重新連線現有分頁。",
            )];
            lines.extend(preview.entries.iter().map(|entry| {
                Line::from(format!(
                    "{} {} [{}] {}",
                    entry.kind,
                    safe(&entry.name),
                    entry.status,
                    if entry.has_secret {
                        "含加密憑證"
                    } else {
                        "不含憑證"
                    }
                ))
            }));
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                ),
            );
            if inner.height > 1 {
                button(
                    frame,
                    &mut hits,
                    Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                    if *replace {
                        "[Replace duplicates: ON]"
                    } else {
                        "[Replace duplicates: OFF]"
                    },
                    HitAction::ToggleReplace,
                );
                button(
                    frame,
                    &mut hits,
                    Rect::new(
                        inner.x,
                        inner.bottom() - 1,
                        (if inner.width >= 18 {
                            8
                        } else {
                            inner.width.saturating_sub(8)
                        })
                        .min(inner.width),
                        1,
                    ),
                    if inner.width >= 15 { "[Apply]" } else { "[OK]" },
                    HitAction::Confirm,
                );
                if inner.width >= 14 {
                    button(
                        frame,
                        &mut hits,
                        Rect::new(
                            if inner.width >= 18 {
                                inner.x + 9
                            } else {
                                inner.right() - 8
                            },
                            inner.bottom() - 1,
                            8,
                            1,
                        ),
                        "[Cancel]",
                        HitAction::Cancel,
                    );
                }
            }
        }
    }
    hits
}
pub(super) fn draw_fields(
    fields: &[Field],
    active: usize,
    title: &'static str,
    rect: Rect,
    frame: &mut Frame,
) -> HitMap {
    let mut hits = HitMap::default();
    let block = panel(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let field_height = if inner.height >= fields.len() as u16 * 3 + 1 {
        3
    } else {
        1
    };
    if inner.height == 0 {
        return hits;
    }
    let capacity = ((inner.height.saturating_sub(1)) / field_height.max(1))
        .max(1)
        .min(fields.len() as u16) as usize;
    let scroll = if fields.len() <= capacity {
        0
    } else {
        active
            .saturating_sub(capacity - 1)
            .min(fields.len() - capacity)
    };
    for (index, field) in fields.iter().enumerate().skip(scroll).take(capacity) {
        let y = inner.y + (index - scroll) as u16 * field_height;
        let region = Rect::new(inner.x, y, inner.width, field_height);
        let style = Style::default().fg(if index == active {
            Color::Cyan
        } else {
            Color::DarkGray
        });
        if field_height == 3 {
            frame.render_widget(
                Paragraph::new(field.visible()).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(field.label)
                        .border_style(style),
                ),
                region,
            );
        } else {
            frame.render_widget(
                Paragraph::new(elide(
                    &format!("{}: {}", field.label, field.visible()),
                    inner.width,
                ))
                .style(style),
                region,
            );
        }
        hits.add(region, HitAction::Field(index));
        if index == active {
            if let Some(position) = field_cursor(field, region, field_height == 3) {
                frame.set_cursor_position(position);
            }
        }
    }
    if inner.height > 0 {
        button(
            frame,
            &mut hits,
            Rect::new(
                inner.x,
                inner.bottom() - 1,
                (if inner.width >= 18 {
                    8
                } else {
                    inner.width.saturating_sub(8)
                })
                .min(inner.width),
                1,
            ),
            "[Save]",
            HitAction::Submit,
        );
        if inner.width >= 14 {
            button(
                frame,
                &mut hits,
                Rect::new(
                    if inner.width >= 18 {
                        inner.x + 9
                    } else {
                        inner.right() - 8
                    },
                    inner.bottom() - 1,
                    8,
                    1,
                ),
                "[Cancel]",
                HitAction::Cancel,
            );
        }
    }
    hits
}
