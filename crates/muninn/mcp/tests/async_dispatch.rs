use async_trait::async_trait;
use muninn_mcp::{Catalog, Server, ToolHandler, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct OrderedHandler(Arc<Mutex<Vec<u64>>>);

#[async_trait]
impl ToolHandler for OrderedHandler {
    async fn call(&self, arguments: &Value) -> Result<Value, String> {
        let sequence = arguments["sequence"].as_u64().unwrap();
        self.0.lock().unwrap().push(sequence);
        tokio::task::yield_now().await;
        Ok(json!({"sequence": sequence}))
    }
}

#[tokio::test]
async fn dispatch_awaits_tool_calls_and_preserves_request_order() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut server = Server::default();
    server
        .register(
            Catalog::Core,
            "memory_search",
            Box::new(OrderedHandler(seen.clone())),
        )
        .unwrap();

    let request = |sequence| {
        json!({"jsonrpc":"2.0", "id":sequence, "method":"tools/call", "params":{
            "_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities":{},
                "io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"}},
            "name":"memory_search", "arguments":{"sequence":sequence}
        }})
    };
    let first = request(1);
    let second = request(2);
    let one = server.handle(first).await.unwrap();
    let two = server.handle(second).await.unwrap();

    assert_eq!(one["result"]["content"][0]["text"], "{\"sequence\":1}");
    assert_eq!(two["result"]["content"][0]["text"], "{\"sequence\":2}");
    assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
}

struct LargeResult;

#[async_trait]
impl ToolHandler for LargeResult {
    async fn call(&self, _: &Value) -> Result<Value, String> {
        Ok(json!({"payload": "x".repeat(muninn_mcp::MAX_RESULT_BYTES)}))
    }
}

#[tokio::test]
async fn oversized_results_fail_without_returning_the_payload() {
    let mut server = Server::default();
    server
        .register(Catalog::Core, "memory_search", Box::new(LargeResult))
        .unwrap();
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
        "_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities":{},
            "io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"}},
        "name":"memory_search", "arguments":{}
    }});

    let response = server.handle(request).await.unwrap();

    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["content"][0]["text"],
        "Tool result exceeds size limit"
    );
    assert!(!response.to_string().contains(&"x".repeat(100)));
}

struct NeverReturns;

#[async_trait]
impl ToolHandler for NeverReturns {
    async fn call(&self, _: &Value) -> Result<Value, String> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn dispatch_times_out_and_returns_a_sanitized_tool_failure() {
    let mut server = Server::default();
    server
        .register(Catalog::Core, "memory_search", Box::new(NeverReturns))
        .unwrap();
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
        "_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities":{},
            "io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"}},
        "name":"memory_search", "arguments":{}
    }});

    let task = tokio::spawn(async move { server.handle(request).await.unwrap() });
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    let response = task.await.unwrap();

    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["content"][0]["text"],
        "Tool execution failed"
    );
}
