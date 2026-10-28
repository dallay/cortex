use huginn_core::{ToolCall, ToolContext};
use huginn_guards::Guardrails;
use huginn_runtime::{guardrails::load_guardrails, tools::native_tools_with_guards};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn native_tools_exclude_secrets_before_previews_and_recursive_reads() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".env"), "sentinel-secret").unwrap();
    std::fs::write(root.path().join("plain.txt"), "sentinel-public").unwrap();
    let ctx = ToolContext {
        workspace: root.path().canonicalize().unwrap(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let tools = native_tools_with_guards(1, Arc::new(Guardrails::default()));
    for (name, arguments) in [
        ("read_file", json!({"path":".env"})),
        ("write_file", json!({"path":".env","content":"changed"})),
        (
            "edit_file",
            json!({"path":".env","old_text":"sentinel-secret","new_text":"changed"}),
        ),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .unwrap();
        let call = ToolCall {
            id: "blocked".into(),
            name: name.into(),
            arguments,
        };
        assert!(tool.prepare(&call, &ctx).await.is_err());
    }
    let tool = tools
        .iter()
        .find(|tool| tool.definition().name == "search_files")
        .unwrap();
    let call = ToolCall {
        id: "search".into(),
        name: "search_files".into(),
        arguments: json!({"path":".","query":"sentinel"}),
    };
    let action = tool.prepare(&call, &ctx).await.unwrap();
    let output = tool.execute(action, &ctx).await.unwrap();
    assert!(output.contains("sentinel-public"));
    assert!(!output.contains("sentinel-secret"));
    assert!(!output.contains(".env"));
}

#[tokio::test]
async fn recursive_search_excludes_existing_uppercase_env() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".ENV"), "sentinel-secret-uppercase").unwrap();
    std::fs::write(root.path().join("plain.txt"), "sentinel-public").unwrap();
    let ctx = ToolContext {
        workspace: root.path().canonicalize().unwrap(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let tools = native_tools_with_guards(1, Arc::new(Guardrails::default()));
    let tool = tools
        .iter()
        .find(|tool| tool.definition().name == "search_files")
        .unwrap();
    let call = ToolCall {
        id: "uppercase-search".into(),
        name: "search_files".into(),
        arguments: json!({"path":".","query":"sentinel"}),
    };
    let action = tool.prepare(&call, &ctx).await.unwrap();
    let output = tool.execute(action, &ctx).await.unwrap();
    assert!(output.contains("sentinel-public"));
    assert!(!output.contains("sentinel-secret-uppercase"), "{output}");
    assert!(!output.contains(".ENV"), "{output}");
}

#[test]
fn local_configuration_is_bounded_additive_and_rejects_nonregular_files() {
    let root = tempfile::tempdir().unwrap();
    assert!(load_guardrails(root.path())
        .unwrap()
        .check_shell("git status")
        .is_ok());
    std::fs::create_dir(root.path().join(".huginn")).unwrap();
    let config = root.path().join(".huginn/guardrails.toml");
    std::fs::write(&config, "block_commands=['curl']").unwrap();
    let policy = load_guardrails(root.path()).unwrap();
    assert!(policy.check_shell("curl https://example.com").is_err());
    assert!(policy.check_shell("rm -rf .").is_err());
    for value in ["enabled=false".into(), "x".repeat(65_537)] {
        std::fs::write(&config, value).unwrap();
        assert!(load_guardrails(root.path()).is_err());
    }
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    assert!(load_guardrails(root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn configuration_rejects_aliases_and_fifo_without_blocking() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("policy"), "").unwrap();
    std::fs::create_dir(root.path().join(".huginn")).unwrap();
    let config = root.path().join(".huginn/guardrails.toml");
    symlink(outside.path().join("policy"), &config).unwrap();
    assert!(load_guardrails(root.path()).is_err());
    std::fs::remove_file(&config).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(&config)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(load_guardrails(root.path()).is_err());
    std::fs::remove_file(&config).unwrap();
    std::fs::remove_dir(root.path().join(".huginn")).unwrap();
    symlink(outside.path(), root.path().join(".huginn")).unwrap();
    assert!(load_guardrails(root.path()).is_err());
}
