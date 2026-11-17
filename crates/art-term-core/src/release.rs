use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const REPOSITORY: &str = "partypeopleland/art-term";
pub const LATEST_JSON_NAME: &str = "latest.json";
pub const SHA256SUMS_NAME: &str = "SHA256SUMS.txt";
const API_BASE: &str = "https://api.github.com";
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const USER_AGENT: &str = "art-term-updater";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Android,
    Linux,
    Windows,
}

impl Platform {
    pub fn required_asset_names(self, version: &ReleaseVersion) -> Vec<String> {
        match self {
            Self::Android => vec![
                format!("art-term-{version}-android-arm64.apk"),
                SHA256SUMS_NAME.to_string(),
            ],
            Self::Linux => vec![
                format!("art-term-{version}-linux-x86_64.tar.gz"),
                linux_manifest_name(version),
                SHA256SUMS_NAME.to_string(),
            ],
            Self::Windows => vec![
                format!("art-term-{version}-windows-x64-setup.exe"),
                LATEST_JSON_NAME.to_string(),
            ],
        }
    }
}

pub fn linux_manifest_name(version: &ReleaseVersion) -> String {
    format!("art-term-{version}-linux-x86_64.manifest.json")
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct ReleaseVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl ReleaseVersion {
    pub fn parse(input: &str) -> Result<Self, ReleaseError> {
        let value = input.trim();
        let value = value.strip_prefix('v').unwrap_or(value);
        let mut parts = value.split('.');
        let version = Self {
            major: version_part(parts.next(), "major", input)?,
            minor: version_part(parts.next(), "minor", input)?,
            patch: version_part(parts.next(), "patch", input)?,
        };
        if parts.next().is_some() {
            return Err(ReleaseError::InvalidVersion(format!(
                "too many components in {input}"
            )));
        }
        Ok(version)
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn version_part(part: Option<&str>, label: &str, input: &str) -> Result<u64, ReleaseError> {
    let part = part.ok_or_else(|| {
        ReleaseError::InvalidVersion(format!("missing {label} in {input}"))
    })?;
    if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
        return Err(ReleaseError::InvalidVersion(format!(
            "invalid {label} in {input}"
        )));
    }
    part.parse::<u64>().map_err(|_| {
        ReleaseError::InvalidVersion(format!("invalid {label} in {input}"))
    })
}

#[derive(Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct LatestRelease {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub version: ReleaseVersion,
    pub assets: Vec<ReleaseAsset>,
}

impl LatestRelease {
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(Clone, Debug)]
pub enum UpdateStatus {
    UpToDate {
        current: ReleaseVersion,
        latest: ReleaseVersion,
    },
    Available {
        current: ReleaseVersion,
        latest: ReleaseVersion,
        release: LatestRelease,
        assets: Vec<ReleaseAsset>,
    },
}

#[derive(Debug)]
pub enum ReleaseError {
    Network(String),
    Api(u16),
    NotFound,
    Decode(String),
    MissingAsset(String),
    InvalidVersion(String),
    InvalidChecksum(String),
    ChecksumMismatch { expected: String, actual: String },
    TooLarge { limit: u64 },
    Io(String),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(message) => write!(f, "network error: {message}"),
            Self::Api(status) => write!(f, "GitHub API returned HTTP {status}"),
            Self::NotFound => write!(f, "no published GitHub Release found"),
            Self::Decode(message) => write!(f, "could not decode release data: {message}"),
            Self::MissingAsset(name) => write!(f, "release is missing asset {name}"),
            Self::InvalidVersion(message) => write!(f, "invalid release version: {message}"),
            Self::InvalidChecksum(message) => write!(f, "invalid checksum: {message}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch: expected {expected}, got {actual}")
            }
            Self::TooLarge { limit } => write!(f, "release asset exceeds {limit} bytes"),
            Self::Io(message) => write!(f, "file error: {message}"),
        }
    }
}

impl std::error::Error for ReleaseError {}

impl From<reqwest::Error> for ReleaseError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.to_string())
    }
}

impl From<serde_json::Error> for ReleaseError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error.to_string())
    }
}

impl From<std::io::Error> for ReleaseError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[derive(Clone)]
pub struct ReleaseClient {
    http: reqwest::Client,
}

