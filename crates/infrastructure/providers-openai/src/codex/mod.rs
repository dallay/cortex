//! Isolated Codex OAuth links. Credentials never enter the routing registry.

mod oauth;
#[cfg(test)]
mod tests;

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rook_core::codex_accounts::{
    CodexAccount, CodexAccountError as Error, CodexAccounts, CodexAccountsPort, CodexAttempt,
    CodexCallback, CodexLogin, CodexLoginStatus as Status,
};
use rook_core::KeyManager;
use shared_kernel::RequestId;
use tokio::sync::Mutex;

const MAX_ACCOUNTS: usize = 32;
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
    _root_lock: fs::File,
    state: Mutex<State>,
    login_timeout: Duration,
}

struct State {
    accounts: Vec<CodexAccount>,
    attempt: Option<CodexAttempt>,
    pending: Option<oauth::Pending>,
    task: Option<tokio::task::JoinHandle<()>>,
    closing: bool,
}

impl CodexProfileStore {
    pub fn open(
        client_id: &str,
        redirect_uri: &str,
        port: u16,
        root: &Path,
        key_manager: Arc<dyn KeyManager>,
    ) -> anyhow::Result<Self> {
        Self::open_with_client(
            oauth::Client::new(client_id, redirect_uri, port)?,
            root,
            key_manager,
        )
    }

    fn open_with_client(
        client: oauth::Client,
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
                _root_lock: root_lock,
                state: Mutex::new(State {
                    accounts,
                    attempt: None,
                    pending: None,
                    task: None,
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
    let profile = root.join(id);
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
        fs::File::open(&profile)?.sync_all()?;
        fs::File::open(root)?.sync_all()
    };
    if let Err(error) = write() {
        if fs::remove_dir_all(profile).is_err() {
            tracing::warn!("could not remove incomplete Codex account directory");
        }
        return Err(error.into());
    }
    Ok(())
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
        if let Some(task) = self.task.take() {
            task.abort();
        }
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
        let id = RequestId::new().to_string();
        let (auth_url, pending) = self.inner.client.authorize(self.inner.login_timeout);
        let deadline = pending.deadline;
        state.pending = Some(pending);
        state.attempt = Some(CodexAttempt {
            id: id.clone(),
            status: Status::Pending,
        });
        let inner = Arc::downgrade(&self.inner);
        let pending_id = id.clone();
        state.task = Some(tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if let Some(inner) = inner.upgrade() {
                let mut state = inner.state.lock().await;
                if state.pending.is_some() && state.active(&pending_id) {
                    state.finish(Status::Expired);
                }
            }
        }));
        Ok(CodexLogin { id, auth_url })
    }

    async fn callback(&self, oauth_state: &str, response: CodexCallback) -> Result<(), Error> {
        let mut state = self.inner.state.lock().await;
        let Some(pending) = state
            .pending
            .take_if(|pending| pending.matches(oauth_state))
        else {
            return Err(Error::NotFound);
        };
        state.stop_task();
        match response {
            CodexCallback::Denied => state.finish(Status::Cancelled),
            CodexCallback::Failed => state.finish(Status::Failed),
            CodexCallback::Code(code) => {
                let id = state.attempt.as_ref().expect("active attempt").id.clone();
                let inner = self.inner.clone();
                state.task = Some(tokio::spawn(async move {
                    let result = inner
                        .client
                        .exchange(&pending.verifier, &code)
                        .await
                        .and_then(|tokens| Ok(inner.key_manager.encrypt(&tokens)?));
                    let mut state = inner.state.lock().await;
                    if !state.active(&id) {
                        return;
                    }
                    if result
                        .and_then(|ciphertext| save_account(&inner.root, &id, &ciphertext))
                        .is_ok()
                    {
                        state.accounts.push(CodexAccount { id });
                        state.finish(Status::Linked);
                    } else {
                        state.finish(Status::Failed);
                    }
                }));
            }
        }
        Ok(())
    }

    async fn cancel(&self, id: &str) -> Result<(), Error> {
        let mut state = self.inner.state.lock().await;
        if !state.active(id) {
            return Err(Error::NotFound);
        }
        state.stop_task();
        state.finish(Status::Cancelled);
        Ok(())
    }

    async fn shutdown(&self) {
        let mut state = self.inner.state.lock().await;
        state.closing = true;
        let task = state.task.take();
        if state
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.status == Status::Pending)
        {
            state.finish(Status::Cancelled);
        }
        drop(state);
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
}
