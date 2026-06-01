# Tasks: format-translation-layer

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 350–450 |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Suggested split | Single PR (Phase 1 only) |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: single-pr
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Phase 1 complete (T-01 → T-08) | PR 1 | All tasks below; Phase 2 (tool translation) is a separate future PR |

---

## Phase 1 Tasks

- [x] T-01: Extend `MessageContent` enum in `rook-core`
  - **Files**: `crates/domain/rook-core/src/model.rs`, `crates/infrastructure/providers-anthropic/src/lib.rs:244`, `crates/infrastructure/providers-openai/src/provider.rs:282,321,349`, `crates/infrastructure/transport-axum/src/openai_adapter.rs:42,100`, `crates/infrastructure/transport-axum/src/anthropic_adapter.rs:40`
  - **Tests**: Unit tests in `rook-core` — `as_text()` returns inner string, `From<String>` constructs `Text` variant, serde round-trips `{"role":"user","content":"hi"}` to `MessageContent::Text`
  - **AC**: SC-04 (domain message content preserved), SC-05 (same for Anthropic), SC-09 (response content field), SC-10 (Anthropic response content field)

- [x] T-02: Remove `deny_unknown_fields` from OpenAI adapter — add forward-compat fields
  - **Files**: `crates/infrastructure/transport-axum/src/openai_adapter.rs`
  - **Tests**: Unit test in `transport-axum` — deserializing `{"model":"gpt-4o","messages":[...],"tools":[{}],"tool_choice":"auto","stream_options":{},"response_format":{}}` into `OpenAIChatRequest` succeeds and all known fields parse correctly; minimal request still works (SC-03 regression)
  - **AC**: SC-01 (tools/tool_choice tolerated), SC-03 (valid minimal request unaffected)

- [x] T-03: Remove `deny_unknown_fields` from Anthropic adapter — add `system` + forward-compat fields
  - **Files**: `crates/infrastructure/transport-axum/src/anthropic_adapter.rs`
  - **Tests**: Unit test — request with top-level `system: "Be concise"` prepends a `Role::System` message as first element of `CompletionRequest.messages`; request with `tools`/`tool_choice` deserializes without error; minimal request works
  - **AC**: SC-02 (Anthropic ingress tolerates unknown fields), SC-05 (system field normalized), SC-16 (system role preserved in domain model)

- [x] T-04: Extract `From<&CompletionResponse> for AnthropicMessagesResponse` to `anthropic_adapter.rs`
  - **Files**: `crates/infrastructure/transport-axum/src/anthropic_adapter.rs`, `crates/infrastructure/transport-axum/src/routes.rs`
  - **Tests**: Unit test — `CompletionResponse` with id, content, usage, model converts to `AnthropicMessagesResponse` with `content[0].type == "text"`, `stop_reason == "end_turn"`, correct `usage.input_tokens`/`output_tokens`; assert `routes.rs` no longer contains the inline construction
  - **AC**: SC-10 (`From` impl in adapter, not routes.rs)

- [x] T-05: Implement `AnthropicProvider::complete()` — non-streaming HTTP path
  - **Files**: `crates/infrastructure/providers-anthropic/src/lib.rs`
  - **Tests**: Integration test in `crates/infrastructure/providers-anthropic/tests/provider.rs` — mock HTTP server returns a valid `AnthropicNonStreamResponse` JSON body; assert `complete()` returns `Ok(CompletionResponse)` with matching content, `prompt_tokens`, `completion_tokens`; assert it no longer returns `Err("not yet implemented")`
  - **AC**: SC-11 (`complete()` returns valid response), SC-08 (Anthropic max_tokens default applied)

- [x] T-06: Error normalization — `map_openai_http_error` and `map_anthropic_http_error`
  - **Files**: `crates/infrastructure/providers-openai/src/provider.rs`, `crates/infrastructure/providers-anthropic/src/lib.rs`
  - **Tests**: Unit tests in each provider — 401 → `CortexError::auth_failed`, 429 with `Retry-After: 30` header → `CortexError::rate_limited` with retry delay, 500 → `CortexError::provider` with sanitized body, 400 → `CortexError::invalid_request`; verify raw provider body is NOT present in the mapped error message
  - **AC**: SC-17 (provider HTTP error → `CortexError`), SC-18 (error normalization — no raw provider leaks)

- [x] T-07: Add `FormatRegistry` skeleton in `transport-axum`
  - **Files**: `crates/infrastructure/transport-axum/src/format_registry.rs` *(new)*, `crates/infrastructure/transport-axum/src/lib.rs` (module declaration), `apps/rook/src/di.rs` (construction + Arc injection)
  - **Tests**: Unit tests in `format_registry.rs` — `format_for("openai")` returns `Some(ApiFormat::OpenAI)`, `format_for("anthropic")` returns `Some(ApiFormat::Anthropic)`, `format_for("unknown")` returns `None`; smoke test in DI verifies registry initializes without panic
  - **AC**: SC-19 (registry lookup by provider kind), SC-20 (extensible for new providers)

- [x] T-08: Integration test — cross-format request/response round-trip
  - **Files**: `crates/infrastructure/transport-axum/tests/format_translation_integration.rs` *(new)*
  - **Tests**: Two test cases using mock providers — (1) POST `/v1/chat/completions` with OpenAI-format body returns OpenAI-format response with `choices[0].message.content` and correct `object: "chat.completion"`; (2) POST `/v1/messages` with Anthropic-format body returns Anthropic-format response with `content[0].type == "text"` and `stop_reason == "end_turn"`; both assert no 422 on requests that include `tools` or `stream_options` fields
  - **AC**: SC-04 + SC-09 (OpenAI round-trip), SC-05 + SC-10 (Anthropic round-trip), SC-01 + SC-02 (no 422 on tool-bearing requests)
