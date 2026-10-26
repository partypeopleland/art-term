use crate::image_view::CellPixels;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const DEFAULT_FILE: &str = include_str!("../display.default.toml");
const MAX_BYTES: u64 = 16384;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    image_mode: Mode,
    cell_width: Option<u16>,
    cell_height: Option<u16>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Auto,
    Compatible,
    Sixel,
}
#[derive(Default)]
struct Environment {
    ssh: bool,
    multiplexer: bool,
    term: String,
    xterm_version: Option<String>,
}
impl Environment {
    fn current() -> Self {
        Self::from_values(|name| std::env::var_os(name))
    }
    fn from_values(mut get: impl FnMut(&str) -> Option<std::ffi::OsString>) -> Self {
        let term = get("TERM")
            .and_then(|value| value.into_string().ok())
            .unwrap_or_default();
        Self {
            ssh: ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
                .iter()
                .any(|name| get(name).is_some()),
            multiplexer: get("TMUX").is_some()
                || get("STY").is_some()
                || term.starts_with("tmux")
                || term.starts_with("screen"),
            term,
            xterm_version: get("XTERM_VERSION").and_then(|value| value.into_string().ok()),
        }
    }
}

fn resolve(
    settings: Settings,
    measured: Option<(u16, u16)>,
    env: &Environment,
) -> Result<CellPixels> {
    anyhow::ensure!(
        !env.ssh,
        "此 SSH 工作階段僅停用桌面預覽；原生預覽限本地執行 art-term"
    );
    anyhow::ensure!(
        !env.multiplexer,
        "原生預覽需本地直接終端，目前不支援 tmux／screen"
    );
    anyhow::ensure!(
        env.term != "linux" && env.term != "dumb",
        "此純文字主控台不支援原生圖片"
    );
    if !matches!(settings.image_mode, Mode::Sixel) {
        anyhow::ensure!(
            env.term == "vt340" && env.xterm_version.as_deref() == Some("XTerm(390)"),
            "目前本地終端未確認支援 Sixel；確認支援後可在 display.toml 設 image_mode='sixel'"
        );
    }
    let actual = match (settings.cell_width, settings.cell_height) {
        (None, None) => measured
            .context("TTY 未提供字格像素；請在 display.toml 填入實際 cell_width／cell_height")?,
        (Some(width), Some(height)) => (width, height),
        _ => anyhow::bail!("cell_width／cell_height 必須同時填入實際像素，或同時省略"),
    };
    CellPixels::new(actual.0, actual.1).context("字格像素超出支援範圍：寬1..64、高1..128")
}
fn measured_pixels() -> Option<(u16, u16)> {
    let size = crossterm::terminal::window_size().ok()?;
    if size.columns == 0
        || size.rows == 0
        || size.width == 0
        || size.height == 0
        || size.width % size.columns != 0
        || size.height % size.rows != 0
    {
        return None;
    }
    Some((size.width / size.columns, size.height / size.rows))
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
pub(crate) fn load(path: &Path) -> DisplaySelection {
    match read_settings(path)
        .with_context(|| format!("{} 圖片設定無法讀取；原檔保留", path.display()))
        .and_then(|settings| resolve(settings, measured_pixels(), &Environment::current()))
    {
        Ok(pixels) => DisplaySelection::Available(pixels),
        Err(error) => DisplaySelection::Unavailable(format!(
            "原生桌面預覽未啟用；桌面工具仍可使用。\n{error:#}"
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn settings(raw: &str) -> Settings {
        toml::from_str(raw).unwrap()
    }
    fn local() -> Environment {
        Environment {
            term: "vt340".into(),
            xterm_version: Some("XTerm(390)".into()),
            ..Environment::default()
        }
    }
    #[test]
    fn native_auto_and_legacy_alias_require_tested_local_terminal_and_actual_pixels() {
        for raw in [
            DEFAULT_FILE,
            "image_mode='compatible'",
            "image_mode='sixel'",
        ] {
            assert_eq!(
                resolve(settings(raw), Some((8, 16)), &local()).unwrap(),
                CellPixels::new(8, 16).unwrap()
            );
            assert!(resolve(settings(raw), None, &local()).is_err());
        }
        let unknown = Environment {
            term: "xterm-256color".into(),
            ..Environment::default()
        };
        assert!(resolve(settings(DEFAULT_FILE), Some((8, 16)), &unknown).is_err());
        assert!(resolve(settings("image_mode='compatible'"), Some((8, 16)), &unknown).is_err());
        assert!(resolve(settings("image_mode='sixel'"), Some((8, 16)), &unknown).is_ok());
        let explicit = settings("image_mode='sixel'\ncell_width=10\ncell_height=20");
        assert_eq!(
            resolve(explicit, Some((8, 16)), &unknown).unwrap(),
            CellPixels::new(10, 20).unwrap()
        );
        for env in [
            Environment {
                ssh: true,
                ..local()
            },
            Environment {
                multiplexer: true,
                ..local()
            },
            Environment {
                term: "linux".into(),
                ..local()
            },
        ] {
            for raw in [
                DEFAULT_FILE,
                "image_mode='sixel'\ncell_width=8\ncell_height=16",
            ] {
                assert!(resolve(settings(raw), Some((8, 16)), &env).is_err());
            }
        }
        for raw in [
            "image_mode='sixel'\ncell_width=0\ncell_height=20",
            "image_mode='sixel'\ncell_width=10",
            "image_mode='sixel'\ncell_width=100\ncell_height=20",
        ] {
            assert!(resolve(settings(raw), None, &local()).is_err());
        }
    }
    #[test]
    fn each_ssh_indicator_disables_native_preview_even_when_empty() {
        for indicator in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            let env = Environment::from_values(|name| match name {
                "TERM" => Some("vt340".into()),
                "XTERM_VERSION" => Some("XTerm(390)".into()),
                name if name == indicator => Some("".into()),
                _ => None,
            });
            for raw in [DEFAULT_FILE, "image_mode='sixel'"] {
                let error = resolve(settings(raw), Some((8, 16)), &env).unwrap_err();
                assert!(error.to_string().contains("SSH"));
            }
        }
    }
    #[test]
    fn creation_and_errors_preserve_existing_user_file_without_blocking_startup() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = root.path().join("display.toml");
        read_settings(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        for raw in [
            "image_mode='compatible'",
            "image_mode='unknown'",
            "invalid toml",
            "image_mode='sixel'\nunknown=true",
        ] {
            std::fs::write(&path, raw).unwrap();
            let _ = load(&path);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
        assert!(load(&path).notice().unwrap().contains("display.toml"));
    }
    #[test]
    fn unsafe_and_oversized_files_disable_only_preview_and_preserve_contents() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-link-")
            .tempdir_in("/tmp")
            .unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, DEFAULT_FILE).unwrap();
        let path = root.path().join("display.toml");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(load(&path).pixels().is_none());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), DEFAULT_FILE);
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        assert!(load(&path).pixels().is_none());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(load(&path).pixels().is_none());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_BYTES + 1);
    }
}
