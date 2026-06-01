# Spec: format-translation-layer

> **Archived from change**: `format-translation-layer`  
> **Archived on**: 2026-06-01  
> **PR**: https://github.com/dallay/cortex/pull/59  
> **Status**: Phase 1 complete (290 tests passing, clippy clean)

---

## Functional Requirements

| ID | Title | Phase |
|----|-------|-------|
| FR-1 | `deny_unknown_fields` removed from both adapter request structs | Phase 1 ✅ |
| FR-2 | OpenAI → domain request translation preserves all text fields | Phase 1 ✅ |
| FR-3 | Anthropic → domain request translation preserves all text fields | Phase 1 ✅ |
| FR-4 | Domain → OpenAI response translation (non-streaming) | Phase 1 ✅ |
| FR-5 | Domain → Anthropic response translation (non-streaming) via `From` impl | Phase 1 ✅ |
| FR-6 | Domain → OpenAI SSE chunk translation (streaming) | Phase 1 ✅ |
| FR-7 | Domain → Anthropic SSE event translation (streaming) | Phase 1 ✅ |
| FR-8 | `system` role normalized to `user` for providers that reject system role | Phase 1 ✅ |
| FR-9 | `AnthropicProvider::complete()` returns a valid non-streaming response | Phase 1 ✅ |
| FR-10 | Missing `max_tokens` handled with provider-appropriate defaults | Phase 1 ✅ |
| FR-11 | All provider errors surface as `CortexError` variants — no raw provider error leaks | Phase 1 ✅ |
| FR-12 | `SseBuffer` extracted to a single shared module within `transport-axum` | Phase 2 🔲 |
| FR-13 | `FormatRegistry` maps `ProviderKind` → format adapter; extensible at DI bootstrap | Phase 1 ✅ |
| FR-14 | Tool call translation: OpenAI `tools`/`tool_choice` ↔ Anthropic `tool_use` | Phase 2 🔲 |

---

## Scenarios

### SC-01: `deny_unknown_fields` removal — unknown fields are tolerated

**Given** an OpenAI-format request body that includes `tools`, `tool_choice`, or any unrecognized field  
**When** the request is deserialized into `OpenAIChatRequest`  
**Then** deserialization succeeds (no `422 Unprocessable Entity`)  
**And** the known fields are mapped to `CompletionRequest` as normal  
**And** the unrecognized fields are silently ignored

### SC-02: `deny_unknown_fields` removal — Anthropic ingress

**Given** an Anthropic-format request body that includes `system`, `tools`, `stream_options`, or any unrecognized field  
**When** the request is deserialized into `AnthropicMessagesRequest`  
**Then** deserialization succeeds  
**And** known fields are mapped to `CompletionRequest` correctly

### SC-03: `deny_unknown_fields` regression — valid minimal request still works

**Given** a minimal OpenAI request `{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}`  
**When** deserialized after removing `deny_unknown_fields`  
**Then** `CompletionRequest` is produced with correct model, role, and content  
**And** `stream` defaults to `false`

### SC-04: OpenAI → domain request translation (text, non-streaming)

**Given** an `OpenAIChatRequest` with model, messages, `max_tokens`, and `temperature`  
**When** `From<OpenAIChatRequest>` is called  
**Then** `CompletionRequest.model` matches the input model string  
**And** each message role (`system`, `user`, `assistant`, `developer`) maps to the corresponding `Role` variant  
**And** `metadata.origin` is `"openai"`  
**And** `stream` is `false`

### SC-05: Anthropic → domain request translation (text, non-streaming)

**Given** an `AnthropicMessagesRequest` with model, messages, `max_tokens`, and `temperature`  
**When** `From<AnthropicMessagesRequest>` is called  
**Then** `CompletionRequest` fields mirror the input  
**And** `metadata.origin` is `"anthropic"`  
**And** `stream` is `false`

### SC-06: OpenAI streaming request translation

**Given** an `OpenAIChatRequest` with `stream: true`  
**When** converted to `CompletionRequest`  
**Then** `CompletionRequest.stream` is `true`

> ⚠ **Note**: No dedicated unit test as of Phase 1. Add in Phase 2 cleanup.

### SC-07: Missing `max_tokens` — OpenAI target default

**Given** an `OpenAIChatRequest` with no `max_tokens` field  
**When** converted to `CompletionRequest` and routed to an OpenAI provider  
**Then** the provider uses an OpenAI-appropriate default (not `None` passed as-is when the upstream API requires it)  
**And** the request does not fail with a missing-parameter error

> ⚠ **Note**: No dedicated unit test as of Phase 1. Add in Phase 2 cleanup.

### SC-08: Missing `max_tokens` — Anthropic target default

**Given** an `AnthropicMessagesRequest` with no `max_tokens` field  
**When** converted to `CompletionRequest` and routed to an Anthropic provider  
**Then** the provider supplies Anthropic's required `max_tokens` default (e.g. 1024)  
**And** the upstream API call succeeds

### SC-09: Domain → OpenAI response (non-streaming)

**Given** a `CompletionResponse` with content, usage, model, and id  
**When** `From<&CompletionResponse>` for `OpenAIChatResponse` is called  
**Then** `choices[0].message.content` equals `CompletionResponse.content`  
**And** `choices[0].finish_reason` is `"stop"`  
**And** `usage.prompt_tokens`, `usage.completion_tokens`, and `usage.total_tokens` match  
**And** `object` is `"chat.completion"`

### SC-10: Domain → Anthropic response (non-streaming) via `From` impl

