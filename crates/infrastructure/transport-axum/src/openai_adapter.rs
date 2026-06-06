// OpenAI adapter — translates between OpenAI wire format and domain model

use rook_core::{
    ApiKeyRestrictions, CompletionRequest, FinishReason, Message, MessageContent, RequestMetadata,
    Role, StreamChunk,
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
        let role = match self.role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "developer" => Role::Developer,
            "tool" => Role::User,
            _ => Role::User,
        };

        let content = if self.role == "tool" {
            MessageContent::ToolResult {
                tool_use_id: self.tool_call_id.unwrap_or_default(),
                content: vec![MessageContent::Text(text_from_openai_content(self.content))],
            }
        } else if let Some(tool_call) = self.tool_calls.into_iter().next() {
            let input = serde_json::from_str(&tool_call.function.arguments)
                .unwrap_or(serde_json::Value::String(tool_call.function.arguments));
            MessageContent::ToolUse {
                id: tool_call.id,
                name: tool_call.function.name,
                input,
            }
        } else {
            MessageContent::Text(text_from_openai_content(self.content))
        };

        Message { role, content }
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
                cache_control_header: None,
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
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAIMessageContent {
    pub role: String,
    pub content: String,
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
        let tool_calls: Vec<OpenAIResponseToolCall> = resp
            .content_blocks
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
            .collect();
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
                    tool_calls: has_tool_calls.then_some(tool_calls),
                },
                finish_reason: if has_tool_calls { "tool_calls" } else { "stop" }.to_string(),
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
    fn assistant_tool_calls_convert_to_domain_tool_use() {
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
            MessageContent::ToolUse {
                id: "call_123".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({"city": "Paris"}),
            }
        );
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
