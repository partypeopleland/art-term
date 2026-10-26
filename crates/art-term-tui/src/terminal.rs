use crate::tab::TerminalSize;
use art_term_core::openab::session::SessionError;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};

const INPUT_LIMIT: usize = 64 * 1024;
const REPLY_LIMIT: usize = 8192;
const OSC_LIMIT: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WheelInput {
    History(i16),
    Bytes(Vec<u8>),
    Ignored,
}

#[derive(Default)]
enum OscAdmission {
    #[default]
    Ground,
    Escape,
    Payload(usize),
}

impl OscAdmission {
    fn admit(&mut self, byte: u8) -> Result<(), SessionError> {
        *self = if byte == 0x1b {
            Self::Escape
        } else {
            match *self {
                Self::Escape if byte == b']' => Self::Payload(0),
                Self::Escape if matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => {
                    Self::Escape
                }
                Self::Payload(_) if matches!(byte, 0x07 | 0x18 | 0x1a) => Self::Ground,
                Self::Payload(length) if length < OSC_LIMIT => Self::Payload(length + 1),
                Self::Payload(_) => return Err(SessionError::Capacity),
                _ => Self::Ground,
            }
        };
        Ok(())
    }
}

#[derive(Default)]
struct Responses {
    enabled: bool,
    bytes: Vec<u8>,
    overflow: bool,
}

impl Responses {
    fn push(&mut self, bytes: &[u8]) {
        if !self.enabled {
            return;
        }
        if self.bytes.len().saturating_add(bytes.len()) > REPLY_LIMIT {
            self.overflow = true;
        } else {
            self.bytes.extend_from_slice(bytes);
        }
    }
}

impl vt100::Callbacks for Responses {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        intermediate: Option<u8>,
        second: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        if second.is_some() || params.len() > 1 {
            return;
        }
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (intermediate, first, command) {
            (None, 5, 'n') => self.push(b"\x1b[0n"),
            (None | Some(b'?'), 6, 'n') => {
                let (row, col) = screen.cursor_position();
                let private = if intermediate == Some(b'?') { "?" } else { "" };
                self.push(format!("\x1b[{private}{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 0, 'c') => self.push(b"\x1b[?1;2c"),
            (Some(b'>'), 0, 'c') => self.push(b"\x1b[>0;1;0c"),
            (None, 18, 't') => {
                let (rows, cols) = screen.size();
                self.push(format!("\x1b[8;{rows};{cols}t").as_bytes());
            }
            _ => (),
        }
    }

    fn unhandled_escape(
        &mut self,
        _: &mut vt100::Screen,
        first: Option<u8>,
        second: Option<u8>,
        command: u8,
    ) {
        if first.is_none() && second.is_none() && command == b'Z' {
            self.push(b"\x1b[?1;2c");
        }
    }
}

#[derive(Clone, Copy, Default)]
enum SequenceOrigin {
    #[default]
    Ground,
    Escape {
        historical: bool,
    },
    Csi {
        historical: bool,
    },
}

pub(crate) struct Terminal {
    parser: vt100::Parser<Responses>,
    size: TerminalSize,
    origin: SequenceOrigin,
    osc_admission: OscAdmission,
}

impl Terminal {
    pub(crate) fn new(size: TerminalSize) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                size.rows,
                size.cols,
                0,
                Responses::default(),
            ),
            size,
            origin: SequenceOrigin::Ground,
            osc_admission: OscAdmission::Ground,
        }
    }

