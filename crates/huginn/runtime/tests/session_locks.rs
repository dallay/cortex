use fs2::FileExt;
use huginn_core::AgentError;
use huginn_runtime::sessions::SqliteSessions;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn spawn_legacy_protocol_holder(id: Uuid) -> (Child, tempfile::TempDir) {
    let ready_dir = tempfile::tempdir().expect("temporary synchronization directory");
    let ready_path = ready_dir.path().join("ready");
    let child = Command::new(env!("CARGO_BIN_EXE_session_lock_holder"))
        .arg(id.as_hyphenated().to_string())
        .arg(&ready_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn lock holder");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready_path.exists() {
        assert!(
            Instant::now() < deadline,
            "child did not acquire legacy lock"
        );
        std::thread::yield_now();
    }
    (child, ready_dir)
}

fn sessions() -> (tempfile::TempDir, SqliteSessions) {
    let directory = tempfile::tempdir().expect("temporary session directory");
    let store =
        SqliteSessions::open(&directory.path().join("sessions.db")).expect("open test database");
    (directory, store)
}

fn error_while_legacy_owner_holds(id: Uuid) -> (AgentError, Child) {
    let (child, _ready_dir) = spawn_legacy_protocol_holder(id);
    let (_directory, store) = sessions();
    let error = match store.lease(&id.to_string()) {
        Ok(_) => panic!("expected session contention"),
        Err(error) => error,
    };
    (error, child)
}

#[test]
fn legacy_lock_holder_blocks_huginn_lease_in_another_process() {
    let (error, mut child) = error_while_legacy_owner_holds(Uuid::new_v4());
    assert!(error.to_string().contains("session is in use"));
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"x")
        .expect("release child");
    assert!(child.wait().expect("wait for lock holder").success());
}

#[test]
fn different_sessions_can_be_leased_concurrently() {
    let other_session = Uuid::new_v4();
    let (mut child, _ready_dir) = spawn_legacy_protocol_holder(other_session);
    let (_directory, store) = sessions();
    let independent = store
        .lease(&Uuid::new_v4().to_string())
        .expect("different session is not blocked");
    drop(independent);
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"x")
        .expect("release child");
    assert!(child.wait().expect("wait for lock holder").success());
}

#[cfg(unix)]
#[test]
fn symlink_lock_path_is_reported_as_io_not_session_contention() {
    let id = Uuid::new_v4();
    let lock_path = std::path::Path::new("/tmp").join(format!("cortex-agent-session-{id}.lock"));
    let target = tempfile::NamedTempFile::new().expect("lock symlink target");
    std::os::unix::fs::symlink(target.path(), &lock_path).expect("create lock symlink");
    let (_directory, store) = sessions();
    let error = match store.lease(&id.to_string()) {
        Ok(_) => panic!("symlink lock must not be followed"),
        Err(error) => error,
    };
    assert!(
        matches!(error, AgentError::Io(_)),
        "unexpected error: {error}"
    );
    std::fs::remove_file(lock_path).expect("remove lock symlink");
}

#[cfg(unix)]
#[test]
fn lease_normalises_canonical_and_noncanonical_session_ids() {
    let id = Uuid::new_v4();
    let canonical = id.as_hyphenated().to_string();
    let upper = canonical.to_ascii_uppercase();
    let stripped: String = canonical.chars().filter(|c| *c != '-').collect();
    // All three spellings must resolve to the same shared lockfile, so a
    // legacy owner that wrote under the canonical form blocks Huginn even
    // when the CLI hands us upper-case or dash-stripped input.
    let lock_path =
        std::path::Path::new("/tmp").join(format!("cortex-agent-session-{canonical}.lock"));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open canonical lock");
    lock.lock_exclusive().expect("hold canonical lock");
    for variant in [canonical.as_str(), upper.as_str(), stripped.as_str()] {
        let (_directory, store) = sessions();
        let error = match store.lease(variant) {
            Ok(_) => panic!("variant {variant} unexpectedly acquired the lock"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("session is in use"),
            "variant {variant} reported wrong error: {error}"
        );
    }
    FileExt::unlock(&lock).expect("release canonical lock");
    let (_directory, store) = sessions();
    store
        .lease(&upper)
        .expect("acquire lock via upper-case after release");
    let (_directory, store) = sessions();
    store
        .lease(&stripped)
        .expect("acquire lock via dash-stripped form after release");
    let (_directory, store) = sessions();
    store
        .lease(&canonical)
        .expect("acquire lock via canonical form after release");
    let _ = std::fs::remove_file(&lock_path);
}

#[test]
fn killed_owner_releases_lease_for_next_process() {
    let id = Uuid::new_v4();
    let (mut child, _ready_dir) = spawn_legacy_protocol_holder(id);
    child.kill().expect("forcefully terminate lock owner");
    assert!(!child.wait().expect("reap killed process").success());

    let (_directory, store) = sessions();
    let lease = store
        .lease(&id.to_string())
        .expect("OS releases advisory lock when owner dies");
    drop(lease);
}
