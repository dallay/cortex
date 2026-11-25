//! Isolated Codex OAuth links. Credentials never enter the routing registry.

mod listener;
mod oauth;
#[cfg(test)]
mod tests;

use std::{
    collections::HashSet,
    fs,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rook_core::codex_accounts::{
    codex_redirect_uri, CodexAccount, CodexAccountError as Error, CodexAccounts, CodexAccountsPort,
    CodexAttempt, CodexCallback, CodexLogin, CodexLoginStatus as Status, CODEX_CALLBACK_PORT,
};
use rook_core::KeyManager;
use shared_kernel::RequestId;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};

const MAX_ACCOUNTS: usize = 32;
const MAX_CONCURRENT_DISCONNECTS: usize = 4;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const CREDENTIALS: &str = "tokens.encrypted";
const MAX_ENCRYPTED_BYTES: u64 = 128 * 1024;

pub struct CodexProfileStore {
    inner: Arc<Inner>,
}

struct Inner {
    client: oauth::Client,
    key_manager: Arc<dyn KeyManager>,
    root: PathBuf,
    callback_addr: SocketAddr,
    _root_lock: fs::File,
    state: Mutex<State>,
    login_timeout: Duration,
}

struct State {
    accounts: Vec<CodexAccount>,
    attempt: Option<CodexAttempt>,
    pending: Option<oauth::Pending>,
    task: Option<tokio::task::JoinHandle<()>>,
    listener: Option<tokio::task::JoinHandle<()>>,
    committing: bool,
    disconnecting: HashSet<String>,
    closing: bool,
}

impl CodexProfileStore {
    pub fn open(
        client_id: &str,
        root: &Path,
        key_manager: Arc<dyn KeyManager>,
    ) -> anyhow::Result<Self> {
        Self::open_with_client(
            oauth::Client::new(client_id)?,
            SocketAddr::from(([127, 0, 0, 1], CODEX_CALLBACK_PORT)),
            root,
            key_manager,
        )
    }

    fn open_with_client(
        client: oauth::Client,
        callback_addr: SocketAddr,
        root: &Path,
        key_manager: Arc<dyn KeyManager>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            root.is_absolute(),
            "Codex state directory path must be absolute"
        );
        let info = root.symlink_metadata()?;
        anyhow::ensure!(
            info.is_dir() && info.permissions().mode() & 0o077 == 0,
            "Codex state directory must be private and not a symlink"
        );
        let root = root.canonicalize()?.join("rook-codex-oauth-v1");
        create_private_directory(&root)?;
        let lock_path = root.join("instance.lock");
        let root_lock = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&lock_path)
            .or_else(|error| {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
                validate_private_file(&lock_path, 0)?;
                fs::OpenOptions::new().write(true).open(lock_path)
            })?;
        fs2::FileExt::try_lock_exclusive(&root_lock)?;
        let accounts = load_profiles(&root)?;
        Ok(Self {
            inner: Arc::new(Inner {
                client,
                key_manager,
                root,
                callback_addr,
                _root_lock: root_lock,
                state: Mutex::new(State {
                    accounts,
                    attempt: None,
                    pending: None,
                    task: None,
                    listener: None,
                    committing: false,
                    disconnecting: HashSet::new(),
                    closing: false,
                }),
                login_timeout: LOGIN_TIMEOUT,
            }),
        })
    }
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = path.symlink_metadata()?;
            if metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0 {
                Ok(())
            } else {
                Err(std::io::Error::other("invalid Codex profile directory"))
            }
        }
        Err(error) => Err(error),
    }
}

