use muninn_observability::init_tracing_subscriber;
use std::process::Command;

const CHILD_ENV: &str = "MUNINN_TRACING_SUBSCRIBER_CHILD";
const ENABLED_MARKER: &str = "muninn-subscriber-enabled-event";
const FILTERED_MARKER: &str = "other-subscriber-filtered-event";

#[test]
fn emits_probe_events_for_parent_capture() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    init_tracing_subscriber(Some("muninn=debug"))
        .expect("the global tracing subscriber should install");
    tracing::debug!(target: "muninn", "{}", ENABLED_MARKER);
    tracing::debug!(target: "other", "{}", FILTERED_MARKER);
}

#[test]
fn init_tracing_subscriber_installs_a_subscriber_that_outputs_events() {
    let exe = std::env::current_exe().expect("the test binary path is available");
    let output = Command::new(exe)
        .args([
            "emits_probe_events_for_parent_capture",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env_remove("RUST_LOG")
        .output()
        .expect("the child test process runs");

    assert!(
        output.status.success(),
        "the child process must succeed: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(ENABLED_MARKER),
        "the subscriber output must contain the enabled event: {stdout}"
    );
    assert!(
        !stdout.contains(FILTERED_MARKER),
        "the supplied filter must exclude the other target: {stdout}"
    );
}
