use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const DEFAULT_FILE: &str = include_str!("../keybindings.default.toml");
const MAX_FILE_BYTES: u64 = 65536;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BindingAction {
    Hosts,
    Terminal,
    Preview,
    Screens,
    Shells,
    Create,
    Previous,
    Next,
    Close,
    Up,
    Down,
    Follow,
    Recover,
    Import,
    Export,
    Help,
    Quit,
    Actions,
    Edit,
    Delete,
    Test,
    Refresh,
    NewHost,
    NewAgent,
    NewScreen,
    ViewerToggle,
    ViewerFit,
    FocusNext,
    TreeUp,
    TreeDown,
    TreeActivate,
    #[serde(rename = "tab_1")]
    Tab1,
    #[serde(rename = "tab_2")]
    Tab2,
    #[serde(rename = "tab_3")]
    Tab3,
    #[serde(rename = "tab_4")]
    Tab4,
    #[serde(rename = "tab_5")]
    Tab5,
    #[serde(rename = "tab_6")]
    Tab6,
    #[serde(rename = "tab_7")]
    Tab7,
    #[serde(rename = "tab_8")]
    Tab8,
    #[serde(rename = "tab_9")]
    Tab9,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Navigation {
    Up,
    Down,
    Activate,
    Cancel,
    NextField,
    PreviousField,
    Save,
    ToggleReplace,
}
impl Navigation {
    fn key(self) -> KeyEvent {
        let (code, modifiers) = match self {
            Self::Up => (KeyCode::Up, KeyModifiers::NONE),
            Self::Down => (KeyCode::Down, KeyModifiers::NONE),
            Self::Activate => (KeyCode::Enter, KeyModifiers::NONE),
            Self::Cancel => (KeyCode::Esc, KeyModifiers::NONE),
            Self::NextField => (KeyCode::Tab, KeyModifiers::NONE),
            Self::PreviousField => (KeyCode::BackTab, KeyModifiers::SHIFT),
            Self::Save => (KeyCode::Char('s'), KeyModifiers::CONTROL),
            Self::ToggleReplace => (KeyCode::Char(' '), KeyModifiers::NONE),
        };
        KeyEvent::new(code, modifiers)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    leader: Option<String>,
    #[serde(default)]
    bindings: BTreeMap<BindingAction, Vec<String>>,
    #[serde(default)]
    navigation: BTreeMap<Navigation, Vec<String>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    code: KeyCode,
    modifiers: KeyModifiers,
    leader: bool,
}
impl Chord {
    fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
    fn matches(self, key: KeyEvent, leader: bool) -> bool {
        let key = normalize(key);
        self.leader == leader && self.code == key.code && self.modifiers == key.modifiers
    }
}
fn normalize(mut key: KeyEvent) -> KeyEvent {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        key.code = match key.code {
            KeyCode::Char('4') => KeyCode::Char('\\'),
            KeyCode::Char('5') => KeyCode::Char(']'),
            KeyCode::Char('6') => KeyCode::Char('^'),
            KeyCode::Char('7') => KeyCode::Char('_'),
            other => other,
        };
        if let KeyCode::Char(c) = key.code {
            key.code = KeyCode::Char(c.to_ascii_lowercase());
        }
        let conventional = match key.code {
            KeyCode::Char('i') => Some(KeyCode::Tab),
            KeyCode::Char('m') => Some(KeyCode::Enter),
            KeyCode::Char('[' | '3') => Some(KeyCode::Esc),
            KeyCode::Char('h' | '?' | '8') => Some(KeyCode::Backspace),
            KeyCode::Char('@' | '2') => {
                key.code = KeyCode::Char(' ');
                None
            }
            _ => None,
        };
        if let Some(code) = conventional {
            key.code = code;
            key.modifiers.remove(KeyModifiers::CONTROL);
        }
    }
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            if c.is_uppercase() {
                key.modifiers.insert(KeyModifiers::SHIFT);
            } else if c.is_ascii_punctuation() {
                key.modifiers.remove(KeyModifiers::SHIFT);
            }
        }
    }
    key
}
fn chord(value: &str) -> Result<Chord> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 128,
        "empty or oversized shortcut"
    );
    let mut rest = value;
    let mut modifiers = KeyModifiers::NONE;
    let mut leader = false;
    while let Some((head, tail)) = rest.split_once('+') {
        match head.to_ascii_lowercase().as_str() {
            "leader" if !leader => leader = true,
            "ctrl" | "control" if !modifiers.contains(KeyModifiers::CONTROL) => {
                modifiers.insert(KeyModifiers::CONTROL)
            }
            "alt" if !modifiers.contains(KeyModifiers::ALT) => modifiers.insert(KeyModifiers::ALT),
            "shift" if !modifiers.contains(KeyModifiers::SHIFT) => {
                modifiers.insert(KeyModifiers::SHIFT)
            }
            _ => bail!("invalid shortcut modifier in {value:?}"),
        }
        rest = tail;
    }
    let lower = rest.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "tab" if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "backtab" => {
            modifiers.insert(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ if lower.starts_with('f') && lower.len() > 1 => {
            let number: u8 = lower[1..].parse().context("function key must be F1..F24")?;
            anyhow::ensure!((1..=24).contains(&number), "function key must be F1..F24");
            KeyCode::F(number)
        }
        _ => {
            let mut chars = rest.chars();
            let c = chars.next().context("missing key")?;
            anyhow::ensure!(
                chars.next().is_none() && !c.is_control(),
                "unknown key {rest:?}"
            );
            KeyCode::Char(if modifiers.contains(KeyModifiers::SHIFT) {
                c.to_ascii_uppercase()
            } else {
                c
            })
        }
    };
    let event = normalize(KeyEvent::new(code, modifiers));
    Ok(Chord {
        code: event.code,
        modifiers: event.modifiers,
        leader,
    })
}

