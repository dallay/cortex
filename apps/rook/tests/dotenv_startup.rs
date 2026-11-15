use std::process::Command;

#[test]
fn malformed_dotenv_does_not_disclose_input() {
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join(".env");
    let marker = "synthetic-private-value";
    std::fs::write(&env_path, format!("KEY=\"{marker}\n")).unwrap();

    // Stop at config loading after dotenv processing; no server or database is started.
    let result = Command::new(env!("CARGO_BIN_EXE_rook"))
        .args(["seed-admin", "--config"])
        .arg(dir.path().join("missing.toml"))
        .env_clear()
        .env("HOME", dir.path())
        .env("ROOK_SEED_PASSWORD", "synthetic-password")
        .env("RUST_LOG", "warn")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    let stderr = String::from_utf8(result.stderr).unwrap();
    let warning: serde_json::Value = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|entry| {
            entry["fields"]["message"] == "failed to parse .env file, continuing without it"
        })
        .expect("a safe parse warning should be logged");
    assert_eq!(warning["fields"]["env_file"], env_path.to_str().unwrap());
    assert!(warning["fields"]["position"].is_number());
    assert!(!stdout.contains(marker));
    assert!(!stderr.contains(marker));
    assert!(warning["fields"].get("error").is_none());
}
