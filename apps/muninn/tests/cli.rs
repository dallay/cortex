//! Binary-level CLI contract tests.
//!
//! Unit tests in `src/main.rs` exercise handlers directly. These tests launch
//! the binary to verify its startup ordering and environment handling.

use std::process::{Command, Output};

const BINARY: &str = env!("CARGO_BIN_EXE_muninn");
const FAST_FAILURE_URI: &str = "bolt://127.0.0.1:99999";

fn run_cli(arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BINARY);
    command.args(arguments).env_clear();
    for (key, value) in environment {
        command.env(key, value);
    }
    command.output().expect("muninn binary must start")
}

fn bootstrap_arguments() -> [&'static str; 6] {
    [
        "--embedding-provider",
        "none",
        "--neo4j-uri",
        FAST_FAILURE_URI,
        "--neo4j-password",
        "synthetic-test-password",
    ]
}

#[test]
fn cli_rejects_ambient_geocoding_override_before_database_bootstrap() {
    let arguments = [
        "--embedding-provider",
        "none",
        "--neo4j-uri",
        FAST_FAILURE_URI,
        "--neo4j-password",
        "synthetic-test-password",
        "--geocoding-enabled",
        "bridge",
    ];
    let output = run_cli(&arguments, &[("NAM_GEOCODING__RATE_LIMIT_PER_SECOND", "0")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("geocoding configuration rejected"),
        "{stderr}"
    );
    assert!(!stderr.contains("Neo4j initialisation failed"), "{stderr}");
}

#[test]
fn cli_uses_geocoding_default_when_environment_is_cleared() {
    let arguments = [
        "--embedding-provider",
        "none",
        "--neo4j-uri",
        FAST_FAILURE_URI,
        "--neo4j-password",
        "synthetic-test-password",
        "--geocoding-enabled",
        "bridge",
    ];
    let output = run_cli(&arguments, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Neo4j initialisation failed"), "{stderr}");
    assert!(
        !stderr.contains("geocoding configuration rejected"),
        "{stderr}"
    );
}

#[test]
fn doctor_stats_and_ontology_cli_fail_before_handler_dispatch_without_neo4j() {
    for (command, output_marker) in [
        (&["doctor"][..], "=== Muninn Doctor ==="),
        (&["stats"][..], "=== Muninn Memory Metrics ==="),
        (&["ontology", "list"][..], "=== Built-in Ontologies ==="),
    ] {
        let mut arguments = bootstrap_arguments().to_vec();
        arguments.extend_from_slice(command);
        let output = run_cli(&arguments, &[]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "{command:?} unexpectedly succeeded"
        );
        assert!(stderr.contains("Neo4j initialisation failed"), "{stderr}");
        assert!(!stdout.contains(output_marker), "{stdout}");
    }
}
