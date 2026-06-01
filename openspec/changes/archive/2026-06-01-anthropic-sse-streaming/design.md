# Design: Anthropic SSE Streaming for `/v1/messages`

## Technical Approach

Implement SSE streaming for the Anthropic `/v1/messages` endpoint by:

1. Adding `AnthropicSseEvent` types to `anthropic_adapter.rs` — SSE event payload shapes (not wire format)
2. Implementing `From<StreamChunk>` to translate domain chunks to Anthropic SSE event payloads
3. Adding `anthropic_messages_stream` handler in `routes.rs` — branches on `stream: true`, maps chunks to SSE, applies SSE headers
4. Implementing `AnthropicProvider::stream()` in `providers-anthropic/src/lib.rs` — parses Anthropic's SSE wire format into `StreamChunk`

Pattern: mirror the existing OpenAI `chat_completions_stream` implementation.

## Architecture Decisions

### Decision: SSE Event Payload vs. Wire Format in Adapter

**Choice**: `AnthropicSseEvent` represents the SSE **data payload** (the JSON inside `data:`), not the SSE wire format itself. The adapter translates `StreamChunk` → `AnthropicSseEvent`; `routes.rs` serializes it to JSON and wraps in `Event::default().data(...)`.

**Alternatives considered**: Embed SSE `data:` prefix and `\n\n` terminator in adapter. Rejected — SSE framing is HTTP-level concerns handled by `Sse::new` and `apply_sse_headers`.

**Rationale**: Follows the OpenAI adapter pattern where `OpenAIChatCompletionChunk` is the JSON data payload. SSE framing is concern of the transport layer (`routes.rs`).

### Decision: Reuse `apply_sse_headers` from OpenAI Path

**Choice**: Use the same `apply_sse_headers` function for both OpenAI and Anthropic streaming.

**Alternatives considered**: Copy `apply_sse_headers` or create a shared utility module. Copy rejected — identical headers; shared utility adds indirection for identical code.

**Rationale**: Headers (`Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`) apply to all SSE responses. One function avoids drift.

### Decision: Error Events via `openai_error_event` Pattern

**Choice**: On provider/stream error, yield an error SSE event, then return. Do not return HTTP 5xx for streaming errors.

**Alternatives considered**: Return HTTP error and close connection. Rejected — SSE clients expect a final error event, not a broken connection. Matches OpenAI behavior.

**Rationale**: SSE streaming convention: error events with `type: error` or provider-shaped error body. Client libraries (Anthropic SDK, Claude Code) handle error events gracefully.

### Decision: `stream()` Returns `BoxStream` via `Box::pin`

**Choice**: `AnthropicProvider::stream()` returns `BoxStream<'static, CortexResult<StreamChunk>>` — `Box::pin(futures::stream::once(async { ... }))` for the stub, real impl will use `reqwest` `Response` bytes stream.

**Alternatives considered**: `impl Stream<Item = ...>` return type. Rejected — cannot return from async fn without boxing in this codebase's MSRV and Rust edition.

**Rationale**: Matches `ProviderPort::stream` signature and existing patterns in the codebase.

## Data Flow

```
HTTP Request (stream: true)
  → anthropic_messages_handler (routes.rs)
      → CompletionRequest::from(body)
      → usecases.route_request.execute_stream(req)
          → route_request.rs (fallback/failover logic)
              → AnthropicProvider::stream()
                  → reqwest POST to /v1/messages with stream: true
                  → SSE bytes → parse → StreamChunk
          ← BoxStream<Result<StreamChunk, CortexError>>
      → stream.map(chunk → AnthropicSseEvent → Event::default().data(json))
      → Sse::new(events).into_response()
  ← HTTP 200 + text/event-stream
```

### SSE Event Sequence

```
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The"}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" capital"}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" of"}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" France"}}
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}
```

Note: `content_block_delta` repeats per token (accumulated by client). `message_delta` is the final event with `stop_reason` and `usage`. No explicit `[DONE]` — the stream terminates after `message_delta`.

## File Changes

| File                                                            | Action | Description                                                                                           |
|-----------------------------------------------------------------|--------|-------------------------------------------------------------------------------------------------------|
| `crates/infrastructure/transport-axum/src/anthropic_adapter.rs` | Modify | Add `AnthropicSseEvent` enum, `AnthropicStreamChunk`, `AnthropicErrorEvent`, `From<StreamChunk>` impl |
| `crates/infrastructure/transport-axum/src/routes.rs`            | Modify | Add `anthropic_messages_stream` handler; branch in `anthropic_messages` on `stream == Some(true)`     |
| `crates/infrastructure/providers-anthropic/src/lib.rs`          | Modify | Implement `AnthropicProvider::stream()` — parse Anthropic SSE wire format into `StreamChunk`          |

## Interfaces / Contracts

### New Types in `anthropic_adapter.rs`

```rust
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
    pub delta_type: String, // always "text_delta"
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
        // Non-final chunks: content_block_delta
        // Final chunk (finish_reason is Some): message_delta with stop_reason and usage
        todo!("implement mapping")
    }
}
```

### Routes Handler Signature

```rust
async fn anthropic_messages_stream(
    State(usecases): State<Usecases>,
    Json(body): Json<AnthropicMessagesRequest>,
) -> Result<Response, HttpError> {
    // mirrors chat_completions_stream pattern
}
```

### Provider Streaming Signature

```rust
async fn stream(
    &self,
    req: &CompletionRequest,
) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>>
```

## Testing Strategy

| Layer       | What to Test                                | Approach                                                                                                        |
|-------------|---------------------------------------------|-----------------------------------------------------------------------------------------------------------------|
| Unit        | `From<StreamChunk>` for `AnthropicSseEvent` | Test text chunk → `content_block_delta`, final chunk → `message_delta` with correct usage                       |
| Unit        | `AnthropicProvider::stream()` error path    | Stub returns error → verify `CortexError::provider(...)` propagates                                             |
| Integration | SSE format end-to-end                       | `POST /v1/messages` with `stream: true` → capture SSE, verify each line is valid JSON with correct `type` field |
| Integration | Non-streaming path not regressed            | Existing `POST /v1/messages` with `stream: false` → verify JSON response                                        |

## Migration / Rollout

No migration required. This is a pure additive change:

- Non-streaming `POST /v1/messages` unaffected
- Streaming returns 200 with SSE instead of 500 (current stub behavior)

Rollout: implement in order (adapter → routes → provider), test incrementally.

## Open Questions

- **Anthropic SSE `message_delta` `usage.input_tokens`**: Should we populate `input_tokens` from the first chunk's usage, or only emit `output_tokens` (as in the non-streaming response)? Based on proposal, only `output_tokens` is mentioned — but the non-streaming response includes both. Confirm: does the streaming API require `input_tokens` in `message_delta` usage?
- **`stop_sequence` in `message_delta`**: Non-streaming response has `stop_sequence: null`. Should streaming include the actual stop sequence number or omit it?
