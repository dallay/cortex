//! MCP discovery/publication rollback is separate from transport timing tests.
use async_trait::async_trait;
use huginn_core::{
    ApprovalPolicy, ApprovalRequest, CancellationToken, Result, ToolCall, ToolContext, ToolRegistry,
};
use huginn_runtime::{
    mcp::{McpClients, ServerConfig},
    tools::Registry,
};
use serde_json::json;
use std::collections::BTreeMap;

struct Allow;
#[async_trait]
impl ApprovalPolicy for Allow {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        Ok(true)
    }
}
fn mcp_config(marker: &std::path::Path) -> ServerConfig {
    ServerConfig {
        name: "fixture".into(),
        command: "python3".into(),
        args: vec![format!(
            "{}/tests/fixtures/mcp_server.py",
            env!("CARGO_MANIFEST_DIR")
        )],
        env: BTreeMap::from([("MARKER".into(), marker.display().to_string())]),
        env_from: BTreeMap::new(),
    }
}

#[tokio::test]
async fn partial_mcp_discovery_and_later_server_failure_leave_registry_unchanged() {
    use huginn_core::kernel::Supervisor;
    use huginn_runtime::{mcp::register_plugins, tools::RegistryPlugin};
    let temp = tempfile::tempdir().unwrap();
    for bad_schema in [true, false] {
        let registry = Registry::native(1).unwrap();
        let before = serde_json::to_value(registry.definitions()).unwrap();
        let mut config = mcp_config(&temp.path().join("marker"));
        let mut configs = vec![];
        if bad_schema {
            config.env.insert("BAD_SCHEMA".into(), "1".into());
        } else {
            configs.push(config.clone());
            config.name = "zz_failure".into();
            config.command = "/nonexistent/mcp-server".into();
        }
        configs.push(config);
        let policy = std::sync::Arc::new(Allow);
        // Standalone API must be transactional as well as the production plugin path.
        assert!(McpClients::connect(
            &configs,
            temp.path(),
            &registry,
            policy.as_ref(),
            CancellationToken::new(),
            3
        )
        .await
        .is_err());
        assert_eq!(
            serde_json::to_value(registry.definitions()).unwrap(),
            before
        );
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(RegistryPlugin(registry.clone())))
            .unwrap();
        supervisor.start().await.unwrap();
        register_plugins(
            &mut supervisor,
            &configs,
            temp.path(),
            policy,
            CancellationToken::new(),
            3,
        )
        .unwrap();
        assert!(supervisor.start().await.is_err());
        assert_eq!(
            serde_json::to_value(registry.definitions()).unwrap(),
            before
        );
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn mcp_plugin_unload_revokes_registered_and_retained_tools() {
    use huginn_core::kernel::Supervisor;
    use huginn_runtime::{mcp::register_plugins, tools::RegistryPlugin};
    let temp = tempfile::tempdir().unwrap();
    let registry = Registry::default();
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    let policy = std::sync::Arc::new(Allow);
    register_plugins(
        &mut supervisor,
        &[mcp_config(&temp.path().join("marker"))],
        temp.path(),
        policy,
        CancellationToken::new(),
        3,
    )
    .unwrap();
    supervisor.start().await.unwrap();
    let stale = registry.get("mcp_fixture_0").unwrap();
    supervisor.remove("mcp.fixture").await.unwrap();
    assert!(registry.definitions().is_empty());
    assert!(stale
        .prepare(
            &ToolCall {
                id: "one".into(),
                name: "mcp_fixture_0".into(),
                arguments: json!({})
            },
            &ToolContext {
                selected_skills: vec![],
                workspace: temp.path().into(),
                cancellation: CancellationToken::new()
            }
        )
        .await
        .is_err());
    supervisor.shutdown().await;
}
