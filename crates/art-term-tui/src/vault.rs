use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use art_term_core::openab::{
    api::ApiError,
    profiles::HostStore,
    secret::{CredentialCipher, CredentialContext, Secret},
};
use std::{
    ffi::OsString,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

const VAULT_MAGIC: &[u8; 8] = b"ARTTVLT\x01";
const VAULT_AAD: &[u8] = b"art-term/tui/wrapped-master/v1\0";
const RECORD_MAGIC: &[u8; 6] = b"ATTUI\x01";
const RECORD_AAD: &[u8] = b"art-term/tui/credential/v1\0";
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const KEY_BYTES: usize = 32;
const METADATA_HEADER: usize = VAULT_MAGIC.len() + SALT_BYTES + NONCE_BYTES;
const METADATA_BYTES: usize = METADATA_HEADER + KEY_BYTES + TAG_BYTES;
const RECORD_HEADER: usize = RECORD_MAGIC.len() + NONCE_BYTES;
const CONFIG_LIMIT: u64 = 1024 * 1024;

pub(crate) struct VaultPaths {
    config: PathBuf,
    metadata: PathBuf,
    lock: PathBuf,
}

pub(crate) enum VaultState {
    Fresh,
    Existing,
}

impl VaultPaths {
    pub(crate) fn resolve(config: Option<PathBuf>) -> Result<Self, ApiError> {
        let config = match config {
            Some(path) => {
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()
                        .map_err(|_| ApiError::Storage)?
                        .join(path)
                };
                safe_components(&path)?;
                writable_parent(&path)?;
                path
            }
            None => {
                let directory = config_directory(
                    std::env::var_os("XDG_CONFIG_HOME"),
                    std::env::var_os("HOME"),
                )?;
                safe_components(&directory)?;
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&directory)
                    .map_err(|_| ApiError::Storage)?;
                let metadata = fs::symlink_metadata(&directory).map_err(|_| ApiError::Storage)?;
                if !metadata.is_dir() || !owned(&metadata) || metadata.mode() & 0o077 != 0 {
                    return Err(ApiError::Storage);
                }
                directory.join("connections.toml")
            }
        };
        Self::from_config(config)
    }

    fn from_config(config: PathBuf) -> Result<Self, ApiError> {
        safe_components(&config)?;
        writable_parent(&config)?;
        let name = config.file_name().ok_or(ApiError::Storage)?;
        let mut metadata = name.to_os_string();
        metadata.push(".vault-v1");
        let mut lock = name.to_os_string();
        lock.push(".vault-lock");
        let parent = config.parent().ok_or(ApiError::Storage)?;
        Ok(Self {
            metadata: parent.join(metadata),
            lock: parent.join(lock),
            config,
        })
    }

    pub(crate) fn state(&self) -> Result<VaultState, ApiError> {
        writable_parent(&self.config)?;
        let config_exists = inspect_optional_file(&self.config, CONFIG_LIMIT)?;
        if inspect_optional_file(&self.metadata, METADATA_BYTES as u64)? {
            Ok(VaultState::Existing)
        } else if config_exists {
            Err(ApiError::Storage)
        } else {
            Ok(VaultState::Fresh)
        }
    }

    fn acquire(&self) -> Result<Arc<File>, ApiError> {
        writable_parent(&self.config)?;
        let lock = private_file(&self.lock, true)?;
        lock.try_lock().map_err(|_| ApiError::Storage)?;
        Ok(Arc::new(lock))
    }
}

fn config_directory(xdg: Option<OsString>, home: Option<OsString>) -> Result<PathBuf, ApiError> {
    if let Some(value) = xdg.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path.join("art-term-tui"));
        }
    }
    let home = home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(ApiError::Storage)?;
    Ok(home.join(".config/art-term-tui"))
}

fn owned(metadata: &fs::Metadata) -> bool {
    // SAFETY: geteuid has no arguments or caller memory preconditions.
    metadata.uid() == unsafe { libc::geteuid() }
}

fn safe_components(path: &Path) -> Result<(), ApiError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(ApiError::Storage);
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(ApiError::Storage);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(ApiError::Storage),
        }
    }
    Ok(())
}

fn writable_parent(path: &Path) -> Result<(), ApiError> {
    safe_components(path)?;
    let parent = path.parent().ok_or(ApiError::Storage)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| ApiError::Storage)?;
    if !metadata.is_dir() || !owned(&metadata) || metadata.mode() & 0o022 != 0 {
        return Err(ApiError::Storage);
    }
    Ok(())
}

