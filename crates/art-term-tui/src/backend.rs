use crate::{
    tab::{self, TabClient, TabId, TerminalSize},
    vault::Vault,
};
use art_term_core::{
    instance_mcp::DesktopProfile,
    openab::{
        api::{AdminClient, ApiError, SessionGrant},
        history::{self, HistoryVersion},
        profiles::{
            ExportSelection, HostProfile, HostSnapshot, HostStore, ImportAction, ImportDetails,
            ImportPreview, ImportSelection, TransferKind,
        },
        screenshot::ShellOwnership,
        secret::Secret,
        session::{SessionBinding, SessionError, SessionHandle},
        transfer,
    },
};
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{watch, Mutex as AsyncMutex},
    task::JoinHandle,
};
use zeroize::Zeroizing;

pub(crate) type Result<T> = std::result::Result<T, BackendError>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum BackendError {
    Api(ApiError),
    Session(SessionError),
    Busy,
    InvalidFile,
}
impl From<ApiError> for BackendError {
    fn from(value: ApiError) -> Self {
        Self::Api(value)
    }
}
impl From<SessionError> for BackendError {
    fn from(value: SessionError) -> Self {
        Self::Session(value)
    }
}
impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api(e) => e.fmt(f),
            Self::Session(e) => e.fmt(f),
            Self::Busy => f.write_str("Operation busy or client shutting down"),
            Self::InvalidFile => f.write_str("Transfer file unavailable or invalid"),
        }
    }
}
impl std::error::Error for BackendError {}

#[derive(Clone, Debug)]
pub(crate) struct HostSelection {
    pub id: String,
    pub revision: u64,
}
impl From<&HostProfile> for HostSelection {
    fn from(p: &HostProfile) -> Self {
        Self {
            id: p.id.clone(),
            revision: p.revision,
        }
    }
}
pub(crate) struct HostEdit {
    pub previous: Option<HostSelection>,
    pub name: String,
    pub base_url: String,
    pub credential: Zeroizing<String>,
}
#[derive(Clone)]
pub(crate) struct ProfileView {
    pub hosts: Vec<HostProfile>,
    pub screens: Vec<DesktopProfile>,
}
#[derive(Clone)]
pub(crate) struct ShellSelection {
    binding: SessionBinding,
    attached: bool,
}
#[derive(Clone)]
pub(crate) struct ShellView {
    pub label: String,
    pub attached: bool,
    pub selection: ShellSelection,
}
pub(crate) struct ShellListView {
    pub host: HostSelection,
    pub shells: Vec<ShellView>,
    pub draining: bool,
}
#[derive(Clone)]
pub(crate) enum OpenShell {
    Create {
        host: HostSelection,
    },
    Attach {
        selection: ShellSelection,
        confirmed: bool,
    },
}
#[derive(Clone)]
pub(crate) struct ImportEntryView {
    pub name: String,
    pub kind: &'static str,
    pub status: &'static str,
    pub has_secret: bool,
}
pub(crate) struct ImportView {
    pub ticket: u64,
    pub entries: Vec<ImportEntryView>,
}
pub(crate) struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

