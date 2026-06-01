// OpenAI adapter — translates between OpenAI wire format and domain model

use rook_core::{
    CompletionRequest, FinishReason, Message, MessageContent, RequestMetadata, Role, StreamChunk,
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
    /// Content is a plain string for text-only messages, or an array of content
    /// parts for multimodal messages.  We accept both without panicking by
    /// deserializing into `serde_json::Value` and extracting text below.
    pub content: serde_json::Value,
}

impl OpenAIMessage {
    /// Extract the text content from either a plain string or an array of
    /// content-part objects (`{"type":"text","text":"…"}`).
    /// Non-text parts (image_url, etc.) are silently skipped — they will be
    /// supported fully in Phase 2 multimodal work.
    pub fn into_text(self) -> String {
        match self.content {
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
}

impl From<OpenAIChatRequest> for CompletionRequest {
    fn from(req: OpenAIChatRequest) -> Self {
        Self {
            id: RequestId::new(),
            model: ModelId::new(req.model),
            messages: req
                .messages
                .into_iter()
                .map(|m| Message {
                    role: match m.role.as_str() {
                        "system" => Role::System,
                        "user" => Role::User,
                        "assistant" => Role::Assistant,
                        "developer" => Role::Developer,
                        _ => Role::User,
                    },
                    content: MessageContent::Text(m.into_text()),
                })
                .collect(),
            stream: req.stream.unwrap_or(false),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            metadata: RequestMetadata {
                origin: "openai".to_string(),
                cacheable: true,
                priority: 5,
            },
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
}

#[derive(Debug, Serialize)]
pub struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl From<&rook_core::CompletionResponse> for OpenAIChatResponse {
    fn from(resp: &rook_core::CompletionResponse) -> Self {
        Self {
            id: format!("rook-{}", resp.id),
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp() as u64,
            model: resp.model.to_string(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIMessageContent {
                    role: "assistant".to_string(),
                    content: resp.content.clone(),
                },
                finish_reason: "stop".to_string(),
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
}