fn private_file(path: &Path, create: bool) -> Result<File, ApiError> {
    safe_components(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| ApiError::Storage)?;
    let metadata = file.metadata().map_err(|_| ApiError::Storage)?;
    if !metadata.is_file()
        || !owned(&metadata)
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(ApiError::Storage);
    }
    Ok(file)
}

fn inspect_optional_file(path: &Path, limit: u64) -> Result<bool, ApiError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ApiError::Storage),
        Ok(_) => {
            if private_file(path, false)?
                .metadata()
                .map_err(|_| ApiError::Storage)?
                .len()
                > limit
            {
                return Err(ApiError::Storage);
            }
            Ok(true)
        }
    }
}

fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_BYTES]>, ApiError> {
    if password.is_empty() || password.len() > 1024 || salt.len() != SALT_BYTES {
        return Err(ApiError::Storage);
    }
    let params = Params::new(64 * 1024, 3, 1, Some(KEY_BYTES)).map_err(|_| ApiError::Storage)?;
    let kdf = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    kdf.hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| ApiError::Storage)?;
    Ok(key)
}

fn metadata_aad(header: &[u8]) -> Vec<u8> {
    let mut aad = VAULT_AAD.to_vec();
    aad.extend_from_slice(header);
    aad
}

fn wrap_master(password: &str, master: &[u8; KEY_BYTES]) -> Result<Vec<u8>, ApiError> {
    let mut header = [0; METADATA_HEADER];
    header[..VAULT_MAGIC.len()].copy_from_slice(VAULT_MAGIC);
    getrandom::fill(&mut header[VAULT_MAGIC.len()..]).map_err(|_| ApiError::Storage)?;
    let key = derive_key(
        password,
        &header[VAULT_MAGIC.len()..VAULT_MAGIC.len() + SALT_BYTES],
    )?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| ApiError::Storage)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&header[METADATA_HEADER - NONCE_BYTES..]),
            Payload {
                msg: master,
                aad: &metadata_aad(&header),
            },
        )
        .map_err(|_| ApiError::Storage)?;
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&ciphertext);
    Ok(bytes)
}

fn unwrap_master(bytes: &[u8], password: &str) -> Result<Zeroizing<[u8; KEY_BYTES]>, ApiError> {
    if bytes.len() != METADATA_BYTES || &bytes[..VAULT_MAGIC.len()] != VAULT_MAGIC {
        return Err(ApiError::Storage);
    }
    let key = derive_key(
        password,
        &bytes[VAULT_MAGIC.len()..VAULT_MAGIC.len() + SALT_BYTES],
    )?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| ApiError::Storage)?;
    let master = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&bytes[METADATA_HEADER - NONCE_BYTES..METADATA_HEADER]),
                Payload {
                    msg: &bytes[METADATA_HEADER..],
                    aad: &metadata_aad(&bytes[..METADATA_HEADER]),
                },
            )
            .map_err(|_| ApiError::Storage)?,
    );
    let mut key = Zeroizing::new([0; KEY_BYTES]);
    if master.len() != KEY_BYTES {
        return Err(ApiError::Storage);
    }
    key.copy_from_slice(&master);
    Ok(key)
}

fn install_metadata(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let parent = path.parent().ok_or(ApiError::Storage)?;
    writable_parent(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| ApiError::Storage)?;
    temp.write_all(bytes).map_err(|_| ApiError::Storage)?;
    temp.as_file().sync_all().map_err(|_| ApiError::Storage)?;
    temp.persist_noclobber(path)
        .map_err(|_| ApiError::Storage)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ApiError::Storage)
}

struct TtyCipher {
    key: Zeroizing<[u8; KEY_BYTES]>,
    _lock: Arc<File>,
}

impl TtyCipher {
    fn aad(context: &CredentialContext) -> Vec<u8> {
        let mut aad = RECORD_AAD.to_vec();
        aad.extend_from_slice(RECORD_MAGIC);
        aad.extend_from_slice(&context.authenticated_bytes());
        aad
    }
}

