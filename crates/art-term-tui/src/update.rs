use art_term_core::release::{self, Platform, ReleaseClient, UpdateStatus};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(crate) struct UpdateCheck {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
}

pub(crate) struct UpdateInstall {
    pub version: String,
    pub backup: PathBuf,
}

static INSTALLED_VERSION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn installed_version() -> Option<String> {
    INSTALLED_VERSION.lock().ok().and_then(|guard| guard.clone())
}

#[derive(Deserialize)]
struct LinuxManifest {
    version: String,
    archive: String,
    binary_sha256: String,
}

pub(crate) async fn check() -> Result<UpdateCheck, String> {
    let current = env!("CARGO_PKG_VERSION");
    if let Some(installed) = installed_version() {
        return Ok(UpdateCheck {
            current: current.to_string(),
            latest: Some(installed),
            available: false,
        });
    }
    let client = ReleaseClient::new().map_err(|error| error.to_string())?;
    match client
        .check_for_update(Platform::Linux, current)
        .await
        .map_err(|error| error.to_string())?
    {
        UpdateStatus::UpToDate { latest, .. } => Ok(UpdateCheck {
            current: current.to_string(),
            latest: Some(latest.to_string()),
            available: false,
        }),
        UpdateStatus::Available { latest, .. } => Ok(UpdateCheck {
            current: current.to_string(),
            latest: Some(latest.to_string()),
            available: true,
        }),
    }
}

pub(crate) async fn install() -> Result<UpdateInstall, String> {
    let current = env!("CARGO_PKG_VERSION");
    if let Some(installed) = installed_version() {
        return Err(format!(
            "已在本次執行中安裝 art-term {installed}，請重新啟動後再檢查更新。"
        ));
    }
    let client = ReleaseClient::new().map_err(|error| error.to_string())?;
    let status = client
        .check_for_update(Platform::Linux, current)
        .await
        .map_err(|error| error.to_string())?;
    let UpdateStatus::Available { release, .. } = status else {
        return Err("目前已是最新版本，沒有可安裝的更新。".to_string());
    };
    let version = release.version.to_string();
    let archive_name = format!("art-term-{version}-linux-x86_64.tar.gz");
    let manifest_name = release::linux_manifest_name(&release.version);
    let sums = client
        .sha256_sums(&release)
        .await
        .map_err(|error| error.to_string())?;
    let archive = release
        .asset(&archive_name)
        .ok_or_else(|| format!("release is missing asset {archive_name}"))?;
    let manifest_asset = release
        .asset(&manifest_name)
        .ok_or_else(|| format!("release is missing asset {manifest_name}"))?;
    let expected = |name: &str| {
        sums.get(name)
            .cloned()
            .ok_or_else(|| format!("SHA256SUMS.txt has no entry for {name}"))
    };
    let staging = tempfile::tempdir().map_err(|error| error.to_string())?;
    let archive_path = staging.path().join(&archive_name);
    let manifest_path = staging.path().join(&manifest_name);
    client
        .download_verified_asset(archive, &archive_path, &expected(&archive_name)?)
        .await
        .map_err(|error| error.to_string())?;
    client
        .download_verified_asset(manifest_asset, &manifest_path, &expected(&manifest_name)?)
        .await
        .map_err(|error| error.to_string())?;
    let manifest_text = tokio::fs::read_to_string(&manifest_path)
        .await
        .map_err(|error| error.to_string())?;
    let manifest: LinuxManifest = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("invalid linux manifest: {error}"))?;
    if manifest.version != version || manifest.archive != archive_name {
        return Err("linux manifest does not match the selected release".to_string());
    }
    let extract = staging.path().join("extract");
    std::fs::create_dir_all(&extract).map_err(|error| error.to_string())?;
    let extracted = tokio::process::Command::new("tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract)
        .status()
        .await
        .map_err(|error| format!("無法執行 tar：{error}"))?;
    if !extracted.success() {
        return Err(format!("tar 解壓失敗：{extracted}"));
    }
    let bundle = extract.join(format!("art-term-{version}-linux-x86_64"));
    let new_binary = bundle.join("art-term");
    if !new_binary.is_file() {
        return Err("更新封裝內找不到 art-term 執行檔。".to_string());
    }
    let actual = release::sha256_file(&new_binary)
        .await
        .map_err(|error| error.to_string())?;
    if !actual.eq_ignore_ascii_case(manifest.binary_sha256.trim()) {
        return Err("更新後的執行檔 SHA-256 與 manifest 不符，已取消安裝。".to_string());
    }
    let install = replace_binary(&new_binary, current, &version).await?;
    if let Ok(mut guard) = INSTALLED_VERSION.lock() {
        *guard = Some(version);
    }
    Ok(install)
}

async fn replace_binary(
    new_binary: &Path,
    current: &str,
    version: &str,
) -> Result<UpdateInstall, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let parent = exe
        .parent()
        .ok_or_else(|| "無法判斷執行檔目錄。".to_string())?;
    let file_name = exe
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "art-term".to_string());
    let backup = parent.join(format!("{file_name}.bak-{current}"));
    std::fs::copy(&exe, &backup)
        .map_err(|error| format!("無法建立舊版備份 {}：{error}", backup.display()))?;
    let install_dir = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let staged = install_dir.path().join(&file_name);
    std::fs::copy(new_binary, &staged).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    std::fs::File::open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(&staged, &exe).map_err(|error| {
        format!(
            "無法替換執行檔（目錄可能不可寫）：{error}。舊版仍保留在 {}。",
            backup.display()
        )
    })?;
    let _ = std::fs::File::open(parent).and_then(|dir| dir.sync_all());
    Ok(UpdateInstall {
        version: version.to_string(),
        backup,
    })
}
