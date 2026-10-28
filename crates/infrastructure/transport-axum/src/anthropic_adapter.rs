// Anthropic adapter — translates between Anthropic wire format and domain model

use rook_core::{
    ApiKeyRestrictions, CompletionRequest, Message, MessageContent, MessageToolCall,
    RequestMetadata, Role, StreamChunk,
};
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
    pub content: AnthropicMessageContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicWireContentBlock>),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicWireContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Vec<AnthropicWireContentBlock>,
    },
}

impl AnthropicMessage {
    /// Split one wire message into domain messages, preserving EVERY block in
    /// order. The first message carries `tool_calls`; each additional content
    /// block becomes its own message so no `tool_result` is ever dropped.
    fn into_domain_messages(self) -> Vec<Message> {
        let role = match self.role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => Role::User,
        };
        let mut tool_calls = Vec::new();
        let mut contents: Vec<MessageContent> = Vec::new();
        match self.content {
            AnthropicMessageContent::Text(text) => {
                contents.push(MessageContent::Text(text));
            }
            AnthropicMessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        AnthropicWireContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(MessageToolCall {
                                id: Some(id),
                                name,
                                arguments: input,
                            });
                        }
                        other => {
                            if let Some(content) = anthropic_block_to_domain(other) {
                                contents.push(content);
                            }
                        }
                    }
                }
            }
        }

        let mut messages = Vec::new();
        let mut contents = contents.into_iter();
        messages.push(Message {
            role,
            content: contents
                .next()
                .unwrap_or_else(|| MessageContent::Text(String::new())),
            tool_calls,
        });
        for content in contents {
            messages.push(Message {
                role,
                content,
                tool_calls: Vec::new(),
            });
        }
        messages
    }
}

fn anthropic_block_to_domain(block: AnthropicWireContentBlock) -> Option<MessageContent> {
    match block {
        AnthropicWireContentBlock::Text { text } => Some(MessageContent::Text(text)),
        AnthropicWireContentBlock::ToolUse { id, name, input } => {
            Some(MessageContent::ToolUse { id, name, input })
        }
        AnthropicWireContentBlock::ToolResult {
            tool_use_id,
            content,
        } => Some(MessageContent::ToolResult {
            tool_use_id,
            content: content
                .into_iter()
                .filter_map(anthropic_block_to_domain)
                .filter(|content| !matches!(content, MessageContent::Text(text) if text.is_empty()))
                .collect(),
        }),
    }
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
                tool_calls: vec![],
            })
            .collect();

        messages.extend(
            req.messages
                .into_iter()
                .flat_map(AnthropicMessage::into_domain_messages),
        );

        Self {
            id: RequestId::new(),
            model: ModelId::new(req.model),
            messages,
            stream: req.stream.unwrap_or(false),
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools: req.tools,
            tool_choice: req.tool_choice,
            metadata: RequestMetadata {
                origin: "anthropic".to_string(),
                cacheable: false,
                priority: 5,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: ApiKeyRestrictions::default(),
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

fn anthropic_content_blocks_from_domain(
    resp: &rook_core::CompletionResponse,
) -> Vec<AnthropicContentBlock> {
    let blocks: Vec<AnthropicContentBlock> = resp
        .content_blocks
        .iter()
        .filter_map(domain_content_to_anthropic_block)
        .collect();

    if blocks.is_empty() {
        vec![AnthropicContentBlock::Text {
            text: resp.content.clone(),
        }]
    } else {
        blocks
    }
}

fn domain_content_to_anthropic_block(content: &MessageContent) -> Option<AnthropicContentBlock> {
    match content {
        MessageContent::Text(text) if text.is_empty() => None,
        MessageContent::Text(text) => Some(AnthropicContentBlock::Text { text: text.clone() }),
        MessageContent::ToolUse { id, name, input } => Some(AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        }),
        MessageContent::ToolResult {
            tool_use_id,
            content,
        } => Some(AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content
                .iter()
                .filter_map(domain_content_to_anthropic_block)
                .collect(),
        }),
    }
}

