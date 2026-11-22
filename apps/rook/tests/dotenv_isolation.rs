//! Dotenv loading coverage that keeps the parent test runner's environment
//! untouched.
//!
//! Each fixture runs in a child process. The parent writes the `.env` fixture
//! into a temporary directory, spawns this same test binary with the fixture
//! location in the child environment, and the child performs the environment
//! mutation and the assertion. The child reports the outcome through its exit
//! status, so a failed assertion fails the parent test.
//!
//! The child entry points are marked `#[ignore]` so that `cargo test` only
//! runs the parent tests. The parent spawns them explicitly.

use std::path::PathBuf;
use std::process::{Command, Output};

use rook::startup::load_dotenv;

/// Selects the fixture the child process must run.
const FIXTURE_ENV: &str = "ROOK_TEST_DOTENV_FIXTURE";
/// Directory that holds the `.env` fixture.
const DIRECTORY_ENV: &str = "ROOK_TEST_DOTENV_DIRECTORY";
/// Name of the environment variable the fixture defines.
const VARIABLE_ENV: &str = "ROOK_TEST_DOTENV_VARIABLE";

/// Fixture with a single valid entry.
const EXPLICIT_FIXTURE: &str = "explicit";
/// Fixture with a valid entry followed by a malformed line.
const MALFORMED_FIXTURE: &str = "malformed";

/// Value the explicit fixture defines.
const EXPLICIT_VALUE: &str = "loaded-from-explicit-config";
/// Value the malformed fixture defines.
const MALFORMED_VALUE: &str = "loaded";

/// The fixture the child process must execute.
struct ChildFixture {
    name: String,
    directory: PathBuf,
    variable: String,
}

/// Read the fixture description the parent process passed to the child.
fn child_fixture() -> Option<ChildFixture> {
    let name = std::env::var(FIXTURE_ENV).ok()?;
    let directory = PathBuf::from(std::env::var(DIRECTORY_ENV).expect("child fixture directory"));
    let variable = std::env::var(VARIABLE_ENV).expect("child fixture variable");
    Some(ChildFixture {
        name,
        directory,
        variable,
    })
}

/// Load the fixture in the child process and assert the expected value.
fn run_child_fixture(fixture: &ChildFixture) {
    let expected = match fixture.name.as_str() {
        EXPLICIT_FIXTURE => EXPLICIT_VALUE,
        MALFORMED_FIXTURE => MALFORMED_VALUE,
        other => panic!("unknown fixture: {other}"),
    };

    let config_path = fixture.directory.join("config.toml");
    // The child owns its environment, so clearing the variable here cannot
    // affect any other test.
    std::env::remove_var(&fixture.variable);

    load_dotenv(Some(&config_path));

    assert_eq!(std::env::var(&fixture.variable).as_deref(), Ok(expected));
}

/// Spawn the child process that runs `test_name` against the given fixture.
fn run_child(test_name: &str, fixture: &str, contents: &str, variable: &str) -> Output {
    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::write(directory.path().join(".env"), contents).expect("write .env fixture");

    let executable = std::env::current_exe().expect("test executable path");
    let output = Command::new(executable)
        .arg("--exact")
        .arg(test_name)
        .arg("--ignored")
        .arg("--nocapture")
        .env(FIXTURE_ENV, fixture)
        .env(DIRECTORY_ENV, directory.path())
        .env(VARIABLE_ENV, variable)
        // Keep the child away from the developer's home directory and from any
        // `.env` file that the ambient environment may point at.
        .env("HOME", directory.path())
        .current_dir(directory.path())
        .output()
        .expect("spawn child test process");

    // The fixture directory must outlive the child process.
    drop(directory);
    output
}

/// Assert that the child process completed the fixture successfully.
fn assert_child_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "child process failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn load_dotenv_reads_from_the_explicit_config_directory() {
    let variable = "ROOK_TEST_DOTENV_ISOLATION_EXPLICIT";
    let output = run_child(
        "dotenv_child_loads_from_the_explicit_config_directory",
        EXPLICIT_FIXTURE,
        &format!("{variable}={EXPLICIT_VALUE}\n"),
        variable,
    );
    assert_child_succeeded(&output);
}

#[test]
fn load_dotenv_keeps_valid_entries_when_a_line_is_malformed() {
    let variable = "ROOK_TEST_DOTENV_ISOLATION_MALFORMED";
    let output = run_child(
        "dotenv_child_keeps_valid_entries_when_a_line_is_malformed",
        MALFORMED_FIXTURE,
        &format!("{variable}={MALFORMED_VALUE}\nthis line is invalid dotenv syntax\n"),
        variable,
    );
    assert_child_succeeded(&output);
}

/// Child process entry point for the explicit-path fixture.
#[test]
#[ignore = "child process entry point; spawned by load_dotenv_reads_from_the_explicit_config_directory"]
fn dotenv_child_loads_from_the_explicit_config_directory() {
    let Some(fixture) = child_fixture() else {
        return;
    };
    run_child_fixture(&fixture);
}

/// Child process entry point for the malformed-line fixture.
#[test]
#[ignore = "child process entry point; spawned by load_dotenv_keeps_valid_entries_when_a_line_is_malformed"]
fn dotenv_child_keeps_valid_entries_when_a_line_is_malformed() {
    let Some(fixture) = child_fixture() else {
        return;
    };
    run_child_fixture(&fixture);
}