    pub(crate) fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.size);
    }

    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }

    pub(crate) fn feed(&mut self, bytes: &[u8], historical: bool) -> Result<Vec<u8>, SessionError> {
        for byte in bytes {
            // vte's std OSC buffer grows until termination, across output batches.
            self.osc_admission.admit(*byte)?;
            // This tag only attributes callbacks to the initiating ESC. vt100 remains
            // the sole parser, including split CSI, UTF-8, cursor state and modes.
            let from_history = match self.origin {
                SequenceOrigin::Ground => historical,
                SequenceOrigin::Escape { historical: origin }
                | SequenceOrigin::Csi { historical: origin } => origin || historical,
            };
            self.parser.callbacks_mut().enabled = !from_history;
            self.parser.process(std::slice::from_ref(byte));
            self.origin = if *byte == 0x1b {
                SequenceOrigin::Escape { historical }
            } else {
                match self.origin {
                    SequenceOrigin::Escape { .. } if *byte == b'[' => SequenceOrigin::Csi {
                        historical: from_history,
                    },
                    SequenceOrigin::Escape { .. } if matches!(*byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => {
                        SequenceOrigin::Escape {
                            historical: from_history,
                        }
                    }
                    SequenceOrigin::Csi { .. }
                        if !(0x40..=0x7e).contains(byte) && *byte != 0x18 && *byte != 0x1a =>
                    {
                        SequenceOrigin::Csi {
                            historical: from_history,
                        }
                    }
                    _ => SequenceOrigin::Ground,
                }
            };
        }
        let responses = self.parser.callbacks_mut();
        if responses.overflow {
            responses.bytes.clear();
            return Err(SessionError::Capacity);
        }
        Ok(std::mem::take(&mut responses.bytes))
    }

    pub(crate) fn wheel(&self, event: MouseEvent) -> WheelInput {
        let (button, lines) = match event.kind {
            MouseEventKind::ScrollUp => (64, -3),
            MouseEventKind::ScrollDown => (65, 3),
            _ => return WheelInput::Ignored,
        };
        let screen = self.screen();
        let (rows, columns) = screen.size();
        if event.row >= rows || event.column >= columns {
            return WheelInput::Ignored;
        }
        match screen.mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => return WheelInput::History(lines),
            vt100::MouseProtocolMode::Press => return WheelInput::Ignored,
            _ => (),
        }
        let button = button
            + 4 * u16::from(event.modifiers.contains(KeyModifiers::SHIFT))
            + 8 * u16::from(event.modifiers.contains(KeyModifiers::ALT))
            + 16 * u16::from(event.modifiers.contains(KeyModifiers::CONTROL));
        let column = event.column + 1;
        let row = event.row + 1;
        let bytes = match screen.mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Sgr => {
                format!("\x1b[<{button};{column};{row}M").into_bytes()
            }
            vt100::MouseProtocolEncoding::Default => {
                if column > 223 || row > 223 {
                    return WheelInput::Ignored;
                }
                vec![
                    0x1b,
                    b'[',
                    b'M',
                    (button + 32) as u8,
                    (column + 32) as u8,
                    (row + 32) as u8,
                ]
            }
            vt100::MouseProtocolEncoding::Utf8 => {
                let mut bytes = b"\x1b[M".to_vec();
                for value in [button + 32, column + 32, row + 32] {
                    let mut encoded = [0; 4];
                    bytes.extend_from_slice(
                        char::from_u32(value.into())
                            .unwrap()
                            .encode_utf8(&mut encoded)
                            .as_bytes(),
                    );
                }
                bytes
            }
        };
        WheelInput::Bytes(bytes)
    }

    pub(crate) fn key(&self, key: KeyEvent) -> Result<Option<Vec<u8>>, SessionError> {
        if key.kind == KeyEventKind::Release {
            return Ok(None);
        }
        let mods = key.modifiers;
        let modifier = 1
            + u8::from(mods.contains(KeyModifiers::SHIFT))
            + 2 * u8::from(mods.contains(KeyModifiers::ALT))
            + 4 * u8::from(mods.contains(KeyModifiers::CONTROL));
        let cursor = |last: char| {
            if modifier != 1 {
                format!("\x1b[1;{modifier}{last}").into_bytes()
            } else if self.screen().application_cursor() {
                format!("\x1bO{last}").into_bytes()
            } else {
                format!("\x1b[{last}").into_bytes()
            }
        };
        let tilde = |number: u8| {
            if modifier == 1 {
                format!("\x1b[{number}~").into_bytes()
            } else {
                format!("\x1b[{number};{modifier}~").into_bytes()
            }
        };
        let mut bytes = match key.code {
            KeyCode::Up => return Ok(Some(cursor('A'))),
            KeyCode::Down => return Ok(Some(cursor('B'))),
            KeyCode::Right => return Ok(Some(cursor('C'))),
            KeyCode::Left => return Ok(Some(cursor('D'))),
            KeyCode::Home => return Ok(Some(cursor('H'))),
            KeyCode::End => return Ok(Some(cursor('F'))),
            KeyCode::Insert => return Ok(Some(tilde(2))),
            KeyCode::Delete => return Ok(Some(tilde(3))),
            KeyCode::PageUp => return Ok(Some(tilde(5))),
            KeyCode::PageDown => return Ok(Some(tilde(6))),
            KeyCode::BackTab => return Ok(Some(b"\x1b[Z".to_vec())),
            KeyCode::F(n @ 1..=4) => {
                let final_byte = (b'P' + n - 1) as char;
                return Ok(Some(
                    if modifier == 1 {
                        format!("\x1bO{final_byte}")
                    } else {
                        format!("\x1b[1;{modifier}{final_byte}")
                    }
                    .into_bytes(),
                ));
            }
            KeyCode::F(n @ 5..=12) => {
                return Ok(Some(tilde(
                    [15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize],
                )))
            }
            KeyCode::Enter => vec![b'\r'],
            KeyCode::Tab => vec![b'\t'],
            KeyCode::Backspace => vec![if mods.contains(KeyModifiers::CONTROL) {
                8
            } else {
                127
            }],
            KeyCode::Esc => vec![0x1b],
            KeyCode::Null => vec![0],
            KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => {
                let c = c.to_ascii_uppercase();
                if ('4'..='7').contains(&c) {
                    vec![c as u8 - b'4' + 0x1c]
                } else if c == ' ' || c == '@' {
                    vec![0]
                } else if ('A'..='_').contains(&c) {
                    vec![c as u8 & 0x1f]
                } else if c == '?' {
                    vec![127]
                } else {
                    return Ok(None);
                }
            }
            KeyCode::Char(c) => c.to_string().into_bytes(),
            _ => return Ok(None),
        };
        if mods.contains(KeyModifiers::ALT) {
            bytes.insert(0, 0x1b);
        }
        Ok(Some(bytes))
    }

    pub(crate) fn paste(&self, text: String) -> Result<Vec<u8>, SessionError> {
        if text.len() > INPUT_LIMIT - 12 {
            return Err(SessionError::Capacity);
        }
        Ok(if self.screen().bracketed_paste() {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            bytes
        } else {
            text.into_bytes()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn wheel_uses_current_mouse_mode_and_returns_to_host_history_when_disabled() {
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        let up = wheel(MouseEventKind::ScrollUp, 13, 7);
        let down = wheel(MouseEventKind::ScrollDown, 13, 7);
        assert_eq!(terminal.wheel(up), WheelInput::History(-3));
        assert_eq!(terminal.wheel(down), WheelInput::History(3));
        terminal.feed(b"\x1b[?1006h", false).unwrap();
        assert_eq!(terminal.wheel(up), WheelInput::History(-3));
        for mode in [1000, 1002, 1003] {
            terminal
                .feed(format!("\x1b[?{mode}h").as_bytes(), false)
                .unwrap();
            assert_eq!(
                terminal.wheel(up),
                WheelInput::Bytes(b"\x1b[<64;14;8M".to_vec())
            );
            assert_eq!(
                terminal.wheel(down),
                WheelInput::Bytes(b"\x1b[<65;14;8M".to_vec())
            );
            terminal
                .feed(format!("\x1b[?{mode}l").as_bytes(), false)
                .unwrap();
            assert_eq!(terminal.wheel(up), WheelInput::History(-3));
        }
        terminal.feed(b"\x1b[?9h", false).unwrap();
        assert_eq!(terminal.wheel(up), WheelInput::Ignored);
        terminal.reset();
        assert_eq!(terminal.wheel(up), WheelInput::History(-3));
    }

    #[test]
    fn wheel_encodes_modifiers_and_legacy_coordinates_without_truncation() {
        let mut terminal = Terminal::new(TerminalSize::new(240, 500));
        terminal.feed(b"\x1b[?1000h\x1b[?1006h", false).unwrap();
        let mut event = wheel(MouseEventKind::ScrollUp, 499, 239);
        event.modifiers = KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL;
        assert_eq!(
            terminal.wheel(event),
            WheelInput::Bytes(b"\x1b[<92;500;240M".to_vec())
        );
        terminal.feed(b"\x1b[?1006l", false).unwrap();
        assert_eq!(terminal.wheel(event), WheelInput::Ignored);
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollDown, 222, 222)),
            WheelInput::Bytes(vec![0x1b, b'[', b'M', 97, 255, 255])
        );
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollDown, 223, 0)),
            WheelInput::Ignored
        );
        terminal.feed(b"\x1b[?1005h", false).unwrap();
        event.modifiers = KeyModifiers::NONE;
        assert_eq!(
            terminal.wheel(event),
            WheelInput::Bytes("\x1b[M`\u{214}\u{110}".as_bytes().to_vec())
        );
    }

    #[test]
    fn wheel_rejects_non_wheel_and_outside_events_and_keeps_modes_per_terminal() {
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        terminal.feed(b"\x1b[?1000h\x1b[?1006h", false).unwrap();
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::Moved, 0, 0)),
            WheelInput::Ignored
        );
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollUp, 80, 0)),
            WheelInput::Ignored
        );
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollUp, 0, 24)),
            WheelInput::Ignored
        );
        terminal.resize(TerminalSize::new(10, 20));
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollUp, 20, 0)),
            WheelInput::Ignored
        );
        assert_eq!(
            terminal.wheel(wheel(MouseEventKind::ScrollUp, 19, 9)),
            WheelInput::Bytes(b"\x1b[<64;20;10M".to_vec())
        );
        let other = Terminal::new(TerminalSize::new(24, 80));
        assert_eq!(
            other.wheel(wheel(MouseEventKind::ScrollUp, 19, 9)),
            WheelInput::History(-3)
        );
    }

    #[test]
    fn queries_use_query_time_cursor_and_suppress_historical_prefixes() {
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        assert_eq!(
            terminal.feed(b"\x1b[3;7H\x1b[6n\x1b[9;4H", false).unwrap(),
            b"\x1b[3;7R"
        );
        assert!(terminal.feed(b"\x1b[", true).unwrap().is_empty());
        assert!(terminal.feed(b"6n", false).unwrap().is_empty());
        terminal.feed(b"\x1b", true).unwrap();
        terminal.feed(b"\0\x7f", false).unwrap();
        assert!(terminal.feed(b"[6n", false).unwrap().is_empty());
        terminal.feed(b"\x1b", false).unwrap();
        terminal.feed(b"[", true).unwrap();
        assert!(terminal.feed(b"6n", false).unwrap().is_empty());
        assert_eq!(
            terminal
                .feed(b"\x1b[5n\x1b[c\x1b[>c\x1b[18t", false)
                .unwrap(),
            b"\x1b[0n\x1b[?1;2c\x1b[>0;1;0c\x1b[8;24;80t"
        );
    }

    #[test]
    fn modes_unicode_resize_and_gap_reset_remain_per_terminal() {
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        terminal.feed("中文e\u{301}".as_bytes(), false).unwrap();
        assert!(terminal.screen().contents().contains("中文e\u{301}"));
        terminal
            .feed(b"\x1b[?1h\x1b[?2004h\x1b[?1049h", false)
            .unwrap();
        assert_eq!(
            terminal
                .key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .unwrap(),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            terminal.paste("a\nb".into()).unwrap(),
            b"\x1b[200~a\nb\x1b[201~"
        );
        assert!(terminal.screen().alternate_screen());
        terminal.resize(TerminalSize::new(30, 100));
        assert_eq!(terminal.screen().size(), (30, 100));
        terminal.feed(b"\x1b[", false).unwrap();
        terminal.reset();
        assert!(terminal.feed(b"6n", false).unwrap().is_empty());
        assert!(!terminal.screen().alternate_screen());
        assert_eq!(
            terminal
                .key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .unwrap(),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn query_flood_and_paste_are_bounded_without_local_osc_effects() {
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        assert_eq!(
            terminal.feed(&b"\x1b[6n".repeat(3000), false),
            Err(SessionError::Capacity)
        );
        assert_eq!(
            terminal.paste("a".repeat(65536)),
            Err(SessionError::Capacity)
        );
        let mut terminal = Terminal::new(TerminalSize::new(24, 80));
        assert!(terminal
            .feed(b"\x1b]52;c;c2VjcmV0\x07", false)
            .unwrap()
            .is_empty());
        assert!(!terminal.screen().contents().contains("secret"));
        assert_eq!(
            terminal
                .key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .unwrap(),
            Some(vec![3])
        );
        for (key, expected) in [('4', 0x1c), ('5', 0x1d), ('6', 0x1e), ('7', 0x1f), (' ', 0)] {
            assert_eq!(
                terminal
                    .key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL))
                    .unwrap(),
                Some(vec![expected])
            );
        }
    }

    #[test]
    fn unterminated_osc_is_bounded_across_batches_and_ignored_escape_controls() {
        for prefix in [b"\x1b]".as_slice(), b"\x1b\0\x7f]"] {
            let mut terminal = Terminal::new(TerminalSize::new(24, 80));
            for byte in prefix {
                terminal.feed(&[*byte], false).unwrap();
            }
            for _ in 0..16 {
                terminal.feed(&[b'x'; 1024], false).unwrap();
            }
            assert_eq!(terminal.feed(b"x", false), Err(SessionError::Capacity));
            terminal.reset();
            assert_eq!(terminal.feed(b"\x1b[6n", false).unwrap(), b"\x1b[1;1R");
        }
    }

    #[test]
    fn osc_terminators_release_the_budget_and_do_not_emit_local_controls() {
        for ending in [b"\x07".as_slice(), b"\x1b\\", b"\x18", b"\x1a"] {
            let mut terminal = Terminal::new(TerminalSize::new(24, 80));
            for _ in 0..3 {
                terminal.feed(b"\x1b]2;", false).unwrap();
                terminal.feed(&[b'x'; OSC_LIMIT - 2], false).unwrap();
                for byte in ending {
                    assert!(terminal.feed(&[*byte], false).unwrap().is_empty());
                }
            }
            terminal.feed(b"VISIBLE", false).unwrap();
            assert_eq!(terminal.screen().contents(), "VISIBLE");
        }
    }
}
