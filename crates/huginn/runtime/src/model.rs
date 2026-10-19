use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_core::{
    AgentError, CancellationToken, Message, ModelDelta, ModelProvider, ModelRequest, Result, Role,
    ToolCall,
};
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
    timeout_secs: u64,
}
impl OpenAiProvider {
    #[cfg(all(feature = "test-hooks", debug_assertions))]
    #[doc(hidden)]
    pub fn trust_test_certificate(&mut self, certificate_der: &[u8]) -> Result<()> {
        self.client = transport_builder(&self.url, self.timeout_secs, false)?
            .add_root_certificate(
                reqwest::Certificate::from_der(certificate_der)
                    .map_err(|_| AgentError::Configuration("invalid test certificate".into()))?,
            )
            .build()
            .map_err(|_| AgentError::Configuration("cannot construct test HTTP client".into()))?;
        Ok(())
    }

    pub fn new(
        base_url: &str,
        model: String,
        key: Option<String>,
        timeout_secs: u64,
    ) -> Result<Self> {
        Self::with_transport(base_url, model, key, timeout_secs, false)
    }
    /// HTTP requires an explicit connection-local opt-in and a literal loopback IP.
    pub fn with_transport(
        base_url: &str,
        model: String,
        key: Option<String>,
        timeout_secs: u64,
        allow_loopback_http: bool,
    ) -> Result<Self> {
        let client = transport_client(base_url, timeout_secs, allow_loopback_http)?;
        Ok(Self {
            client,
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            model,
            key,
            timeout_secs,
        })
    }
}
pub(crate) fn transport_client(
    base_url: &str,
    timeout_secs: u64,
    allow_loopback_http: bool,
) -> Result<reqwest::Client> {
    transport_builder(base_url, timeout_secs, allow_loopback_http)?
        .build()
        .map_err(|_| AgentError::Configuration("cannot construct HTTP client".into()))
}
fn transport_builder(
    base_url: &str,
    timeout_secs: u64,
    allow_loopback_http: bool,
) -> Result<reqwest::ClientBuilder> {
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|_| AgentError::Configuration("invalid model base_url".into()))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || timeout_secs == 0
        || (parsed.scheme() == "http"
            && (!allow_loopback_http || !huginn_core::connections::is_loopback_endpoint(&parsed)))
    {
        return Err(AgentError::Configuration(
                "endpoint requires validated HTTPS or explicitly enabled literal loopback HTTP; no credentials, query or fragment".into(),
            ));
    }
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::none());
    // A loopback exception must never forward content or authorization through
    // an environment-configured HTTP proxy.
    if huginn_core::connections::is_loopback_endpoint(&parsed) {
        builder = builder.no_proxy();
    }
    Ok(builder)
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
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            response = tokio::time::timeout(Duration::from_secs(self.timeout_secs), builder.send()) => response
                .map_err(|_| AgentError::Model("HTTP response headers timed out".into()))?
                .map_err(|_| AgentError::Model("HTTP request failed (check endpoint/network)".into()))?,
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
                if state.ended && state.queued.is_empty() { return None; }
                if state.cancellation.is_cancelled() {
                    state.queued.clear();
                    state.ended = true;
                    return Some((Err(AgentError::Cancelled), state));
                }
                if let Some(item) = state.queued.pop_front() { return Some((item, state)); }
                if state.ended { return None; }
                let chunk = tokio::select! {
                    biased;
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
            if end + len > 1_048_576 {
                return Err(AgentError::Model("SSE event exceeds 1 MiB".into()));
            }
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
                        return Err(AgentError::Model(
                            "incomplete model response (unsupported finish reason)".into(),
                        ));
                    }
                    self.finished = true;
                }
            }
        }
        if self.buffer.len() > 1_048_576 {
            return Err(AgentError::Model("SSE event exceeds 1 MiB".into()));
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

/// Sentinel used only when no inference provider has been configured.
/// It never fabricates a response and fails before any model request is sent.
pub struct UnconfiguredModelProvider;

#[async_trait]
impl ModelProvider for UnconfiguredModelProvider {
    fn is_configured(&self) -> bool {
        false
    }
    async fn stream(
        &self,
        _request: ModelRequest,
        _cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        Err(AgentError::Configuration(
            "No model configured. Set HUGINN_BASE_URL and HUGINN_MODEL before requesting inference.".into(),
        ))
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

/// Adapter construction lives outside the CLI, loop and kernel.
pub struct ProviderConfig {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub key: Option<String>,
    pub timeout_secs: u64,
    pub allow_loopback_http: bool,
}
pub type ProviderFactory = fn(&ProviderConfig) -> Result<std::sync::Arc<dyn ModelProvider>>;
#[derive(Default)]
pub struct ProviderFactories(BTreeMap<String, (ProviderFactory, bool)>);
impl ProviderFactories {
    pub fn register(
        &mut self,
        name: &str,
        credentials: bool,
        factory: ProviderFactory,
    ) -> Result<()> {
        if self.0.contains_key(name) {
            return Err(AgentError::Configuration(format!(
                "duplicate provider {name}"
            )));
        }
        self.0.insert(name.into(), (factory, credentials));
        Ok(())
    }
    pub fn builtin() -> Self {
        let mut factories = Self::default();
        factories
            .register("mock", false, |_| Ok(std::sync::Arc::new(MockProvider)))
            .expect("unique builtin");
        factories
            .register("openai", true, |config| {
                Ok(std::sync::Arc::new(OpenAiProvider::with_transport(
                    config
                        .base_url
                        .as_deref()
                        .ok_or_else(|| AgentError::Configuration("base_url is required".into()))?,
                    config
                        .model
                        .clone()
                        .filter(|model| !model.trim().is_empty())
                        .ok_or_else(|| AgentError::Configuration("model is required".into()))?,
                    config.key.clone(),
                    config.timeout_secs,
                    config.allow_loopback_http,
                )?))
            })
            .expect("unique builtin");
        factories
    }
    pub fn uses_credentials(&self, name: &str) -> Result<bool> {
        self.0
            .get(name)
            .map(|(_, credentials)| *credentials)
            .ok_or_else(|| AgentError::Configuration(format!("unknown provider {name}")))
    }
    pub fn create(
        &self,
        name: &str,
        config: &ProviderConfig,
    ) -> Result<std::sync::Arc<dyn ModelProvider>> {
        let (factory, _) = self
            .0
            .get(name)
            .ok_or_else(|| AgentError::Configuration(format!("unknown provider {name}")))?;
        factory(config)
    }
}