impl ReleaseClient {
    pub fn new() -> Result<Self, ReleaseError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self { http })
    }

    pub async fn latest_release(&self) -> Result<LatestRelease, ReleaseError> {
        let response = self
            .http
            .get(format!("{API_BASE}/repos/{REPOSITORY}/releases/latest"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;
        let status = response.status();
        if status.as_u16() == 404 {
            return Err(ReleaseError::NotFound);
        }
        if !status.is_success() {
            return Err(ReleaseError::Api(status.as_u16()));
        }
        let body: GitHubReleaseResponse = response.json().await?;
        LatestRelease::from_github(body)
    }

    pub async fn check_for_update(
        &self,
        platform: Platform,
        current_version: &str,
    ) -> Result<UpdateStatus, ReleaseError> {
        let current = ReleaseVersion::parse(current_version)?;
        let release = self.latest_release().await?;
        match release.version.cmp(&current) {
            Ordering::Less | Ordering::Equal => Ok(UpdateStatus::UpToDate {
                current,
                latest: release.version,
            }),
            Ordering::Greater => {
                let mut assets = Vec::new();
                for name in platform.required_asset_names(&release.version) {
                    let asset = release
                        .asset(&name)
                        .ok_or_else(|| ReleaseError::MissingAsset(name.clone()))?;
                    if asset.size > MAX_ASSET_BYTES {
                        return Err(ReleaseError::TooLarge {
                            limit: MAX_ASSET_BYTES,
                        });
                    }
                    assets.push(asset.clone());
                }
                Ok(UpdateStatus::Available {
                    current,
                    latest: release.version.clone(),
                    release,
                    assets,
                })
            }
        }
    }

    pub async fn sha256_sums(
        &self,
        release: &LatestRelease,
    ) -> Result<HashMap<String, String>, ReleaseError> {
        let asset = release
            .asset(SHA256SUMS_NAME)
            .ok_or_else(|| ReleaseError::MissingAsset(SHA256SUMS_NAME.to_string()))?;
        let bytes = self.metadata_bytes(asset).await?;
        let text = String::from_utf8(bytes)
            .map_err(|error| ReleaseError::Decode(format!("SHA256SUMS is not UTF-8: {error}")))?;
        parse_sha256_sums(&text)
    }

    pub async fn download_asset(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), ReleaseError> {
        self.download_to_path(asset, dest).await
    }

    pub async fn download_verified_asset(
        &self,
        asset: &ReleaseAsset,
        dest: &Path,
        expected_sha256: &str,
    ) -> Result<(), ReleaseError> {
        let expected = normalize_hex(expected_sha256).ok_or_else(|| {
            ReleaseError::InvalidChecksum(
                "expected SHA-256 must be 64 hexadecimal characters".to_string(),
            )
        })?;
        let staging = staging_dir(dest).await?;
        let tmp = staging.path().join("asset");
        self.download_raw(asset, &tmp).await?;
        let actual = sha256_file(&tmp).await?;
        if actual != expected {
            return Err(ReleaseError::ChecksumMismatch { expected, actual });
        }
        tokio::fs::rename(&tmp, dest).await?;
        Ok(())
    }

    async fn metadata_bytes(&self, asset: &ReleaseAsset) -> Result<Vec<u8>, ReleaseError> {
        let response = self.http.get(&asset.url).send().await?;
        let status = response.status();
        if status.as_u16() == 404 {
            return Err(ReleaseError::NotFound);
        }
        if !status.is_success() {
            return Err(ReleaseError::Api(status.as_u16()));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_METADATA_BYTES as u64 {
                return Err(ReleaseError::TooLarge {
                    limit: MAX_METADATA_BYTES as u64,
                });
            }
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len() + chunk.len() > MAX_METADATA_BYTES {
                return Err(ReleaseError::TooLarge {
                    limit: MAX_METADATA_BYTES as u64,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn download_to_path(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), ReleaseError> {
        let staging = staging_dir(dest).await?;
        let tmp = staging.path().join("asset");
        self.download_raw(asset, &tmp).await?;
        tokio::fs::rename(&tmp, dest).await?;
        Ok(())
    }

    async fn download_raw(&self, asset: &ReleaseAsset, dest: &Path) -> Result<(), ReleaseError> {
        let response = self.http.get(&asset.url).send().await?;
        let status = response.status();
        if status.as_u16() == 404 {
            return Err(ReleaseError::NotFound);
        }
        if !status.is_success() {
            return Err(ReleaseError::Api(status.as_u16()));
        }
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = response.bytes_stream();
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total = total.saturating_add(chunk.len() as u64);
            if total > MAX_ASSET_BYTES {
                return Err(ReleaseError::TooLarge {
                    limit: MAX_ASSET_BYTES,
                });
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
}

impl Default for ReleaseClient {
    fn default() -> Self {
        Self::new().expect("reqwest client should build with rustls")
    }
}

async fn staging_dir(dest: &Path) -> Result<tempfile::TempDir, ReleaseError> {
    let file_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "asset".to_string());
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&parent).await?;
    Ok(tempfile::Builder::new()
        .prefix(&format!(".{file_name}."))
        .tempdir_in(parent)?)
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseResponse {
    tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAssetResponse>,
}

#[derive(Debug, Deserialize)]
struct GitHubAssetResponse {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

impl LatestRelease {
    fn from_github(response: GitHubReleaseResponse) -> Result<Self, ReleaseError> {
        let tag = response.tag_name;
        let version = ReleaseVersion::parse(&tag)?;
        let name = response.name.unwrap_or_else(|| tag.clone());
        let assets = response
            .assets
            .into_iter()
            .filter_map(|asset| {
                if valid_download_url(&asset.browser_download_url) {
                    Some(ReleaseAsset {
                        name: asset.name,
                        url: asset.browser_download_url,
                        size: asset.size.unwrap_or(0),
                    })
                } else {
                    None
                }
            })
            .collect();
        Ok(Self {
            tag,
            name,
            published_at: response.published_at.unwrap_or_default(),
            version,
            assets,
        })
    }
}

fn valid_download_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path().starts_with("/partypeopleland/art-term/releases/download/")
}

pub fn parse_sha256_sums(text: &str) -> Result<HashMap<String, String>, ReleaseError> {
    let mut sums = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(split) = line.find(char::is_whitespace) else {
            return Err(ReleaseError::Decode(format!(
                "malformed SHA256SUMS line {}",
                index + 1
            )));
        };
        let (sum, rest) = line.split_at(split);
        let name = rest.trim_start().trim_start_matches('*').trim();
        if name.is_empty() {
            return Err(ReleaseError::Decode(format!(
                "missing filename on SHA256SUMS line {}",
                index + 1
            )));
        }
        let sum = normalize_hex(sum).ok_or_else(|| {
            ReleaseError::Decode(format!("invalid SHA-256 on line {}", index + 1))
        })?;
        sums.insert(name.to_string(), sum);
    }
    if sums.is_empty() {
        return Err(ReleaseError::Decode("SHA256SUMS is empty".to_string()));
    }
    Ok(sums)
}

fn normalize_hex(input: &str) -> Option<String> {
    let value = input.trim().to_ascii_lowercase();
    if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(value)
    } else {
        None
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub async fn sha256_file(path: &Path) -> Result<String, ReleaseError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub async fn verify_file_sha256(path: &Path, expected_sha256: &str) -> Result<(), ReleaseError> {
    let expected = normalize_hex(expected_sha256).ok_or_else(|| {
        ReleaseError::InvalidChecksum(
            "expected SHA-256 must be 64 hexadecimal characters".to_string(),
        )
    })?;
    let actual = sha256_file(path).await?;
    if actual != expected {
        return Err(ReleaseError::ChecksumMismatch { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_versions() {
        assert_eq!(
            ReleaseVersion::parse("v1.2.3").unwrap(),
            ReleaseVersion {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert!(ReleaseVersion::parse("1.2").is_err());
        assert!(ReleaseVersion::parse("1.2.3-rc.1").is_err());
        assert!(ReleaseVersion::parse("v1.2.3.4").is_err());
    }

    #[test]
    fn orders_versions_numerically() {
        assert!(ReleaseVersion::parse("0.1.10").unwrap() > ReleaseVersion::parse("0.1.9").unwrap());
        assert!(ReleaseVersion::parse("1.0.0").unwrap() > ReleaseVersion::parse("0.99.99").unwrap());
    }

    #[test]
    fn names_platform_assets() {
        let version = ReleaseVersion::parse("0.1.38").unwrap();
        assert_eq!(
            Platform::Android.required_asset_names(&version),
            vec![
                "art-term-0.1.38-android-arm64.apk".to_string(),
                "SHA256SUMS.txt".to_string()
            ]
        );
        assert_eq!(
            Platform::Linux.required_asset_names(&version),
            vec![
                "art-term-0.1.38-linux-x86_64.tar.gz".to_string(),
                "art-term-0.1.38-linux-x86_64.manifest.json".to_string(),
                "SHA256SUMS.txt".to_string()
            ]
        );
        assert_eq!(
            Platform::Windows.required_asset_names(&version),
            vec![
                "art-term-0.1.38-windows-x64-setup.exe".to_string(),
                "latest.json".to_string()
            ]
        );
    }

    #[test]
    fn parses_github_release_json() {
        let response: GitHubReleaseResponse = serde_json::from_str(
            r#"{
                "tag_name":"v0.1.38",
                "name":"art-term v0.1.38",
                "published_at":"2026-11-17T00:00:00Z",
                "assets":[
                    {"name":"art-term-0.1.38-android-arm64.apk","browser_download_url":"https://github.com/partypeopleland/art-term/releases/download/v0.1.38/art-term-0.1.38-android-arm64.apk","size":123},
                    {"name":"evil","browser_download_url":"https://example.com/evil","size":1}
                ]
            }"#,
        )
        .unwrap();
        let release = LatestRelease::from_github(response).unwrap();
        assert_eq!(release.version, ReleaseVersion::parse("0.1.38").unwrap());
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].size, 123);
    }

    #[test]
    fn parses_sha256_sums() {
        let sums = parse_sha256_sums(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  art-term-0.1.38-android-arm64.apk\n\
             # comment\n\
             3a6eb0790f39ac87c94f3856b2dd2c5d110e6811602261a9a923d3bb50adc1dd *latest.json\n",
        )
        .unwrap();
        assert_eq!(sums.len(), 2);
        assert_eq!(
            sums["art-term-0.1.38-android-arm64.apk"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sums["latest.json"],
            "3a6eb0790f39ac87c94f3856b2dd2c5d110e6811602261a9a923d3bb50adc1dd"
        );
    }

    #[tokio::test]
    async fn hashes_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        tokio::fs::write(&path, b"abc").await.unwrap();
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
