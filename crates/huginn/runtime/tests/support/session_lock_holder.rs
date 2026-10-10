use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;

fn main() {
    let id = std::env::args()
        .nth(1)
        .and_then(|value| Uuid::parse_str(&value).ok())
        .expect("expected a valid session UUID argument");
    let ready_path = std::env::args_os()
        .nth(2)
        .map(std::path::PathBuf::from)
        .expect("expected a ready-file path argument");
    let temp_dir = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temporary directory");
    let parent = ready_path
        .parent()
        .and_then(|path| path.canonicalize().ok())
        .expect("canonical ready-file parent");
    assert!(
        parent.starts_with(&temp_dir),
        "ready file must be under temp_dir"
    );
    assert_eq!(
        ready_path.file_name().and_then(|name| name.to_str()),
        Some("ready")
    );
    let ready_path = parent.join("ready");

    let lock_path =
        Path::new("/tmp").join(format!("cortex-agent-session-{}.lock", id.as_hyphenated()));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .expect("open legacy lock file");
    lock.lock_exclusive().expect("hold legacy lock");
    std::fs::write(&ready_path, "ready").expect("signal lock readiness");
    let mut release = [0; 1];
    let _ = std::io::stdin().read(&mut release);
    FileExt::unlock(&lock).expect("release legacy lock");
}