impl From<&rook_core::CompletionResponse> for AnthropicMessagesResponse {
    fn from(resp: &rook_core::CompletionResponse) -> Self {
        Self {
            id: format!("rook-{}", resp.id),
            type_: "message".to_string(),
            role: "assistant".to_string(),
            content: anthropic_content_blocks_from_domain(resp),
            model: resp.model.to_string(),
            stop_reason: anthropic_stop_reason(resp.finish_reason),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<AnthropicContentBlock>,
    },
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
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: AnthropicContentBlockStart,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: AnthropicDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDeltaDetails,
        usage: AnthropicMessageDeltaUsage,
    },
    #[serde(rename = "error")]
    Error(AnthropicErrorEvent),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AnthropicContentBlockStart {
    Thinking(AnthropicThinkingBlock),
    Text(AnthropicTextBlock),
    ToolUse(AnthropicStartBlock),
}

#[derive(Debug, Serialize)]
pub struct AnthropicThinkingBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub thinking: String,
}

#[derive(Debug, Serialize)]
pub struct AnthropicTextBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
}

/// `content_block` payload for a `tool_use` start.
#[derive(Debug, Serialize)]
pub struct AnthropicStartBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Delta payload: text fragments or tool-argument JSON fragments.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum AnthropicDelta {
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
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

/// Map an Anthropic stop reason from a domain finish reason.
fn anthropic_stop_reason(finish: Option<rook_core::FinishReason>) -> String {
    match finish {
        Some(rook_core::FinishReason::ToolCalls) => "tool_use".to_string(),
        Some(rook_core::FinishReason::Length) => "max_tokens".to_string(),
        Some(rook_core::FinishReason::ContentFilter) => "refusal".to_string(),
        _ => "end_turn".to_string(),
    }
}

impl AnthropicSseEvent {
    /// Translate one domain chunk into zero or more SSE events.
    ///
    /// - Non-empty text delta (no finish) → `content_block_delta/text_delta`.
    /// - Tool entries with id/name (no finish) → `content_block_start/tool_use`.
    /// - Tool entries with arguments → `content_block_delta/input_json_delta`.
    /// - Final chunk (`finish_reason` set) → `content_block_stop` per tool
    ///   index present, then `message_delta` with the mapped `stop_reason`.
    ///   Starts are NOT re-emitted on the final chunk: its tool entries are a
    ///   summary for stop generation (see provider `ToolUseTracker`).
    pub fn events_from_chunk(chunk: &StreamChunk) -> Vec<AnthropicSseEvent> {
        let mut events = Vec::new();
        let is_final = chunk.finish_reason.is_some();

        if !is_final {
            if !chunk.delta.is_empty() {
                events.push(AnthropicSseEvent::ContentBlockDelta {
                    index: 0,
                    delta: AnthropicDelta::TextDelta {
                        text: chunk.delta.clone(),
                    },
                });
            }
            for tc in &chunk.tool_calls {
                if tc.id.is_some() || tc.function.name.is_some() {
                    events.push(AnthropicSseEvent::ContentBlockStart {
                        index: tc.index,
                        content_block: AnthropicContentBlockStart::ToolUse(AnthropicStartBlock {
                            block_type: "tool_use".to_string(),
                            id: tc.id.clone().unwrap_or_default(),
                            name: tc.function.name.clone().unwrap_or_default(),
                            input: serde_json::Value::Object(Default::default()),
                        }),
                    });
                }
                if let Some(args) = &tc.function.arguments {
                    events.push(AnthropicSseEvent::ContentBlockDelta {
                        index: tc.index,
                        delta: AnthropicDelta::InputJsonDelta {
                            partial_json: args.clone(),
                        },
                    });
                }
            }
            return events;
        }

        // Final chunk: a combined text fragment still reaches clients as a
        // text delta first; arguments still flow; stops are emitted for the
        // tool indices present (never re-emitted as starts — see above).
        if !chunk.delta.is_empty() {
            events.push(AnthropicSseEvent::ContentBlockDelta {
                index: 0,
                delta: AnthropicDelta::TextDelta {
                    text: chunk.delta.clone(),
                },
            });
        }
        for tc in &chunk.tool_calls {
            if let Some(args) = &tc.function.arguments {
                events.push(AnthropicSseEvent::ContentBlockDelta {
                    index: tc.index,
                    delta: AnthropicDelta::InputJsonDelta {
                        partial_json: args.clone(),
                    },
                });
            }
        }
        for tc in &chunk.tool_calls {
            events.push(AnthropicSseEvent::ContentBlockStop { index: tc.index });
        }
        let output_tokens = chunk
            .usage
            .as_ref()
            .map(|u| u.completion_tokens)
            .unwrap_or(0);
        let input_tokens = chunk.usage.as_ref().map(|u| u.prompt_tokens);
        events.push(AnthropicSseEvent::MessageDelta {
            delta: AnthropicMessageDeltaDetails {
                stop_reason: anthropic_stop_reason(chunk.finish_reason),
                stop_sequence: None,
            },
            usage: AnthropicMessageDeltaUsage {
                output_tokens,
                input_tokens,
            },
        });
        events
    }
}

impl From<&StreamChunk> for AnthropicSseEvent {
    fn from(chunk: &StreamChunk) -> Self {
        // Backward-compatible single-event mapping: first event wins.
        // Streaming paths should use `events_from_chunk`.
        Self::events_from_chunk(chunk)
            .into_iter()
            .next()
            .unwrap_or_else(|| AnthropicSseEvent::MessageDelta {
                delta: AnthropicMessageDeltaDetails {
                    stop_reason: anthropic_stop_reason(chunk.finish_reason),
                    stop_sequence: None,
                },
                usage: AnthropicMessageDeltaUsage {
                    output_tokens: 0,
                    input_tokens: None,
                },
            })
    }
}

/// Per-response metadata needed to bridge native provider streams. Only tool
/// indices, usage, and a content-free terminal chunk are retained, never output.
/// Finish serialization waits for EOF because OpenAI can report usage afterward.
#[derive(Default)]
pub(crate) struct AnthropicStreamEncoder {
    tools: std::collections::BTreeMap<u32, u32>,
    text_index: Option<u32>,
    thinking_index: Option<u32>,
    next_index: u32,
    terminal: Option<StreamChunk>,
    usage: Option<rook_core::TokenUsage>,
}

impl AnthropicStreamEncoder {
    pub(crate) fn push(
        &mut self,
        mut chunk: StreamChunk,
    ) -> Result<Vec<AnthropicSseEvent>, CortexError> {
        if chunk.usage.is_some() {
            self.usage = chunk.usage.take();
        }
        let finish = chunk.finish_reason.take();
        let mut events = Vec::new();
        if let Some(thinking) = chunk.thinking.take().filter(|value| !value.is_empty()) {
            let index = if let Some(index) = self.thinking_index {
                index
            } else {
                let index = self.next_index;
                self.next_index += 1;
                self.thinking_index = Some(index);
                events.push(AnthropicSseEvent::ContentBlockStart {
                    index,
                    content_block: AnthropicContentBlockStart::Thinking(AnthropicThinkingBlock {
                        block_type: "thinking",
                        thinking: String::new(),
                    }),
                });
                index
            };
            events.push(AnthropicSseEvent::ContentBlockDelta {
                index,
                delta: AnthropicDelta::ThinkingDelta { thinking },
            });
        }
        if !chunk.delta.is_empty() {
            let index = if let Some(index) = self.text_index {
                index
            } else {
                let index = self.next_index;
                self.next_index += 1;
                self.text_index = Some(index);
                events.push(AnthropicSseEvent::ContentBlockStart {
                    index,
                    content_block: AnthropicContentBlockStart::Text(AnthropicTextBlock {
                        block_type: "text",
                        text: String::new(),
                    }),
                });
                index
            };
            events.push(AnthropicSseEvent::ContentBlockDelta {
                index,
                delta: AnthropicDelta::TextDelta {
                    text: std::mem::take(&mut chunk.delta),
                },
            });
        }
        for tool in &mut chunk.tool_calls {
            if let Some(index) = self.tools.get(&tool.index) {
                // Anthropic upstream repeats starts as terminal summaries.
                // Preserve argument fragments without emitting duplicate starts.
                tool.id = None;
                tool.function.name = None;
                tool.index = *index;
            } else {
                if self.tools.len() >= 64 {
                    return Err(CortexError::provider(
                        "Anthropic output exceeds 64 tool calls",
                    ));
                }
                let index = self.next_index;
                self.next_index += 1;
                self.tools.insert(tool.index, index);
                if tool.id.is_none() {
                    tool.id = Some(format!("call_{}_{}", chunk.id, tool.index));
                }
                tool.index = index;
            }
        }
        events.extend(AnthropicSseEvent::events_from_chunk(&chunk));
        if let Some(finish) = finish {
            // Drop output before storing terminal metadata. A single final
            // chunk (e.g. Ollama) still emits its starts and arguments above.
            chunk.delta.clear();
            chunk.thinking = None;
            chunk.tool_calls.clear();
            chunk.finish_reason = Some(finish);
            self.terminal = Some(chunk);
        }
        Ok(events)
    }

