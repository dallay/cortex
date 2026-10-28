//! Adaptación stateless de texto y funciones; no implementa estado nativo de Responses.

use std::collections::HashSet;

use rook_core::{CompletionRequest, CompletionResponse, FinishReason, MessageContent, TokenUsage};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::openai_adapter::{OpenAIChatRequest, OpenAIFunctionCall, OpenAIMessage, OpenAIToolCall};

pub(super) const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_TOOLS: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: Input,
    pub instructions: Option<String>,
    pub stream: Option<bool>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Option<Vec<FunctionTool>>,
    pub tool_choice: Option<Value>,
    pub store: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Text(String),
    Items(Vec<InputItem>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum InputItem {
    Message(InputMessage),
    Call(FunctionCall),
    Result(FunctionResult),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputMessage {
    #[serde(rename = "type")]
    kind: Option<String>,
    role: String,
    content: TextContent,
    id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TextContent {
    Text(String),
    Parts(Vec<TextPart>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPart {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    annotations: Option<Vec<Value>>,
    logprobs: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionCall {
    #[serde(rename = "type")]
    kind: String,
    call_id: String,
    name: String,
    arguments: String,
    id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionResult {
    #[serde(rename = "type")]
    kind: String,
    call_id: String,
    output: String,
    id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    description: Option<String>,
    parameters: Value,
    strict: Option<bool>,
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

fn validate_replay(id: &Option<String>, status: &Option<String>) -> Result<(), String> {
    if id.as_deref().is_some_and(|id| !valid_id(id))
        || status
            .as_deref()
            .is_some_and(|s| !matches!(s, "completed" | "incomplete"))
    {
        return Err("Identidad o estado de entrada no soportado".into());
    }
    Ok(())
}

impl ResponsesRequest {
    pub fn into_domain(self) -> Result<CompletionRequest, String> {
        if self.model.trim().is_empty() || self.model.len() > 256 || self.store == Some(true) {
            return Err("Se requiere model y no se admite store:true".into());
        }
        if self.max_output_tokens == Some(0)
            || self
                .temperature
                .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
        {
            return Err("max_output_tokens o temperature fuera del rango admitido".into());
        }
        let mut messages = vec![];
        if let Some(instructions) = self.instructions {
            messages.push(text_message("system", instructions));
        }
        let mut calls = HashSet::new();
        let mut results = HashSet::new();
        match self.input {
            Input::Text(text) => messages.push(text_message("user", text)),
            Input::Items(items) => {
                if items.is_empty() || items.len() > 1024 {
                    return Err("input requiere entre 1 y 1024 elementos".into());
                }
                for item in items {
                    match item {
                        InputItem::Message(message) => {
                            validate_replay(&message.id, &message.status)?;
                            if message.kind.as_deref().is_some_and(|t| t != "message")
                                || !matches!(
                                    message.role.as_str(),
                                    "user" | "assistant" | "system" | "developer"
                                )
                            {
                                return Err("Tipo o rol de mensaje no soportado".into());
                            }
                            let text = match message.content {
                                TextContent::Text(text) => text,
                                TextContent::Parts(parts) => {
                                    let mut text = String::new();
                                    for part in parts {
                                        if !(part.kind == "input_text"
                                            || (message.role == "assistant"
                                                && part.kind == "output_text"))
                                            || part.annotations.is_some_and(|a| !a.is_empty())
                                            || part.logprobs.is_some_and(|a| !a.is_empty())
                                        {
                                            return Err(
                                                "Solo se admiten partes de texto sin anotaciones"
                                                    .into(),
                                            );
                                        }
                                        text.push_str(&part.text);
                                    }
                                    text
                                }
                            };
                            // Una llamada pendiente y su texto intercalado pertenecen
                            // al mismo turno; resultados u otros roles lo delimitan.
                            if message.role == "assistant"
                                && messages.last().is_some_and(|m| {
                                    m.role == "assistant" && !m.tool_calls.is_empty()
                                })
                            {
                                let pending =
                                    messages.last_mut().ok_or("Falta el mensaje assistant")?;
                                if let Value::String(content) = &mut pending.content {
                                    content.push_str(&text);
                                }
                            } else {
                                messages.push(text_message(&message.role, text));
                            }
                        }
                        InputItem::Call(call) => {
                            validate_replay(&call.id, &call.status)?;
                            let arguments: Value =
                                serde_json::from_str(&call.arguments).map_err(|_| {
                                    "Los argumentos de función deben ser un objeto JSON".to_string()
                                })?;
                            if call.kind != "function_call"
                                || !valid_id(&call.call_id)
                                || !valid_name(&call.name)
                                || !arguments.is_object()
                                || !calls.insert(call.call_id.clone())
                            {
                                return Err("Llamada de función inválida o duplicada".into());
                            }
                            if !messages
                                .last()
                                .is_some_and(|message| message.role == "assistant")
                            {
                                messages.push(text_message("assistant", String::new()));
                            }
                            let message =
                                messages.last_mut().ok_or("Falta el mensaje assistant")?;
                            message.tool_calls.push(OpenAIToolCall {
                                id: call.call_id,
                                call_type: "function".into(),
                                function: OpenAIFunctionCall {
                                    name: call.name,
                                    arguments: call.arguments,
                                },
                            });
                        }
                        InputItem::Result(result) => {
                            validate_replay(&result.id, &result.status)?;
                            if result.kind != "function_call_output"
                                || !calls.contains(&result.call_id)
                                || !results.insert(result.call_id.clone())
                            {
                                return Err(
                                    "Resultado de función sin llamada previa o duplicado".into()
                                );
                            }
                            let mut message = text_message("tool", result.output);
                            message.tool_call_id = Some(result.call_id);
                            messages.push(message);
                        }
                    }
                }
            }
        }
        let mut names = HashSet::new();
        let tools = if let Some(tools) = self.tools {
            if tools.len() > 64 {
                return Err("Se admiten hasta 64 funciones".into());
            }
            let mut normalized = vec![];
            for tool in tools {
                if tool.kind != "function"
                    || !valid_name(&tool.name)
                    || tool.strict == Some(true)
                    || !tool.parameters.is_object()
                    || tool.parameters["type"] != "object"
                    || !names.insert(tool.name.clone())
                {
                    return Err("Función no soportada: requiere nombre único, parameters de objeto y strict:false".into());
                }
                let mut function = json!({"name":tool.name,"parameters":tool.parameters});
                if let Some(description) = tool.description {
                    function["description"] = json!(description);
                }
                normalized.push(json!({"type":"function","function":function}));
            }
            Some(Value::Array(normalized))
        } else {
            None
        };
        let tool_choice = match self.tool_choice {
            None => None,
            Some(Value::String(choice))
                if matches!(choice.as_str(), "auto" | "none" | "required") =>
            {
                if choice == "required" && names.is_empty() {
                    return Err("tool_choice requiere funciones".into());
                }
                Some(json!(choice))
            }
            Some(Value::Object(choice))
                if choice.len() == 2 && choice.get("type") == Some(&json!("function")) =>
            {
                let name = choice
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("tool_choice inválido")?;
                if !names.contains(name) {
                    return Err("tool_choice debe seleccionar una función declarada".into());
                }
                Some(json!({"type":"function", "function":{"name":name}}))
            }
            _ => return Err("tool_choice no soportado".into()),
        };
        let mut request = CompletionRequest::from(OpenAIChatRequest {
            model: self.model,
            messages,
            stream: self.stream,
            max_tokens: self.max_output_tokens,
            temperature: self.temperature,
            tools,
            tool_choice,
            n: None,
            stream_options: None,
            response_format: None,
        });
        request.metadata.origin = "responses".into();
        request.metadata.cacheable = false;
        Ok(request)
    }
}

fn text_message(role: &str, text: String) -> OpenAIMessage {
    OpenAIMessage {
        role: role.into(),
        content: json!(text),
        tool_calls: vec![],
        tool_call_id: None,
    }
}

/// Metadatos estables de la respuesta, independientes de los IDs del proveedor.
#[derive(Clone)]
pub struct ResponseContext {
    pub id: String,
    pub model: String,
    created_at: i64,
    tools: Value,
    tool_choice: Value,
}

impl ResponseContext {
    pub fn new(request: &CompletionRequest) -> Self {
        let tools = request
            .tools
            .as_ref()
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .map(|t| {
                        let mut tool = t["function"].clone();
                        tool["type"] = json!("function");
                        tool["strict"] = json!(false);
                        tool
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut choice = request.tool_choice.clone().unwrap_or(json!("auto"));
        if choice.is_object() {
            choice = json!({"type":"function","name":choice["function"]["name"]});
        }
        Self {
            id: format!("resp_{}", request.id),
            model: request.model.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            tools: json!(tools),
            tool_choice: choice,
        }
    }

    pub fn envelope(
        &self,
        status: &str,
        output: Vec<Value>,
        usage: Option<&TokenUsage>,
        finish: Option<FinishReason>,
    ) -> Value {
        json!({"id":self.id, "object":"response", "created_at":self.created_at,
            "status":status, "error":null, "incomplete_details":match finish {
                Some(FinishReason::Length) => json!({"reason":"max_output_tokens"}),
                Some(FinishReason::ContentFilter) => json!({"reason":"content_filter"}), _ => Value::Null },
            "model":self.model, "store":false, "parallel_tool_calls":true,
            "tools":self.tools, "tool_choice":self.tool_choice, "output":output,
            "usage":usage.map(|u| json!({"input_tokens":u.prompt_tokens,
                "output_tokens":u.completion_tokens,"total_tokens":u.total_tokens,
                "input_tokens_details":{"cached_tokens":u.cache_read_tokens.unwrap_or(0)},
                "output_tokens_details":{"reasoning_tokens":u.reasoning_tokens.unwrap_or(0)}})) })
    }

    pub fn response_body(&self, response: &CompletionResponse) -> Result<Value, String> {
        if response.finish_reason.is_none() {
            return Err("Respuesta sin estado final upstream".into());
        }
        let status = finish_status(response.finish_reason);
        if response.content.len() > MAX_OUTPUT_BYTES
            || response.tool_calls.len() > MAX_TOOLS
            || (response.tool_calls.is_empty()
                && response
                    .content_blocks
                    .iter()
                    .filter(|b| matches!(b, MessageContent::ToolUse { .. }))
                    .count()
                    > MAX_TOOLS)
        {
            return Err("Se superó el límite de salida Responses".into());
        }
        let mut output = vec![];
        if !response.content.is_empty() {
            output.push(message_item(
                format!("msg_{}", self.id),
                &response.content,
                status,
            ));
        }
        let mut calls = response.tool_calls.clone();
        // Anthropic conserva las llamadas en ambas representaciones del dominio.
        if calls.is_empty() {
            calls.extend(response.content_blocks.iter().filter_map(|b| match b {
                MessageContent::ToolUse { id, name, input } => Some(rook_core::MessageToolCall {
                    id: Some(id.clone()),
                    name: name.clone(),
                    arguments: input.clone(),
                }),
                _ => None,
            }));
        }
        if response.finish_reason == Some(FinishReason::ToolCalls) && calls.is_empty() {
            return Err("Estado tool_calls sin funciones".into());
        }
        if response.finish_reason == Some(FinishReason::Stop) && !calls.is_empty() {
            return Err("Estado stop con funciones".into());
        }
        let mut ids = HashSet::new();
        let mut argument_bytes = 0usize;
        for (index, call) in calls.iter().enumerate() {
            let call_id = call
                .id
                .clone()
                .unwrap_or_else(|| format!("call_{}_{}", self.id, index));
            if !valid_id(&call_id) || !valid_name(&call.name) || !ids.insert(call_id.clone()) {
                return Err("El proveedor devolvió una llamada de función inválida".into());
            }
            let arguments = match &call.arguments {
                Value::Object(_) => call.arguments.to_string(),
                Value::String(raw) if status == "incomplete" => raw.clone(),
                _ => return Err("El proveedor devolvió argumentos de función inválidos".into()),
            };
            argument_bytes = argument_bytes.saturating_add(arguments.len());
            if argument_bytes > MAX_ARGUMENT_BYTES
                || argument_bytes.saturating_add(response.content.len()) > MAX_OUTPUT_BYTES
            {
                return Err("Se superó el límite de argumentos Responses".into());
            }
            output.push(
                json!({"type":"function_call", "id":format!("fc_{}_{}",self.id,index),
                "call_id":call_id,"name":call.name,"arguments":arguments, "status":status}),
            );
        }
        Ok(self.envelope(
            status,
            output,
            Some(&response.usage),
            response.finish_reason,
        ))
    }
}

pub(super) fn finish_status(finish: Option<FinishReason>) -> &'static str {
    match finish {
        Some(FinishReason::Length | FinishReason::ContentFilter) => "incomplete",
        _ => "completed",
    }
}

pub(super) fn message_item(id: String, text: &str, status: &str) -> Value {
    json!({"id":id,"type":"message","role":"assistant","status":status,
        "content":[text_part(text)]})
}

pub(super) fn text_part(text: &str) -> Value {
    json!({"type":"output_text","text":text,"annotations":[],"logprobs":[]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion() -> CompletionResponse {
        CompletionResponse {
            id: shared_kernel::RequestId::new(),
            provider: shared_kernel::ProviderId::new("test"),
            model: shared_kernel::ModelId::new("test"),
            content: String::new(),
            content_blocks: vec![],
            thinking: None,
            tool_calls: vec![],
            finish_reason: Some(FinishReason::Stop),
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 0,
            cache_hit: None,
        }
    }

    fn context() -> ResponseContext {
        let request: ResponsesRequest =
            serde_json::from_value(json!({"model":"test","input":"hola"})).unwrap();
        ResponseContext::new(&request.into_domain().unwrap())
    }

    #[test]
    fn nonstream_requires_coherent_terminal_state() {
        let mut response = completion();
        response.finish_reason = None;
        assert!(context().response_body(&response).is_err());
        response.finish_reason = Some(FinishReason::ToolCalls);
        assert!(context().response_body(&response).is_err());
    }

    #[test]
    fn nonstream_response_has_same_resource_limits_as_streaming() {
        let mut response = completion();
        response.tool_calls = (0..65)
            .map(|i| rook_core::MessageToolCall {
                id: Some(format!("call_{i}")),
                name: "lookup".into(),
                arguments: json!({}),
            })
            .collect();
        assert!(context().response_body(&response).is_err());
        response.tool_calls.clear();
        response.content = "x".repeat(4 * 1024 * 1024 + 1);
        assert!(context().response_body(&response).is_err());
    }

    #[test]
    fn nonstream_partial_tool_arguments_preserve_incomplete_state() {
        let mut response = completion();
        response.finish_reason = Some(FinishReason::Length);
        response.tool_calls.push(rook_core::MessageToolCall {
            id: Some("call_a".into()),
            name: "lookup".into(),
            arguments: json!("{\"a\":"),
        });
        let body = context().response_body(&response).unwrap();
        assert_eq!(body["status"], "incomplete");
        assert_eq!(body["output"][0]["arguments"], "{\"a\":");
    }

    #[test]
    fn test_valid_name_and_valid_id() {
        assert!(valid_name("tool_name-123"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(65)));
        assert!(!valid_name("invalid name!"));

        assert!(valid_id("id-123"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(257)));
        assert!(!valid_id("id\nwith\ncontrol"));
    }

    #[test]
    fn test_validate_replay_cases() {
        assert!(validate_replay(&Some("valid_id".into()), &Some("completed".into())).is_ok());
        assert!(validate_replay(&Some("valid_id".into()), &Some("incomplete".into())).is_ok());
        assert!(validate_replay(&Some("".into()), &Some("completed".into())).is_err());
        assert!(validate_replay(&Some("valid_id".into()), &Some("other".into())).is_err());
    }

    #[test]
    fn test_responses_request_validation_errors() {
        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": "hi", "max_output_tokens": 0
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": "hi", "temperature": -1.0
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": "hi", "store": true
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": []
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": [{"role": "unknown", "content": "hi"}]
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": [{"type": "function_call", "call_id": "c", "name": "f", "arguments": "invalid_json"}]
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": [{"type": "function_call_output", "call_id": "unmatched", "output": "o"}]
        })).unwrap();
        assert!(req.into_domain().is_err());
    }

    #[test]
    fn test_responses_request_tool_choice_validation() {
        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": "hi", "tool_choice": "required"
        })).unwrap();
        assert!(req.into_domain().is_err());

        let req: ResponsesRequest = serde_json::from_value(json!({
            "model": "m", "input": "hi", "tool_choice": {"type": "function", "name": "undeclared"}
        })).unwrap();
        assert!(req.into_domain().is_err());
    }

    #[test]
    fn test_response_context_envelope_content_filter() {
        let ctx = context();
        let env = ctx.envelope("completed", vec![], None, Some(FinishReason::ContentFilter));
        assert_eq!(env["incomplete_details"]["reason"], "content_filter");
    }
}
