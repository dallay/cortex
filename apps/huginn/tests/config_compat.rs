use std::path::{Path, PathBuf};
use std::process::Command;

fn huginn() -> Command {
    Command::new(env!("CARGO_BIN_EXE_huginn"))
}

/// Mirror `dirs::data_local_dir()` resolution using only the env vars we
/// care about, so the test stays platform-portable. The production code
/// uses `dirs::data_local_dir()`, which on Linux consults `XDG_DATA_HOME`
/// and on macOS uses `$HOME/Library/Application Support`.
fn fake_data_local_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        // Tests explicitly set XDG_DATA_HOME to this isolated path for both
        // parent and child, matching dirs::data_local_dir() on Linux.
        home.join("xdg-data")
    }
}

#[test]
fn canonical_environment_values_take_precedence_over_legacy_values() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let data_root = fake_data_local_dir(home.path());
    let canonical_db = data_root.join("cortex/huginn/sessions.db");
    let _ = std::fs::remove_file(&canonical_db);
    let output = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("doctor")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("XDG_CONFIG_HOME")
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
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor JSON report");
    assert_eq!(report["base_url"], "http://canonical.invalid/v1");
    assert_eq!(report["model"], "canonical-model");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("AGENT_BASE_URL is deprecated"));
    assert!(!stderr.contains("AGENT_MODEL is deprecated"));
}

#[test]
fn legacy_credential_variables_fill_canonical_and_default_key_name() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let data_root = fake_data_local_dir(home.path());
    let canonical_db = data_root.join("cortex/huginn/sessions.db");
    let _ = std::fs::remove_file(&canonical_db);
    let output = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("doctor")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("HUGINN_API_KEY")
        .env("AGENT_API_KEY", "legacy-key")
        .env("AGENT_BASE_URL", "http://legacy.invalid/v1")
        .env("AGENT_MODEL", "legacy-model")
        .output()
        .expect("run doctor with only legacy env vars");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor JSON report");
    assert_eq!(report["base_url"], "http://legacy.invalid/v1");
    assert_eq!(report["model"], "legacy-model");
    assert_eq!(report["api_key_env"], "HUGINN_API_KEY");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("AGENT_API_KEY is deprecated"),
        "doctor should warn about the legacy API key: {stderr}"
    );
    assert!(stderr.contains("AGENT_BASE_URL is deprecated"));
    assert!(stderr.contains("AGENT_MODEL is deprecated"));
}

#[test]
fn custom_api_key_env_is_not_migrated_from_legacy_name() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let data_root = fake_data_local_dir(home.path());
    let canonical_db = data_root.join("cortex/huginn/sessions.db");
    let _ = std::fs::remove_file(&canonical_db);
    let config_toml = home.path().join("custom-huginn.toml");
    std::fs::write(&config_toml, "api_key_env = \"MY_PROVIDER_KEY\"\n")
        .expect("write custom config");
    let output = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--config")
        .arg(&config_toml)
        .arg("doctor")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("HUGINN_API_KEY")
        .env("AGENT_API_KEY", "legacy-key")
        .env("MY_PROVIDER_KEY", "custom-key")
        .env("HUGINN_BASE_URL", "http://canonical.invalid/v1")
        .env("HUGINN_MODEL", "canonical-model")
        .output()
        .expect("run doctor with custom env name");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor JSON report");
    assert_eq!(report["api_key_env"], "MY_PROVIDER_KEY");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("AGENT_API_KEY is deprecated"),
        "custom env names should not trigger the legacy warning: {stderr}"
    );
}

#[test]
fn uses_existing_legacy_database_in_place_when_canonical_database_is_absent() {
    let home = tempfile::tempdir().expect("home directory");
    let workspace = tempfile::tempdir().expect("workspace");
    let data_root = fake_data_local_dir(home.path());
    let legacy_dir = data_root.join("cortex/agent");
    std::fs::create_dir_all(&legacy_dir).expect("create legacy data directory");
    let legacy_db = legacy_dir.join("sessions.db");
    let setup = huginn()
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&legacy_db)
        .arg("sessions")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("XDG_CONFIG_HOME")
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
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("XDG_CONFIG_HOME")
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
        !data_root.join("cortex/huginn/sessions.db").exists(),
        "do not create/copy a canonical DB during legacy fallback"
    );
}

#[test]
fn credential_fallback_respects_the_configured_environment_name() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = workspace.path().join("config.toml");
    let db = workspace.path().join("sessions.db");
    // configured name, canonical key, legacy key, custom key, success, fallback warning
    for (name, canonical, legacy, custom, success, warning) in [
        (
            "HUGINN_API_KEY",
            None,
            Some("legacy-test-key"),
            None,
            true,
            true,
        ),
        (
            "HUGINN_API_KEY",
            Some("canonical-test-key"),
            Some("legacy-test-key"),
            None,
            true,
            false,
        ),
        ("HUGINN_API_KEY", None, None, None, false, false),
        (
            "CUSTOM_API_KEY",
            Some("canonical-test-key"),
            Some("legacy-test-key"),
            None,
            false,
            false,
        ),
        (
            "CUSTOM_API_KEY",
            None,
            Some("legacy-test-key"),
            Some("custom-test-key"),
            true,
            false,
        ),
        (
            "AGENT_API_KEY",
            None,
            Some("legacy-test-key"),
            None,
            true,
            false,
        ),
        ("", None, Some("legacy-test-key"), None, true, false),
    ] {
        std::fs::write(&config, format!("api_key_env = {name:?}\n")).expect("write config");
        let mut command = huginn();
        command
            .arg("--config")
            .arg(&config)
            .arg("--workspace")
            .arg(workspace.path())
            .arg("--db")
            .arg(&db)
            .args([
                "--base-url",
                "http://127.0.0.1:1/v1",
                "--model",
                "test-model",
                "doctor",
            ]);
        for (key, value) in [
            ("HUGINN_API_KEY", canonical),
            ("AGENT_API_KEY", legacy),
            ("CUSTOM_API_KEY", custom),
        ] {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        let output = command.output().expect("run doctor");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), success, "{name}: {stderr}");
        assert_eq!(
            stderr.contains("AGENT_API_KEY is deprecated"),
            warning,
            "{name}: {stderr}"
        );
        if !success {
            assert!(
                stderr.contains(&format!("credential environment variable {name} is unset")),
                "{stderr}"
            );
        }
    }
}
