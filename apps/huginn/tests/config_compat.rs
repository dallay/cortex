use std::process::Command;

fn huginn() -> Command {
    Command::new(env!("CARGO_BIN_EXE_huginn"))
}

#[test]
fn canonical_environment_values_take_precedence_over_legacy_values() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let output = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("doctor")
        .env("HOME", home.path())
        .env_remove("HUGINN_API_KEY")
        .env_remove("AGENT_API_KEY")
        .env("HUGINN_BASE_URL", "http://canonical.invalid/v1")
        .env("AGENT_BASE_URL", "http://legacy.invalid/v1")
        .env("HUGINN_MODEL", "canonical-model")
        .env("AGENT_MODEL", "legacy-model")
        .output()
        .expect("run doctor");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"{"), "doctor should return JSON");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("AGENT_BASE_URL is deprecated"));
    assert!(!stderr.contains("AGENT_MODEL is deprecated"));
}

#[test]
fn uses_existing_legacy_database_in_place_when_canonical_database_is_absent() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let legacy_dir = home.path().join("Library/Application Support/cortex/agent");
    std::fs::create_dir_all(&legacy_dir).expect("create legacy data directory");
    let legacy_db = legacy_dir.join("sessions.db");
    let setup = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&legacy_db)
        .arg("sessions")
        .env("HOME", home.path())
        .env_remove("HUGINN_API_KEY")
        .env_remove("AGENT_API_KEY")
        .env_remove("HUGINN_BASE_URL")
        .env_remove("HUGINN_MODEL")
        .env_remove("AGENT_BASE_URL")
        .env_remove("AGENT_MODEL")
        .output()
        .expect("initialize legacy database with Huginn");
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let output = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("doctor")
        .env("HOME", home.path())
        .env_remove("HUGINN_API_KEY")
        .env_remove("AGENT_API_KEY")
        .env_remove("HUGINN_BASE_URL")
        .env_remove("HUGINN_MODEL")
        .env_remove("AGENT_BASE_URL")
        .env_remove("AGENT_MODEL")
        .output()
        .expect("run doctor");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(report["database"], legacy_db.to_string_lossy().as_ref());
    assert!(legacy_db.exists(), "legacy database must remain in place");
    assert!(
        !home
            .path()
            .join("Library/Application Support/cortex/huginn/sessions.db")
            .exists(),
        "do not create/copy a canonical DB during legacy fallback"
    );
}
