//! Provider metadata is separate from sessions. OS stores never fall back to files.
use async_trait::async_trait;
use huginn_core::connections::*;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

/// Constructing this store is an explicit choice; it is never a fallback backend.
#[derive(Default)]
pub struct EphemeralSecrets(Mutex<BTreeMap<SecretReference, Secret>>);
#[async_trait]
impl SecretStore for EphemeralSecrets {
    fn mode(&self) -> SecretMode {
        SecretMode::Ephemeral
    }
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret> {
        let entries = self.0.lock().map_err(|_| ConnectionError::SecretStore)?;
        Secret::new(
            entries
                .get(reference)
                .ok_or(ConnectionError::MissingSecret)?
                .expose()
                .to_owned(),
        )
    }
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()> {
        self.0
            .lock()
            .map_err(|_| ConnectionError::SecretStore)?
            .insert(reference.clone(), Secret::new(secret.expose().to_owned())?);
        Ok(())
    }
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()> {
        self.0
            .lock()
            .map_err(|_| ConnectionError::SecretStore)?
            .remove(reference);
        Ok(())
    }
}

pub struct OsSecrets;
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn os_operation() -> tokio::sync::OwnedMutexGuard<()> {
    static SERIAL: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> = std::sync::OnceLock::new();
    SERIAL
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
        .lock_owned()
        .await
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl OsSecrets {
    fn entry(reference: &SecretReference) -> ConnectionResult<keyring::Entry> {
        // Explicit credentials bypass process-global defaults and mock stores.
        #[cfg(target_os = "macos")]
        let credential = keyring::macos::MacCredential::new_with_target(
            None,
            "org.dallay.cortex.huginn.connections",
            &reference.account(),
        );
        #[cfg(target_os = "linux")]
        let credential = keyring::secret_service::SsCredential::new_with_target(
            None,
            "org.dallay.cortex.huginn.connections",
            &reference.account(),
        );
        credential
            .map(|c| keyring::Entry::new_with_credential(Box::new(c)))
            .map_err(Self::error)
    }
    fn error(error: keyring::Error) -> ConnectionError {
        // Never format source errors: vendor errors may contain credential bytes.
        match error {
            keyring::Error::NoEntry => ConnectionError::MissingSecret,
            keyring::Error::NoStorageAccess(_) => ConnectionError::Locked,
            keyring::Error::PlatformFailure(_) => ConnectionError::BackendUnavailable,
            _ => ConnectionError::SecretStore,
        }
    }
}
#[async_trait]
impl SecretStore for OsSecrets {
    fn mode(&self) -> SecretMode {
        SecretMode::Secure
    }
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let reference = reference.clone();
            let guard = os_operation().await;
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                Secret::new(
                    Self::entry(&reference)?
                        .get_password()
                        .map_err(Self::error)?,
                )
            })
            .await
            .map_err(|_| ConnectionError::BackendUnavailable)?
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = reference;
            Err(ConnectionError::BackendUnavailable)
        }
    }
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let reference = reference.clone();
            let secret = Secret::new(secret.expose().to_owned())?;
            let guard = os_operation().await;
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                Self::entry(&reference)?
                    .set_password(secret.expose())
                    .map_err(Self::error)
            })
            .await
            .map_err(|_| ConnectionError::BackendUnavailable)?
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (reference, secret);
            Err(ConnectionError::BackendUnavailable)
        }
    }
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let reference = reference.clone();
            let guard = os_operation().await;
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                match Self::entry(&reference)?.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(Self::error(error)),
                }
            })
            .await
            .map_err(|_| ConnectionError::BackendUnavailable)?
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = reference;
            Err(ConnectionError::BackendUnavailable)
        }
    }
}