**Given** a `CompletionResponse`  
**When** `From<&CompletionResponse>` for `AnthropicMessagesResponse` is called  
**Then** `content[0].type` is `"text"` and `content[0].text` equals `CompletionResponse.content`  
**And** `stop_reason` is `"end_turn"`  
**And** `usage.input_tokens` equals `prompt_tokens` and `usage.output_tokens` equals `completion_tokens`  
**And** the response is NOT constructed inline in `routes.rs` (construction is in `anthropic_adapter.rs`)

### SC-11: `AnthropicProvider::complete()` — non-streaming path

**Given** a `CompletionRequest` with `stream: false` routed to an Anthropic provider  
**When** `AnthropicProvider::complete()` is called  
**Then** it returns `Ok(CompletionResponse)` (does NOT return `Err("not yet implemented")`)  
**And** the response includes non-empty content and valid token usage

### SC-12: Streaming — OpenAI SSE chunk (text delta)

**Given** a `StreamChunk` with non-empty `delta` and `finish_reason: None`  
**When** `From<&StreamChunk>` for `OpenAIChatCompletionChunk` is called  
**Then** `choices[0].delta.content` equals the chunk delta  
**And** `choices[0].finish_reason` is `None`  
**And** `usage` is absent

> ⚠ **Note**: No dedicated unit test as of Phase 1. Add in Phase 2 cleanup.

### SC-13: Streaming — OpenAI SSE chunk (final chunk with usage)

**Given** a `StreamChunk` with `finish_reason: Some(Stop)` and `usage: Some(_)`  
**When** converted to `OpenAIChatCompletionChunk`  
**Then** `choices[0].finish_reason` is `"stop"`  
**And** `usage` is present with correct token counts

> ⚠ **Note**: No dedicated unit test as of Phase 1. Add in Phase 2 cleanup.

### SC-14: Streaming — Anthropic SSE event (text delta)

**Given** a `StreamChunk` with non-empty `delta` and `finish_reason: None`  
**When** `From<&StreamChunk>` for `AnthropicSseEvent` is called  
**Then** the event type is `content_block_delta`  
**And** `delta.type` is `"text_delta"` and `delta.text` equals the chunk delta  
**And** the event JSON does NOT contain `output_tokens`

### SC-15: Streaming — Anthropic SSE event (final chunk)

**Given** a `StreamChunk` with `finish_reason: Some(Stop)` and `usage: Some(_)`  
**When** converted to `AnthropicSseEvent`  
**Then** the event type is `message_delta`  
**And** `delta.stop_reason` is `"end_turn"`  
**And** `usage.output_tokens` equals `completion_tokens`  
**And** `usage.input_tokens` equals `prompt_tokens`

### SC-16: Role normalization — `system` → `user` for Anthropic provider

**Given** a `CompletionRequest` with a message of `role: Role::System`  
**When** the request is forwarded to a provider that does not support the system role  
**Then** the system message role is converted to `user` in the outgoing upstream request  
**And** the message content is preserved unchanged

### SC-17: Error normalization — provider HTTP error → `CortexError`

**Given** an upstream provider returns a 4xx or 5xx HTTP error  
**When** the provider adapter processes the response  
**Then** the error is mapped to a `CortexError` variant (e.g. `CortexError::Provider`)  
**And** the raw provider error body is NOT exposed in the API response  
**And** the transport layer serializes `CortexError` into the appropriate wire-format error shape

### SC-18: Error normalization — Anthropic SSE error event

**Given** a `CortexError` occurs during streaming to an Anthropic-format client  
**When** `From<CortexError>` for `AnthropicSseEvent` is called  
**Then** the event type is `error`  
**And** the error JSON contains `type: "invalid_request_error"` and a human-readable `message`  
**And** no internal stack trace or raw provider detail is included

### SC-19: `FormatRegistry` — lookup by provider kind

**Given** a `FormatRegistry` initialized at DI bootstrap with OpenAI and Anthropic adapters  
**When** a request arrives for a known `ProviderKind`  
**Then** the registry returns the correct format adapter for that provider  
**And** the adapter successfully converts the domain request to the provider's wire format

### SC-20: `FormatRegistry` — extensible for new providers

**Given** a new provider adapter implementing the `FormatAdapter` trait  
**When** it is registered in the `FormatRegistry` at bootstrap  
**Then** the registry routes requests to the new provider without changes to existing adapters  
**And** existing OpenAI and Anthropic entries remain unaffected

> **Implementation note**: Phase 1 uses a `match` in `format_for()` instead of a `HashMap` + `with_defaults()` API.  
> This is functionally equivalent but means adding a new provider requires editing `format_for()` directly.  
> Add `register()` + `with_defaults()` API before Phase 3 provider additions.

### SC-21: *(Phase 2 — deferred)* `SseBuffer` shared — no duplication between providers

**Given** `SseBuffer` is extracted to a single shared module in `transport-axum`  
**When** both `providers-openai` and `providers-anthropic` use it  
**Then** both use the same implementation  
**And** a bug fix in `SseBuffer` applies to all consumers without separate changes

### SC-22: *(Phase 2 — deferred)* Tool call translation — OpenAI → Anthropic

**Given** an OpenAI request with a `tools` array and `tool_choice`  
**When** translated to Anthropic wire format  
**Then** `tools` are rendered as Anthropic `tool_use` blocks  
**And** `tool_choice` maps to Anthropic's `tool_choice` field

### SC-23: *(Phase 2 — deferred)* Tool call translation — Anthropic → OpenAI

**Given** an Anthropic response with `tool_use` content blocks  
**When** translated to OpenAI wire format  
**Then** they appear as `tool_calls` entries in `choices[0].message`  
**And** `finish_reason` is `"tool_calls"`
