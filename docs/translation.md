# Format Translation Layer

Rook translates LLM requests and responses between provider wire formats using a **domain model as the translation pivot**. This document describes the architecture, data flow, current state, and known gaps.

---

## Design Philosophy

```
Client wire format  →  Domain model  →  Provider wire format
  (OpenAI body)         (neutral)          (Anthropic body)
```

Instead of translating wire-to-wire (e.g., OpenAI JSON → Anthropic JSON directly), every format goes through the domain model first. This means:

- **N formats → 2×N conversions** (one `from_wire` + one `to_wire` per format)
- vs. **N×(N-1) conversions** for direct wire-to-wire mapping
- The domain model is the single source of truth — no implicit knowledge of "what OpenAI calls X" inside an Anthropic adapter

---

## Layer Responsibilities

```
┌──────────────────────────────────────────────────────────┐
│  transport-axum (openai_adapter.rs / anthropic_adapter.rs) │
│  Wire format ↔ Domain model conversions                  │
│  Format detection via route path                         │
└────────────────────┬─────────────────────────────────────┘
                     │  CompletionRequest / CompletionResponse
┌────────────────────▼─────────────────────────────────────┐
│  rook-core (model.rs)                                    │
│  Neutral domain types — no wire format knowledge         │
└────────────────────┬─────────────────────────────────────┘
                     │  CompletionRequest (provider-specific wire)
┌────────────────────▼─────────────────────────────────────┐
│  providers-openai / providers-anthropic / …              │
│  Serialize domain → provider-native HTTP body            │
│  Deserialize provider HTTP response → domain             │
└──────────────────────────────────────────────────────────┘
```

---

## Domain Model

Defined in `crates/domain/rook-core/src/model.rs`.

```rust
pub struct CompletionRequest {
    pub id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub metadata: RequestMetadata,
}

pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

pub enum Role {
    System,
    Developer,   // OpenAI Responses API; treated as system in Anthropic
    User,
    Assistant,
}

pub enum MessageContent {
    Text(String),
    // Phase 2:
    // ToolUse { id: String, name: String, input: serde_json::Value },
    // ToolResult { tool_use_id: String, content: Vec<MessageContent> },
}
```

**Phase 1 scope:** `MessageContent::Text` only. Tool calls, multimodal images, and thinking blocks are deferred to Phase 2.

---

## Adapters

### OpenAI Adapter (`openai_adapter.rs`)

Handles the `/v1/chat/completions` endpoint.

**Inbound (OpenAI wire → domain):**

| OpenAI field                                                | Domain field           | Notes                                                                                                                                         |
|-------------------------------------------------------------|------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------|
| `model`                                                     | `ModelId`              | Wrapped newtype                                                                                                                               |
| `messages[].role`                                           | `Role`                 | `"developer"` → `Role::Developer`                                                                                                             |
| `messages[].content`                                        | `MessageContent::Text` | `serde_json::Value` — handles both `"string"` and `[{type:"text"}]` array forms via `into_text()`. Non-text parts silently skipped (Phase 2). |
| `stream`                                                    | `stream: bool`         | Defaults to `false`                                                                                                                           |
| `max_tokens`, `temperature`                                 | Direct                 | Optional passthrough                                                                                                                          |
| `tools`, `tool_choice`, `response_format`, `stream_options` | Accepted, not routed   | Forward-compat; no `deny_unknown_fields` to avoid silent 422                                                                                  |

**Outbound (domain → OpenAI wire):**

```json
{
  "id": "rook-{uuid}",
  "object": "chat.completion",
  "choices": [
    {
      "message": {
        "role": "assistant",
        "content": "…"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 1232414,
    "completion_tokens": 12441241,
    "total_tokens": 98787
  }
}
```

**Streaming (SSE):** Each `StreamChunk` from the domain emits one `data: {…}` line with a `choices[0].delta.content` field, followed by `data: [DONE]`.

---

### Anthropic Adapter (`anthropic_adapter.rs`)

Handles the `/v1/messages` endpoint.

**Inbound (Anthropic wire → domain):**

| Anthropic field      | Domain field                     | Notes                                                       |
|----------------------|----------------------------------|-------------------------------------------------------------|
| `model`              | `ModelId`                        | Wrapped newtype                                             |
| `messages[].role`    | `Role`                           | `"user"` / `"assistant"` only (system is a top-level field) |
| `messages[].content` | `MessageContent::Text`           | Accepts string or content block array; extracts text blocks |
| `system`             | `Role::System` message prepended | Inserted as first message in domain `messages` vec          |
| `max_tokens`         | Direct                           | Required by Anthropic spec                                  |
| `stream`             | `stream: bool`                   | Defaults to `false`                                         |

**Outbound (domain → Anthropic wire):**

```json
{
  "id": "rook-{uuid}",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "…"
    }
  ],
  "model": "claude-opus-4-5",
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 1232414,
    "output_tokens": 12441241
  }
}
```

**System message handling:** When building an Anthropic `AnthropicStreamRequest` in `providers-anthropic`, `Role::System` and `Role::Developer` messages are extracted from the messages vec and promoted to the top-level `system: Option<String>` field. They are filtered out of the `messages` array before sending to the Anthropic API.

