use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use art_term_core::openab::{
    api::ApiError,
    profiles::HostStore,
    secret::{CredentialCipher, CredentialContext, Secret},
};
use secret_service::{blocking::SecretService, EncryptionType};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

const HEADER: usize = 21;
const NONCE: usize = 12;
const TAG: usize = 16;
const MAGIC: &[u8; 5] = b"ATLC\x01";
const DOMAIN: &[u8] = b"art-term/linux/operator-credential/v1\0";
const MARKER: &str = "operator-key-used-v1";

struct LinuxCredentialCipher {
    key: Zeroizing<[u8; 32]>,
    fingerprint: [u8; 32],
}

impl LinuxCredentialCipher {
    fn new(key: Zeroizing<[u8; 32]>) -> Self {
        let fingerprint = Sha256::digest(key.as_ref()).into();
        Self { key, fingerprint }
    }

    fn aad(header: &[u8], context: &CredentialContext) -> Vec<u8> {
        let mut aad = DOMAIN.to_vec();
        aad.extend_from_slice(header);
        aad.extend_from_slice(&context.authenticated_bytes());
        aad
    }
}

impl CredentialCipher for LinuxCredentialCipher {
    fn seal(&self, secret: &Secret, context: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        let mut bytes = Vec::with_capacity(HEADER + NONCE + secret.expose().len() + TAG);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.fingerprint[..16]);
        let aad = Self::aad(&bytes, context);
        let mut nonce = [0; NONCE];
        getrandom::fill(&mut nonce).map_err(|_| ApiError::Storage)?;
        bytes.extend_from_slice(&nonce);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| ApiError::Storage)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: secret.expose().as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| ApiError::Storage)?;
        bytes.extend_from_slice(&ciphertext);
        Ok(bytes)
    }

    fn unseal(&self, bytes: &[u8], context: &CredentialContext) -> Result<Secret, ApiError> {
        if bytes.len() < HEADER + NONCE + TAG + 1
            || bytes.len() > HEADER + NONCE + TAG + 1024
            || &bytes[..5] != MAGIC
            || bytes[5..HEADER] != self.fingerprint[..16]
        {
            return Err(ApiError::Storage);
        }
        let aad = Self::aad(&bytes[..HEADER], context);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| ApiError::Storage)?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&bytes[HEADER..HEADER + NONCE]),
                    Payload {
                        msg: &bytes[HEADER + NONCE..],
                        aad: &aad,
                    },
                )
                .map_err(|_| ApiError::Storage)?,
        );
        let text = std::str::from_utf8(&plain).map_err(|_| ApiError::Storage)?;
        Secret::new(text.to_owned()).map_err(|_| ApiError::Storage)
    }
}

struct RejectCipher;
impl CredentialCipher for RejectCipher {
    fn seal(&self, _: &Secret, _: &CredentialContext) -> Result<Vec<u8>, ApiError> {
        Err(ApiError::Storage)
    }
    fn unseal(&self, _: &[u8], _: &CredentialContext) -> Result<Secret, ApiError> {
        Err(ApiError::Storage)
    }
}

fn application_component(application_id: &str) -> Result<&str, ApiError> {
    if application_id.is_empty()
        || application_id.len() > 255
        || !application_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        || matches!(application_id, "." | "..")
    {
        return Err(ApiError::Storage);
    }
    Ok(application_id)
}

fn base_directory(variable: &str, fallback: &str) -> Result<PathBuf, ApiError> {
    resolve_base(
        std::env::var_os(variable),
        std::env::var_os("HOME"),
        fallback,
    )
}

fn resolve_base(
    value: Option<OsString>,
    home: Option<OsString>,
    fallback: &str,
) -> Result<PathBuf, ApiError> {
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home = home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or(ApiError::Storage)?;
    Ok(home.join(fallback))
}

fn no_symlink_components(path: &Path) -> Result<(), ApiError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ApiError::Storage);
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(ApiError::Storage),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(ApiError::Storage),
        }
    }
    Ok(())
}

fn owned(metadata: &fs::Metadata) -> bool {
    // SAFETY: geteuid has no preconditions and does not access caller memory.
    metadata.uid() == unsafe { libc::geteuid() }
}

fn private_directory(path: &Path) -> Result<(), ApiError> {
    no_symlink_components(path)?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| ApiError::Storage)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| ApiError::Storage)?;
    if !metadata.is_dir() || !owned(&metadata) || metadata.mode() & 0o077 != 0 {
        return Err(ApiError::Storage);
    }
    Ok(())
}