impl CredentialCipher for TtyCipher {
    fn seal(&self, secret: &Secret, context: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let mut header = [0; RECORD_HEADER];
        header[..RECORD_MAGIC.len()].copy_from_slice(RECORD_MAGIC);
        getrandom::fill(&mut header[RECORD_MAGIC.len()..]).map_err(|_| ApiError::Storage)?;
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| ApiError::Storage)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&header[RECORD_MAGIC.len()..]),
                Payload {
                    msg: secret.expose().as_bytes(),
                    aad: &Self::aad(context),
                },
            )
            .map_err(|_| ApiError::Storage)?;
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&ciphertext);
        Ok(bytes)
    }

    fn unseal(&self, bytes: &[u8], context: &CredentialContext) -> Result<Secret, ApiError> {
        if bytes.len() < RECORD_HEADER + TAG_BYTES + 1
            || bytes.len() > RECORD_HEADER + TAG_BYTES + 1024
            || &bytes[..RECORD_MAGIC.len()] != RECORD_MAGIC
        {
            return Err(ApiError::Storage);
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| ApiError::Storage)?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&bytes[RECORD_MAGIC.len()..RECORD_HEADER]),
                    Payload {
                        msg: &bytes[RECORD_HEADER..],
                        aad: &Self::aad(context),
                    },
                )
                .map_err(|_| ApiError::Storage)?,
        );
        let text = std::str::from_utf8(&plain).map_err(|_| ApiError::Storage)?;
        Secret::new(text.to_owned()).map_err(|_| ApiError::Storage)
    }
}

pub(crate) struct Vault {
    store: Arc<HostStore>,
}

impl Vault {
    pub(crate) fn initialize(
        paths: VaultPaths,
        password: Zeroizing<String>,
    ) -> Result<Self, ApiError> {
        let lock = paths.acquire()?;
        if !matches!(paths.state()?, VaultState::Fresh) {
            return Err(ApiError::Storage);
        }
        let mut key = Zeroizing::new([0; KEY_BYTES]);
        getrandom::fill(key.as_mut()).map_err(|_| ApiError::Storage)?;
        let metadata = wrap_master(&password, &key)?;
        install_metadata(&paths.metadata, &metadata)?;
        Self::load(paths, key, lock)
    }

    pub(crate) fn unlock(paths: VaultPaths, password: Zeroizing<String>) -> Result<Self, ApiError> {
        let lock = paths.acquire()?;
        if !matches!(paths.state()?, VaultState::Existing) {
            return Err(ApiError::Storage);
        }
        let mut bytes = Vec::new();
        private_file(&paths.metadata, false)?
            .take(METADATA_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ApiError::Storage)?;
        let key = unwrap_master(&bytes, &password)?;
        Self::load(paths, key, lock)
    }

    fn load(
        paths: VaultPaths,
        key: Zeroizing<[u8; KEY_BYTES]>,
        lock: Arc<File>,
    ) -> Result<Self, ApiError> {
        let store = HostStore::load(paths.config, Arc::new(TtyCipher { key, _lock: lock }))
            .map_err(|_| ApiError::Storage)?;
        for screen in store.desktops().map_err(|_| ApiError::Storage)? {
            store.desktop(&screen.id).map_err(|_| ApiError::Storage)?;
        }
        Ok(Self {
            store: Arc::new(store),
        })
    }

