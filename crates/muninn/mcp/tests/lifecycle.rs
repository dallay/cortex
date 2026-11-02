mod common;
use common::request;
use muninn_mcp::Server;
use serde_json::{json, Value};

#[test]
fn requests_are_independent_and_need_no_handshake() {
    let mut server = Server::default();
    let result = server.handle(request("tools/list", json!({}))).unwrap();
    assert_eq!(result["result"]["tools"], json!([]));
    assert_eq!(result["result"]["resultType"], "complete");
    let unknown_tool = server
        .handle(request("tools/call", json!({"name":"memory_search"})))
        .unwrap();
    assert_eq!(unknown_tool["result"]["isError"], true);
    assert_eq!(
        unknown_tool["result"]["content"][0]["text"],
        "Unknown tool: 'memory_search'"
    );
}

#[test]
fn validates_version_on_every_request_and_rejects_bad_envelopes() {
    let mut server = Server::default();
    let mut bad = request("ping", json!({}));
    bad["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    assert_eq!(server.handle(bad).unwrap()["error"]["code"], -32022);
    assert_eq!(
        server
            .handle(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .unwrap()["error"]["code"],
        -32602
    );
    assert_eq!(server.handle(json!([])).unwrap()["error"]["code"], -32600);
    assert_eq!(
        server
            .handle(json!({"jsonrpc":"1.0","id":true,"method":"ping"}))
            .unwrap()["id"],
        Value::Null
    );
    assert!(server
        .handle(json!({"jsonrpc":"2.0","method":"unknown"}))
        .is_none());
    assert_eq!(
        server.handle(request("ping", json!({}))).unwrap()["result"]["resultType"],
        "complete"
    );
    assert_eq!(
        server.handle(request("initialize", json!({}))).unwrap()["error"]["code"],
        -32601
    );
}