pub(super) fn default_config_path(application_id: &str) -> Result<PathBuf, ApiError> {
    let directory =
        base_directory("XDG_CONFIG_HOME", ".config")?.join(application_component(application_id)?);
    private_directory(&directory)?;
    Ok(directory.join("connections.toml"))
}

fn private_file(path: &Path, create: bool) -> Result<File, ApiError> {
    no_symlink_components(path)?;
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

fn validate_config_path(path: &Path) -> Result<(), ApiError> {
    no_symlink_components(path)?;
    let parent = path.parent().ok_or(ApiError::Storage)?;
    let metadata = fs::metadata(parent).map_err(|_| ApiError::Storage)?;
    if !metadata.is_dir() || !owned(&metadata) || metadata.mode() & 0o022 != 0 {
        return Err(ApiError::Storage);
    }
    match fs::symlink_metadata(path) {
        Ok(_) => {
            private_file(path, false)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err(ApiError::Storage),
    }
    Ok(())
}

fn read_marker(path: &Path) -> Result<Option<[u8; 32]>, ApiError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ApiError::Storage),
        Ok(_) => (),
    }
    let mut bytes = Vec::new();
    private_file(path, false)?
        .take(38)
        .read_to_end(&mut bytes)
        .map_err(|_| ApiError::Storage)?;
    if bytes.len() != 37 || &bytes[..5] != MAGIC {
        return Err(ApiError::Storage);
    }
    Ok(Some(bytes[5..].try_into().map_err(|_| ApiError::Storage)?))
}

fn write_marker(directory: &Path, fingerprint: &[u8; 32]) -> Result<(), ApiError> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| ApiError::Storage)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| ApiError::Storage)?;
    temporary
        .write_all(MAGIC)
        .and_then(|_| temporary.write_all(fingerprint))
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| ApiError::Storage)?;
    temporary
        .persist_noclobber(directory.join(MARKER))
        .map_err(|_| ApiError::Storage)?;
    File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| ApiError::Storage)
}

fn attributes(application_id: &str) -> HashMap<&str, &str> {
    HashMap::from([
        ("application", application_id),
        ("purpose", "operator-master-key"),
        ("version", "1"),
    ])
}