    pub(crate) fn store(&self) -> Arc<HostStore> {
        self.store.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn directory() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("art-term-tui-vault-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap()
    }

    fn paths(root: &Path) -> VaultPaths {
        VaultPaths::resolve(Some(root.join("connections.toml"))).unwrap()
    }

    fn password() -> Zeroizing<String> {
        Zeroizing::new("fixture vault password 測試".into())
    }

    fn secret(value: &str) -> Secret {
        Secret::new(value.to_owned()).unwrap()
    }

    fn cipher(value: u8) -> TtyCipher {
        TtyCipher {
            key: Zeroizing::new([value; KEY_BYTES]),
            _lock: Arc::new(tempfile::tempfile().unwrap()),
        }
    }

    fn context() -> CredentialContext {
        CredentialContext {
            schema: 1,
            host_id: "fixture-host".into(),
            origin: "https://fixture.example".into(),
            revision: 1,
        }
    }

    #[test]
    fn credentials_authenticate_context_and_every_envelope_byte() {
        let cipher = cipher(7);
        let original = context();
        let encoded = cipher.seal(&secret("fixture-token"), &original).unwrap();
        assert_eq!(
            cipher.unseal(&encoded, &original).unwrap().expose(),
            "fixture-token"
        );
        assert_ne!(
            encoded,
            cipher.seal(&secret("fixture-token"), &original).unwrap()
        );
        for position in 0..encoded.len() {
            let mut changed = encoded.clone();
            changed[position] ^= 1;
            assert!(cipher.unseal(&changed, &original).is_err());
        }
        for length in 0..encoded.len() {
            assert!(cipher.unseal(&encoded[..length], &original).is_err());
        }
        for changed in [
            CredentialContext {
                schema: 2,
                ..context()
            },
            CredentialContext {
                host_id: "other-host".into(),
                ..context()
            },
            CredentialContext {
                origin: "https://other.example".into(),
                ..context()
            },
            CredentialContext {
                revision: 2,
                ..context()
            },
        ] {
            assert!(cipher.unseal(&encoded, &changed).is_err());
        }
        assert!(self::cipher(8).unseal(&encoded, &original).is_err());
        assert!(cipher.unseal(&vec![0; 32 * 1024], &original).is_err());
    }

    #[test]
    fn host_and_screen_survive_unlock_while_refusals_keep_original_bytes() {
        let root = directory();
        let original_paths = paths(root.path());
        let metadata_path = original_paths.metadata.clone();
        let config_path = original_paths.config.clone();
        let vault = Vault::initialize(original_paths, password()).unwrap();
        let store = vault.store();
        let host = store
            .add(
                "Fixture Host".into(),
                "https://fixture.example".into(),
                secret("host-fixture-secret"),
            )
            .unwrap();
        let screen = store
            .save_desktop(
                None,
                None,
                "Fixture Screen".into(),
                "https://screen.example/mcp".into(),
                Some(secret("screen-fixture-secret")),
            )
            .unwrap();
        let config_bytes = fs::read(&config_path).unwrap();
        let metadata_bytes = fs::read(&metadata_path).unwrap();
        assert!(!config_bytes
            .windows(19)
            .any(|bytes| bytes == b"host-fixture-secret"));
        assert!(!config_bytes
            .windows(21)
            .any(|bytes| bytes == b"screen-fixture-secret"));
        assert_eq!(fs::metadata(&config_path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&metadata_path).unwrap().mode() & 0o777, 0o600);
        drop(vault);
        assert!(Vault::unlock(paths(root.path()), password()).is_err());
        drop(store);
        assert!(Vault::initialize(paths(root.path()), password()).is_err());
        assert!(
            Vault::unlock(paths(root.path()), Zeroizing::new("wrong password".into())).is_err()
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_bytes);
        assert_eq!(fs::read(&metadata_path).unwrap(), metadata_bytes);
        let unlocked = Vault::unlock(paths(root.path()), password()).unwrap();
        assert_eq!(
            unlocked
                .store()
                .snapshot(&host.id)
                .unwrap()
                .credential
                .expose(),
            "host-fixture-secret"
        );
        assert_eq!(
            unlocked.store().desktop(&screen.id).unwrap().token.expose(),
            "screen-fixture-secret"
        );
        drop(unlocked);
        let mut damaged = metadata_bytes.clone();
        *damaged.last_mut().unwrap() ^= 1;
        fs::write(&metadata_path, &damaged).unwrap();
        assert!(Vault::unlock(paths(root.path()), password()).is_err());
        assert_eq!(fs::read(&metadata_path).unwrap(), damaged);
        assert_eq!(fs::read(&config_path).unwrap(), config_bytes);
    }

    #[test]
    fn screen_only_missing_key_never_creates_a_replacement() {
        let root = directory();
        let original_paths = paths(root.path());
        let metadata_path = original_paths.metadata.clone();
        let config_path = original_paths.config.clone();
        let vault = Vault::initialize(original_paths, password()).unwrap();
        vault
            .store()
            .save_desktop(
                None,
                None,
                "Only Screen".into(),
                "https://screen.example/mcp".into(),
                Some(secret("screen-only-fixture")),
            )
            .unwrap();
        assert!(vault.store().list().unwrap().is_empty());
        drop(vault);
        fs::remove_file(&metadata_path).unwrap();
        let original = fs::read(&config_path).unwrap();
        assert!(paths(root.path()).state().is_err());
        assert!(Vault::initialize(paths(root.path()), password()).is_err());
        assert!(Vault::unlock(paths(root.path()), password()).is_err());
        assert!(!metadata_path.exists());
        assert_eq!(fs::read(&config_path).unwrap(), original);
    }

    #[test]
    fn interrupted_initialization_preserves_installed_key_and_unlocks_empty_store() {
        let root = directory();
        let original_paths = paths(root.path());
        let metadata_path = original_paths.metadata.clone();
        let config_path = original_paths.config.clone();
        let vault = Vault::initialize(original_paths, password()).unwrap();
        assert!(!config_path.exists());
        let original = fs::read(&metadata_path).unwrap();
        assert!(install_metadata(&metadata_path, &[1; METADATA_BYTES]).is_err());
        drop(vault);
        assert!(matches!(
            paths(root.path()).state().unwrap(),
            VaultState::Existing
        ));
        assert!(Vault::initialize(paths(root.path()), password()).is_err());
        let reopened = Vault::unlock(paths(root.path()), password()).unwrap();
        assert!(reopened.store().list().unwrap().is_empty());
        assert_eq!(fs::read(&metadata_path).unwrap(), original);
    }

    #[test]
    fn unlocking_checks_screen_ciphertext_even_when_host_list_is_empty() {
        let root = directory();
        let original_paths = paths(root.path());
        let config_path = original_paths.config.clone();
        let vault = Vault::initialize(original_paths, password()).unwrap();
        vault
            .store()
            .save_desktop(
                None,
                None,
                "Only Screen".into(),
                "https://screen.example/mcp".into(),
                Some(secret("screen-only-fixture")),
            )
            .unwrap();
        drop(vault);
        let text = fs::read_to_string(&config_path).unwrap();
        let start = text.find("token_ciphertext = [").unwrap() + "token_ciphertext = [".len();
        let end = text[start..].find(']').unwrap() + start;
        let changed = format!("{}1{}", &text[..start], &text[end..]);
        fs::write(&config_path, &changed).unwrap();
        assert!(Vault::unlock(paths(root.path()), password()).is_err());
        assert_eq!(fs::read_to_string(&config_path).unwrap(), changed);
    }

    #[test]
    fn private_paths_refuse_symlinks_hardlinks_fifo_and_unsafe_permissions() {
        let root = directory();
        let path = root.path().join("private");
        fs::write(&path, b"private fixture").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.path().join("symlink");
        symlink(&path, &link).unwrap();
        assert!(private_file(&link, false).is_err());
        let hard = root.path().join("hardlink");
        fs::hard_link(&path, &hard).unwrap();
        assert!(private_file(&path, false).is_err());
        fs::remove_file(&hard).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_file(&path, false).is_err());
        let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: name is NUL-terminated, and mkfifo only reads it during this call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(private_file(&fifo, false).is_err());
        let nested = root.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(VaultPaths::resolve(Some(nested.join("connections.toml"))).is_err());
        let directory_link = root.path().join("directory-link");
        symlink(root.path(), &directory_link).unwrap();
        assert!(VaultPaths::resolve(Some(directory_link.join("connections.toml"))).is_err());
        assert!(VaultPaths::resolve(Some(root.path().join(".."))).is_err());
    }

