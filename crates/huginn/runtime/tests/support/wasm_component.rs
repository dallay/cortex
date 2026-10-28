//! Rebuild the independent guest, never silently skip real-component tests.
use std::{path::PathBuf, process::Command, sync::OnceLock};

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
            let path = target.join("rust-file-outline.component.wasm");
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .clone()
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
