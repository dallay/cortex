mod common;
use muninn_mcp::{Catalog, Server, ToolHandler};
use serde_json::{json, Value};

struct Failing;
#[async_trait::async_trait]
impl ToolHandler for Failing {
    async fn call(&self, _: &Value) -> Result<Value, String> {
        Err("secret-canary-must-not-escape".to_owned())
    }
}

#[tokio::test]
async fn handler_errors_are_sanitized_tool_results() {
    let mut server = Server::default();
    server
        .register(Catalog::Core, "memory_search", Box::new(Failing))
        .unwrap();
    let result = server
        .handle(common::request(
            "tools/call",
            json!({"name":"memory_search","arguments":{"query":"hola"}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(result["result"]["resultType"], "complete");
    assert!(!result.to_string().contains("secret-canary"));
    assert!(result.get("error").is_none());
}

#[tokio::test]
async fn unknown_tools_answer_with_a_tool_error_not_a_protocol_error() {
    // Upstream's MCP server reports an unknown tool as a tool result with
    // `isError: true`, so the client can show the failure to the model.
    let mut server = Server::default();
    let result = server
        .handle(common::request(
            "tools/call",
            json!({"name":"muninn_missing_tool","arguments":{}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(result["result"]["resultType"], "complete");
    assert_eq!(
        result["result"]["content"][0]["text"],
        "Unknown tool: 'muninn_missing_tool'"
    );
    assert!(result.get("error").is_none());
}

#[tokio::test]
async fn malformed_notifications_never_dispatch_handlers() {
    let mut server = Server::default();
    server
        .register(Catalog::Core, "memory_search", Box::new(Failing))
        .unwrap();
    assert!(server
        .handle(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"memory_search"}}))
        .await
        .is_none());
    // A notification cannot freeze registration because it never executes a request.
    assert!(server
        .register(Catalog::Core, "memory_store_message", Box::new(Failing))
        .is_ok());
}