fn load_profiles(root: &Path) -> anyhow::Result<Vec<CodexAccount>> {
    let mut accounts = vec![];
    // Operator-configured state root is canonicalized and owner-private before enumeration.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    for file in fs::read_dir(root)? {
        let file = file?;
        let id = file.file_name();
        let Some(id) = id.to_str().filter(|id| valid_id(id)) else {
            continue;
        };
        let profile = file.path();
        anyhow::ensure!(
            file.file_type()?.is_dir()
                && profile.symlink_metadata()?.permissions().mode() & 0o077 == 0,
            "invalid Codex profile directory"
        );
        match validate_private_file(&profile.join(CREDENTIALS), MAX_ENCRYPTED_BYTES) {
            Ok(()) => {
                accounts.push(CodexAccount { id: id.to_owned() });
                anyhow::ensure!(
                    accounts.len() <= MAX_ACCOUNTS,
                    "Codex account limit exceeded"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::remove_dir_all(profile)?
            }
            Err(error) => return Err(error.into()),
        }
    }
    accounts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(accounts)
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn validate_private_file(path: &Path, max_len: u64) -> std::io::Result<()> {
    let info = path.symlink_metadata()?;
    if !info.is_file() || info.permissions().mode() & 0o077 != 0 || info.len() > max_len {
        return Err(std::io::Error::other("invalid Codex state file"));
    }
    Ok(())
}

fn save_account(root: &Path, id: &str, ciphertext: &str) -> anyhow::Result<()> {
    use std::io::Write;
    anyhow::ensure!(
        ciphertext.len() as u64 <= MAX_ENCRYPTED_BYTES,
        "Codex ciphertext exceeds limit"
    );
    anyhow::ensure!(valid_id(id), "invalid Codex profile ID");
    // The profile ID is validated as a UUID-shaped path component before joining the private root.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    let profile = root.join(id);
    // The joined profile path is confined to the private root by the validated profile ID.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    fs::DirBuilder::new().mode(0o700).create(&profile)?;
    let write = || -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(profile.join("tokens.tmp"))?;
        file.write_all(ciphertext.as_bytes())?;
        file.sync_all()?;
        fs::rename(profile.join("tokens.tmp"), profile.join(CREDENTIALS))?;
        Ok(())
    };
    if let Err(error) = write() {
        if fs::remove_dir_all(profile).is_err() {
            tracing::warn!("could not remove incomplete Codex account directory");
        }
        return Err(error.into());
    }

    let sync_directories = || -> std::io::Result<()> {
        // The profile ID is a generated UUID v4 and validated before any profile path is joined.
        // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
        fs::File::open(&profile)?.sync_all()?;
        // Operator-configured state root is canonicalized and owner-private before directory sync.
        // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
        fs::File::open(root)?.sync_all()
    };
    let _commit = finish_save_commit(sync_directories());
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveCommit {
    Durable,
    CommittedWithSyncWarning,
}

fn finish_save_commit(sync: std::io::Result<()>) -> SaveCommit {
    match sync {
        Ok(()) => SaveCommit::Durable,
        Err(_) => {
            tracing::warn!(
                failure_stage = "persistence_sync",
                "Codex account was committed but its directory sync failed"
            );
            SaveCommit::CommittedWithSyncWarning
        }
    }
}

fn remove_account(root: &Path, id: &str) -> Result<(), RemoveError> {
    if !valid_id(id) {
        return Err(RemoveError::Failed);
    }
    // The account ID is validated and the root is private and canonicalized before joining.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    let profile = root.join(id);
    let info = profile.symlink_metadata().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RemoveError::NotFound
        } else {
            RemoveError::Failed
        }
    })?;
    if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
        return Err(RemoveError::Failed);
    }
    // The profile path is derived from a validated ID inside the owner-private root.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    fs::remove_dir_all(&profile).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RemoveError::NotFound
        } else {
            RemoveError::Failed
        }
    })?;
    // The canonical owner-private root is opened only to sync its directory entry.
    // Evidence: docs/security/semgrep-path-exceptions.md#codex-profile-store
    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    fs::File::open(root)
        .and_then(|root| root.sync_all())
        .map_err(|_| RemoveError::RemovedButUnsynced)
}

#[derive(Debug)]
enum RemoveError {
    NotFound,
    Failed,
    RemovedButUnsynced,
}