    pub(crate) fn finish(mut self) -> Vec<AnthropicSseEvent> {
        let Some(mut terminal) = self.terminal.take() else {
            return vec![];
        };
        terminal.usage = self.usage;
        terminal.tool_calls = self
            .tools
            .into_values()
            .chain(self.text_index)
            .chain(self.thinking_index)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|index| rook_core::ToolCallDelta {
                index,
                id: None,
                function: rook_core::ToolCallFunctionDelta {
                    name: None,
                    arguments: None,
                },
            })
            .collect();
        AnthropicSseEvent::events_from_chunk(&terminal)
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
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: None,
        }
    }

    #[test]
    fn content_block_start_serialization_preserves_block_types() {
        let events = [
            AnthropicSseEvent::ContentBlockStart {
                index: 0,
                content_block: AnthropicContentBlockStart::Thinking(AnthropicThinkingBlock {
                    block_type: "thinking",
                    thinking: String::new(),
                }),
            },
            AnthropicSseEvent::ContentBlockStart {
                index: 1,
                content_block: AnthropicContentBlockStart::Text(AnthropicTextBlock {
                    block_type: "text",
                    text: String::new(),
                }),
            },
            AnthropicSseEvent::ContentBlockStart {
                index: 2,
                content_block: AnthropicContentBlockStart::ToolUse(AnthropicStartBlock {
                    block_type: "tool_use".to_string(),
                    id: "call-1".to_string(),
                    name: "lookup".to_string(),
                    input: serde_json::Value::Object(Default::default()),
                }),
            },
        ];
        let serialized: Vec<_> = events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect();

        assert_eq!(serialized[0]["type"], "content_block_start");
        assert_eq!(serialized[0]["content_block"]["type"], "thinking");
        assert_eq!(serialized[1]["type"], "content_block_start");
        assert_eq!(serialized[1]["content_block"]["type"], "text");
        assert_eq!(serialized[2]["type"], "content_block_start");
        assert_eq!(serialized[2]["content_block"]["type"], "tool_use");
    }

    #[test]
    fn content_block_delta_serialization() {
        let chunk = StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: "Hello".to_string(),
            thinking: None,
            tool_calls: vec![],
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
            thinking: None,
            tool_calls: vec![],
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
            thinking: None,
            tool_calls: vec![],
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
            thinking: None,
            tool_calls: vec![],
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
            content_blocks: vec![MessageContent::Text("Hello there".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 100,
            cache_hit: None,
        };

        let anthropic_resp = AnthropicMessagesResponse::from(&resp);
        assert_eq!(anthropic_resp.type_, "message");
        assert_eq!(anthropic_resp.role, "assistant");
        assert_eq!(anthropic_resp.stop_reason, "end_turn");
        assert_eq!(anthropic_resp.content.len(), 1);
        assert!(matches!(
            &anthropic_resp.content[0],
            AnthropicContentBlock::Text { text } if text == "Hello there"
        ));
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
        let json =
            r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hello"}]}"#;
        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: rook_core::CompletionRequest = req.into();
        assert_eq!(domain.messages.len(), 1);
        assert_eq!(domain.messages[0].content.as_text(), "hello");
    }

    #[test]
    fn anthropic_tool_use_block_converts_to_neutral_domain_call() {
        let json = r#"{
            "model": "claude-3-5-sonnet",
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_123",
                    "name": "get_weather",
                    "input": {"city": "Paris"}
                }]
            }]
        }"#;

        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: rook_core::CompletionRequest = req.into();

        assert_eq!(domain.messages[0].role, rook_core::Role::Assistant);
        assert_eq!(
            domain.messages[0].content,
            MessageContent::Text(String::new())
        );
        assert_eq!(
            domain.messages[0].tool_calls[0],
            rook_core::MessageToolCall {
                id: Some("toolu_123".to_string()),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "Paris"}),
            }
        );
    }

    #[test]
    fn serializes_domain_tool_content_as_anthropic_blocks() {
        let resp = rook_core::CompletionResponse {
            id: RequestId::new(),
            provider: shared_kernel::ProviderId::new("test"),
            model: ModelId::new("claude-3-5-sonnet"),
            content: String::new(),
            content_blocks: vec![MessageContent::ToolUse {
                id: "toolu_123".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({"city": "Paris"}),
            }],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: make_token_usage(1, 2),
            latency_ms: 1,
            cache_hit: None,
        };

        let anthropic_resp = AnthropicMessagesResponse::from(&resp);
        let json = serde_json::to_value(anthropic_resp).unwrap();

        assert_eq!(json["content"][0]["type"], "tool_use");
        assert_eq!(json["content"][0]["id"], "toolu_123");
        assert_eq!(json["content"][0]["name"], "get_weather");
        assert_eq!(json["stop_reason"], "end_turn");
    }

    #[test]
    fn anthropic_tool_result_block_filters_empty_nested_text() {
        let json = r#"{
            "model": "claude-3-5-sonnet",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_123",
                    "content": [
                        {"type": "text", "text": ""},
                        {"type": "text", "text": "sunny"}
                    ]
                }]
            }]
        }"#;

        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: rook_core::CompletionRequest = req.into();

        assert_eq!(
            domain.messages[0].content,
            MessageContent::ToolResult {
                tool_use_id: "toolu_123".to_string(),
                content: vec![MessageContent::Text("sunny".to_string())],
            }
        );
    }

    // R3 SSE-out tests ----------------------------------------------------

    use rook_core::{ToolCallDelta, ToolCallFunctionDelta};

    fn tool_chunk(
        tools: Vec<ToolCallDelta>,
        finish: Option<FinishReason>,
        usage: Option<TokenUsage>,
    ) -> StreamChunk {
        StreamChunk {
            id: RequestId::new(),
            model: ModelId::new("claude-3-5-sonnet"),
            delta: String::new(),
            thinking: None,
            tool_calls: tools,
            finish_reason: finish,
            usage,
        }
    }

    fn tool_delta(
        index: u32,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) -> ToolCallDelta {
        ToolCallDelta {
            index,
            id: id.map(str::to_string),
            function: ToolCallFunctionDelta {
                name: name.map(str::to_string),
                arguments: args.map(str::to_string),
            },
        }
    }

    #[test]
    fn sse_start_event_for_tool_chunk() {
        let events = AnthropicSseEvent::events_from_chunk(&tool_chunk(
            vec![tool_delta(1, Some("toolu_123"), Some("get_weather"), None)],
            None,
            None,
        ));
        assert_eq!(events.len(), 1);
        let json = serde_json::to_string(&events[0]).unwrap();
        assert!(json.contains(r#""type":"content_block_start""#));
        assert!(json.contains(r#""index":1"#));
        assert!(json.contains(r#""type":"tool_use""#));
        assert!(json.contains(r#""id":"toolu_123""#));
        assert!(json.contains(r#""name":"get_weather""#));
    }

    #[test]
    fn sse_input_json_delta_for_args_chunk() {
        let events = AnthropicSseEvent::events_from_chunk(&tool_chunk(
            vec![tool_delta(1, None, None, Some("{\"city\":"))],
            None,
            None,
        ));
        assert_eq!(events.len(), 1);
        let json = serde_json::to_string(&events[0]).unwrap();
        assert!(json.contains(r#""type":"content_block_delta""#));
        assert!(json.contains(r#""type":"input_json_delta""#));
        assert!(json.contains("partial_json"));
    }

    #[test]
    fn sse_stops_and_tool_use_reason_on_final_chunk() {
        let events = AnthropicSseEvent::events_from_chunk(&tool_chunk(
            vec![
                tool_delta(0, Some("toolu_1"), Some("get_weather"), None),
                tool_delta(1, Some("toolu_2"), Some("get_time"), None),
            ],
            Some(FinishReason::ToolCalls),
            Some(make_token_usage(10, 25)),
        ));
        // 2 stops + message_delta, NO duplicate starts.
        assert_eq!(events.len(), 3);
        let kinds: Vec<String> = events
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["content_block_stop", "content_block_stop", "message_delta"]
        );
        let last = serde_json::to_string(events.last().unwrap()).unwrap();
        assert!(last.contains(r#""stop_reason":"tool_use""#));
    }

    #[test]
    fn sse_stop_reason_mapping() {
        for (finish, expected) in [
            (FinishReason::Stop, "end_turn"),
            (FinishReason::Length, "max_tokens"),
            (FinishReason::ContentFilter, "refusal"),
            (FinishReason::ToolCalls, "tool_use"),
        ] {
            let events = AnthropicSseEvent::events_from_chunk(&tool_chunk(
                vec![],
                Some(finish),
                Some(make_token_usage(1, 1)),
            ));
            assert_eq!(events.len(), 1);
            let json = serde_json::to_string(&events[0]).unwrap();
            assert!(json.contains(&format!(r#""stop_reason":"{expected}""#)));
        }
    }

    #[test]
    fn sse_empty_chunk_emits_nothing() {
        let events = AnthropicSseEvent::events_from_chunk(&tool_chunk(vec![], None, None));
        assert!(events.is_empty());
    }

    #[test]
    fn sse_final_chunk_forwards_combined_text_first() {
        let mut chunk = tool_chunk(
            vec![tool_delta(2, Some("toolu_9"), Some("lookup"), None)],
            Some(FinishReason::ToolCalls),
            Some(make_token_usage(1, 1)),
        );
        chunk.delta = "done reading".to_string();
        let events = AnthropicSseEvent::events_from_chunk(&chunk);
        assert_eq!(events.len(), 3, "text delta, stop, message_delta");
        let first = serde_json::to_string(&events[0]).unwrap();
        assert!(first.contains(r#""type":"content_block_delta""#));
        assert!(first.contains(r#""type":"text_delta""#));
        assert!(first.contains("done reading"));
        let kinds: Vec<String> = events
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(kinds[1], "content_block_stop");
        assert_eq!(kinds[2], "message_delta");
    }

    #[test]
    fn multiple_tool_results_split_into_ordered_messages() {
        let json = r#"{
            "model": "claude-3-5-sonnet",
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "a", "input": {}},
                    {"type": "tool_result", "tool_use_id": "toolu_1",
                     "content": [{"type": "text", "text": "first"}]},
                    {"type": "tool_result", "tool_use_id": "toolu_2",
                     "content": [{"type": "text", "text": "second"}]}
                ]
            }]
        }"#;
        let req: AnthropicMessagesRequest = serde_json::from_str(json).expect("should deserialize");
        let domain: rook_core::CompletionRequest = req.into();
        assert_eq!(domain.messages.len(), 2, "no tool_result may be dropped");
        assert_eq!(domain.messages[0].tool_calls.len(), 1);
        assert_eq!(
            domain.messages[0].content,
            MessageContent::ToolResult {
                tool_use_id: "toolu_1".to_string(),
                content: vec![MessageContent::Text("first".to_string())],
            }
        );
        assert_eq!(
            domain.messages[1].content,
            MessageContent::ToolResult {
                tool_use_id: "toolu_2".to_string(),
                content: vec![MessageContent::Text("second".to_string())],
            }
        );
    }
}
