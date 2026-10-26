use crate::image_view::CellPixels;
use anyhow::{Context, Result};
use crossterm::event::TerminalResponse;
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant},
};

const DEFAULT_FILE: &str = include_str!("../display.default.toml");
const MAX_BYTES: u64 = 16384;
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const QUERIES: &[u8] = b"\x1b[c\x1b[16t\x1b[14t\x1b[18t";

#[derive(Clone, Debug)]
pub(crate) enum DisplaySelection {
    Available(CellPixels),
    Unavailable(String),
}
impl DisplaySelection {
    pub(crate) fn pixels(&self) -> Option<CellPixels> {
        match self {
            Self::Available(pixels) => Some(*pixels),
            Self::Unavailable(_) => None,
        }
    }
    pub(crate) fn notice(&self) -> Option<&str> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(message) => Some(message),
        }
    }
}

fn unavailable(reason: impl std::fmt::Display) -> DisplaySelection {
    DisplaySelection::Unavailable(format!("原生桌面預覽未啟用；桌面工具仍可使用。\n{reason}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    image_mode: Mode,
    cell_width: Option<u16>,
    cell_height: Option<u16>,
}
#[derive(Deserialize)]
enum Mode {
    #[serde(rename = "auto", alias = "compatible", alias = "sixel")]
    Native,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct DisplayPreferences {
    explicit_cell: Option<CellPixels>,
}
impl Settings {
    fn preferences(self) -> Result<DisplayPreferences> {
        let Mode::Native = self.image_mode;
        let explicit_cell = match (self.cell_width, self.cell_height) {
            (None, None) => None,
            (Some(width), Some(height)) => Some(
                CellPixels::new(width, height)
                    .context("字格像素超出支援範圍：寬1..64、高1..128")?,
            ),
            _ => anyhow::bail!("cell_width／cell_height 必須同時填入實際像素，或同時省略"),
        };
        Ok(DisplayPreferences { explicit_cell })
    }
}

#[derive(Default)]
struct ProbeFacts {
    sixel: Option<bool>,
    direct: Option<CellPixels>,
    area_pixels: Option<(u16, u16)>,
    area_cells: Option<(u16, u16)>,
}
impl ProbeFacts {
    fn fallback_pixels(&self) -> Option<CellPixels> {
        let (width, height) = self.area_pixels?;
        let (columns, rows) = self.area_cells?;
        if columns == 0 || rows == 0 || width % columns != 0 || height % rows != 0 {
            return None;
        }
        CellPixels::new(width / columns, height / rows)
    }
}
enum ProbeState {
    Pending {
        facts: ProbeFacts,
        preferences: DisplayPreferences,
        queries_written: bool,
    },
    Closed(DisplaySelection),
}
pub(crate) struct TerminalProbe {
    deadline: Instant,
    state: ProbeState,
}
impl TerminalProbe {
    pub(crate) fn new(preferences: Result<DisplayPreferences, String>, now: Instant) -> Self {
        Self {
            deadline: now + PROBE_TIMEOUT,
            state: match preferences {
                Ok(preferences) => ProbeState::Pending {
                    facts: ProbeFacts::default(),
                    preferences,
                    queries_written: false,
                },
                Err(reason) => ProbeState::Closed(unavailable(reason)),
            },
        }
    }
    pub(crate) fn initial_selection(&self) -> DisplaySelection {
        match &self.state {
            ProbeState::Pending { .. } => {
                DisplaySelection::Unavailable("正在偵測原生圖片能力…\n桌面工具仍可使用。".into())
            }
            ProbeState::Closed(selection) => selection.clone(),
        }
    }
    pub(crate) fn write_queries(&mut self, output: &mut impl Write) -> Option<DisplaySelection> {
        let ProbeState::Pending {
            queries_written, ..
        } = &mut self.state
        else {
            return None;
        };
        if *queries_written {
            return None;
        }
        *queries_written = true;
        match output.write_all(QUERIES).and_then(|_| output.flush()) {
            Ok(()) => None,
            Err(_) => self.close(unavailable("無法送出終端圖片能力查詢")),
        }
    }
    fn close(&mut self, selection: DisplaySelection) -> Option<DisplaySelection> {
        self.state = ProbeState::Closed(selection.clone());
        Some(selection)
    }
    pub(crate) fn observe(
        &mut self,
        reply: TerminalResponse,
        now: Instant,
    ) -> Option<DisplaySelection> {
        if let Some(selection) = self.expire(now) {
            return Some(selection);
        }
        let ProbeState::Pending { facts, .. } = &mut self.state else {
            return None;
        };
        match reply {
            TerminalResponse::PrimaryDeviceAttributes(attributes) => {
                facts
                    .sixel
                    .get_or_insert_with(|| attributes.iter().skip(1).any(|value| *value == 4));
            }
            TerminalResponse::CellSizePixels { width, height } => {
                if facts.direct.is_none() {
                    facts.direct = CellPixels::new(width, height);
                }
            }
            TerminalResponse::TextAreaPixels { width, height } => {
                facts.area_pixels.get_or_insert((width, height));
            }
            TerminalResponse::TextAreaCells { columns, rows } => {
                facts.area_cells.get_or_insert((columns, rows));
            }
        }
        match facts.sixel {
            Some(false) => self.close(unavailable("終端回覆未宣告支援 Sixel 原生圖片")),
            Some(true) => match facts.direct {
                Some(pixels) => self.close(DisplaySelection::Available(pixels)),
                None => None,
            },
            None => None,
        }
    }
    pub(crate) fn expire(&mut self, now: Instant) -> Option<DisplaySelection> {
        if now < self.deadline {
            return None;
        }
        let ProbeState::Pending {
            facts, preferences, ..
        } = &self.state
        else {
            return None;
        };
        let selection = match facts.sixel {
            Some(true) => match facts
                .direct
                .or_else(|| facts.fallback_pixels())
                .or(preferences.explicit_cell)
            {
                Some(pixels) => DisplaySelection::Available(pixels),
                None => unavailable("終端未在期限內回覆有效的實際字格像素尺寸"),
            },
            Some(false) => unavailable("終端回覆未宣告支援 Sixel 原生圖片"),
            None => unavailable("終端圖片能力偵測逾時，未收到有效的 Sixel 支援回覆"),
        };
        self.close(selection)
    }
}

fn read_settings(path: &Path) -> Result<Settings> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(DEFAULT_FILE.as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid takes no pointers and has no caller preconditions.
    let uid = unsafe { libc::geteuid() };
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == uid
            && metadata.nlink() == 1
            && metadata.len() <= MAX_BYTES,
        "圖片設定須為自己擁有、單一連結且不超過16KiB的普通檔案"
    );
    let mut bytes = Vec::new();
    (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_BYTES, "圖片設定超過16KiB");
    let raw = std::str::from_utf8(&bytes)?;
    Ok(toml::from_str(raw)?)
}
pub(crate) fn load_preferences(path: &Path) -> Result<DisplayPreferences, String> {
    read_settings(path)
        .and_then(Settings::preferences)
        .with_context(|| format!("{} 圖片設定無法讀取；原檔保留", path.display()))
        .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preferences(raw: &str) -> Result<DisplayPreferences, String> {
        toml::from_str::<Settings>(raw)
            .map_err(anyhow::Error::from)
            .and_then(Settings::preferences)
            .map_err(|error| error.to_string())
    }
    fn new_probe(now: Instant) -> TerminalProbe {
        TerminalProbe::new(preferences(DEFAULT_FILE), now)
    }
    fn da(attributes: &[u16]) -> TerminalResponse {
        TerminalResponse::PrimaryDeviceAttributes(attributes.to_vec())
    }
    fn cell(width: u16, height: u16) -> TerminalResponse {
        TerminalResponse::CellSizePixels { width, height }
    }
    fn pixels(selection: Option<DisplaySelection>) -> Option<CellPixels> {
        selection.and_then(|selection| selection.pixels())
    }
    #[test]
    fn every_config_mode_requires_real_sixel_attribute_and_direct_pixels() {
        let now = Instant::now();
        for raw in [
            DEFAULT_FILE,
            "image_mode='compatible'",
            "image_mode='sixel'",
        ] {
            let mut probe = TerminalProbe::new(preferences(raw), now);
            assert!(probe.observe(cell(10, 20), now).is_none());
            assert_eq!(
                pixels(probe.observe(da(&[61, 4, 6]), now)),
                CellPixels::new(10, 20)
            );
            assert!(probe.observe(cell(8, 16), now).is_none());
            assert!(probe.expire(now + PROBE_TIMEOUT).is_none());
        }
        for attributes in [&[4][..], &[61, 6][..], &[][..]] {
            let mut probe = new_probe(now);
            let selection = probe.observe(da(attributes), now).unwrap();
            assert!(selection.pixels().is_none());
            assert!(selection.notice().unwrap().contains("未宣告"));
        }
    }
    #[test]
    fn fallback_waits_for_deadline_and_direct_report_has_precedence_over_settings() {
        let now = Instant::now();
        let mut probe = TerminalProbe::new(
            preferences("image_mode='sixel'\ncell_width=8\ncell_height=16"),
            now,
        );
        assert!(probe.observe(da(&[61, 4]), now).is_none());
        assert!(probe
            .observe(
                TerminalResponse::TextAreaPixels {
                    width: 1200,
                    height: 720
                },
                now
            )
            .is_none());
        assert!(probe
            .observe(
                TerminalResponse::TextAreaCells {
                    columns: 120,
                    rows: 36
                },
                now
            )
            .is_none());
        assert!(probe
            .expire(now + PROBE_TIMEOUT - Duration::from_nanos(1))
            .is_none());
        assert_eq!(
            pixels(probe.observe(cell(9, 18), now + Duration::from_millis(900))),
            CellPixels::new(9, 18)
        );
        assert!(probe.expire(now + PROBE_TIMEOUT).is_none());
    }
    #[test]
    fn only_exact_valid_area_measurements_can_supply_fallback_geometry() {
        let now = Instant::now();
        for (width, height, columns, rows, expected) in [
            (1200, 720, 120, 36, CellPixels::new(10, 20)),
            (1201, 720, 120, 36, None),
            (1200, 721, 120, 36, None),
            (0, 720, 120, 36, None),
            (1200, 720, 0, 36, None),
            (1200, 720, 120, 0, None),
            (12000, 720, 120, 36, None),
        ] {
            let mut probe = new_probe(now);
            probe.observe(da(&[61, 4]), now);
            probe.observe(TerminalResponse::TextAreaPixels { width, height }, now);
            probe.observe(TerminalResponse::TextAreaCells { columns, rows }, now);
            assert_eq!(pixels(probe.expire(now + PROBE_TIMEOUT)), expected);
        }
    }
    #[test]
    fn explicit_pixels_are_last_fallback_and_never_replace_positive_capability() {
        let now = Instant::now();
        let settings = "image_mode='sixel'\ncell_width=8\ncell_height=16";
        let mut explicit = TerminalProbe::new(preferences(settings), now);
        explicit.observe(da(&[61, 4]), now);
        assert_eq!(
            pixels(explicit.expire(now + PROBE_TIMEOUT)),
            CellPixels::new(8, 16)
        );
        let mut area = TerminalProbe::new(preferences(settings), now);
        area.observe(da(&[61, 4]), now);
        area.observe(
            TerminalResponse::TextAreaPixels {
                width: 1200,
                height: 720,
            },
            now,
        );
        area.observe(
            TerminalResponse::TextAreaCells {
                columns: 120,
                rows: 36,
            },
            now,
        );
        assert_eq!(
            pixels(area.expire(now + PROBE_TIMEOUT)),
            CellPixels::new(10, 20)
        );
        let mut absent = TerminalProbe::new(preferences(settings), now);
        assert!(absent
            .expire(now + PROBE_TIMEOUT)
            .unwrap()
            .pixels()
            .is_none());
        let mut negative = TerminalProbe::new(preferences(settings), now);
        assert!(negative
            .observe(da(&[61, 6]), now)
            .unwrap()
            .pixels()
            .is_none());
    }
    #[test]
    fn absolute_deadline_ignores_late_reports_without_restarting_or_changing_selection() {
        let now = Instant::now();
        let mut probe = new_probe(now);
        probe.observe(da(&[61, 4]), now);
        for milliseconds in 1..1000 {
            assert!(probe
                .observe(cell(0, 0), now + Duration::from_millis(milliseconds))
                .is_none());
        }
        let selection = probe.observe(cell(10, 20), now + PROBE_TIMEOUT).unwrap();
        assert!(selection.pixels().is_none());
        assert!(selection.notice().unwrap().contains("期限"));
        assert!(probe
            .observe(cell(10, 20), now + Duration::from_secs(2))
            .is_none());
        assert!(probe
            .observe(da(&[61, 4]), now + Duration::from_secs(2))
            .is_none());
        assert!(probe.expire(now + Duration::from_secs(3)).is_none());
        let mut silent = new_probe(now);
        assert!(silent
            .expire(now + PROBE_TIMEOUT)
            .unwrap()
            .notice()
            .unwrap()
            .contains("逾時"));
    }
    #[test]
    fn invalid_direct_pixels_do_not_enable_images_and_can_be_followed_by_valid_measurement() {
        let now = Instant::now();
        let mut probe = new_probe(now);
        probe.observe(da(&[61, 4]), now);
        for (width, height) in [(0, 20), (10, 0), (65, 20), (10, 129), (u16::MAX, u16::MAX)] {
            assert!(probe.observe(cell(width, height), now).is_none());
        }
        assert_eq!(
            pixels(probe.observe(cell(10, 20), now)),
            CellPixels::new(10, 20)
        );
    }
    #[test]
    fn queries_are_once_and_write_failure_disables_only_preview() {
        let now = Instant::now();
        let mut probe = new_probe(now);
        let mut output = Vec::new();
        assert!(probe.write_queries(&mut output).is_none());
        assert!(probe.write_queries(&mut output).is_none());
        assert_eq!(output, QUERIES);
        struct Refuse;
        impl Write for Refuse {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut failed = new_probe(now);
        let selection = failed.write_queries(&mut Refuse).unwrap();
        assert!(selection.pixels().is_none());
        assert!(selection.notice().unwrap().contains("桌面工具仍可使用"));
        assert!(failed.observe(da(&[61, 4]), now).is_none());
    }
    #[test]
    fn creation_and_errors_preserve_user_file_and_do_not_probe_invalid_settings() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = root.path().join("display.toml");
        assert!(load_preferences(&path).is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        std::fs::write(&path, "image_mode='compatible'").unwrap();
        assert!(load_preferences(&path).is_ok());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "image_mode='compatible'"
        );
        for raw in [
            "image_mode='unknown'",
            "invalid toml",
            "image_mode='sixel'\nunknown=true",
            "image_mode='auto'\ncell_width=10",
            "image_mode='auto'\ncell_width=0\ncell_height=20",
        ] {
            std::fs::write(&path, raw).unwrap();
            let mut probe = TerminalProbe::new(load_preferences(&path), Instant::now());
            let selection = probe.initial_selection();
            assert!(selection.pixels().is_none());
            assert!(selection.notice().unwrap().contains("display.toml"));
            let mut output = Vec::new();
            assert!(probe.write_queries(&mut output).is_none());
            assert!(output.is_empty());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
    }
    #[test]
    fn unsafe_and_oversized_files_preserve_contents_and_return_nonfatal_preferences_error() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-link-")
            .tempdir_in("/tmp")
            .unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, DEFAULT_FILE).unwrap();
        let path = root.path().join("display.toml");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(load_preferences(&path).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), DEFAULT_FILE);
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        assert!(load_preferences(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(load_preferences(&path).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_BYTES + 1);
    }
}
