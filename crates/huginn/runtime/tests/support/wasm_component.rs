//! Rebuild the independent guest, never silently skip real-component tests.
use std::{path::PathBuf, process::Command, sync::OnceLock};

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
            let output = Command::new(env!("CARGO"))
                .current_dir(root)
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
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
