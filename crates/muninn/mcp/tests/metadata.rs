mod common;
use muninn_mcp::Server;
use serde_json::json;

#[test]
fn client_identity_is_optional_but_validated_when_present() {
    let mut server = Server::default();
    let mut request = common::request("server/discover", json!({}));
    request["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientInfo");
    assert!(server
        .handle(request.clone())
        .unwrap()
        .get("result")
        .is_some());
    for invalid in [json!(null), json!({"name":"missing-version"}), json!(42)] {
        request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = invalid;
        assert_eq!(
            server.handle(request.clone()).unwrap()["error"]["code"],
            -32602
        );
    }
}

#[test]
fn required_metadata_is_checked_on_each_request() {
    for key in [
        "io.modelcontextprotocol/protocolVersion",
        "io.modelcontextprotocol/clientCapabilities",
    ] {
        let mut request = common::request("tools/list", json!({}));
        request["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert_eq!(
            Server::default().handle(request).unwrap()["error"]["code"],
            -32602
        );
    }
}