fn find_key(
    service: &SecretService<'_>,
    application_id: &str,
    collection_path: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, ApiError> {
    let matches = service
        .search_items(attributes(application_id))
        .map_err(|_| ApiError::Storage)?;
    if matches.unlocked.len() + matches.locked.len() > 1 {
        return Err(ApiError::Storage);
    }
    let Some(item) = matches.unlocked.into_iter().chain(matches.locked).next() else {
        return Ok(None);
    };
    if !item
        .item_path
        .as_str()
        .starts_with(&format!("{collection_path}/"))
    {
        return Err(ApiError::Storage);
    }
    item.unlock().map_err(|_| ApiError::Storage)?;
    if item.is_locked().map_err(|_| ApiError::Storage)? {
        return Err(ApiError::Storage);
    }
    let bytes = Zeroizing::new(item.get_secret().map_err(|_| ApiError::Storage)?);
    if bytes.len() != 32 {
        return Err(ApiError::Storage);
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&bytes);
    Ok(Some(key))
}

fn validate_empty(path: &Path) -> Result<(), ApiError> {
    let store =
        HostStore::load(path.to_owned(), Arc::new(RejectCipher)).map_err(|_| ApiError::Storage)?;
    if !store.desktops().map_err(|_| ApiError::Storage)?.is_empty() {
        return Err(ApiError::Storage);
    }
    Ok(())
}

fn load_verified(path: PathBuf, cipher: Arc<LinuxCredentialCipher>) -> Result<HostStore, ApiError> {
    let store = HostStore::load(path, cipher).map_err(|_| ApiError::Storage)?;
    for profile in store.desktops().map_err(|_| ApiError::Storage)? {
        store.desktop(&profile.id).map_err(|_| ApiError::Storage)?;
    }
    Ok(store)
}

pub(super) fn open_store(path: PathBuf, application_id: &str) -> Result<HostStore, ApiError> {
    application_component(application_id)?;
    validate_config_path(&path)?;
    let directory = base_directory("XDG_DATA_HOME", ".local/share")?.join(application_id);
    private_directory(&directory)?;
    let lock = private_file(&directory.join("operator-key-init.lock"), true)?;
    lock.try_lock().map_err(|_| ApiError::Storage)?;
    let marker = read_marker(&directory.join(MARKER))?;
    let service = SecretService::connect(EncryptionType::Dh).map_err(|_| ApiError::Storage)?;
    let collection = service
        .get_default_collection()
        .map_err(|_| ApiError::Storage)?;
    let collection_path = collection.collection_path.as_str();
    if collection_path == "/org/freedesktop/secrets/collection/session" {
        return Err(ApiError::Storage);
    }
    let key = match find_key(&service, application_id, collection_path)? {
        Some(key) => key,
        None => {
            if marker.is_some() {
                return Err(ApiError::Storage);
            }
            validate_empty(&path)?;
            collection.unlock().map_err(|_| ApiError::Storage)?;
            if collection.is_locked().map_err(|_| ApiError::Storage)? {
                return Err(ApiError::Storage);
            }
            let mut key = Zeroizing::new([0; 32]);
            getrandom::fill(key.as_mut()).map_err(|_| ApiError::Storage)?;
            collection
                .create_item(
                    "Art Term operator credentials",
                    attributes(application_id),
                    key.as_ref(),
                    false,
                    "application/octet-stream",
                )
                .map_err(|_| ApiError::Storage)?;
            let persisted =
                find_key(&service, application_id, collection_path)?.ok_or(ApiError::Storage)?;
            if persisted.as_ref() != key.as_ref() {
                return Err(ApiError::Storage);
            }
            persisted
        }
    };
    let cipher = Arc::new(LinuxCredentialCipher::new(key));
    if marker.is_some_and(|fingerprint| fingerprint != cipher.fingerprint) {
        return Err(ApiError::Storage);
    }
    let store = load_verified(path, cipher.clone())?;
    if marker.is_none() {
        write_marker(&directory, &cipher.fingerprint)?;
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn cipher(byte: u8) -> Arc<LinuxCredentialCipher> {
        Arc::new(LinuxCredentialCipher::new(Zeroizing::new([byte; 32])))
    }
    fn context(schema: u32) -> CredentialContext {
        CredentialContext {
            schema,
            host_id: if schema == 2 {
                "instance-mcp:fixture-screen"
            } else {
                "fixture-host"
            }
            .into(),
            origin: "https://fixture.example:8443".into(),
            revision: 1,
        }
    }
    fn directory() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("art-term-linux-vault-")
            .tempdir_in("/tmp")
            .unwrap()
    }

    #[test]
    fn encryption_authenticates_host_screen_context_and_randomizes_every_seal() {
        let cipher = cipher(7);
        let secret = Secret::new("fixture-private-credential".into()).unwrap();
        for schema in [1, 2] {
            let context = context(schema);
            let bytes = cipher.seal(&secret, &context).unwrap();
            assert_eq!(
                cipher.unseal(&bytes, &context).unwrap().expose(),
                secret.expose()
            );
            assert_ne!(bytes, cipher.seal(&secret, &context).unwrap());
            assert!(!bytes
                .windows(secret.expose().len())
                .any(|p| p == secret.expose().as_bytes()));
            for drift in [
                CredentialContext {
                    schema: schema + 1,
                    ..context.clone()
                },
                CredentialContext {
                    host_id: "another-id".into(),
                    ..context.clone()
                },
                CredentialContext {
                    origin: "https://another.example".into(),
                    ..context.clone()
                },
                CredentialContext {
                    revision: 2,
                    ..context.clone()
                },
            ] {
                assert!(cipher.unseal(&bytes, &drift).is_err());
            }
            assert!(super::tests::cipher(8).unseal(&bytes, &context).is_err());
            for index in 0..bytes.len() {
                let mut damaged = bytes.clone();
                damaged[index] ^= 1;
                assert!(cipher.unseal(&damaged, &context).is_err());
                assert!(cipher.unseal(&bytes[..index], &context).is_err());
            }
            assert!(cipher.unseal(&vec![0; 2048], &context).is_err());
        }
    }

    #[test]
    fn xdg_resolution_requires_absolute_roots_and_never_uses_cwd() {
        for value in [
            None,
            Some(OsString::from("")),
            Some(OsString::from("relative")),
        ] {
            assert_eq!(
                resolve_base(value.clone(), Some("/home/fixture".into()), ".config").unwrap(),
                PathBuf::from("/home/fixture/.config")
            );
            assert!(resolve_base(value.clone(), None, ".config").is_err());
            assert!(resolve_base(value, Some("relative-home".into()), ".config").is_err());
        }
        assert_eq!(
            resolve_base(Some("/private/config".into()), None, ".config").unwrap(),
            PathBuf::from("/private/config")
        );
        let dir = directory();
        let absent = dir.path().join("missing/override.toml");
        assert!(validate_config_path(&absent).is_err());
        assert!(!absent.parent().unwrap().exists());
        let path = dir.path().join("override.toml");
        assert!(validate_config_path(&path).is_ok());
        fs::write(&path, "fixture").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_config_path(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"fixture");
    }

    #[test]
    fn authenticated_invalid_plaintext_is_still_rejected() {
        let cipher = cipher(7);
        let context = context(1);
        for plain in [
            vec![0xff],
            b"contains whitespace".to_vec(),
            Vec::new(),
            vec![b'a'; 1025],
        ] {
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&cipher.fingerprint[..16]);
            let aad = LinuxCredentialCipher::aad(&bytes, &context);
            bytes.extend_from_slice(&[0; NONCE]);
            let encryption = Aes256Gcm::new_from_slice(cipher.key.as_ref()).unwrap();
            bytes.extend_from_slice(
                &encryption
                    .encrypt(
                        Nonce::from_slice(&[0; NONCE]),
                        Payload {
                            msg: &plain,
                            aad: &aad,
                        },
                    )
                    .unwrap(),
            );
            assert!(cipher.unseal(&bytes, &context).is_err());
        }
    }

    #[test]
    fn reject_cipher_prevents_host_and_screen_only_key_replacement() {
        let dir = directory();
        let path = dir.path().join("connections.toml");
        assert!(validate_empty(&path).is_ok());
        assert!(!path.exists());
        let store = HostStore::load(path.clone(), cipher(7)).unwrap();
        let screen = store
            .save_desktop(
                None,
                None,
                "Screen".into(),
                "https://fixture.example/mcp".into(),
                Some(Secret::new("fixture-screen-token".into()).unwrap()),
            )
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(validate_empty(&path).is_err());
        assert!(load_verified(path.clone(), cipher(8)).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let reopened = load_verified(path.clone(), cipher(7)).unwrap();
        assert_eq!(
            reopened.desktop(&screen.id).unwrap().token.expose(),
            "fixture-screen-token"
        );
        reopened
            .delete_desktop(&screen.id, screen.revision)
            .unwrap();
        assert!(validate_empty(&path).is_ok());
        reopened
            .add(
                "Host".into(),
                "https://fixture.example:8443".into(),
                Secret::new("fixture-host-credential".into()).unwrap(),
            )
            .unwrap();
        assert!(validate_empty(&path).is_err());
        let reopened = load_verified(path.clone(), cipher(7)).unwrap();
        let host = reopened.list().unwrap().remove(0);
        reopened
            .update(
                &host.id,
                host.revision,
                "Renamed".into(),
                host.base_url.clone(),
                None,
            )
            .unwrap();
        assert!(load_verified(path, cipher(7)).is_ok());
    }

    #[test]
    fn marker_and_private_paths_fail_closed_without_changing_existing_files() {
        let dir = directory();
        let state = dir.path().join("state");
        private_directory(&state).unwrap();
        assert_eq!(fs::metadata(&state).unwrap().mode() & 0o777, 0o700);
        let marker = state.join(MARKER);
        assert_eq!(read_marker(&marker).unwrap(), None);
        let fingerprint = cipher(7).fingerprint;
        write_marker(&state, &fingerprint).unwrap();
        assert_eq!(read_marker(&marker).unwrap(), Some(fingerprint));
        assert_eq!(fs::metadata(&marker).unwrap().mode() & 0o777, 0o600);
        assert!(write_marker(&state, &[0; 32]).is_err());
        assert_eq!(read_marker(&marker).unwrap(), Some(fingerprint));
        fs::write(&marker, b"bad-marker").unwrap();
        assert!(read_marker(&marker).is_err());
        fs::remove_file(&marker).unwrap();
        symlink(dir.path().join("elsewhere"), &marker).unwrap();
        assert!(read_marker(&marker).is_err());
        let unsafe_dir = dir.path().join("unsafe");
        fs::create_dir(&unsafe_dir).unwrap();
        fs::set_permissions(&unsafe_dir, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(private_directory(&unsafe_dir).is_err());
        assert_eq!(fs::metadata(&unsafe_dir).unwrap().mode() & 0o777, 0o777);
        assert!(validate_config_path(&unsafe_dir.join("connections.toml")).is_err());
        let link = dir.path().join("link");
        symlink(&state, &link).unwrap();
        assert!(private_directory(&link).is_err());
        assert!(application_component("../outside").is_err());
        assert!(application_component("net.missart.artterm.desktop").is_ok());
        let regular = state.join("regular");
        private_file(&regular, true).unwrap();
        let hardlink = state.join("hardlink");
        fs::hard_link(&regular, &hardlink).unwrap();
        assert!(private_file(&hardlink, false).is_err());
        let fifo = state.join("fifo");
        use std::os::unix::ffi::OsStrExt;
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: the CString is live and NUL-terminated; the path is test-owned.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert!(private_file(&fifo, false).is_err());
        let lock_path = state.join("init.lock");
        let first = private_file(&lock_path, true).unwrap();
        first.try_lock().unwrap();
        let second = private_file(&lock_path, true).unwrap();
        assert!(second.try_lock().is_err());
    }

    // The external driver owns a fresh HOME, private D-Bus and persistent test
    // keyring. Separate invocations permit an actual service/process restart.
    #[test]
    #[ignore = "requires explicitly isolated external Secret Service fixture"]
    fn private_secret_service_fixture() {
        assert_eq!(
            std::env::var("ART_TERM_TEST_PRIVATE_SS").as_deref(),
            Ok("1")
        );
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(home.starts_with("/tmp/art-term-ubuntu"));
        let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
        assert!(bus.starts_with("unix:path=/tmp/"));
        let application = "net.missart.artterm.test-vault";
        let path = default_config_path(application).unwrap();
        let phase = std::env::var("ART_TERM_TEST_SS_PHASE").unwrap();
        match phase.as_str() {
            "save" | "save-screen" => {
                let store = open_store(path.clone(), application).unwrap();
                if phase == "save" {
                    store
                        .add(
                            "Host".into(),
                            "https://fixture.example:8443".into(),
                            Secret::new("fixture-host-credential".into()).unwrap(),
                        )
                        .unwrap();
                }
                store
                    .save_desktop(
                        None,
                        None,
                        "Screen".into(),
                        "https://fixture.example/mcp".into(),
                        Some(Secret::new("fixture-screen-token".into()).unwrap()),
                    )
                    .unwrap();
                let bytes = fs::read(&path).unwrap();
                for secret in ["fixture-host-credential", "fixture-screen-token"] {
                    assert!(!bytes
                        .windows(secret.len())
                        .any(|part| part == secret.as_bytes()));
                }
                assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
            }
            "read" | "repair" => {
                if phase == "repair" {
                    fs::remove_file(
                        base_directory("XDG_DATA_HOME", ".local/share")
                            .unwrap()
                            .join(application)
                            .join(MARKER),
                    )
                    .unwrap();
                }
                let store = open_store(path, application).unwrap();
                let host = store.list().unwrap().remove(0);
                assert_eq!(
                    store.snapshot(&host.id).unwrap().credential.expose(),
                    "fixture-host-credential"
                );
                let screen = store.desktops().unwrap().remove(0);
                assert_eq!(
                    store.desktop(&screen.id).unwrap().token.expose(),
                    "fixture-screen-token"
                );
            }
            "remove-marker" | "corrupt-marker" => {
                let marker = base_directory("XDG_DATA_HOME", ".local/share")
                    .unwrap()
                    .join(application)
                    .join(MARKER);
                if phase == "remove-marker" {
                    fs::remove_file(marker).unwrap();
                } else {
                    fs::write(marker, "corrupt-marker").unwrap();
                }
            }
            "assert-no-key" => {
                let service = SecretService::connect(EncryptionType::Dh).unwrap();
                let matches = service.search_items(attributes(application)).unwrap();
                assert!(matches.unlocked.is_empty() && matches.locked.is_empty());
            }
            "delete-key" | "duplicate-key" | "replace-key" | "lock" => {
                let service = SecretService::connect(EncryptionType::Dh).unwrap();
                let collection = service.get_default_collection().unwrap();
                if phase == "lock" {
                    collection.lock().unwrap();
                    return;
                }
                if phase != "duplicate-key" {
                    let items = service.search_items(attributes(application)).unwrap();
                    for item in items.unlocked.into_iter().chain(items.locked) {
                        item.delete().unwrap();
                    }
                }
                if phase != "delete-key" {
                    collection
                        .create_item(
                            "Fixture replacement",
                            attributes(application),
                            &[9; 32],
                            false,
                            "application/octet-stream",
                        )
                        .unwrap();
                }
            }
            "refuse" => {
                let before = fs::read(&path).unwrap();
                assert!(open_store(path.clone(), application).is_err());
                assert_eq!(fs::read(path).unwrap(), before);
            }
            _ => panic!("Unknown fixture phase"),
        }
    }
}
