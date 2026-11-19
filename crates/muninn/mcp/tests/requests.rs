mod common;

use common::request;
use muninn_mcp::{Server, PROTOCOL_VERSION};
use serde_json::json;

#[tokio::test]
async fn initialize_requires_object_parameters() {
    let response = Server::default()
        .handle(json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":null}))
        .await
        .expect("requests with an id return a response");
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(
        response["error"]["message"],
        "Initialization parameters required"
    );
}

#[tokio::test]
async fn initialize_reports_unsupported_protocol_version() {
    let response = Server::default()
        .handle(json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{
                "protocolVersion":"unsupported",
                "capabilities":{},
                "clientInfo":{"name":"test", "version":"1"}
            }
        }))
        .await
        .expect("requests with an id return a response");
    assert_eq!(response["error"]["code"], -32022);
    assert_eq!(
        response["error"]["message"],
        "Unsupported protocol version or invalid client information"
    );
    assert_eq!(
        response["error"]["data"]["supported"],
        json!([PROTOCOL_VERSION])
    );
    assert_eq!(response["error"]["data"]["requested"], "unsupported");
}

#[tokio::test]
async fn tools_call_requires_a_string_name() {
    let response = Server::default()
        .handle(request("tools/call", json!({})))
        .await
        .expect("requests with an id return a response");
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(response["error"]["message"], "Invalid tool name");
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let response = Server::default()
        .handle(request("unknown/method", json!({})))
        .await
        .expect("requests with an id return a response");
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "Method not found");
    assert!(response.get("result").is_none());
}