pub(crate) struct Bindings {
    leader: Chord,
    leader_text: String,
    actions: Vec<(Chord, BindingAction)>,
    labels: BTreeMap<BindingAction, Vec<String>>,
    navigation: Vec<(Chord, Navigation)>,
}
impl Default for Bindings {
    fn default() -> Self {
        Self::parse(DEFAULT_FILE).expect("bundled keybindings are valid")
    }
}
impl Bindings {
    fn parse(raw: &str) -> Result<Self> {
        let mut defaults: File = toml::from_str(DEFAULT_FILE)?;
        let input: File = toml::from_str(raw)
            .context("invalid keybindings TOML; check action names and line indicated below")?;
        defaults.bindings.extend(input.bindings);
        defaults.navigation.extend(input.navigation);
        let leader_text = input.leader.or(defaults.leader).context("missing leader")?;
        let leader = chord(&leader_text).context("invalid leader")?;
        anyhow::ensure!(!leader.leader, "leader cannot itself contain leader+");
        let mut actions = Vec::new();
        let mut labels = BTreeMap::new();
        for (action, keys) in defaults.bindings {
            for key in &keys {
                let parsed = chord(key).with_context(|| format!("invalid {action:?} shortcut"))?;
                anyhow::ensure!(
                    !actions.iter().any(|(other, _)| *other == parsed),
                    "duplicate shortcut {key:?} for {action:?}"
                );
                anyhow::ensure!(
                    parsed.leader || parsed != leader,
                    "shortcut {key:?} conflicts with leader"
                );
                actions.push((parsed, action));
            }
            labels.insert(action, keys);
        }
        let mut navigation = Vec::new();
        for (action, keys) in defaults.navigation {
            for key in keys {
                let parsed =
                    chord(&key).with_context(|| format!("invalid navigation {action:?}"))?;
                anyhow::ensure!(!parsed.leader, "navigation cannot contain leader+");
                anyhow::ensure!(
                    !navigation.iter().any(|(other, _)| *other == parsed),
                    "duplicate navigation shortcut {key:?}"
                );
                navigation.push((parsed, action));
            }
        }
        Ok(Self {
            leader,
            leader_text,
            actions,
            labels,
            navigation,
        })
    }
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let context = || {
            format!("Unable to load {}. Edit the keybindings TOML or restore the bundled defaults; existing file was preserved", path.display())
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(DEFAULT_FILE.as_bytes())
                    .with_context(context)?;
                file.sync_all().with_context(context)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error).with_context(context),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .with_context(context)?;
        let metadata = file.metadata().with_context(context)?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.nlink() == 1
                && metadata.len() <= MAX_FILE_BYTES,
            "{} must be an owned regular keybindings file of at most 64 KiB",
            path.display()
        );
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(context)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "keybindings file exceeds64KiB"
        );
        let raw = std::str::from_utf8(&bytes).with_context(context)?;
        Self::parse(raw).map_err(|error| anyhow::anyhow!("{}: {error:#}", context()))
    }
    pub(crate) fn action(&self, key: KeyEvent, leader_active: bool) -> Option<BindingAction> {
        self.actions
            .iter()
            .find_map(|(chord, action)| chord.matches(key, leader_active).then_some(*action))
    }
    pub(crate) fn is_leader(&self, key: KeyEvent) -> bool {
        self.leader.matches(key, false)
    }
    pub(crate) fn literal_leader(&self) -> KeyEvent {
        self.leader.event()
    }
    pub(crate) fn leader_label(&self) -> &str {
        &self.leader_text
    }
    pub(crate) fn label(&self, action: BindingAction) -> String {
        self.labels
            .get(&action)
            .and_then(|keys| keys.first())
            .map(|key| key.replace("leader+", &format!("{} → ", self.leader_text)))
            .unwrap_or_else(|| "未綁定".into())
    }
    pub(crate) fn navigation_key(&self, key: KeyEvent) -> KeyEvent {
        self.navigation
            .iter()
            .find_map(|(chord, action)| {
                chord.matches(key, false).then(|| {
                    let mut mapped = action.key();
                    mapped.kind = key.kind;
                    mapped
                })
            })
            .unwrap_or(key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn ssh_control_aliases_match_actual_terminal_keys_and_conflicts_are_rejected() {
        for (shortcut, code) in [
            ("Ctrl+i", KeyCode::Tab),
            ("Ctrl+m", KeyCode::Enter),
            ("Ctrl+[", KeyCode::Esc),
            ("Ctrl+h", KeyCode::Backspace),
        ] {
            let b = Bindings::parse(&format!("[bindings]\nhelp=[{shortcut:?}]")).unwrap();
            assert_eq!(
                b.action(KeyEvent::new(code, KeyModifiers::NONE), false),
                Some(BindingAction::Help)
            );
        }
        assert!(Bindings::parse("[bindings]\nhelp=['Ctrl+i','Tab']").is_err());
        let b = Bindings::parse("[bindings]\nhelp=['Ctrl+j']").unwrap();
        assert_eq!(
            b.action(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                false
            ),
            Some(BindingAction::Help)
        );
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false),
            None
        );
    }

    #[test]
    fn uppercase_and_punctuation_match_terminal_events_and_detect_alias_conflicts() {
        let b = Bindings::parse("[bindings]\nhelp=[\"H\"]").unwrap();
        assert_eq!(
            b.action(
                KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT),
                false
            ),
            Some(BindingAction::Help)
        );
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE), false),
            Some(BindingAction::Help)
        );
        assert!(Bindings::parse("[bindings]\nhelp=[\"H\",\"Shift+h\"]").is_err());
        assert_eq!(
            Bindings::default()
                .action(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT), true),
            Some(BindingAction::Help)
        );
    }
    #[test]
    fn defaults_and_terminal_control_alias_use_same_leader() {
        let b = Bindings::default();
        assert!(b.is_leader(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL)));
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), true),
            Some(BindingAction::Hosts)
        );
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE), false),
            Some(BindingAction::Screens)
        );
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), false),
            None
        );
    }
    #[test]
    fn custom_leader_direct_keys_disabled_actions_and_navigation() {
        let b = Bindings::parse(
            "leader='Ctrl+g'\n[bindings]\nhosts=['Alt+h']\nhelp=[]\n[navigation]\nup=['k']",
        )
        .unwrap();
        assert!(b.is_leader(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)));
        assert!(!b.is_leader(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL)));
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT), false),
            Some(BindingAction::Hosts)
        );
        assert_eq!(
            b.action(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), false),
            None
        );
        assert_eq!(
            b.navigation_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
                .code,
            KeyCode::Up
        );
    }
    #[test]
    fn malformed_unknown_and_conflicting_bindings_fail() {
        for raw in [
            "[bindings]\nhelp=['leader+h']",
            "[bindings]\nunknown=['F1']",
            "[bindings]\nhelp=['Ctrl++']",
            "[bindings]\nhelp=['F25']",
            "[navigation]\nup=['Down']",
            "leader='leader+x'",
        ] {
            assert!(Bindings::parse(raw).is_err(), "{raw}")
        }
    }
    #[test]
    fn loading_creates_once_and_invalid_file_remains_byte_identical() {
        let root = tempfile::Builder::new()
            .prefix("art-term-keybindings-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = root.path().join("keybindings.toml");
        Bindings::load(&path).unwrap();
        fs::write(&path, "[bindings]\nhelp=['Alt+j']\n").unwrap();
        Bindings::load(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("Alt+j"));
        fs::write(&path, "[bindings]\nhelp=['leader+h']\n").unwrap();
        let before = fs::read(&path).unwrap();
        let error = Bindings::load(&path).err().unwrap();
        assert!(format!("{error:#}").contains("keybindings.toml"));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    #[test]
    fn special_file_and_symlink_are_rejected_without_following() {
        let root = tempfile::Builder::new()
            .prefix("art-term-keybindings-")
            .tempdir_in("/tmp")
            .unwrap();
        let actual = root.path().join("actual");
        fs::write(&actual, DEFAULT_FILE).unwrap();
        let path = root.path().join("keybindings.toml");
        std::os::unix::fs::symlink(&actual, &path).unwrap();
        assert!(Bindings::load(&path).is_err());
        assert_eq!(fs::read_to_string(actual).unwrap(), DEFAULT_FILE);
    }
}
