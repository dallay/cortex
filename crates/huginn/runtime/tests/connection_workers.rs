use huginn_core::connections::*;
use huginn_runtime::connections::{test_hooks::WorkerProbe, SqliteConnections};
use std::sync::{atomic::Ordering, Arc};

struct ReleaseOnDrop(Arc<WorkerProbe>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release_worker();
    }
}

async fn paused_save(
    store: &Arc<SqliteConnections>,
) -> (ReleaseOnDrop, tokio::task::JoinHandle<ConnectionResult<()>>) {
    let probe = store.test_probe();
    probe.pause_worker();
    let release = ReleaseOnDrop(probe.clone());
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
    probe.entered.notified().await;
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
    if store.test_probe().admitted.load(Ordering::SeqCst) == 1 {
        release.0.release_worker();
    }
    let observed = recovery.await.unwrap();
    release.0.release_worker();
    store.test_probe().completed.notified().await;
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
    release.0.release_worker();
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
    assert_eq!(store.test_probe().admitted.load(Ordering::SeqCst), 1);
    release.0.release_worker();
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
    store.break_metadata_table_for_test();
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