    #[test]
    fn xdg_directory_uses_only_absolute_bases_and_separates_custom_sidecars() {
        assert_eq!(
            config_directory(Some("/tmp/fixture-xdg".into()), None).unwrap(),
            PathBuf::from("/tmp/fixture-xdg/art-term-tui")
        );
        assert_eq!(
            config_directory(Some("relative".into()), Some("/tmp/fixture-home".into())).unwrap(),
            PathBuf::from("/tmp/fixture-home/.config/art-term-tui")
        );
        assert!(config_directory(None, Some("relative".into())).is_err());
        let root = directory();
        let first = paths(root.path());
        let second = VaultPaths::resolve(Some(root.path().join("other.toml"))).unwrap();
        assert_ne!(first.metadata, second.metadata);
        assert_ne!(first.lock, second.lock);
    }

    #[test]
    fn lock_survives_store_clone_and_blocks_another_process() {
        let root = directory();
        let original_paths = paths(root.path());
        let lock_path = original_paths.lock.clone();
        let guard = original_paths.acquire().unwrap();
        let outcome = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "vault::tests::lock_probe",
                "--ignored",
                "--nocapture",
            ])
            .env("ART_TERM_TEST_VAULT_LOCK_PROBE", &lock_path)
            .output()
            .unwrap();
        assert!(
            outcome.status.success(),
            "{}",
            String::from_utf8_lossy(&outcome.stderr)
        );
        assert!(String::from_utf8_lossy(&outcome.stdout).contains("1 passed"));
        drop(guard);
        let guard = paths(root.path()).acquire().unwrap();
        assert!(paths(root.path()).acquire().is_err());
        drop(guard);
        assert!(paths(root.path()).acquire().is_ok());
    }

    #[test]
    #[ignore = "Invoked by the cross-process lock test with its private path"]
    fn lock_probe() {
        let path = PathBuf::from(std::env::var_os("ART_TERM_TEST_VAULT_LOCK_PROBE").unwrap());
        assert!(path.starts_with("/tmp") && path.to_string_lossy().contains("art-term-tui-vault-"));
        assert!(private_file(&path, false).unwrap().try_lock().is_err());
    }
}
