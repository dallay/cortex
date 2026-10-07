use agent_core::{
    AgentError, CancellationToken, Message, ModelDelta, ModelProvider, ModelRequest, Result, Role,
    ToolCall,
};
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

pub struct OpenAiProvider {
    client: reqwest::Client,
    url: String,
    model: String,
    key: Option<String>,
}
impl OpenAiProvider {
    pub fn new(
        base_url: &str,
        model: String,
        key: Option<String>,
        timeout_secs: u64,
    ) -> Result<Self> {
        let parsed = reqwest::Url::parse(base_url)
            .map_err(|_| AgentError::Configuration("invalid model base_url".into()))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(AgentError::Configuration(
                "base_url must be HTTP(S) without credentials or query parameters".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AgentError::Configuration("cannot construct HTTP client".into()))?;
        Ok(Self {
            client,
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            model,
            key,
        })
    }
}
fn wire_message(message: &Message) -> Value {
    let mut result = json!({"role": message.role, "content": message.content});
    if !message.tool_calls.is_empty() {
        result["tool_calls"] = Value::Array(message.tool_calls.iter().map(|call| json!({
            "id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments.to_string()}
        })).collect());
    }
    if let Some(id) = &message.tool_call_id {
        result["tool_call_id"] = json!(id);
    }
    result
}
#[async_trait]
impl ModelProvider for OpenAiProvider {
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let mut body = json!({"model":self.model,"messages":request.messages.iter().map(wire_message).collect::<Vec<_>>(),
            "stream":true,"max_tokens":request.max_tokens,"stream_options":{"include_usage":true}});
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.iter().map(|t| json!({
            "type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}
        })).collect());
        }
        let mut builder = self.client.post(&self.url).json(&body);
        if let Some(key) = &self.key {
            builder = builder.bearer_auth(key);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            response = builder.send() => response.map_err(|_| AgentError::Model("HTTP request failed (check endpoint/network)".into()))?,
        };
        if !response.status().is_success() {
            // Do not echo provider bodies or authenticated request URLs into session history.
            return Err(AgentError::Model(format!(
                "endpoint returned HTTP {}",
                response.status()
            )));
        }
        let state = StreamState {
            bytes: response.bytes_stream().boxed(),
            parser: SseParser::default(),
            queued: VecDeque::new(),
            cancellation: cancel,
            ended: false,
        };
        Ok(futures::stream::unfold(state, |mut state| async move {
            loop {
                if let Some(item) = state.queued.pop_front() { return Some((item, state)); }
                if state.ended { return None; }
                let chunk = tokio::select! {
                    _ = state.cancellation.cancelled() => { state.ended=true; state.queued.push_back(Err(AgentError::Cancelled)); continue; }
                    item = state.bytes.next() => item,
                };
                let parsed = match chunk {
                    Some(Ok(bytes)) => state.parser.push(&bytes),
                    Some(Err(_)) => Err(AgentError::Model("stream interrupted".into())),
                    None => state.parser.eof(),
                };
                match parsed {
                    Ok(items) => state.queued.extend(items.into_iter().map(Ok)),
                    Err(error) => { state.queued.push_back(Err(error)); state.ended=true; }
                }
                if state.parser.done { state.ended=true; }
            }
        }).boxed())
    }
}
struct StreamState {
    bytes: BoxStream<'static, std::result::Result<bytes::Bytes, reqwest::Error>>,
    parser: SseParser,
    queued: VecDeque<Result<ModelDelta>>,
    cancellation: CancellationToken,
    ended: bool,
}
#[derive(Default)]
struct CallParts {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    calls: BTreeMap<usize, CallParts>,
    finished: bool,
    done: bool,
}
impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ModelDelta>> {
        if self.done {
            return Ok(vec![]);
        }
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > 1_048_576 {
            return Err(AgentError::Model("SSE event exceeds 1 MiB".into()));
        }
        let mut result = vec![];
        loop {
            let delimiter = self
                .buffer
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|p| (p, 2))
                .into_iter()
                .chain({
                    self.buffer
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|p| (p, 4))
                })
                .min_by_key(|(position, _)| *position);
            let Some((end, len)) = delimiter else {
                break;
            };
            let event: Vec<_> = self.buffer.drain(..end + len).collect();
            let text = std::str::from_utf8(&event)
                .map_err(|_| AgentError::Model("invalid UTF-8 in SSE event".into()))?;
            let data = text
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            if data.trim() == "[DONE]" {
                result.extend(self.finish()?);
                break;
            }
            let value: Value = serde_json::from_str(&data)
                .map_err(|_| AgentError::Model("malformed SSE JSON".into()))?;
            if value.get("error").is_some() {
                return Err(AgentError::Model("endpoint sent a stream error".into()));
            }
            if let Some(usage) = value.get("usage").filter(|v| v.is_object()) {
                result.push(ModelDelta::Usage {
                    input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                    output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
                });
            }
            if let Some(choice) = value["choices"].as_array().and_then(|c| c.first()) {
                if let Some(text) = choice["delta"]["content"].as_str() {
                    if self.finished {
                        return Err(AgentError::Model("content after finish marker".into()));
                    }
                    result.push(ModelDelta::Text(text.into()));
                }
                if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
                    if self.finished {
                        return Err(AgentError::Model(
                            "tool arguments after finish marker".into(),
                        ));
                    }
                    for call in calls {
                        let index = call["index"]
                            .as_u64()
                            .and_then(|i| usize::try_from(i).ok())
                            .filter(|i| *i < 64)
                            .ok_or_else(|| AgentError::Model("invalid tool-call index".into()))?;
                        let parts = self.calls.entry(index).or_default();
                        if let Some(id) = call["id"].as_str() {
                            if !parts.id.is_empty() && parts.id != id {
                                return Err(AgentError::Model("tool-call id changed".into()));
                            }
                            parts.id = id.into();
                        }
                        if let Some(name) = call["function"]["name"].as_str() {
                            parts.name.push_str(name);
                        }
                        if let Some(args) = call["function"]["arguments"].as_str() {
                            parts.arguments.push_str(args);
                        }
                        if parts.arguments.len() > 262_144
                            || parts.name.len() > 256
                            || parts.id.len() > 256
                        {
                            return Err(AgentError::Model("tool call exceeds limits".into()));
                        }
                    }
                }
                if let Some(reason) = choice["finish_reason"].as_str() {
                    if !matches!(reason, "stop" | "tool_calls") {
                        return Err(AgentError::Model(format!(
                            "incomplete model response: {reason}"
                        )));
                    }
                    self.finished = true;
                }
            }
        }
        Ok(result)
    }
    pub fn eof(&mut self) -> Result<Vec<ModelDelta>> {
        if self.done {
            return Ok(vec![]);
        }
        if !self.buffer.iter().all(u8::is_ascii_whitespace) {
            return Err(AgentError::Model("truncated SSE event".into()));
        }
        self.finish()
    }
    fn finish(&mut self) -> Result<Vec<ModelDelta>> {
        if !self.finished {
            return Err(AgentError::Model(
                "stream ended without a finish marker".into(),
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut result = vec![];
        for parts in std::mem::take(&mut self.calls).into_values() {
            if parts.id.is_empty() || parts.name.is_empty() || !ids.insert(parts.id.clone()) {
                return Err(AgentError::Model(
                    "missing or duplicate tool identity".into(),
                ));
            }
            let arguments: Value = serde_json::from_str(&parts.arguments)
                .map_err(|_| AgentError::Model("invalid tool arguments".into()))?;
            if !arguments.is_object() {
                return Err(AgentError::Model("tool arguments must be an object".into()));
            }
            result.push(ModelDelta::ToolCall(ToolCall {
                id: parts.id,
                name: parts.name,
                arguments,
            }));
        }
        self.done = true;
        result.push(ModelDelta::Finished);
        Ok(result)
    }
}

/// An offline provider for exercising the same loop and read-only tool path.
pub struct MockProvider;
#[async_trait]
impl ModelProvider for MockProvider {
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let last = request
            .messages
            .last()
            .ok_or_else(|| AgentError::Model("empty context".into()))?;
        let output = if last.role == Role::Tool {
            vec![
                ModelDelta::Text(format!("Workspace result:\n{}", last.content)),
                ModelDelta::Finished,
            ]
        } else if last.content.trim() == "list files" {
            vec![
                ModelDelta::ToolCall(ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "list_files".into(),
                    arguments: json!({"path":"."}),
                }),
                ModelDelta::Finished,
            ]
        } else {
            vec![
                ModelDelta::Text(format!("Offline mock: {}", last.content)),
                ModelDelta::Finished,
            ]
        };
        Ok(futures::stream::iter(output.into_iter().map(Ok)).boxed())
    }
}
