use crate::tab::TerminalSize;
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Connections,
    Preview,
    Terminal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceLayout {
    pub connections: Rect,
    pub preview: Rect,
    pub preview_inner: Rect,
    pub tabs: Rect,
    pub terminal: Rect,
    pub terminal_inner: Rect,
}
impl WorkspaceLayout {
    pub(crate) fn new(area: Rect, focus: Focus) -> Self {
        let mut result = Self {
            connections: Rect::default(),
            preview: Rect::default(),
            preview_inner: Rect::default(),
            tabs: Rect::default(),
            terminal: Rect::default(),
            terminal_inner: Rect::default(),
        };
        if area.width < 20 || area.height < 6 {
            return result;
        }
        let body = Rect::new(area.x, area.y + 1, area.width, area.height - 3);
        if area.width >= 80 && area.height >= 18 {
            let left = (area.width / 3).clamp(26, 40);
            let upper = (body.height * 2 / 3).max(6);
            result.connections = Rect::new(body.x, body.y, left, upper);
            result.preview = Rect::new(body.x, body.y + upper, left, body.height - upper);
            result.tabs = Rect::new(body.x + left, body.y, body.width - left, 1);
            result.terminal = Rect::new(
                body.x + left,
                body.y + 1,
                body.width - left,
                body.height - 1,
            );
        } else {
            match focus {
                Focus::Connections => result.connections = body,
                Focus::Preview => result.preview = body,
                Focus::Terminal => {
                    result.tabs = Rect::new(body.x, body.y, body.width, 1);
                    result.terminal = Rect::new(body.x, body.y + 1, body.width, body.height - 1);
                }
            }
        }
        let block = Block::default().borders(Borders::ALL);
        result.preview_inner = block.inner(result.preview);
        result.preview_inner.width = result.preview_inner.width.min(160);
        result.preview_inner.height = result.preview_inner.height.min(80);
        result.terminal_inner = block.inner(result.terminal);
        result.terminal_inner.width = result.terminal_inner.width.min(500);
        result.terminal_inner.height = result.terminal_inner.height.min(240);
        result
    }
    pub(crate) fn terminal_size(self) -> Option<TerminalSize> {
        let area = self.terminal_inner;
        (area.width > 0 && area.height > 0).then(|| TerminalSize::new(area.height, area.width))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_layout_displays_three_disjoint_panes_and_excludes_chrome() {
        let l = WorkspaceLayout::new(Rect::new(0, 0, 100, 30), Focus::Terminal);
        assert_eq!(l.terminal_size(), Some(TerminalSize::new(24, 65)));
        assert_eq!(l.connections.right(), l.terminal.x);
        assert_eq!(l.connections.bottom(), l.preview.y);
        assert!(l.preview_inner.height > 0);
    }
    #[test]
    fn narrow_focus_keeps_each_pane_accessible() {
        for f in [Focus::Connections, Focus::Preview, Focus::Terminal] {
            let l = WorkspaceLayout::new(Rect::new(0, 0, 50, 12), f);
            assert!(
                match f {
                    Focus::Connections => l.connections.width,
                    Focus::Preview => l.preview.width,
                    Focus::Terminal => l.terminal.width,
                } > 0
            );
        }
    }
}
