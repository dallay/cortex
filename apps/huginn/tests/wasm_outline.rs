#![cfg(feature = "experimental-wasm")]
use serde_json::{json, Value};
use std::process::Command;
#[path = "../../../crates/huginn/runtime/tests/support/wasm_component.rs"]
mod wasm_component;

#[test]
fn real_headless_flow_uses_the_independent_component_result_in_continuation() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("sample.rs"),
        "pub fn navigate() -> bool { true }\n",
    )
    .unwrap();
    let component = wasm_component::component();
    let output = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .env("HOME", home.path())
        .args(["--provider", "mock", "--json", "--workspace"])
        .arg(root.path())
        .arg("--db")
        .arg(home.path().join("sessions.db"))
        .arg("--experimental-wasm-outline")
        .arg(component)
        .arg("run")
        .arg(format!(
            "huginn-mock: rust_file_outline {}",
            json!({"path":"sample.rs"})
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let result = events
        .iter()
        .find(|event| event["type"] == "tool_finished")
        .expect("tool result");
    assert_eq!(result["is_error"], false, "{result}");
    let outline: Value = serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
    assert_eq!(outline["declarations"][0]["name"], "navigate");
    assert!(
        events.iter().any(|event| event["type"] == "text"
            && event["text"].as_str().unwrap_or("").contains("navigate")),
        "result must reach model continuation: {events:?}"
    );
    assert!(!events
        .iter()
        .any(|event| event["type"] == "approval_requested"));
}

#[test]
fn absent_activation_preserves_native_reads_and_does_not_register_the_component() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("sample.rs"), "pub fn native_only() {}\n").unwrap();
    for tool in ["read_file", "rust_file_outline"] {
        let output = Command::new(env!("CARGO_BIN_EXE_huginn"))
            .env("HOME", home.path())
            .args(["--provider", "mock", "--json", "--workspace"])
            .arg(root.path())
            .arg("--db")
            .arg(home.path().join("sessions.db"))
            .arg("run")
            .arg(format!(
                "huginn-mock: {tool} {}",
                json!({"path":"sample.rs"})
            ))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let result = events
            .iter()
            .find(|event| event["type"] == "tool_finished")
            .unwrap();
        assert_eq!(result["is_error"], tool == "rust_file_outline", "{result}");
        if tool == "read_file" {
            assert!(result["output"].as_str().unwrap().contains("native_only"));
        }
    }
}

#[test]
fn interactive_activation_is_rejected_before_opening_a_component() {
    let output = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args([
            "--experimental-wasm-outline",
            "missing.component.wasm",
            "chat",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires run or resume --prompt"));
}
