// OpenAI adapter — translates between OpenAI wire format and domain model

use rook_core::{
    ApiKeyRestrictions, CompletionRequest, FinishReason, Message, MessageContent, MessageToolCall,
    RequestMetadata, Role, StreamChunk,
};
use serde::{Deserialize, Serialize};
use shared_kernel::{ModelId, RequestId};

/// Incoming request from OpenAI-compatible clients
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub stream: Option<bool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub n: Option<u32>, // ignored for now
    // Forward-compat fields — accepted but not yet routed to providers
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
    pub stream_options: Option<serde_json::Value>,
    pub response_format: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenAIMessage {
    pub role: String,
    /// Content is a plain string for text-only messages, an array of content
    /// parts for multimodal messages, or null on assistant tool-call messages.
    #[serde(default)]
    pub content: serde_json::Value,
    #[serde(default)]
    pub tool_calls: Vec<OpenAIToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIFunctionCall {
    pub name: String,
    pub arguments: String,
}

impl OpenAIMessage {
    /// Extract the text content from either a plain string or an array of
    /// content-part objects (`{"type":"text","text":"…"}`).
    /// Non-text parts (image_url, etc.) are silently skipped.
    pub fn into_text(self) -> String {
        text_from_openai_content(self.content)
    }

    fn into_domain_message(self) -> Message {
        let OpenAIMessage {
            role: wire_role,
            content: wire_content,
            tool_calls: wire_tool_calls,
            tool_call_id,
        } = self;
        let role = match wire_role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "developer" => Role::Developer,
            "tool" => Role::User,
            _ => Role::User,
        };

        let content = if wire_role == "tool" {
            MessageContent::ToolResult {
                tool_use_id: tool_call_id.unwrap_or_default(),
                content: vec![MessageContent::Text(text_from_openai_content(wire_content))],
            }
        } else {
            MessageContent::Text(text_from_openai_content(wire_content))
        };
        let tool_calls = wire_tool_calls
            .into_iter()
            .map(|tool_call| MessageToolCall {
                id: Some(tool_call.id),
                name: tool_call.function.name,
                arguments: serde_json::from_str(&tool_call.function.arguments)
                    .unwrap_or(serde_json::Value::String(tool_call.function.arguments)),
            })
            .collect();

        Message {
            role,
            content,
            tool_calls,
        }
    }
}