#[derive(Clone)]
pub(super) struct Connector {
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    ca: Option<Arc<Vec<u8>>>,
}
impl Connector {
    fn new() -> Result<Self> {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        {
            let ca = match std::env::var_os("ART_TERM_OPENAB_FIXTURE_CA") {
                None => None,
                Some(path) => {
                    let path = PathBuf::from(path);
                    let metadata =
                        std::fs::symlink_metadata(&path).map_err(|_| ApiError::Storage)?;
                    if !path.is_absolute()
                        || !metadata.is_file()
                        || metadata.file_type().is_symlink()
                        || metadata.len() > 65536
                    {
                        return Err(ApiError::Storage.into());
                    }
                    Some(Arc::new(
                        std::fs::read(path).map_err(|_| ApiError::Storage)?,
                    ))
                }
            };
            Ok(Self { ca })
        }
        #[cfg(not(all(debug_assertions, feature = "openab-fixture")))]
        Ok(Self {})
    }
    fn admin(&self) -> Result<AdminClient> {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        if let Some(ca) = &self.ca {
            return Ok(AdminClient::fixture_ca(ca)?);
        }
        Ok(AdminClient::new()?)
    }
    pub(super) async fn attach(
        &self,
        host: HostSnapshot,
        grant: SessionGrant,
        since: u64,
        generation: u64,
    ) -> std::result::Result<SessionHandle, SessionError> {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        if let Some(ca) = &self.ca {
            return SessionHandle::attach_fixture(host, grant, since, generation, ca).await;
        }
        SessionHandle::attach(host, grant, since, generation).await
    }
    pub(super) async fn attach_token(
        &self,
        binding: SessionBinding,
        token: Secret,
        since: u64,
        generation: u64,
    ) -> std::result::Result<SessionHandle, SessionError> {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        if let Some(ca) = &self.ca {
            return SessionHandle::attach_token_fixture(binding, token, since, generation, ca)
                .await;
        }
        SessionHandle::attach_token(binding, token, since, generation).await
    }
}

pub(super) struct Registration {
    pub host_id: String,
    pub cancel: watch::Sender<bool>,
    pub task: JoinHandle<()>,
}
type OwnedNames = Arc<Mutex<HashSet<(String, String)>>>;
pub(super) struct Ownership {
    host: String,
    names: OwnedNames,
}
impl ShellOwnership for Ownership {
    fn mark(&self, name: &str) -> std::result::Result<(), SessionError> {
        let mut names = self.names.lock().map_err(|_| SessionError::Internal)?;
        if names.len() >= 128 || !names.insert((self.host.clone(), name.into())) {
            return Err(SessionError::Capacity);
        }
        Ok(())
    }
    fn unmark(&self, name: &str) {
        if let Ok(mut names) = self.names.lock() {
            names.remove(&(self.host.clone(), name.into()));
        }
    }
}

struct Inner {
    vault: Mutex<Option<Vault>>,
    pub store: Arc<HostStore>,
    api: AdminClient,
    connector: Connector,
    operations: AsyncMutex<()>,
    tabs: Mutex<HashMap<TabId, Registration>>,
    previews: Mutex<HashMap<u64, ImportPreview>>,
    next_id: AtomicU64,
    owned: OwnedNames,
    closed: AtomicBool,
}
#[derive(Clone)]
pub(crate) struct Backend {
    inner: Arc<Inner>,
}

