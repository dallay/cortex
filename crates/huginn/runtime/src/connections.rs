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
    #[cfg(test)]
    probe: Arc<tests::WorkerProbe>,
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
            #[cfg(test)]
            probe: Arc::new(tests::WorkerProbe::default()),
        })
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
        #[cfg(test)]
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
        #[cfg(test)]
        let probe = self.probe.clone();
        #[cfg(test)]
        probe
            .admitted
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let _completion = tests::WorkerCompletion(probe.clone());
            let _operation = operation;
            let _lease = lease;
            let db = db;
            #[cfg(test)]
            probe.pause();
            let db = db.lock().map_err(|_| ConnectionError::Metadata)?;
            let rows = db.execute("UPDATE connection_metadata SET version=?1, document=?2 WHERE id=1 AND version=?3", rusqlite::params![version, document, previous]).map_err(|_| ConnectionError::Metadata)?;
            drop(db);
            if rows == 1 { Ok(()) } else { Err(ConnectionError::Conflict) }
        }).await.map_err(|_| ConnectionError::Metadata)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Condvar,
    };

    #[derive(Default)]
    pub(super) struct WorkerProbe {
        pub(super) admitted: AtomicUsize,
        paused: Mutex<bool>,
        released: Condvar,
        entered: tokio::sync::Notify,
        completed: tokio::sync::Notify,
    }
    impl WorkerProbe {
        pub(super) fn pause(&self) {
            let mut paused = self.paused.lock().unwrap();
            if *paused {
                self.entered.notify_one();
                while *paused {
                    paused = self.released.wait(paused).unwrap();
                }
            }
            drop(paused);
        }
        fn release(&self) {
            *self.paused.lock().unwrap() = false;
            self.released.notify_all();
        }
    }
    pub(super) struct WorkerCompletion(pub(super) Arc<WorkerProbe>);
    impl Drop for WorkerCompletion {
        fn drop(&mut self) {
            self.0.completed.notify_one();
        }
    }
    struct ReleaseOnDrop(Arc<WorkerProbe>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    async fn paused_save(
        store: &Arc<SqliteConnections>,
    ) -> (ReleaseOnDrop, tokio::task::JoinHandle<ConnectionResult<()>>) {
        *store.probe.paused.lock().unwrap() = true;
        let release = ReleaseOnDrop(store.probe.clone());
        let writer = store.clone();
        let task = tokio::spawn(async move {
            writer
                .save(
                    0,
                    &ConnectionMetadata {
                        version: 1,
                        ..Default::default()
                    },
                )
                .await
        });
        store.probe.entered.notified().await;
        (release, task)
    }

    #[tokio::test]
    async fn recovery_load_cannot_overtake_cancelled_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteConnections::open(&dir.path().join("metadata.db")).unwrap());
        let (release, task) = paused_save(&store).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let mut recovery = Box::pin(store.load());
        assert!(futures::poll!(&mut recovery).is_pending());
        // If recovery was admitted before the paused save, force its real SQLite
        // read to finish before releasing the writer: deterministic stale read.
        if store.probe.admitted.load(Ordering::SeqCst) == 1 {
            release.0.release();
        }
        let observed = recovery.await.unwrap();
        release.0.release();
        store.probe.completed.notified().await;
        assert_eq!(observed.version, 1);
    }

    #[tokio::test]
    async fn pending_worker_retains_lease_after_store_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.db");
        let store = Arc::new(SqliteConnections::open(&path).unwrap());
        let (release, task) = paused_save(&store).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(store);
        let reopened = SqliteConnections::open(&path);
        let retained = matches!(reopened, Err(ConnectionError::Conflict));
        drop(reopened);
        release.0.release();
        release.0.completed.notified().await;
        let reopened = SqliteConnections::open(&path).unwrap();
        assert_eq!(reopened.load().await.unwrap().version, 1);
        assert!(retained, "pending worker released the ownership lease");
    }

    #[tokio::test]
    async fn cancellation_while_waiting_does_not_admit_a_worker() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteConnections::open(&dir.path().join("metadata.db")).unwrap());
        let (release, task) = paused_save(&store).await;
        let next = ConnectionMetadata {
            version: 2,
            ..Default::default()
        };
        let mut waiting_save = Box::pin(store.save(1, &next));
        assert!(futures::poll!(&mut waiting_save).is_pending());
        drop(waiting_save);
        let mut waiting_load = Box::pin(store.load());
        assert!(futures::poll!(&mut waiting_load).is_pending());
        drop(waiting_load);
        assert_eq!(store.probe.admitted.load(Ordering::SeqCst), 1);
        release.0.release();
        task.await.unwrap().unwrap();
        assert_eq!(store.load().await.unwrap().version, 1);
        store.save(1, &next).await.unwrap();
        assert_eq!(store.load().await.unwrap().version, 2);
    }

    #[tokio::test]
    async fn failed_worker_releases_admission_and_lease() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.db");
        let store = SqliteConnections::open(&path).unwrap();
        store
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE connection_metadata")
            .unwrap();
        let next = ConnectionMetadata {
            version: 1,
            ..Default::default()
        };
        assert!(matches!(
            store.save(0, &next).await,
            Err(ConnectionError::Metadata)
        ));
        assert!(matches!(store.load().await, Err(ConnectionError::Metadata)));
        drop(store);
        let reopened = SqliteConnections::open(&path).unwrap();
        reopened.save(0, &next).await.unwrap();
        assert_eq!(reopened.load().await.unwrap().version, 1);
    }
}
