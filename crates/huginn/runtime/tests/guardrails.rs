use huginn_core::{ToolCall, ToolContext};
use huginn_guards::Guardrails;
use huginn_runtime::tools::native_tools_with_guards;
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