fn finish_disconnect(
    state: &mut State,
    id: &str,
    removal: Result<(), RemoveError>,
) -> Result<(), Error> {
    state.disconnecting.remove(id);
    tracing::debug!(
        active_disconnects = state.disconnecting.len(),
        "Codex account disconnect completed"
    );
    match removal {
        Ok(()) => {
            state.accounts.retain(|account| account.id != id);
            Ok(())
        }
        Err(RemoveError::NotFound) => {
            state.accounts.retain(|account| account.id != id);
            Err(Error::NotFound)
        }
        Err(RemoveError::RemovedButUnsynced) => {
            state.accounts.retain(|account| account.id != id);
            tracing::warn!(
                failure_stage = "disconnect_sync",
                "Codex account was removed but the profile root could not be synced"
            );
            Err(Error::Unavailable)
        }
        Err(RemoveError::Failed) => Err(Error::Unavailable),
    }
}

#[derive(Clone, Copy)]
enum FailureStage {
    Exchange,
    Encryption,
    Persistence,
}

impl FailureStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Encryption => "encryption",
            Self::Persistence => "persistence",
        }
    }
}

fn record_failure(state: &mut State, id: &str, stage: FailureStage) {
    if state.active(id) {
        tracing::warn!(
            failure_stage = stage.as_str(),
            "Codex account linking failed"
        );
        state.finish(Status::Failed);
    }
}

impl State {
    fn finish(&mut self, status: Status) {
        self.pending = None;
        if let Some(attempt) = &mut self.attempt {
            attempt.status = status;
        }
    }

    fn active(&self, id: &str) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|attempt| attempt.id == id && attempt.status == Status::Pending)
    }

    fn stop_task(&mut self) {
        if !self.committing {
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }

    async fn stop_listener(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
            let _ = listener.await;
        }
    }
}

impl CodexProfileStore {
    fn from_inner(inner: Arc<Inner>) -> Self {
        Self { inner }
    }

