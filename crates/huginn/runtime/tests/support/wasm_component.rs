//! Rebuild the independent guest, never silently skip real-component tests.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

const COMPONENT_NAME: &str = "rust-file-outline.component.wasm";

/// Publishes the component so a concurrent reader never observes partial bytes.
///
/// The runtime and application integration tests share one artifact path, and
/// `OnceLock` is per process. A direct write truncates the final path before
/// the bytes land, so an overlapping test process can load an incomplete
/// component. Staging in the same directory and renaming keeps the visible
/// path complete at all times.
fn publish_component(target: &Path, bytes: &[u8]) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let path = target.join(COMPONENT_NAME);
    let staging = target.join(format!(
        "{COMPONENT_NAME}.{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&staging, bytes).unwrap();
    match std::fs::rename(&staging, &path) {
        Ok(()) => {}
        Err(_) => {
            // Windows cannot rename over an existing file.
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&staging, &path).unwrap();
        }
    }
    path
}

fn isolate_guest_build(command: &mut Command) {
    // This child builds a WASM fixture, not another host coverage target.
    // cargo-llvm-cov also injects instrumentation through its rustc wrapper;
    // clearing only RUSTFLAGS does not isolate the guest build.
    for key in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES",
        "CARGO_LLVM_COV",
        "CARGO_LLVM_COV_SHOW_ENV",
        "CARGO_LLVM_COV_TARGET_DIR",
        "CARGO_LLVM_COV_BUILD_DIR",
        "LLVM_PROFILE_FILE",
    ] {
        command.env_remove(key);
    }
}

pub fn component() -> PathBuf {
    static COMPONENT: OnceLock<PathBuf> = OnceLock::new();
    COMPONENT
        .get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let root = root
                .ancestors()
                .find(|path| path.join("rust-toolchain.toml").is_file())
                .expect("workspace root");
            let target = root.join("target/wasm-component-tests");
            let mut command = Command::new(env!("CARGO"));
            isolate_guest_build(&mut command);
            let output = command
                .current_dir(root)
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "huginn-rust-outline-component",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--release",
                    "--target-dir",
                ])
                .arg(&target)
                .output()
                .expect("run independent guest build");
            assert!(
                output.status.success(),
                "guest build failed; install rustup target add wasm32-unknown-unknown:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let module = std::fs::read(
                target.join("wasm32-unknown-unknown/release/huginn_rust_outline_component.wasm"),
            )
            .unwrap();
            let bytes = wit_component::ComponentEncoder::default()
                .module(&module)
                .unwrap()
                .validate(true)
                .encode()
                .unwrap();
            publish_component(&target, &bytes)
        })
        .clone()
}

#[cfg(unix)]
#[test]
fn component_artifact_is_never_observed_partially_published() {
    use std::sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        Arc,
    };

    let directory = tempfile::tempdir().unwrap();
    let first = Arc::new(vec![0xAAu8; 1 << 20]);
    let second = Arc::new(vec![0xBBu8; 1 << 20]);
    let done = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicU64::new(0));

    let writer = std::thread::spawn({
        let directory = directory.path().to_path_buf();
        let done = Arc::clone(&done);
        let first = Arc::clone(&first);
        let second = Arc::clone(&second);
        move || {
            for index in 0..200 {
                let bytes = if index % 2 == 0 {
                    first.as_slice()
                } else {
                    second.as_slice()
                };
                publish_component(&directory, bytes);
            }
            done.store(true, AtomicOrdering::Release);
        }
    });

    let reader = std::thread::spawn({
        let directory = directory.path().to_path_buf();
        let done = Arc::clone(&done);
        let observed = Arc::clone(&observed);
        let first = Arc::clone(&first);
        let second = Arc::clone(&second);
        move || {
            let path = directory.join(COMPONENT_NAME);
            while !done.load(AtomicOrdering::Acquire) {
                if let Ok(bytes) = std::fs::read(&path) {
                    observed.fetch_add(1, Ordering::Relaxed);
                    assert!(
                        bytes == first.as_slice() || bytes == second.as_slice(),
                        "observed a partially published artifact of {} bytes",
                        bytes.len()
                    );
                }
            }
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();
    assert!(
        observed.load(Ordering::Relaxed) > 0,
        "the reader never observed the published artifact"
    );
    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging files left behind: {leftovers:?}"
    );
}

#[cfg(unix)]
#[test]
fn guest_build_environment_drops_coverage_wrappers_without_changing_other_values() {
    let mut command = Command::new("/usr/bin/env");
    command
        .env("RUSTFLAGS", "coverage-flags")
        .env("RUSTC_WRAPPER", "coverage-wrapper")
        .env("RUSTC_WORKSPACE_WRAPPER", "workspace-coverage-wrapper")
        .env("__CARGO_LLVM_COV_RUSTC_WRAPPER", "1")
        .env("__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS", "coverage-flags")
        .env(
            "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
            "coverage-flags",
        )
        .env("HUGINN_TEST_UNRELATED_ENV", "preserved");
    isolate_guest_build(&mut command);
    let output = command.output().unwrap();
    assert!(output.status.success());
    let keys = [
        "RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER",
        "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS",
        "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
        "HUGINN_TEST_UNRELATED_ENV",
    ];
    // Never include unrelated inherited environment values in failure output.
    let observed: Vec<_> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| {
            keys.iter()
                .any(|key| line.starts_with(key.as_bytes()) && line.get(key.len()) == Some(&b'='))
        })
        .collect();
    assert_eq!(
        observed,
        vec![b"HUGINN_TEST_UNRELATED_ENV=preserved".as_slice()]
    );
}
