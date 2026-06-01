# Exploration: Provider Format Translation Layer (Issue #40)

---

## Current State

### Translation pipeline (end-to-end today)

```
Client (OpenAI format)                Client (Anthropic format)
       │                                       │
POST /v1/chat/completions              POST /v1/messages
       │                                       │
OpenAIChatRequest ──From──▶ CompletionRequest ◀──From── AnthropicMessagesRequest
       │                            │
       │                   RouteRequest.execute()
       │                            │
       │                   FallbackRouter.select()
       │                            │
       │              ┌─────────────┴─────────────┐
       │              ▼                            ▼
       │    OpenAIProvider.complete()   AnthropicProvider.complete()
       │    (internal: CompletionRequest → OpenAI wire → OpenAI API → OpenAIResponse → CompletionResponse)
       │    (internal: CompletionRequest → Anthropic wire → Anthropic API → domain types)
       │              │                            │
       │              └────────────┬──────────────┘
       │                           ▼
       │                   CompletionResponse
       │                           │
       ▼                           ▼
OpenAIChatResponse          AnthropicMessagesResponse
(built via From impl)        (built inline in routes.rs)
```

### Key observations

1. **The domain model (`CompletionRequest`) is already the canonical internal format.** Both
   adapters convert to it on ingress, and both providers convert from it on egress to their
   upstream APIs. Clean hexagonal boundary is already in place.

2. **Cross-format routing already works for plain text.** An Anthropic-format client request
   goes through `CompletionRequest`, gets routed to whichever provider supports the model
   (OpenAI or Anthropic), and the response comes back as `CompletionResponse` — then the
   *route handler* renders it back in Anthropic wire format. The provider used is invisible to
   the client. This is correct behavior.

3. **`metadata.origin`** records `"openai"` or `"anthropic"` — this is the only
   "format hint" that survives in the domain model today. It is informational only; nothing
   reads it to drive response format (the route handler already knows its format statically).

4. **Response construction asymmetry.** `CompletionResponse → OpenAIChatResponse` uses a
   `From` impl in `openai_adapter.rs`. `CompletionResponse → AnthropicMessagesResponse` is
   constructed inline inside the `anthropic_messages` handler in `routes.rs`. This is a
   maintenance inconsistency — the Anthropic path should have its own `From` impl.

5. **`SseBuffer` is copy-pasted** between `providers-openai/src/provider.rs` and
   `providers-anthropic/src/lib.rs`. Neither implementation has diverged yet but they will.

---

## Affected Areas

| Path | Why it's affected |
|------|-------------------|
| `crates/domain/rook-core/src/model.rs` | `CompletionRequest` / `CompletionResponse` / `Message` lack tool call, multi-modal content, and system prompt (top-level) fields |
| `crates/infrastructure/transport-axum/src/openai_adapter.rs` | Needs tool call types; `deny_unknown_fields` will reject tool payloads |
| `crates/infrastructure/transport-axum/src/anthropic_adapter.rs` | Same; also missing `From<CompletionResponse>` impl; `deny_unknown_fields` is risky |
| `crates/infrastructure/transport-axum/src/routes.rs` | Inline `AnthropicMessagesResponse` construction should move to adapter |
| `crates/infrastructure/providers-openai/src/provider.rs` | `OpenAIRequest` struct has no `tools` field; duplicates `SseBuffer` |
| `crates/infrastructure/providers-anthropic/src/lib.rs` | `AnthropicStreamRequest` has no `tools` field; duplicates `SseBuffer`; `complete()` returns unimplemented error |

---

## Tool Support

**Zero tool/function call support exists anywhere in the codebase today.**

Specifically:
- `CompletionRequest.messages` is `Vec<Message>` where `Message.content: String` — content
  is a plain string only, no content block variants (text, image, tool_use, tool_result).
- `CompletionRequest` has no `tools: Vec<Tool>` or `tool_choice` field.
- `FinishReason::ToolCalls` exists as an enum variant but it's purely cosmetic — it can be
  produced when parsing a provider response, but the domain model has nowhere to store the
  actual tool call data, and neither adapter serializes tool calls into the response.
- Provider-level internal structs (`OpenAIRequest`, `AnthropicStreamRequest`) also have no
  `tools` field.

**Tool calls are a hard blocker for any client that relies on function calling.**

---

## Format Detection

Client format is detected **statically by route path**, not dynamically by inspecting headers
or body:

- `/v1/chat/completions` → always OpenAI format in, OpenAI format out
- `/v1/messages` → always Anthropic format in, Anthropic format out

There is **no runtime format negotiation**, no `Content-Type` sniffing, and no
`X-Provider-Format` header mechanism. The `metadata.origin` field in `CompletionRequest`
records `"openai"` or `"anthropic"` as a string but nothing downstream reads it for
response formatting — the format is fixed by which handler is executing.

**This means**: there is no "format translation" problem for the response direction — each
handler already knows its output format. The translation challenge is in the *content
fidelity* direction: can the domain model represent everything a client might send?

---

## Gaps for Issue #40 Acceptance Criteria

Based on the issue title and the observed architecture, the likely acceptance criteria are:

