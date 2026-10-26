use crate::image_view::ImageMode;
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
    Compatible,
    Sixel,
}

fn resolve(
    settings: Settings,
    measured: Option<(u16, u16)>,
    multiplexer: bool,
) -> Result<ImageMode> {
    let explicit = match (settings.cell_width, settings.cell_height) {
        (None, None) => None,
        (Some(width), Some(height)) => {
            anyhow::ensure!(
                (1..=64).contains(&width) && (1..=128).contains(&height),
                "cell_width must be1..64 and cell_height1..128 actual pixels"
            );
            Some((width, height))
        }
        _ => anyhow::bail!("Set both cell_width and cell_height, or omit both"),
    };
    match settings.image_mode {
        Mode::Compatible => Ok(ImageMode::Compatible),
        Mode::Sixel => {
            anyhow::ensure!(!multiplexer,"Sixel mode currently requires a direct terminal. Use image_mode='compatible' inside tmux/screen; no multiplexer setting was changed");
            let (cell_width,cell_height)=explicit.or(measured).context("TTY did not provide cell pixels. Set actual cell_width/cell_height, or use image_mode='compatible'")?;
            anyhow::ensure!(
                (1..=64).contains(&cell_width) && (1..=128).contains(&cell_height),
                "TTY cell pixel dimensions are outside supported limits"
            );
            Ok(ImageMode::Sixel {
                cell_width,
                cell_height,
            })
        }
    }
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
pub(crate) fn load(path: &Path) -> Result<ImageMode> {
    let context = || {
        format!("Unable to load {}. Edit display.toml or restore compatible mode; existing file was preserved",path.display())
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
    // SAFETY: geteuid takes no pointers and has no caller preconditions.
    let uid = unsafe { libc::geteuid() };
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == uid
            && metadata.nlink() == 1
            && metadata.len() <= MAX_BYTES,
        "{} must be an owned regular display file no larger than16KiB",
        path.display()
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(context)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "{} exceeds16KiB",
        path.display()
    );
    let raw = std::str::from_utf8(&bytes).with_context(context)?;
    let settings: Settings =
        toml::from_str(raw).map_err(|error| anyhow::anyhow!("{}: {error}", context()))?;
    let multiplexer = std::env::var_os("TMUX").is_some()
        || std::env::var_os("STY").is_some()
        || std::env::var("TERM")
            .is_ok_and(|term| term.starts_with("tmux") || term.starts_with("screen"));
    resolve(settings, measured_pixels(), multiplexer)
        .map_err(|error| anyhow::anyhow!("{}: {error:#}", context()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn settings(raw: &str) -> Settings {
        toml::from_str(raw).unwrap()
    }
    #[test]
    fn compatible_needs_no_probe_and_native_requires_actual_valid_metrics() {
        assert_eq!(
            resolve(settings(DEFAULT_FILE), None, true).unwrap(),
            ImageMode::Compatible
        );
        assert!(resolve(settings("image_mode='sixel'"), None, false).is_err());
        assert_eq!(
            resolve(settings("image_mode='sixel'"), Some((8, 16)), false).unwrap(),
            ImageMode::Sixel {
                cell_width: 8,
                cell_height: 16
            }
        );
        assert_eq!(
            resolve(
                settings("image_mode='sixel'\ncell_width=10\ncell_height=20"),
                Some((8, 16)),
                false
            )
            .unwrap(),
            ImageMode::Sixel {
                cell_width: 10,
                cell_height: 20
            }
        );
        assert!(resolve(
            settings("image_mode='sixel'\ncell_width=10\ncell_height=20"),
            None,
            true
        )
        .is_err());
        for raw in [
            "image_mode='sixel'\ncell_width=0\ncell_height=20",
            "image_mode='sixel'\ncell_width=10",
            "image_mode='sixel'\ncell_width=100\ncell_height=20",
        ] {
            assert!(resolve(settings(raw), None, false).is_err());
        }
    }
    #[test]
    fn creation_and_errors_preserve_existing_user_file() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = root.path().join("display.toml");
        assert_eq!(load(&path).unwrap(), ImageMode::Compatible);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
        std::fs::write(&path, "image_mode='unknown'").unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = load(&path).unwrap_err();
        assert!(format!("{error:#}").contains("display.toml"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn links_are_rejected_without_touching_the_destination() {
        let root = tempfile::Builder::new()
            .prefix("art-term-display-link-")
            .tempdir_in("/tmp")
            .unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, DEFAULT_FILE).unwrap();
        let path = root.path().join("display.toml");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), DEFAULT_FILE);
    }
}
