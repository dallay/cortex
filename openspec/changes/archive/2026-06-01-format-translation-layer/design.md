# Design: format-translation-layer

## Architecture overview

The domain model (`CompletionRequest` / `CompletionResponse`) is already the canonical
translation pivot. Every wire format maps **to** domain types on ingress and **from** domain
types on egress — no provider ever speaks directly to another. The gaps are:

- `Message.content` is a `String` (can't represent tool call blocks)
- `deny_unknown_fields` silently rejects `tools`, `tool_choice`, `system`, `stream_options`
- `AnthropicProvider.complete()` panics on every non-streaming call
- `SseBuffer` is copy-pasted with identical logic in two provider crates
- Provider HTTP errors are raw-wrapped strings, not typed `CortexError` variants
- `AnthropicMessagesResponse` is built inline in `routes.rs` instead of via a `From` impl

```
Client (OpenAI)          Client (Anthropic)
      │                         │
POST /v1/chat/completions   POST /v1/messages
      │                         │
 openai_adapter.rs          anthropic_adapter.rs
 OpenAIChatRequest ──From──▶ CompletionRequest ◀──From── AnthropicMessagesRequest
      │                         │                              │
      │                 RouteRequest.execute[_stream]()        │
      │                         │                              │
      │            ┌────────────┴───────────┐                  │
      │            ▼                        ▼                  │
      │   OpenAIProvider            AnthropicProvider          │
      │   .complete() / .stream()   .complete() / .stream()   │
      │            │                        │                  │
      │            └────────────┬───────────┘                  │
      │                         ▼                              │
      │                 CompletionResponse                      │
      │                         │                              │
      ▼                         ▼                              ▼
OpenAIChatResponse      From<&CompletionResponse>    AnthropicMessagesResponse
(From impl, exists)     (both From impls after fix)   (From impl — Phase 1 fix)
```

---

## Component changes

### rook-core: MessageContent enum

**File**: `crates/domain/rook-core/src/model.rs`

Replace `pub content: String` in `Message` with an enum. Phase 1 ships only `Text`; Phase 2
adds tool call blocks.

```rust
/// Phase 1 only. Phase 2 adds Blocks(Vec<ContentBlock>).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
}

impl MessageContent {
    /// Borrow the text content. Panics only if a future variant is passed to
    /// a Phase-1-only code path — use pattern matching in Phase 2 callers.
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(s) => s.as_str(),
        }
    }

    pub fn into_text(self) -> String {
        match self {
            Self::Text(s) => s,
        }
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self { Self::Text(s) }
}

pub struct Message {
    pub role: Role,
    pub content: MessageContent,   // was: String
}
```

**Migration impact** — every `m.content` used as `&str` / `String` becomes `m.content.as_text()` or `m.content.into_text()`:

| File                                                    | Current              | After                             |
|---------------------------------------------------------|----------------------|-----------------------------------|
| `providers-anthropic/src/lib.rs:244`                    | `m.content.clone()`  | `m.content.as_text().to_string()` |
| `providers-openai/src/provider.rs` (build request body) | `m.content.clone()`  | `m.content.as_text().to_string()` |
| `transport-axum/src/openai_adapter.rs` From impl        | `content: m.content` | `content: m.content.into_text()`  |
| `transport-axum/src/anthropic_adapter.rs` From impl     | `content: m.content` | `content: m.content.into_text()`  |
| `rook-usecases` (any `message.content` access)          | direct string        | `.as_text()`                      |

Phase 2 adds:

```rust
pub enum ContentBlock {
    Text { text: String },
    ToolUse  { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String },
}

// MessageContent gains:
//   Blocks(Vec<ContentBlock>)

pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,   // JSON Schema object
}

pub enum ToolChoice { Auto, Any, Tool { name: String } }

// CompletionRequest gains:
//   pub tools: Option<Vec<Tool>>,
//   pub tool_choice: Option<ToolChoice>,
```

---

### transport-axum: adapter updates

**Files**: `openai_adapter.rs`, `anthropic_adapter.rs`

#### 1. Remove `deny_unknown_fields` and add forward-compat fields

`deny_unknown_fields` is the root cause of silent `422` rejections for any client sending
`tools`, `tool_choice`, `stream_options`, or any new OpenAI/Anthropic field. Remove it from
both structs. Add the known Phase-2 fields as `Option<serde_json::Value>` placeholders so
they are accepted and round-tripped to the domain model when Phase 2 lands.

**`OpenAIChatRequest`** — remove attribute, add fields:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]   // deny_unknown_fields REMOVED
pub struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub stream: Option<bool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub n: Option<u32>,
    // Phase-2 forward compat — accepted but not yet translated:
    pub tools: Option<Vec<serde_json::Value>>,
    pub tool_choice: Option<serde_json::Value>,
    pub stream_options: Option<serde_json::Value>,
    pub response_format: Option<serde_json::Value>,
}
```

**`AnthropicMessagesRequest`** — remove attribute, add fields:

```rust
#[derive(Debug, Deserialize)]          // deny_unknown_fields REMOVED
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub stream: Option<bool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    // Anthropic top-level system prompt (Anthropic API accepts this separately):
    pub system: Option<String>,
    // Phase-2 forward compat:
    pub tools: Option<Vec<serde_json::Value>>,
    pub tool_choice: Option<serde_json::Value>,
}
```

The `system` field fix: when present, prepend a `Role::System` message to the messages list
inside the `From<AnthropicMessagesRequest>` impl (Anthropic allows system at top level *or*
as a system-role message; the domain model already supports `Role::System`).

#### 2. Add `From<&CompletionResponse> for AnthropicMessagesResponse`

Currently the Anthropic response is constructed inline in `routes.rs:292-302`. Move this to
`anthropic_adapter.rs`:

```rust
impl From<&CompletionResponse> for AnthropicMessagesResponse {
    fn from(resp: &CompletionResponse) -> Self {
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
```

Then `routes.rs` becomes:

```rust
Ok(resp) => Ok(Json(AnthropicMessagesResponse::from(&resp)).into_response()),
```

---

### transport-axum: FormatRegistry

**File**: `crates/infrastructure/transport-axum/src/format_registry.rs` *(new)*

The current route-per-path design already handles format selection statically (the route
handler knows its own format). The `FormatRegistry` is an **enumeration layer** that makes
the mapping explicit and extensible without further logic today.

```rust
/// The wire format a client is speaking (inbound format, detected by route path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenAI,
    Anthropic,
    // Phase 3: Gemini, Ollama, Groq
}

/// Maps a provider kind string (from ProviderConnection config) to the upstream
/// wire format that provider expects. Used when providers-* crates need to know
/// how to serialize outbound requests — currently implicit, made explicit here.
#[derive(Debug, Default)]
pub struct FormatRegistry {
    map: std::collections::HashMap<String, ApiFormat>,
}

impl FormatRegistry {
    pub fn with_defaults() -> Self {
        let mut map = std::collections::HashMap::new();
        map.insert("openai".to_string(),    ApiFormat::OpenAI);
        map.insert("anthropic".to_string(), ApiFormat::Anthropic);
        Self { map }
    }