impl Backend {
    pub(crate) fn start(vault: Vault) -> Result<Self> {
        let connector = Connector::new()?;
        let api = connector.admin()?;
        let store = vault.store();
        Ok(Self {
            inner: Arc::new(Inner {
                vault: Mutex::new(Some(vault)),
                store,
                api,
                connector,
                operations: AsyncMutex::new(()),
                tabs: Mutex::new(HashMap::new()),
                previews: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                owned: Arc::new(Mutex::new(HashSet::new())),
                closed: AtomicBool::new(false),
            }),
        })
    }
    fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            Err(BackendError::Busy)
        } else {
            Ok(())
        }
    }
    fn snapshot(&self, selected: &HostSelection) -> Result<HostSnapshot> {
        let snapshot = self.inner.store.snapshot(&selected.id)?;
        if snapshot.profile.revision != selected.revision {
            return Err(ApiError::StateChanged.into());
        }
        Ok(snapshot)
    }
    fn current(&self, profile: &HostProfile) -> Result<()> {
        self.check_open()?;
        if !self.inner.store.is_current(profile)? {
            return Err(ApiError::StateChanged.into());
        }
        Ok(())
    }
    pub(crate) async fn profiles(&self) -> Result<ProfileView> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        Ok(ProfileView {
            hosts: self.inner.store.list()?,
            screens: self.inner.store.desktops()?,
        })
    }
    pub(crate) async fn save_host(&self, edit: HostEdit) -> Result<HostSelection> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        let credential = if edit.credential.is_empty() {
            None
        } else {
            Some(Secret::new(edit.credential.to_string())?)
        };
        let profile = match edit.previous {
            Some(previous) => {
                let profile = self.inner.store.update(
                    &previous.id,
                    previous.revision,
                    edit.name,
                    edit.base_url,
                    credential,
                )?;
                self.cancel_host(&previous.id);
                profile
            }
            None => {
                self.inner
                    .store
                    .add(edit.name, edit.base_url, credential.ok_or(ApiError::Auth)?)?
            }
        };
        Ok(HostSelection::from(&profile))
    }
    fn cancel_host(&self, id: &str) {
        if let Ok(tabs) = self.inner.tabs.lock() {
            for registration in tabs.values().filter(|r| r.host_id == id) {
                let _ = registration.cancel.send(true);
            }
        }
    }
    pub(crate) async fn delete_host(&self, host: HostSelection) -> Result<()> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        self.inner.store.delete(&host.id, host.revision)?;
        self.cancel_host(&host.id);
        Ok(())
    }
    pub(crate) async fn test_host(&self, host: HostSelection) -> Result<usize> {
        Ok(self.list_shells(host).await?.shells.len())
    }
    pub(crate) async fn list_shells(&self, host: HostSelection) -> Result<ShellListView> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        let snapshot = self.snapshot(&host)?;
        let list = self.inner.api.list(&snapshot).await?;
        self.current(&snapshot.profile)?;
        let owned = self.inner.owned.lock().map_err(|_| ApiError::Storage)?;
        let mut shells = Vec::new();
        for item in &list.sessions {
            if !item.alive || owned.contains(&(host.id.clone(), item.name.clone())) {
                continue;
            }
            let binding = SessionBinding::from_list(snapshot.profile.clone(), &list, &item.name)?;
            shells.push(ShellView {
                label: item.name.clone(),
                attached: item.attached,
                selection: ShellSelection {
                    binding,
                    attached: item.attached,
                },
            });
        }
        Ok(ShellListView {
            host,
            shells,
            draining: list.draining,
        })
    }
    pub(crate) async fn open(&self, action: OpenShell, size: TerminalSize) -> Result<TabClient> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        {
            let mut tabs = self.inner.tabs.lock().map_err(|_| ApiError::Storage)?;
            tabs.retain(|_, r| !r.task.is_finished());
            if tabs.len() >= 32 {
                return Err(ApiError::Capacity.into());
            }
        }
        let (snapshot, grant, bootstrap) = match action {
            OpenShell::Create { host } => {
                let snapshot = self.snapshot(&host)?;
                let raw = uuid::Uuid::new_v4().simple().to_string();
                let name = format!("th2-{}", &raw[..24]);
                let grant = self.inner.api.create(&snapshot, &name).await?;
                self.current(&snapshot.profile)?;
                (snapshot, grant, true)
            }
            OpenShell::Attach {
                selection,
                confirmed,
            } => {
                if !confirmed {
                    return Err(ApiError::StateChanged.into());
                }
                let snapshot = self.snapshot(&HostSelection::from(&selection.binding.profile))?;
                self.current(&selection.binding.profile)?;
                let listed = self.inner.api.list(&snapshot).await?;
                selection.binding.verify_list(&listed)?;
                let item = listed
                    .sessions
                    .iter()
                    .find(|s| s.name == selection.binding.name)
                    .ok_or(ApiError::SessionMissing)?;
                if item.attached != selection.attached {
                    return Err(ApiError::StateChanged.into());
                }
                self.current(&snapshot.profile)?;
                let grant = self
                    .inner
                    .api
                    .renew(&snapshot, &selection.binding.name)
                    .await?;
                selection.binding.verify_grant(&grant, false)?;
                self.current(&snapshot.profile)?;
                (snapshot, grant, false)
            }
        };
        let profile = snapshot.profile.clone();
        let handle = self.inner.connector.attach(snapshot, grant, 0, 1).await?;
        if let Err(error) = self.current(&profile) {
            handle.shutdown().await;
            return Err(error);
        }
        handle.resize(size.rows, size.cols)?;
        if bootstrap {
            let key =
                history::key_from_name(&handle.binding().name).ok_or(SessionError::Protocol)?;
            handle.write(HistoryVersion::Paged.bootstrap_command(&key)?.as_bytes())?;
        }
        let id = TabId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let label = format!("{} / {}", profile.name, handle.binding().name);
        let ownership = Arc::new(Ownership {
            host: profile.id.clone(),
            names: self.inner.owned.clone(),
        });
        let (client, registration) = tab::spawn(
            label,
            handle,
            size,
            self.inner.store.clone(),
            self.inner.api.clone(),
            self.inner.connector.clone(),
            ownership,
        );
        self.inner
            .tabs
            .lock()
            .map_err(|_| ApiError::Storage)?
            .insert(id, registration);
        Ok(client)
    }
    pub(crate) async fn delete_shell(&self, shell: ShellSelection, confirmed: bool) -> Result<()> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        if !confirmed {
            return Err(ApiError::StateChanged.into());
        }
        let snapshot = self.snapshot(&HostSelection::from(&shell.binding.profile))?;
        let list = self.inner.api.list(&snapshot).await?;
        shell.binding.verify_list(&list)?;
        self.current(&snapshot.profile)?;
        self.inner
            .api
            .delete(&snapshot, &shell.binding.name)
            .await?;
        Ok(())
    }
    pub(crate) async fn preview_import(
        &self,
        path: PathBuf,
        password: Option<Zeroizing<String>>,
    ) -> Result<ImportView> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        let store = self.inner.store.clone();
        let preview = tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|_| BackendError::InvalidFile)?;
            if !file
                .metadata()
                .map_err(|_| BackendError::InvalidFile)?
                .is_file()
            {
                return Err(BackendError::InvalidFile);
            }
            let mut bytes = Zeroizing::new(Vec::new());
            std::io::Read::by_ref(&mut file)
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| BackendError::InvalidFile)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(BackendError::InvalidFile);
            }
            let payload = match transfer::decode(&bytes)? {
                transfer::Decoded::Public(payload) => payload,
                transfer::Decoded::Encrypted(file) => {
                    transfer::unlock(file, password.ok_or(ApiError::Auth)?)?
                }
            };
            Ok::<_, BackendError>(store.preview_import(payload)?)
        })
        .await
        .map_err(|_| ApiError::Storage)??;
        self.check_open()?;
        let entries = preview
            .entries
            .iter()
            .map(|entry| ImportEntryView {
                name: entry.name.clone(),
                kind: match entry.details {
                    ImportDetails::Host { .. } => "Host",
                    ImportDetails::Screen { .. } => "Screen",
                },
                status: entry.match_status,
                has_secret: entry.has_secret(),
            })
            .collect();
        let ticket = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let mut previews = self.inner.previews.lock().map_err(|_| ApiError::Storage)?;
        previews.clear();
        previews.insert(ticket, preview);
        Ok(ImportView { ticket, entries })
    }
    pub(crate) async fn apply_import(
        &self,
        ticket: u64,
        replace_existing: bool,
    ) -> Result<ImportSummary> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        let preview = self
            .inner
            .previews
            .lock()
            .map_err(|_| ApiError::Storage)?
            .remove(&ticket)
            .ok_or(ApiError::StateChanged)?;
        let selections = preview
            .entries
            .iter()
            .map(|entry| ImportSelection {
                key: entry.key.clone(),
                action: if entry.match_status == "new" && entry.has_secret() {
                    ImportAction::Add
                } else if entry.match_status == "duplicate" && replace_existing {
                    ImportAction::Update
                } else {
                    ImportAction::Skip
                },
                credential: None,
            })
            .collect();
        let result = self.inner.store.apply_import(preview, selections)?;
        if result.updated != 0 {
            if let Ok(tabs) = self.inner.tabs.lock() {
                for registration in tabs.values() {
                    let _ = registration.cancel.send(true);
                }
            }
        }
        Ok(ImportSummary {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
        })
    }
    pub(crate) async fn cancel_import(&self, ticket: u64) {
        if let Ok(mut previews) = self.inner.previews.lock() {
            previews.remove(&ticket);
        }
    }
    pub(crate) async fn export(&self, path: PathBuf, password: Zeroizing<String>) -> Result<()> {
        let _operation = self.inner.operations.lock().await;
        self.check_open()?;
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut selected: Vec<_> = store
                .list()?
                .into_iter()
                .map(|p| ExportSelection {
                    kind: TransferKind::Host,
                    id: p.id,
                    expected_revision: p.revision,
                })
                .collect();
            selected.extend(store.desktops()?.into_iter().map(|p| ExportSelection {
                kind: TransferKind::Screen,
                id: p.id,
                expected_revision: p.revision,
            }));
            let payload = store.export_snapshot(&selected, true, None)?;
            let bytes = Zeroizing::new(transfer::encode(payload, Some(password))?);
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut output =
                tempfile::NamedTempFile::new_in(parent).map_err(|_| ApiError::Storage)?;
            output.write_all(&bytes).map_err(|_| ApiError::Storage)?;
            output.as_file().sync_all().map_err(|_| ApiError::Storage)?;
            output
                .persist_noclobber(&path)
                .map_err(|_| ApiError::Storage)?;
            std::fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|_| ApiError::Storage)?;
            Ok::<_, BackendError>(())
        })
        .await
        .map_err(|_| ApiError::Storage)??;
        Ok(())
    }
    pub(crate) async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let _operation = self.inner.operations.lock().await;
        let registrations: Vec<_> = self
            .inner
            .tabs
            .lock()
            .map(|mut tabs| tabs.drain().map(|(_, r)| r).collect())
            .unwrap_or_default();
        for registration in &registrations {
            let _ = registration.cancel.send(true);
        }
        for registration in registrations {
            let _ = registration.task.await;
        }
        // Core cleanup guards can finish a final bounded DELETE after a failed close.
        let until = tokio::time::Instant::now() + Duration::from_secs(11);
        while self
            .inner
            .owned
            .lock()
            .map(|names| !names.is_empty())
            .unwrap_or(false)
            && tokio::time::Instant::now() < until
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if let Ok(mut previews) = self.inner.previews.lock() {
            previews.clear();
        }
        if let Ok(mut vault) = self.inner.vault.lock() {
            vault.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::VaultPaths;

    fn backend() -> (tempfile::TempDir, Backend) {
        let root = tempfile::Builder::new()
            .prefix("art-term-tui-backend-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let paths = VaultPaths::resolve(Some(root.path().join("connections.toml"))).unwrap();
        let vault =
            Vault::initialize(paths, Zeroizing::new("backend fixture password".into())).unwrap();
        (root, Backend::start(vault).unwrap())
    }

    async fn add(backend: &Backend) -> HostSelection {
        backend
            .save_host(HostEdit {
                previous: None,
                name: "Fixture".into(),
                base_url: "https://fixture.example".into(),
                credential: Zeroizing::new("fixture-secret".into()),
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn profile_edits_preserve_metadata_and_reject_stale_revisions_or_missing_replacement() {
        let (_root, backend) = backend();
        let first = add(&backend).await;
        let metadata = art_term_core::connections::ConnectionMetadata {
            category: art_term_core::connections::Category::Agent,
            ..Default::default()
        };
        let changed = backend
            .inner
            .store
            .update_with_metadata(
                &first.id,
                first.revision,
                "Fixture".into(),
                "https://fixture.example".into(),
                None,
                Some(metadata.clone()),
            )
            .unwrap();
        let current = HostSelection::from(&changed);
        let saved = backend
            .save_host(HostEdit {
                previous: Some(current.clone()),
                name: "Renamed".into(),
                base_url: "https://fixture.example".into(),
                credential: Zeroizing::new(String::new()),
            })
            .await
            .unwrap();
        let snapshot = backend.snapshot(&saved).unwrap();
        assert_eq!(snapshot.credential.expose(), "fixture-secret");
        assert_eq!(snapshot.profile.metadata, Some(metadata));
        assert!(backend
            .save_host(HostEdit {
                previous: Some(current),
                name: "Stale".into(),
                base_url: "https://fixture.example".into(),
                credential: Zeroizing::new(String::new())
            })
            .await
            .is_err());
        assert!(backend
            .save_host(HostEdit {
                previous: Some(saved.clone()),
                name: "Redirect".into(),
                base_url: "https://another.example".into(),
                credential: Zeroizing::new(String::new())
            })
            .await
            .is_err());
        assert_eq!(
            backend.snapshot(&saved).unwrap().profile.base_url,
            "https://fixture.example/"
        );
        backend.shutdown().await;
        assert!(matches!(backend.profiles().await, Err(BackendError::Busy)));
    }

    #[tokio::test]
    async fn encrypted_host_screen_transfer_is_private_no_overwrite_and_one_shot() {
        use std::os::unix::fs::PermissionsExt;
        let (source_root, source) = backend();
        add(&source).await;
        source
            .inner
            .store
            .save_desktop(
                None,
                None,
                "Screen".into(),
                "https://screen.example/mcp".into(),
                Some(Secret::new("screen-fixture-token".into()).unwrap()),
            )
            .unwrap();
        let output = source_root.path().join("transfer.json");
        source
            .export(
                output.clone(),
                Zeroizing::new("portable fixture password".into()),
            )
            .await
            .unwrap();
        let before = std::fs::read(&output).unwrap();
        assert!(!before
            .windows(b"fixture-secret".len())
            .any(|window| window == b"fixture-secret"));
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o077,
            0
        );
        assert!(source
            .export(
                output.clone(),
                Zeroizing::new("portable fixture password".into())
            )
            .await
            .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), before);
        let (_destination_root, destination) = backend();
        let preview = destination
            .preview_import(
                output.clone(),
                Some(Zeroizing::new("portable fixture password".into())),
            )
            .await
            .unwrap();
        assert_eq!(preview.entries.len(), 2);
        assert!(preview.entries.iter().all(|entry| entry.has_secret));
        let result = destination
            .apply_import(preview.ticket, false)
            .await
            .unwrap();
        assert_eq!((result.added, result.updated, result.skipped), (2, 0, 0));
        assert!(destination
            .apply_import(preview.ticket, false)
            .await
            .is_err());
        let profiles = destination.profiles().await.unwrap();
        assert_eq!((profiles.hosts.len(), profiles.screens.len()), (1, 1));
        assert_eq!(
            destination
                .inner
                .store
                .snapshot(&profiles.hosts[0].id)
                .unwrap()
                .credential
                .expose(),
            "fixture-secret"
        );
        assert_eq!(
            destination
                .inner
                .store
                .desktop(&profiles.screens[0].id)
                .unwrap()
                .token
                .expose(),
            "screen-fixture-token"
        );
        let duplicate = destination
            .preview_import(
                output,
                Some(Zeroizing::new("portable fixture password".into())),
            )
            .await
            .unwrap();
        let skipped = destination
            .apply_import(duplicate.ticket, false)
            .await
            .unwrap();
        assert_eq!((skipped.added, skipped.updated, skipped.skipped), (0, 0, 2));
        source.shutdown().await;
        destination.shutdown().await;
    }

    #[tokio::test]
    async fn import_refuses_fifo_without_waiting_for_a_writer() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let (root, backend) = backend();
        let path = root.path().join("fifo");
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), backend.preview_import(path, None))
                .await
                .unwrap()
                .is_err()
        );
        backend.shutdown().await;
    }
}