| Gap | Severity | Notes |
|-----|----------|-------|
| Tool / function call support in domain model | **Critical** | `Message.content` must become an enum of content blocks; `CompletionRequest` needs `tools` and `tool_choice` |
| Multi-modal content (images, documents) in domain model | High | Both OpenAI and Anthropic support image content blocks; `content: String` is too narrow |
| `deny_unknown_fields` on adapter request types | High | Will reject any field not explicitly modeled (tools, vision, etc.) — must be relaxed or extended |
| `AnthropicMessagesResponse` `From` impl missing | Medium | Inline construction in `routes.rs` is a maintenance risk |
| `SseBuffer` duplication | Low | Should be extracted to a shared crate (e.g., `shared-kernel` or a new `sse-utils` crate) |
| `AnthropicProvider.complete()` unimplemented | Medium | Returns `Err("not yet implemented")` — non-streaming Anthropic requests always fail |
| No `ProviderFormat`/`ClientApiFormat` type | Low | Not needed for basic routing; only needed if format negotiation becomes dynamic |

---

## Recommended Approach

### Where to put the translator

**Do NOT create a new crate for basic format translation.** The existing layering already
handles it correctly:

```
transport-axum  ← format adapters live here (already)
rook-core       ← domain model lives here (needs extension)
providers-*     ← upstream API adapters live here (already)
```

### Recommended work breakdown

**Phase 1 — Extend the domain model (rook-core)**

```rust
// model.rs additions
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: Vec<ContentBlock> },
}

pub struct Message {
    pub role: Role,
    pub content: MessageContent,  // was: pub content: String
}

pub enum MessageContent {
    Text(String),           // backward-compat fast path
    Blocks(Vec<ContentBlock>),
}

pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

pub struct CompletionRequest {
    // ... existing fields ...
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
}

pub enum ToolChoice {
    Auto,
    Any,
    Tool { name: String },
}
```

**Phase 2 — Extend adapters (transport-axum)**

- Remove `deny_unknown_fields` from both adapter request structs, OR explicitly add every
  supported field.
- Add `tools` / `tool_choice` fields to `OpenAIChatRequest` and `AnthropicMessagesRequest`.
- Add `From<CompletionResponse> for AnthropicMessagesResponse` to `anthropic_adapter.rs` and
  remove the inline construction from `routes.rs`.
- Extend `From<&StreamChunk> for AnthropicSseEvent` and `From<&StreamChunk> for OpenAIChatCompletionChunk`
  to handle `ToolCalls` finish reason and tool call delta content.

**Phase 3 — Extend providers (providers-openai, providers-anthropic)**

- Add `tools` and `tool_choice` to `OpenAIRequest` and `AnthropicStreamRequest` internal
  structs.
- Extract `SseBuffer` to a shared location (e.g., a new internal `crate/infrastructure/sse-utils`
  or inline in `shared-kernel` as a feature-gated utility).
- Implement `AnthropicProvider.complete()` (currently returns unimplemented error).

**Phase 4 — Translation helpers (in transport-axum or a new format-translator crate)**

Only if cross-format API compatibility (e.g., Anthropic client → OpenAI `/v1/chat/completions`
endpoint) is an explicit requirement. The current route-per-format model already achieves
this for plain text. If a unified `/v1/ai/chat` endpoint that accepts both formats is wanted,
a lightweight `ClientFormat` enum + middleware layer to normalize the request before hitting
the domain would be the cleanest approach.

### Decision: new crate or not?

| Option | When to choose |
|--------|---------------|
| Extend `transport-axum` adapters | If format adapters only need to handle the request/response boundary — **recommended for now** |
| New `format-translator` crate | Only if format conversion logic becomes complex enough to need independent testing, or if multiple transport crates (HTTP + gRPC) need to share it |

**Start in `transport-axum`. Extract later if needed.**

---

## Risk Areas

1. **`deny_unknown_fields` will break silently in production.** Today, any client sending
   `tools`, `tool_choice`, `stream_options`, or any new OpenAI/Anthropic field will get a
   `422 Unprocessable Entity`. This is likely already causing issues with real clients.
   **Immediate fix: remove `deny_unknown_fields` from both adapter request structs.**

2. **`AnthropicProvider.complete()` is unimplemented.** Non-streaming Anthropic requests
   always fail with `"Anthropic provider not yet implemented"`. This is a hidden gap —
   the stream path works but the non-stream path does not.

3. **`Message.content: String` is the deepest coupling point.** Changing it to an enum
   is a breaking change inside the domain model and will require updating every caller:
   both provider adapters, both transport adapters, all tests, and the `RouteRequest`
   use case. Plan for a migration window.

4. **Cost estimation (`estimated_cost_usd`) is always `None`.** This is a placeholder in
   both providers. Not a blocker for #40 but a visible gap in production observability.

5. **`SseBuffer` duplication creates a divergence risk.** If one copy gets a bug fix
   (e.g., the double-newline scanning logic), the other won't. Extract before extending
   streaming to support tool call deltas.

6. **`metadata.origin` vs. actual client format.** `origin` stores `"openai"` or
   `"anthropic"` as the *request* origin. If a future feature needs to validate that the
   response format matches what the client expects (e.g., for validation tests), this string
   is the only hook — but it is not strongly typed. Consider a `ClientApiFormat` enum in
   `RequestMetadata` if this becomes important.

---

## Ready for Proposal

**Yes, with the following clarifications from the issue owner:**

1. Is tool call support a hard requirement for #40, or is it a follow-up issue?
2. Is multi-modal content (images) in scope?
3. Is a unified endpoint that accepts multiple formats required, or is route-per-format
   sufficient?
4. What is the priority of fixing `AnthropicProvider.complete()` (non-streaming path)?

The core infrastructure (domain model, ports, routing, adapters) is well-structured and
ready to extend. The main work is widening the domain model's `Message` type and threading
that change through both provider adapters and both transport adapters.
