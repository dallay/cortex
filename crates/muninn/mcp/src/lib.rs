//! MCP protocol boundary, independent of storage and application services.

use serde_json::{json, Value};
use std::collections::BTreeMap;

pub enum Catalog {
    Core,
    Extended,
    ExtendedPlatinum,
}

impl Catalog {
    pub fn tools(&self) -> Value {
        let source = match self {
            Self::Core => include_str!("../registry/core.json"),
            Self::Extended => include_str!("../registry/extended.json"),
            Self::ExtendedPlatinum => include_str!("../registry/extended-platinum.json"),
        };
        serde_json::from_str::<Value>(source).expect("embedded registry must be valid JSON")
            ["tools"]
            .clone()
    }
}

/// Adapters validate tool-specific arguments and enforce authorization before use.
/// Errors must be safe for external display (no credentials or sensitive payloads).
pub trait ToolHandler: Send + Sync {
    fn call(&self, arguments: &Value) -> Result<Value, String>;
}

struct RegisteredTool {
    definition: Value,
    handler: Box<dyn ToolHandler>,
}

pub const PROTOCOL_VERSION: &str = "2026-07-28";

#[derive(Default)]
pub struct Server {
    tools: BTreeMap<String, RegisteredTool>,
    started: bool,
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

impl Server {
    pub fn register(
        &mut self,
        catalog: Catalog,
        name: &str,
        handler: Box<dyn ToolHandler>,
    ) -> Result<(), &'static str> {
        if self.started {
            return Err("registry is frozen after first request");
        }
        if self.tools.contains_key(name) {
            return Err("duplicate handler");
        }
        let definition = catalog
            .tools()
            .as_array()
            .and_then(|tools| tools.iter().find(|t| t["name"] == name))
            .cloned()
            .ok_or("unknown tool")?;
        self.tools.insert(
            name.to_owned(),
            RegisteredTool {
                definition,
                handler,
            },
        );
        Ok(())
    }

    pub fn handle(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || !request["method"].is_string()
            || !valid_id
        {
            return Some(error(Value::Null, -32600, "Invalid Request"));
        }
        let method = request["method"].as_str()?;
        let params = request.get("params");
        let id = id?;
        self.started = true;
        let Some(params) = params.filter(|p| p.is_object()) else {
            return Some(error(id, -32602, "Request metadata required"));
        };
        let meta = &params["_meta"];
        if !meta["io.modelcontextprotocol/protocolVersion"].is_string()
            || !meta["io.modelcontextprotocol/clientCapabilities"].is_object()
            || meta
                .get("io.modelcontextprotocol/clientInfo")
                .is_some_and(|info| !info["name"].is_string() || !info["version"].is_string())
        {
            return Some(error(id, -32602, "Invalid request metadata"));
        }
        if meta["io.modelcontextprotocol/protocolVersion"] != PROTOCOL_VERSION {
            let mut response = error(id, -32022, "Unsupported protocol version");
            response["error"]["data"] = json!({"supported":[PROTOCOL_VERSION],"requested":meta["io.modelcontextprotocol/protocolVersion"]});
            return Some(response);
        }
        let result = match method {
            "server/discover" => Some(json!({"jsonrpc":"2.0","id":id,"result":{
                "supportedVersions":[PROTOCOL_VERSION],
                "capabilities":{"tools":{"listChanged":false}}
            }})),
            "ping" => Some(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            "tools/list" => Some(
                json!({"jsonrpc":"2.0","id":id,"result":{"tools":self.tools.values().map(|t| &t.definition).collect::<Vec<_>>()}}),
            ),
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Some(error(id, -32602, "Invalid tool name"));
                };
                let Some(tool) = self.tools.get(name) else {
                    return Some(json!({
                        "jsonrpc":"2.0",
                        "id":id,
                        "result":{
                            "content":[{"type":"text","text":format!("Unknown tool: '{name}'")}],
                            "isError":true,
                            "resultType":"complete",
                            "_meta":{"io.modelcontextprotocol/serverInfo":{
                                "name":"muninn",
                                "version":env!("CARGO_PKG_VERSION")
                            }}
                        }
                    }));
                };
                let empty = json!({});
                let arguments = params.get("arguments").unwrap_or(&empty);
                if !arguments.is_object() {
                    return Some(error(id, -32602, "Invalid tool arguments"));
                }
                let (payload, failed) = tool.handler.call(arguments).map_or_else(
                    |_| ("Tool execution failed".to_owned(), true),
                    |value| (value.to_string(), false),
                );
                Some(
                    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":payload}],"isError":failed}}),
                )
            }
            _ => Some(error(id, -32601, "Method not found")),
        };
        result.map(|mut response| {
            if let Some(result) = response.get_mut("result") {
                result["resultType"] = json!("complete");
                result["_meta"] = json!({"io.modelcontextprotocol/serverInfo":{
                    "name":"muninn", "version":env!("CARGO_PKG_VERSION")
                }});
            }
            response
        })
    }
}

/// Maximum wire bytes per frame, excluding its terminating newline.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Run one stdio session. Oversized input closes the session without echoing it.
pub fn serve<R: std::io::BufRead, W: std::io::Write>(
    mut reader: R,
    mut writer: W,
    server: &mut Server,
) -> std::io::Result<()> {
    use std::io::{BufRead, Error, ErrorKind, Read};
    loop {
        let mut frame = Vec::new();
        let read = reader
            .by_ref()
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut frame)?;
        if read == 0 {
            return Ok(());
        }
        if frame.last() == Some(&b'\n') {
            frame.pop();
        }
        if frame.len() > MAX_MESSAGE_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "MCP message exceeds size limit",
            ));
        }
        let response = serde_json::from_slice::<Value>(&frame).map_or_else(
            |_| Some(error(Value::Null, -32700, "Parse error")),
            |request| server.handle(request),
        );
        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}
