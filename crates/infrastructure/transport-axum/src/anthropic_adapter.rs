// Anthropic adapter — translates between Anthropic wire format and domain model

use rook_core::{CompletionRequest, Message, MessageContent, RequestMetadata, Role, StreamChunk};
use serde::{Deserialize, Serialize};
use shared_kernel::{CortexError, ModelId, RequestId};

/// Incoming request to the Anthropic `/v1/messages` endpoint
#[derive(Debug, Deserialize)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub stream: Option<bool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Top-level system prompt — prepended as a System message in the domain model
    pub system: Option<String>,
    // Forward-compat fields — accepted but not yet routed to providers
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

impl From<AnthropicMessagesRequest> for CompletionRequest {
    fn from(req: AnthropicMessagesRequest) -> Self {
        // Prepend top-level system prompt as a System message (SC-05, SC-16)
        let mut messages: Vec<Message> = req
            .system
            .into_iter()
            .map(|s| Message {
                role: Role::System,
                content: MessageContent::Text(s),
            })
            .collect();

        messages.extend(req.messages.into_iter().map(|m| Message {
            role: match m.role.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => Role::User,
            },
            content: MessageContent::Text(m.content),
        }));

        Self {
            id: RequestId::new(),
            model: ModelId::new(req.model),
            messages,
            stream: req.stream.unwrap_or(false),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            metadata: RequestMetadata {
                origin: "anthropic".to_string(),
                cacheable: false,
                priority: 5,
            },
        }
    }
}

/// Anthropic success response
#[derive(Debug, Serialize)]
pub struct AnthropicMessagesResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
    pub model: String,
    pub stop_reason: String,
    pub stop_sequence: Option<()>,
    pub usage: AnthropicUsage,
}

impl From<&rook_core::CompletionResponse> for AnthropicMessagesResponse {
    fn from(resp: &rook_core::CompletionResponse) -> Self {
        Self {
            id: format!("rook-{}", resp.id),
            type_: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![AnthropicContentBlock {
                block_type: "text".to_string(),
                text: resp.content.clone(),
            }],
            model: resp.model.to_string(),
            stop_reason: "end_turn".to_string(),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
            },
        }
    }
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

    #[test]
    fn from_completion_response_builds_anthropic_response() {
        use rook_core::{CompletionResponse, TokenUsage};
        use shared_kernel::{ModelId, ProviderId, RequestId};

        let resp = CompletionResponse {
            id: RequestId::new(),
            provider: ProviderId::new("anthropic-test"),
            model: ModelId::new("claude-3-5-sonnet"),
            content: "Hello there".to_string(),
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                estimated_cost_usd: None,
            },
            latency_ms: 100,
        };

        let anthropic_resp = AnthropicMessagesResponse::from(&resp);
        assert_eq!(anthropic_resp.type_, "message");
        assert_eq!(anthropic_resp.role, "assistant");
        assert_eq!(anthropic_resp.stop_reason, "end_turn");
        assert_eq!(anthropic_resp.content.len(), 1);
        assert_eq!(anthropic_resp.content[0].block_type, "text");
        assert_eq!(anthropic_resp.content[0].text, "Hello there");
        assert_eq!(anthropic_resp.usage.input_tokens, 10);
        assert_eq!(anthropic_resp.usage.output_tokens, 5);

        // Verify it serializes with correct field names
        let json = serde_json::to_string(&anthropic_resp).unwrap();
        assert!(json.contains(r#""type":"message""#));
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""stop_reason":"end_turn""#));
    }

    // T-03 tests ---------------------------------------------------------------

    #[test]
    fn system_field_prepended_as_system_message() {
        let json = r#"{
            "model": "claude-3-5-sonnet",
            "system": "Be concise",
            "messages": [{"role": "user", "content": "Hi"}]
        }"#;
        let req: AnthropicMessagesRequest = serde_json::from_str(json).unwrap();
        let domain: rook_core::CompletionRequest = req.into();
        assert_eq!(domain.messages.len(), 2);
        assert_eq!(domain.messages[0].role, rook_core::Role::System);
        assert_eq!(domain.messages[0].content.as_text(), "Be concise");
        assert_eq!(domain.messages[1].role, rook_core::Role::User);
    }

    #[test]
    fn request_with_tools_deserializes_without_error() {
        let json = r#"{
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hi"}],
            "tools": [{"name": "get_weather"}],
            "tool_choice": {"type": "auto"}
        }"#;
        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        assert!(req.tools.is_some());
        assert!(req.tool_choice.is_some());
    }

    #[test]
    fn minimal_anthropic_request_parses_correctly() {
        let json = r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hello"}]}"#;
        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: rook_core::CompletionRequest = req.into();
        assert_eq!(domain.messages.len(), 1);
        assert_eq!(domain.messages[0].content.as_text(), "hello");
    }
}