---

## Provider Translation (`providers-anthropic`)

When the selected provider is Anthropic, the domain `CompletionRequest` is converted to the Anthropic Messages API body:

```rust
// providers-anthropic/src/lib.rs
AnthropicStreamRequest {
model: req.model.as_str().to_string(),
max_tokens: req.max_tokens.or(Some(4096)),   // default if not set
stream: true,
messages: /* domain messages → Anthropic blocks */,
system: /* extracted Role::System / Role::Developer */,
}
```

**Key behaviors:**

- `max_tokens` defaults to `4096` if not provided (mirrors `complete()` path)
- System/Developer roles are extracted into the top-level `system` field — Anthropic does not accept them inline in `messages`
- Truncation uses `chars().take(N)` (not byte slices) to avoid panicking on multibyte UTF-8

---

## Format Detection

Route-based detection in `transport-axum/src/routes.rs`:

| Path                   | Detected format | Adapter used        |
|------------------------|-----------------|---------------------|
| `/v1/chat/completions` | OpenAI          | `openai_adapter`    |
| `/v1/messages`         | Anthropic       | `anthropic_adapter` |

The `ApiFormat` enum and `FormatRegistry` scaffold exist in `format_registry.rs` but are not yet wired into runtime routing (Phase 2).

---

## Cross-Format Routing (Current State)

A client using the OpenAI format can already be routed to an Anthropic provider — this works today for plain text because:

1. OpenAI wire → domain (`openai_adapter`)
2. Domain → Anthropic provider wire (`providers-anthropic`) ← independent of step 1
3. Anthropic response → domain
4. Domain → OpenAI wire response (`openai_adapter`) ← matches client format

The translation happens implicitly through the domain pivot. No explicit "translate OpenAI to Anthropic" step exists — each adapter only knows its own wire format.

---

## Known Gaps (Phase 2)

### Issue #61 — Tool calls (`tool_use` / `tool_result`)

`MessageContent` only has a `Text` variant. Tool call flows are accepted at the HTTP layer (`tools` / `tool_choice` fields on `OpenAIChatRequest`) but not propagated to providers or translated in responses.

**What's missing:**

- `MessageContent::ToolUse { id, name, input }` and `MessageContent::ToolResult { tool_use_id, content }` variants
- Serialization of `tool_use` blocks into Anthropic Messages API format
- Deserialization of `tool_use` stop reason and content blocks back to domain
- OpenAI `tool_calls` array round-trip

See [issue #61](https://github.com/dallay/cortex/issues/61).

### Issue #62 — `SseBuffer` for stateful streaming

SSE streaming currently parses each `data:` chunk independently. Multi-block responses (text + tool_use, thinking + text), correct `content_block_start`/`content_block_stop` sequencing, and tool argument accumulation across partial chunks all require a stateful buffer.

**What's missing:**

- `SseBuffer` struct tracking `message_id`, `next_block_index`, per-block state, tool call buffers
- Anthropic SSE event sequencing (`message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_delta` → `message_stop`)
- Proper `[DONE]` / `message_stop` termination with accumulated usage

See [issue #62](https://github.com/dallay/cortex/issues/62).

### Issue #63 — `FormatRegistry::register()`

The `FormatRegistry` is a scaffold with no registered translators. Cross-format routing is implicit and works only because the domain model is the pivot. Making it explicit allows:

- Runtime validation that a translator exists for a given (client_format, provider_format) pair
- Pluggable translator registration at boot in `di.rs`
- Future support for additional formats (Gemini, Ollama, Groq) without modifying route handlers

See [issue #63](https://github.com/dallay/cortex/issues/63).

---

## Comparison with Direct Wire-to-Wire Translation

For reference, `tmp/OmniRoute/open-sse/translator/` uses a **wire-to-wire** approach:

| Aspect               | Rook (domain pivot)                              | OmniRoute (wire-to-wire)                         |
|----------------------|--------------------------------------------------|--------------------------------------------------|
| Translation path     | `wire → domain → wire`                           | `wire → wire`                                    |
| N formats cost       | `2×N` adapters                                   | `N×(N-1)` translator pairs                       |
| Format isolation     | Each adapter knows only its own format           | Each translator knows two formats                |
| Edge case handling   | Phase 1: minimal; Phase 2: in-domain             | All edge cases in per-pair files (~500 LOC/pair) |
| Type safety          | Rust types enforced at compile time              | TypeScript, runtime validation                   |
| Registry key         | `(ApiFormat, ApiFormat)` → `Box<dyn Translator>` | `"from:to"` → `Fn` in a `Map`                    |
| Tool prefix strategy | N/A yet (Phase 2)                                | `proxy_` prefix for Claude OAuth                 |
| Streaming state      | `SseBuffer` (Phase 2)                            | `state` object passed per chunk                  |

The domain-pivot approach reduces translator count and enforces a clean boundary: wire-format knowledge never leaks into the domain. The tradeoff is that Phase 2 work is required to reach feature parity on tool calls and streaming state.