    pub fn format_for(&self, provider_kind: &str) -> Option<ApiFormat> {
        self.map.get(provider_kind).copied()
    }
}
```

**Construction**: `FormatRegistry::with_defaults()` is called in `di.rs` (DI bootstrap) and
passed to the router as an `Arc<FormatRegistry>` state extension. No routing logic reads it
in Phase 1; it exists to unblock Phase 3 provider additions without another structural change.

---

### providers-anthropic: implement complete()

**File**: `crates/infrastructure/providers-anthropic/src/lib.rs`

Add a `AnthropicNonStreamResponse` deserialization struct and implement the non-streaming
HTTP call. The Anthropic Messages API returns identical JSON whether streaming or not, with
the response body being the complete message object.

```rust
#[derive(Debug, Deserialize)]
struct AnthropicNonStreamResponse {
    id: String,
    model: String,
    content: Vec<AnthropicResponseContentBlock>,
    usage: AnthropicNonStreamUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    // Phase 2: tool_use
}

#[derive(Debug, Deserialize)]
struct AnthropicNonStreamUsage {
    input_tokens: u32,
    output_tokens: u32,
}
```

Implementation:

```rust
async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
    let start = std::time::Instant::now();
    let body = AnthropicStreamRequest {
        model: req.model.to_string(),
        messages: /* same as stream path */,
        stream: false,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
    };

