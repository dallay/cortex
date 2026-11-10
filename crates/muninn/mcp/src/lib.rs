//! MCP protocol boundary, independent of storage and application services.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

pub enum Catalog {
    Core,
    Extended,
    ExtendedPlatinum,
    ScopedRetrieval,
}

impl Catalog {
    pub fn tools(&self) -> Value {
        let source = match self {
            Self::Core => include_str!("../registry/core.json"),
            Self::Extended => include_str!("../registry/extended.json"),
            Self::ExtendedPlatinum => include_str!("../registry/extended-platinum.json"),
            Self::ScopedRetrieval => include_str!("../registry/scoped-retrieval.json"),
        };
        serde_json::from_str::<Value>(source).expect("embedded registry must be valid JSON")
            ["tools"]
            .clone()
    }
}

/// Adapters validate tool-specific arguments and enforce authorization before use.
/// Errors must be safe for external display (no credentials or sensitive payloads).
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, arguments: &Value) -> Result<Value, String>;
}

struct RegisteredTool {
    definition: Value,
    handler: Box<dyn ToolHandler>,
}

pub const PROTOCOL_VERSION: &str = "2026-07-28";
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
const TOOL_TIMEOUT: Duration = Duration::from_secs(30);

struct BoundedBuffer(Vec<u8>);

impl std::io::Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = (MAX_RESULT_BYTES + 1).saturating_sub(self.0.len());
        let written = remaining.min(bytes.len());
        self.0.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

enum ResultSerializationError {
    TooLarge,
    Failed,
}

fn serialize_result(value: &Value) -> Result<Vec<u8>, ResultSerializationError> {
    let mut buffer = BoundedBuffer(Vec::with_capacity(MAX_RESULT_BYTES + 1));
    match serde_json::to_writer(&mut buffer, value) {
        Ok(()) if buffer.0.len() <= MAX_RESULT_BYTES => Ok(buffer.0),
        Ok(()) => Err(ResultSerializationError::TooLarge),
        Err(_) if buffer.0.len() > MAX_RESULT_BYTES => Err(ResultSerializationError::TooLarge),
        Err(_) => Err(ResultSerializationError::Failed),
    }
}

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

    pub async fn handle(&mut self, request: Value) -> Option<Value> {
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
        if method == "initialize" {
            let Some(params) = params.filter(|p| p.is_object()) else {
                return Some(error(id, -32602, "Initialization parameters required"));
            };
            if params["protocolVersion"] != PROTOCOL_VERSION
                || !params["capabilities"].is_object()
                || !params["clientInfo"]["name"].is_string()
                || !params["clientInfo"]["version"].is_string()
            {
                let requested = params
                    .get("protocolVersion")
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut response = error(
                    id,
                    -32022,
                    "Unsupported protocol version or invalid client information",
                );
                response["error"]["data"] =
                    json!({"supported":[PROTOCOL_VERSION],"requested":requested});
                return Some(response);
            }
            return Some(json!({"jsonrpc":"2.0","id":id,"result":{
                "protocolVersion":PROTOCOL_VERSION,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"muninn","version":env!("CARGO_PKG_VERSION")}
            }}));
        }
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
                "capabilities":{"tools":{"listChanged":false}},
                "ttlMs":0,
                "cacheScope":"private"
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
                let result = tokio::time::timeout(TOOL_TIMEOUT, tool.handler.call(arguments))
                    .await
                    .unwrap_or_else(|_| Err("timeout".to_owned()));
                let (payload, failed) = result.map_or_else(
                    |_| ("Tool execution failed".to_owned(), true),
                    |value| match serialize_result(&value) {
                        Ok(bytes) => (
                            String::from_utf8(bytes).expect("JSON serialization is valid UTF-8"),
                            false,
                        ),
                        Err(ResultSerializationError::TooLarge) => {
                            ("Tool result exceeds size limit".to_owned(), true)
                        }
                        Err(ResultSerializationError::Failed) => {
                            ("Tool execution failed".to_owned(), true)
                        }
                    },
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
pub async fn serve<R: tokio::io::AsyncBufRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    server: &mut Server,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, Error, ErrorKind};
    loop {
        let mut frame = Vec::new();
        let read = (&mut reader)
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut frame)
            .await?;
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
        let response = match serde_json::from_slice::<Value>(&frame) {
            Ok(request) => server.handle(request).await,
            Err(_) => Some(error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            frame.clear();
            serde_json::to_writer(&mut frame, &response)?;
            if frame.len() > MAX_MESSAGE_BYTES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "MCP response exceeds size limit",
                ));
            }
            writer.write_all(&frame).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedBuffer, MAX_RESULT_BYTES};
    use std::io::Write;

    #[test]
    fn result_buffer_caps_serialized_bytes_at_limit_plus_one() {
        let mut buffer = BoundedBuffer(Vec::new());
        let oversized = vec![b'x'; MAX_RESULT_BYTES + 128];

        let written = buffer.write(&oversized).unwrap();

        assert_eq!(written, MAX_RESULT_BYTES + 1);
        assert_eq!(buffer.0.len(), MAX_RESULT_BYTES + 1);
    }
}