fn text_from_openai_content(content: serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s,
        serde_json::Value::Array(parts) => parts
            .into_iter()
            .filter_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text").and_then(|t| t.as_str()).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

impl From<OpenAIChatRequest> for CompletionRequest {
    fn from(req: OpenAIChatRequest) -> Self {
        Self {
            id: RequestId::new(),
            model: ModelId::new(req.model),
            messages: req
                .messages
                .into_iter()
                .map(OpenAIMessage::into_domain_message)
                .collect(),
            stream: req.stream.unwrap_or(false),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools: req.tools,
            tool_choice: req.tool_choice,
            metadata: RequestMetadata {
                origin: "openai".to_string(),
                cacheable: true,
                priority: 5,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: ApiKeyRestrictions::default(),
        }
    }
}

/// Outgoing response in OpenAI format
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenAIChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
    pub usage: OpenAIUsage,
}

#[derive(Debug, Serialize)]
pub struct OpenAIChoice {
    pub index: u32,
    pub message: OpenAIMessageContent,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIMessageContent {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIResponseToolCall>>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIResponseToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIResponseFunctionCall,
}

#[derive(Debug, Serialize)]
pub struct OpenAIResponseFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl From<&rook_core::CompletionResponse> for OpenAIChatResponse {
    fn from(resp: &rook_core::CompletionResponse) -> Self {
        let mut tool_calls: Vec<OpenAIResponseToolCall> = resp
            .tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| OpenAIResponseToolCall {
                id: call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}_{}", resp.id, index)),
                call_type: "function".to_string(),
                function: OpenAIResponseFunctionCall {
                    name: call.name.clone(),
                    arguments: serde_json::to_string(&call.arguments)
                        .unwrap_or_else(|_| "null".to_string()),
                },
            })
            .collect();
        tool_calls.extend(
            resp.content_blocks
                .iter()
                .filter_map(|block| match block {
                    MessageContent::ToolUse { id, name, input } => Some(OpenAIResponseToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: OpenAIResponseFunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input)
                                .unwrap_or_else(|_| "{}".to_string()),
                        },
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        );
        let has_tool_calls = !tool_calls.is_empty();

        Self {
            id: format!("rook-{}", resp.id),
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp() as u64,
            model: resp.model.to_string(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIMessageContent {
                    role: "assistant".to_string(),
                    content: if has_tool_calls {
                        String::new()
                    } else {
                        resp.content.clone()
                    },
                    reasoning_content: resp.thinking.clone(),
                    tool_calls: has_tool_calls.then_some(tool_calls),
                },
                finish_reason: resp
                    .finish_reason
                    .map(finish_reason_to_openai)
                    .or_else(|| has_tool_calls.then_some("tool_calls".to_string())),
            }],
            usage: OpenAIUsage {
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                total_tokens: resp.usage.total_tokens,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenAIChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAIChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIChunkChoice {
    pub index: u32,
    pub delta: OpenAIChunkDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIChunkToolCall>>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIChunkToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIChunkToolFunction,
}

#[derive(Debug, Serialize)]
pub struct OpenAIChunkToolFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

impl From<&StreamChunk> for OpenAIChatCompletionChunk {
    fn from(chunk: &StreamChunk) -> Self {
        let finish_reason = chunk.finish_reason.map(|reason| match reason {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::ToolCalls => "tool_calls",
        });

        Self {
            id: format!("rook-{}", chunk.id),
            object: "chat.completion.chunk".to_string(),
            created: chrono::Utc::now().timestamp() as u64,
            model: chunk.model.to_string(),
            choices: vec![OpenAIChunkChoice {
                index: 0,
                delta: OpenAIChunkDelta {
                    role: None,
                    content: if chunk.delta.is_empty() {
                        None
                    } else {
                        Some(chunk.delta.clone())
                    },
                    reasoning_content: chunk.thinking.clone(),
                    tool_calls: (!chunk.tool_calls.is_empty()).then(|| {
                        chunk
                            .tool_calls
                            .iter()
                            .map(|tool_call| OpenAIChunkToolCall {
                                index: tool_call.index,
                                id: Some(tool_call.id.clone().unwrap_or_else(|| {
                                    format!("call_{}_{}", chunk.id, tool_call.index)
                                })),
                                call_type: "function".to_string(),
                                function: OpenAIChunkToolFunction {
                                    name: tool_call.function.name.clone(),
                                    arguments: tool_call.function.arguments.clone(),
                                },
                            })
                            .collect()
                    }),
                },
                finish_reason: finish_reason.map(str::to_string),
            }],
            usage: chunk.usage.as_ref().map(|usage| OpenAIUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            }),
        }
    }
}

fn finish_reason_to_openai(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::ToolCalls => "tool_calls",
    }
    .to_string()
}

/// OpenAI error response shape
#[derive(Debug, Serialize)]
pub struct OpenAIErrorResponse {
    pub error: OpenAIErrorBody,
}

#[derive(Debug, Serialize)]
pub struct OpenAIErrorBody {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
    pub message: String,
    pub param: Option<String>,
}

#[cfg(test)]
mod openai_adapter_tests {
    use super::*;
    use rook_core::{ToolCallDelta, ToolCallFunctionDelta};

    #[test]
    fn deserializes_request_with_tool_fields_without_error() {
        let json = r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{}],
            "tool_choice": "auto",
            "stream_options": {},
            "response_format": {}
        }"#;
        let req: OpenAIChatRequest = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(req.model, "gpt-4o");
        assert!(req.tools.is_some());
        assert!(req.tool_choice.is_some());
        assert!(req.stream_options.is_some());
        assert!(req.response_format.is_some());
    }

    #[test]
    fn minimal_request_still_parses_correctly() {
        // SC-03 regression: minimal request without optional fields must still work
        let json = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;
        let req: OpenAIChatRequest = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(req.model, "gpt-4o");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].content, "hello");
        assert!(req.tools.is_none());
    }

    #[test]
    fn serializes_domain_tool_use_as_openai_tool_calls() {
        let resp = rook_core::CompletionResponse {
            id: RequestId::new(),
            provider: shared_kernel::ProviderId::new("test"),
            model: ModelId::new("gpt-4o"),
            content: String::new(),
            content_blocks: vec![MessageContent::ToolUse {
                id: "call_123".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({"city": "Paris"}),
            }],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: rook_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        };

        let openai_resp = OpenAIChatResponse::from(&resp);
        let json = serde_json::to_value(openai_resp).unwrap();

        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["id"],
            "call_123"
        );
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
    }

    #[test]
    fn serializes_typed_finish_reason_thinking_and_neutral_tool_call() {
        let response = rook_core::CompletionResponse {
            id: RequestId::new(),
            provider: shared_kernel::ProviderId::new("ollama"),
            model: ModelId::new("qwen3"),
            content: String::new(),
            content_blocks: vec![],
            thinking: Some("checking the key".to_string()),
            tool_calls: vec![MessageToolCall {
                id: None,
                name: "lookup".to_string(),
                arguments: serde_json::json!({"key": "value"}),
            }],
            finish_reason: Some(FinishReason::Length),
            usage: rook_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        };

        let json = serde_json::to_value(OpenAIChatResponse::from(&response)).unwrap();
        assert_eq!(json["choices"][0]["finish_reason"], "length");
        assert_eq!(
            json["choices"][0]["message"]["reasoning_content"],
            "checking the key"
        );
        let call = &json["choices"][0]["message"]["tool_calls"][0];
        assert!(call["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("call_")));
        assert_eq!(call["function"]["name"], "lookup");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                call["function"]["arguments"].as_str().unwrap()
            )
            .unwrap(),
            serde_json::json!({"key": "value"})
        );
    }

    #[test]
    fn assistant_tool_calls_convert_to_neutral_domain_calls() {
        let json = r#"{
            "model": "gpt-4o",
            "messages": [{
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"city\":\"Paris\"}"
                    }
                }]
            }]
        }"#;

        let req: OpenAIChatRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: CompletionRequest = req.into();

        assert_eq!(domain.messages[0].role, Role::Assistant);
        assert_eq!(
            domain.messages[0].content,
            MessageContent::Text(String::new())
        );
        assert_eq!(
            domain.messages[0].tool_calls[0],
            MessageToolCall {
                id: Some("call_123".to_string()),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "Paris"}),
            }
        );
    }

    #[test]
    fn assistant_message_preserves_every_structured_tool_call() {
        let json = r#"{
            "model": "gpt-4o",
            "messages": [{
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {"id":"call_1","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}},
                    {"id":"call_2","type":"function","function":{"name":"second","arguments":"{\"b\":2}"}}
                ]
            }]
        }"#;

        let req: OpenAIChatRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: CompletionRequest = req.into();
        let message = serde_json::to_value(&domain.messages[0]).unwrap();

        assert_eq!(message["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(message["tool_calls"][0]["id"], "call_1");
        assert_eq!(message["tool_calls"][0]["name"], "first");
        assert_eq!(message["tool_calls"][1]["id"], "call_2");
        assert_eq!(message["tool_calls"][1]["name"], "second");
    }

    #[test]
    fn serializes_structured_streaming_tool_call_delta() {
        let chunk = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("gpt-4o"),
            delta: String::new(),
            thinking: None,
            tool_calls: vec![
                ToolCallDelta {
                    index: 2,
                    id: Some("call-2".to_string()),
                    function: ToolCallFunctionDelta {
                        name: Some("lookup".to_string()),
                        arguments: Some("{\"key\":".to_string()),
                    },
                },
                ToolCallDelta {
                    index: 3,
                    id: Some("call-3".to_string()),
                    function: ToolCallFunctionDelta {
                        name: Some("count".to_string()),
                        arguments: Some("{}".to_string()),
                    },
                },
            ],
            finish_reason: None,
            usage: None,
        };

        let json = serde_json::to_value(OpenAIChatCompletionChunk::from(&chunk)).unwrap();
        assert_eq!(json["choices"][0]["delta"]["tool_calls"][0]["index"], 2);
        assert_eq!(json["choices"][0]["delta"]["tool_calls"][0]["id"], "call-2");
        assert_eq!(
            json["choices"][0]["delta"]["tool_calls"][0]["type"],
            "function"
        );
        assert_eq!(
            json["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
        assert_eq!(
            json["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"key\":"
        );
        assert_eq!(json["choices"][0]["delta"]["tool_calls"][1]["index"], 3);
        assert_eq!(json["choices"][0]["delta"]["tool_calls"][1]["id"], "call-3");
    }

    #[test]
    fn tool_role_message_converts_to_domain_tool_result() {
        let json = r#"{
            "model": "gpt-4o",
            "messages": [{
                "role": "tool",
                "tool_call_id": "call_123",
                "content": "{\"temperature\":20}"
            }]
        }"#;

        let req: OpenAIChatRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: CompletionRequest = req.into();

        assert_eq!(domain.messages[0].role, Role::User);
        assert_eq!(
            domain.messages[0].content,
            MessageContent::ToolResult {
                tool_use_id: "call_123".to_string(),
                content: vec![MessageContent::Text(r#"{"temperature":20}"#.to_string())],
            }
        );
    }
}
