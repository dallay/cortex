// Anthropic adapter — translates between Anthropic wire format and domain model

use rook_core::{CompletionRequest, Message, RequestMetadata, Role, StreamChunk};
use serde::{Deserialize, Serialize};
use shared_kernel::{CortexError, ModelId, RequestId};

/// Incoming request to the Anthropic `/v1/messages` endpoint
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub stream: Option<bool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

impl From<AnthropicMessagesRequest> for CompletionRequest {
    fn from(req: AnthropicMessagesRequest) -> Self {
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
                        _ => Role::User,
                    },
                    content: m.content,
                })
                .collect(),
            stream: req.stream.unwrap_or(false),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            metadata: RequestMetadata {
                origin: "anthropic".to_string(),
                cacheable: false, // Anthropic doesn't support caching in the same way
                priority: 5,
            },
        }
    }
}

/// Anthropic success response
#[derive(Debug, Serialize)]
pub struct AnthropicMessagesResponse {
    pub id: String,
    pub type_: String,
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
    pub model: String,
    pub stop_reason: String,
    pub stop_sequence: Option<()>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Serialize)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ---------------------------------------------------------------------------
// SSE streaming types
// ---------------------------------------------------------------------------

/// SSE event types for Anthropic streaming
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum AnthropicSseEvent {
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u32,
        delta: AnthropicTextDelta,
    },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDeltaDetails,
        usage: AnthropicMessageDeltaUsage,
    },
    #[serde(rename = "error")]
    Error(AnthropicErrorEvent),
}

#[derive(Debug, Serialize)]
pub struct AnthropicTextDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessageDeltaDetails {
    pub stop_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessageDeltaUsage {
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct AnthropicErrorEvent {
    pub error: AnthropicErrorBody,
}

#[derive(Debug, Serialize)]
pub struct AnthropicErrorBody {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

impl From<&StreamChunk> for AnthropicSseEvent {
    fn from(chunk: &StreamChunk) -> Self {
        if chunk.finish_reason.is_some() {
            let output_tokens = chunk
                .usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0);
            let input_tokens = chunk.usage.as_ref().map(|u| u.prompt_tokens);
            AnthropicSseEvent::MessageDelta {
                delta: AnthropicMessageDeltaDetails {
                    stop_reason: "end_turn".to_string(),
                    stop_sequence: None,
                },
                usage: AnthropicMessageDeltaUsage {
                    output_tokens,
                    input_tokens,
                },
            }
        } else {
            AnthropicSseEvent::ContentBlockDelta {
                index: 0,
                delta: AnthropicTextDelta {
                    delta_type: "text_delta".to_string(),
                    text: chunk.delta.clone(),
                },
            }
        }
    }
}

impl From<CortexError> for AnthropicSseEvent {
    fn from(error: CortexError) -> Self {
        AnthropicSseEvent::Error(AnthropicErrorEvent {
            error: AnthropicErrorBody {
                error_type: "invalid_request_error".to_string(),
                message: error.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rook_core::{FinishReason, StreamChunk, TokenUsage};
    use shared_kernel::RequestId;

    fn make_token_usage(prompt: u32, completion: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            estimated_cost_usd: None,
        }
    }

    #[test]
    fn content_block_delta_serialization() {
        let chunk = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: "Hello".to_string(),
            finish_reason: None,
            usage: None,
        };
        let event: AnthropicSseEvent = (&chunk).into();
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""type":"content_block_delta""#));
        assert!(json.contains(r#""index":0"#));
        assert!(json.contains(r#""type":"text_delta""#));
        assert!(json.contains(r#""text":"Hello""#));
    }

    #[test]
    fn message_delta_serialization_with_usage() {
        let chunk = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: "".to_string(),
            finish_reason: Some(FinishReason::Stop),
            usage: Some(make_token_usage(10, 25)),
        };
        let event: AnthropicSseEvent = (&chunk).into();
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""type":"message_delta""#));
        assert!(json.contains(r#""stop_reason":"end_turn""#));
        assert!(json.contains(r#""output_tokens":25"#));
    }

    #[test]
    fn message_delta_usage_from_final_chunk_only() {
        // Only the final chunk (with finish_reason) should have usage
        let non_final = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: "part".to_string(),
            finish_reason: None,
            usage: Some(make_token_usage(10, 5)), // Should be ignored
        };
        let event: AnthropicSseEvent = (&non_final).into();
        let json = serde_json::to_string(&event).unwrap();

        // Non-final chunks should NOT have usage
        assert!(!json.contains("output_tokens"));
        assert!(json.contains("content_block_delta"));

        // Final chunk SHOULD have usage
        let final_chunk = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: "".to_string(),
            finish_reason: Some(FinishReason::Stop),
            usage: Some(make_token_usage(10, 25)),
        };
        let final_event: AnthropicSseEvent = (&final_chunk).into();
        let final_json = serde_json::to_string(&final_event).unwrap();
        assert!(final_json.contains("output_tokens"));
    }

    #[test]
    fn error_event_serialization() {
        let error = CortexError::provider("upstream error".to_string());
        let event: AnthropicSseEvent = error.into();
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains(r#""type":"invalid_request_error""#));
        assert!(json.contains("upstream error"));
    }
}
