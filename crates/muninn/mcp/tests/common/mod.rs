use muninn_mcp::PROTOCOL_VERSION;
use serde_json::{json, Value};

pub fn request(method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities":{},
        "io.modelcontextprotocol/clientInfo":{"name":"test", "version":"1"}
    });
    json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params})
}