/// One owner for each database across processes. SQLite CAS also rejects stale writers.
pub struct SqliteConnections {
    db: Arc<Mutex<rusqlite::Connection>>,
    operation: Arc<tokio::sync::Mutex<()>>,
    lease: Arc<std::fs::File>,
    #[cfg(all(feature = "test-hooks", debug_assertions))]
    probe: Arc<test_hooks::WorkerProbe>,
}
impl SqliteConnections {
    pub fn open(path: &Path) -> ConnectionResult<Self> {
        use fs2::FileExt;
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
        if let Some(parent) = parent {
            std::fs::create_dir_all(parent).map_err(|_| ConnectionError::Metadata)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lease = options
            .open(path.with_extension("connections.lock"))
            .map_err(|_| ConnectionError::Metadata)?;
        lease
            .try_lock_exclusive()
            .map_err(|_| ConnectionError::Conflict)?;
        // Create with restrictive permissions before SQLite opens it.
        options.open(path).map_err(|_| ConnectionError::Metadata)?;
        let db = rusqlite::Connection::open(path).map_err(|_| ConnectionError::Metadata)?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS connection_metadata (id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL, document TEXT NOT NULL);")
            .map_err(|_| ConnectionError::Metadata)?;
        let initial = serde_json::to_string(&ConnectionMetadata::default())
            .map_err(|_| ConnectionError::Metadata)?;
        db.execute(
            "INSERT OR IGNORE INTO connection_metadata VALUES(1, 0, ?1)",
            [&initial],
        )
        .map_err(|_| ConnectionError::Metadata)?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            operation: Arc::new(tokio::sync::Mutex::new(())),
            lease: Arc::new(lease),
            #[cfg(all(feature = "test-hooks", debug_assertions))]
            probe: Arc::new(test_hooks::WorkerProbe::default()),
        })
    }
}
#[cfg(all(feature = "test-hooks", debug_assertions))]
impl SqliteConnections {
    pub fn test_probe(&self) -> Arc<test_hooks::WorkerProbe> {
        self.probe.clone()
    }
    pub fn break_metadata_table_for_test(&self) {
        self.db
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE connection_metadata")
            .unwrap();
    }
}

#[async_trait]
impl ConnectionStore for SqliteConnections {
    async fn load(&self) -> ConnectionResult<ConnectionMetadata> {
        // Admit before spawning: cancellation must not let recovery overtake a
        // worker that has been queued but has not acquired the SQLite lock yet.
        let operation = self.operation.clone().lock_owned().await;
        let lease = self.lease.clone();
        let db = self.db.clone();
        #[cfg(all(feature = "test-hooks", debug_assertions))]
        self.probe
            .admitted
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            let _operation = operation;
            let _lease = lease;
            let db = db.lock().map_err(|_| ConnectionError::Metadata)?;
            let document: String = db
                .query_row(
                    "SELECT document FROM connection_metadata WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| ConnectionError::Metadata)?;
            drop(db);
            serde_json::from_str(&document).map_err(|_| ConnectionError::Metadata)
        })
        .await
        .map_err(|_| ConnectionError::Metadata)?
    }
    async fn save(&self, previous: u64, next: &ConnectionMetadata) -> ConnectionResult<()> {
        if previous.checked_add(1) != Some(next.version) {
            return Err(ConnectionError::Conflict);
        }
        let document = serde_json::to_string(next).map_err(|_| ConnectionError::Metadata)?;
        let version = i64::try_from(next.version).map_err(|_| ConnectionError::Metadata)?;
        let previous = i64::try_from(previous).map_err(|_| ConnectionError::Metadata)?;
        let operation = self.operation.clone().lock_owned().await;
        let lease = self.lease.clone();
        let db = self.db.clone();
        #[cfg(all(feature = "test-hooks", debug_assertions))]
        let probe = self.probe.clone();
        #[cfg(all(feature = "test-hooks", debug_assertions))]
        probe
            .admitted
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            #[cfg(all(feature = "test-hooks", debug_assertions))]
            let _completion = test_hooks::WorkerCompletion(probe.clone());
            let _operation = operation;
            let _lease = lease;
            let db = db;
            #[cfg(all(feature = "test-hooks", debug_assertions))]
            probe.pause();
            let db = db.lock().map_err(|_| ConnectionError::Metadata)?;
            let rows = db.execute("UPDATE connection_metadata SET version=?1, document=?2 WHERE id=1 AND version=?3", rusqlite::params![version, document, previous]).map_err(|_| ConnectionError::Metadata)?;
            drop(db);
            if rows == 1 { Ok(()) } else { Err(ConnectionError::Conflict) }
        }).await.map_err(|_| ConnectionError::Metadata)?
    }
}

#[cfg(all(feature = "test-hooks", debug_assertions))]
pub mod test_hooks {
    use std::sync::{atomic::AtomicUsize, Arc, Condvar, Mutex};

    #[derive(Default)]
    pub struct WorkerProbe {
        pub admitted: AtomicUsize,
        paused: Mutex<bool>,
        released: Condvar,
        pub entered: tokio::sync::Notify,
        pub completed: tokio::sync::Notify,
    }
    impl WorkerProbe {
        pub fn pause_worker(&self) {
            *self.paused.lock().unwrap() = true;
        }
        pub fn pause(&self) {
            let mut paused = self.paused.lock().unwrap();
            if *paused {
                self.entered.notify_one();
                while *paused {
                    paused = self.released.wait(paused).unwrap();
                }
                drop(paused);
            }
        }
        pub fn release_worker(&self) {
            *self.paused.lock().unwrap() = false;
            self.released.notify_all();
        }
    }
    pub struct WorkerCompletion(pub Arc<WorkerProbe>);
    impl Drop for WorkerCompletion {
        fn drop(&mut self) {
            self.0.completed.notify_one();
        }
    }
}