    let resp = self.client
        .post(format!("{}/v1/messages", self.config.base_url))
        .header("x-api-key", &self.config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CortexError::provider(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(map_anthropic_http_error(status, &body));
    }

    let parsed: AnthropicNonStreamResponse = resp
        .json()
        .await
        .map_err(|e| CortexError::provider(e.to_string()))?;

    let text = parsed.content.into_iter()
        .filter_map(|b| match b { AnthropicResponseContentBlock::Text { text } => Some(text) })
        .collect::<Vec<_>>()
        .join("");

    Ok(CompletionResponse {
        id: req.id.clone(),
        provider: self.config.id.clone(),
        model: ModelId::new(parsed.model),
        content: text,
        usage: TokenUsage {
            prompt_tokens: parsed.usage.input_tokens,
            completion_tokens: parsed.usage.output_tokens,
            total_tokens: parsed.usage.input_tokens + parsed.usage.output_tokens,
            estimated_cost_usd: None,
        },
        latency_ms: start.elapsed().as_millis() as u64,
    })
}
```

The `AnthropicStreamRequest` struct is renamed `AnthropicRequest` (same fields, `stream`
controls the mode) to serve both paths.

---

### Error normalization

**Design**: Add a free function `map_anthropic_http_error(status, body) -> CortexError` in
`providers-anthropic/src/lib.rs`, and a matching `map_openai_http_error(status, body) ->
CortexError` in `providers-openai/src/provider.rs`. Both replace the current raw
`format!("{status}: {body}")` string-wrapping.

| HTTP Status     | Provider context | → CortexError variant                                |
|-----------------|------------------|------------------------------------------------------|
| 400             | Any              | `CortexError::invalid_request(body)`                 |
| 401 / 403       | Any              | `CortexError::auth_failed("invalid api key")`        |
| 404             | Any              | `CortexError::invalid_request("model not found")`    |
| 429             | Any              | `CortexError::rate_limited(retry_after_from_header)` |
| 408 / 504       | Any              | `CortexError::timeout()`                             |
| 500 / 502 / 503 | Any              | `CortexError::provider(sanitized_body)`              |
| Other 4xx       | Any              | `CortexError::invalid_request(sanitized_body)`       |
| Other 5xx       | Any              | `CortexError::provider(sanitized_body)`              |

The `Retry-After` header (when present on 429) is parsed to feed `CortexError::rate_limited`
with a concrete retry delay. The existing `sanitize_error_body()` helper in
`providers-openai/src/provider.rs` is reused for both providers (candidate for extraction
alongside `SseBuffer` in Phase 2).

---

## Data flow diagrams

### Before (Phase 0 — current state)

```
OpenAI client → POST /v1/chat/completions
  │
  OpenAIChatRequest (deny_unknown_fields BLOCKS tools/tool_choice)
  │ From
  CompletionRequest { messages: Vec<Message { content: String }> }
  │
  AnthropicProvider.complete() → Err("not yet implemented")   ← BROKEN
  │
  CompletionResponse { content: String }
  │ inline construction in routes.rs
  AnthropicMessagesResponse
```

### After (Phase 1 — this change)

```
OpenAI client → POST /v1/chat/completions
  │
  OpenAIChatRequest (no deny_unknown_fields; tools/tool_choice accepted as Value)
  │ From
  CompletionRequest { messages: Vec<Message { content: MessageContent::Text(..) }> }
  │
  AnthropicProvider.complete() → Ok(CompletionResponse)       ← FIXED
  │
  CompletionResponse
  │ From impl (moved out of routes.rs)
  AnthropicMessagesResponse
```

---

## Phase boundaries

| Item                                                    | Phase 1 | Phase 2           |
|---------------------------------------------------------|---------|-------------------|
| `MessageContent::Text` enum variant                     | ✅       | —                 |
| `MessageContent::Blocks` + `ContentBlock`               | —       | ✅                 |
| `CompletionRequest.tools` / `tool_choice` (typed)       | —       | ✅                 |
| `deny_unknown_fields` removal                           | ✅       | —                 |
| `tools`/`tool_choice` as `Option<Value>` forward-compat | ✅       | —                 |
| `tools`/`tool_choice` as typed structs                  | —       | ✅                 |
| `AnthropicMessagesResponse` From impl                   | ✅       | —                 |
| `AnthropicProvider.complete()` implemented              | ✅       | —                 |
| `FormatRegistry` struct + defaults                      | ✅       | —                 |
| Error normalization (`map_*_http_error`)                | ✅       | —                 |
| `SseBuffer` extraction to shared location               | —       | ✅ (shared-kernel) |
| OpenAI `tool_calls` ↔ Anthropic `tool_use` translation  | —       | ✅                 |
| Streaming tool call delta reassembly                    | —       | ✅                 |

---

## Test strategy

| Layer                                  | Scope                        | Approach                                                                                   |
|----------------------------------------|------------------------------|--------------------------------------------------------------------------------------------|
| Unit: `MessageContent`                 | `rook-core`                  | `as_text()`, `From<String>`, serde round-trip                                              |
| Unit: `openai_adapter` From impls      | `transport-axum`             | Deserialize request with `tools` field — must not 422                                      |
| Unit: `anthropic_adapter` From impls   | `transport-axum`             | Deserialize request with `system` top-level field; verify prepended message                |
| Unit: `AnthropicMessagesResponse` From | `transport-axum`             | `CompletionResponse` → `AnthropicMessagesResponse` JSON matches spec                       |
| Unit: `map_*_http_error`               | each provider crate          | 429 → rate_limited with retry_after; 401 → auth_failed                                     |
| Unit: `FormatRegistry`                 | `transport-axum`             | `format_for("openai")` = OpenAI; unknown key = None                                        |
| Integration: non-stream Anthropic      | `providers-anthropic` tests  | Mock HTTP server returns `AnthropicNonStreamResponse`; verify `CompletionResponse.content` |
| Integration: cross-format text route   | `transport-axum` integration | POST `/v1/chat/completions` → routed to Anthropic provider → OpenAI response shape         |
| Golden file: OpenAI wire → domain      | `transport-axum`             | Snapshot `CompletionRequest` from real OpenAI payloads including `tools`                   |
| Golden file: Anthropic wire → domain   | `transport-axum`             | Snapshot `CompletionRequest` from real Anthropic payloads including `system`               |

The mock HTTP server approach (using `wiremock` or `httpmock`) avoids live API calls in CI
and matches the existing test infrastructure pattern used in `providers-openai`.
