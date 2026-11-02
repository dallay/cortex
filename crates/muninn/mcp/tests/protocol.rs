use muninn_mcp::{Server, PROTOCOL_VERSION};
use serde_json::json;

mod common;
use common::request;

#[test]
fn discovery_has_own_identity_and_only_implemented_capabilities() {
    let response = Server::default()
        .handle(request("server/discover", json!({})))
        .unwrap();
    assert_eq!(
        response["result"]["supportedVersions"],
        json!([PROTOCOL_VERSION])
    );
    assert_eq!(
        response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "muninn"
    );
    assert_eq!(
        response["result"]["capabilities"],
        json!({"tools":{"listChanged":false}})
    );
    assert_eq!(response["result"]["resultType"], "complete");
    assert!(response["result"].get("instructions").is_none());
}