    async fn handle_callback(
        &self,
        oauth_state: &str,
        response: CodexCallback,
        stop_listener: bool,
    ) -> Result<(), Error> {
        let mut state = self.inner.state.lock().await;
        let Some(pending) = state
            .pending
            .take_if(|pending| pending.matches(oauth_state))
        else {
            return Err(Error::NotFound);
        };
        state.stop_task();
        if stop_listener {
            state.stop_listener().await;
        }
        match response {
            CodexCallback::Denied => state.finish(Status::Cancelled),
            CodexCallback::Failed => state.finish(Status::Failed),
            CodexCallback::Code(code) => {
                let id = state.attempt.as_ref().expect("active attempt").id.clone();
                let inner = self.inner.clone();
                state.task = Some(tokio::spawn(async move {
                    let tokens = match inner
                        .client
                        .exchange(&pending.redirect_uri, &pending.verifier, &code)
                        .await
                    {
                        Ok(tokens) => tokens,
                        Err(_) => {
                            let mut state = inner.state.lock().await;
                            record_failure(&mut state, &id, FailureStage::Exchange);
                            return;
                        }
                    };
                    let ciphertext = match inner.key_manager.encrypt(&tokens) {
                        Ok(ciphertext) => ciphertext,
                        Err(_) => {
                            let mut state = inner.state.lock().await;
                            record_failure(&mut state, &id, FailureStage::Encryption);
                            return;
                        }
                    };

                    let mut state = inner.state.lock().await;
                    if !state.active(&id) || state.closing {
                        return;
                    }
                    state.committing = true;
                    drop(state);

                    let root = inner.root.clone();
                    let profile_id = id.clone();
                    let persisted = tokio::task::spawn_blocking(move || {
                        save_account(&root, &profile_id, &ciphertext)
                    })
                    .await
                    .is_ok_and(|result| result.is_ok());

                    let mut state = inner.state.lock().await;
                    state.committing = false;
                    if persisted && state.active(&id) {
                        state.accounts.push(CodexAccount { id });
                        state.finish(Status::Linked);
                    } else {
                        record_failure(&mut state, &id, FailureStage::Persistence);
                    }
                }));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl CodexAccountsPort for CodexProfileStore {
    async fn list(&self) -> Result<CodexAccounts, Error> {
        let state = self.inner.state.lock().await;
        Ok(CodexAccounts {
            enabled: !state.closing,
            accounts: state.accounts.clone(),
            attempt: state.attempt.clone(),
        })
    }

    async fn start(&self) -> Result<CodexLogin, Error> {
        let mut state = self.inner.state.lock().await;
        if state.closing {
            return Err(Error::Unavailable);
        }
        if state.committing {
            return Err(Error::Busy);
        }
        if state
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.status == Status::Pending)
        {
            return Err(Error::Busy);
        }
        if state.accounts.len() >= MAX_ACCOUNTS {
            return Err(Error::Capacity);
        }
        state.stop_listener().await;
        let listener = TcpListener::bind(self.inner.callback_addr)
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    Error::CallbackBusy
                } else {
                    Error::Unavailable
                }
            })?;
        let port = listener
            .local_addr()
            .map_err(|_| Error::Unavailable)?
            .port();
        let redirect_uri = codex_redirect_uri(port);
        let id = RequestId::new().to_string();
        let (auth_url, pending) = self
            .inner
            .client
            .authorize(&redirect_uri, self.inner.login_timeout);
        let deadline = pending.deadline;
        state.pending = Some(pending);
        state.attempt = Some(CodexAttempt {
            id: id.clone(),
            status: Status::Pending,
        });
        state.listener = Some(listener::spawn(Arc::downgrade(&self.inner), listener));
        let inner = Arc::downgrade(&self.inner);
        let pending_id = id.clone();
        state.task = Some(tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if let Some(inner) = inner.upgrade() {
                let mut state = inner.state.lock().await;
                if state.pending.is_some() && state.active(&pending_id) {
                    state.finish(Status::Expired);
                    state.stop_listener().await;
                }
            }
        }));
        Ok(CodexLogin { id, auth_url })
    }

    async fn callback(&self, oauth_state: &str, response: CodexCallback) -> Result<(), Error> {
        self.handle_callback(oauth_state, response, true).await
    }

    async fn cancel(&self, id: &str) -> Result<(), Error> {
        let mut state = self.inner.state.lock().await;
        if state.committing {
            return Err(Error::Busy);
        }
        if !state.active(id) {
            return Err(Error::NotFound);
        }
        state.stop_task();
        state.finish(Status::Cancelled);
        state.stop_listener().await;
        Ok(())
    }

    async fn disconnect(&self, id: &str) -> Result<(), Error> {
        let mut state = self.inner.state.lock().await;
        if state.closing {
            return Err(Error::Unavailable);
        }
        if !valid_id(id) || !state.accounts.iter().any(|account| account.id == id) {
            return Err(Error::NotFound);
        }
        if state.disconnecting.contains(id)
            || state.disconnecting.len() >= MAX_CONCURRENT_DISCONNECTS
        {
            return Err(Error::Busy);
        }
        state.disconnecting.insert(id.to_owned());
        tracing::debug!(
            active_disconnects = state.disconnecting.len(),
            "Starting Codex account disconnect"
        );
        drop(state);

        let root = self.inner.root.clone();
        let profile_id = id.to_owned();
        let inner = self.inner.clone();
        let (response, result) = oneshot::channel();
        tokio::spawn(async move {
            let removal_id = profile_id.clone();
            let removal = tokio::task::spawn_blocking(move || remove_account(&root, &removal_id))
                .await
                .unwrap_or(Err(RemoveError::Failed));
            let mut state = inner.state.lock().await;
            let result = finish_disconnect(&mut state, &profile_id, removal);
            let _ = response.send(result);
        });
        result.await.unwrap_or(Err(Error::Unavailable))
    }

    async fn shutdown(&self) {
        let mut state = self.inner.state.lock().await;
        state.closing = true;
        let wait_for_commit = state.committing;
        let task = state.task.take();
        let listener = state.listener.take();
        if !wait_for_commit
            && state
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.status == Status::Pending)
        {
            state.finish(Status::Cancelled);
        }
        drop(state);
        if let Some(listener) = listener {
            listener.abort();
            let _ = listener.await;
        }
        if let Some(task) = task {
            if !wait_for_commit {
                task.abort();
            }
            let _ = task.await;
        }
    }
}
