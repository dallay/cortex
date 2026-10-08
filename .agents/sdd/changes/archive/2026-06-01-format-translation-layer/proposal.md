# Proposal: format-translation-layer

## Intent

The Rook proxy already uses the domain model as a translation pivot between wire formats, but the current implementation is incomplete: `Message.content` is a plain `String` (no tool support), `deny_unknown_fields` causes request rejection (422 Unprocessable Entity) when `tools`/`tool_choice` are present in incoming requests, `AnthropicProvider.complete()` is unimplemented, and error shapes are provider-specific. This change delivers a complete, extensible bidirectional translation layer between OpenAI and Anthropic wire formats for Phase 1 (text + error normalization + fix of `deny_unknown_fields`). Tool call translation is planned for Phase 2.

## Scope

### In Scope

- Extend `rook-core` domain types: `MessageContent` enum (`Text`); `ToolUse`/`ToolResult` deferred to Phase 2
- Remove `deny_unknown_fields` from OpenAI and Anthropic adapters; accept `tools`/`tool_choice` without rejection
- Implement `AnthropicProvider.complete()` (non-streaming path was unimplemented)
- `FormatRegistry` in `transport-axum` mapping provider kind → inbound format (scaffold for Phase 3)
- Error normalization: all provider errors → `CortexError`
- **Phase 1** (text + error normalization + fix `deny_unknown_fields` + `AnthropicProvider.complete()` + `FormatRegistry` scaffold)
- **Phase 2** (tool call translation: OpenAI `tools`/`tool_choice` ↔ Anthropic `tool_use`; `Tool`, `ToolChoice`, `ToolResult` domain types; `SseBuffer` extraction)

### Out of Scope

- Gemini, Ollama, Groq format adapters (Phase 3, deferred)
- Streaming tool call reassembly beyond basic delta merging
- Role inference beyond `system → user` normalization for providers that reject system role

## Approach

- **Domain model as translation pivot** — no `serde_json::Value` passthrough; every wire format maps to/from the domain types in `rook-core`. This keeps provider adapters decoupled from each other.
- **`MessageContent` enum replaces `String`** — `Text(String) | ToolUse { id, name, input } | ToolResult { tool_use_id, content }` in `rook-core/src/model.rs`. Both adapters serialize/deserialize to this enum.
- **`FormatRegistry`** is a `HashMap<ProviderKind, FormatAdapter>` struct in `transport-axum/src/format_registry.rs`. Each adapter implements `fn to_domain(req) -> DomainRequest` and `fn from_domain(resp) -> WireResponse`. Registry is constructed at DI bootstrap.
- **Phased delivery** — Phase 1 is independently shippable (unblocks all acceptance criteria except tool calls). Phase 2 adds tool call support on top of the stable Phase 1 foundation.
- **Error normalization via `From` impls** — each provider crate implements `From<ProviderError> for CortexError`; transport layer maps at the boundary, never leaks provider error shapes.

## Affected Components

| Component                               | Change   | Summary                                                                          |
|-----------------------------------------|----------|----------------------------------------------------------------------------------|
| `rook-core/src/model.rs`                | Modified | Add `MessageContent`, `Tool`, `ToolChoice`, `FinishReason::ToolCalls` wired up   |
| `transport-axum/src/openai/`            | Modified | Remove `deny_unknown_fields`, add `tools`/`tool_choice` fields, map to domain    |
| `transport-axum/src/anthropic/`         | Modified | Remove `deny_unknown_fields`, add tool_use serialization, fix `content` handling |
| `transport-axum/src/format_registry.rs` | New      | `FormatRegistry` mapping provider kind → format adapter                          |
| `transport-axum/src/sse_buffer.rs`      | New      | Extract `SseBuffer` from provider-specific locations to shared module            |
| `providers-anthropic/src/lib.rs`        | Modified | Implement `complete()` (non-streaming), normalize errors via `From`              |
| `rook-usecases/src/`                    | Modified | Route tool-bearing requests; pass `Tool` list through use-case boundary          |

## Risks

| Risk                                                                                             | Likelihood | Mitigation                                                                               |
|--------------------------------------------------------------------------------------------------|------------|------------------------------------------------------------------------------------------|
| `MessageContent` enum breaks existing snapshot/integration tests that assert on `String` content | High       | Update tests in same PR; add golden-file tests for both wire formats                     |
| Anthropic `complete()` edge cases (rate limits, vision content) not covered by Phase 1           | Med        | Scope Phase 1 to text-only; mark unsupported content types as `CortexError::Unsupported` |
| `FormatRegistry` initialization order at DI bootstrap causes runtime panic                       | Low        | Initialize registry before router construction; add a startup smoke test                 |

## Rollback Plan

All changes are additive (new enum variants, new fields with `Option`, new struct). Removing `deny_unknown_fields` is safe to revert by re-adding the attribute. `AnthropicProvider.complete()` can revert to returning `Err(CortexError::NotImplemented)` without breaking the streaming path. The `FormatRegistry` is only constructed in DI — removing it restores prior behavior.

## Success Criteria

- [ ] Requests with `tools`/`tool_choice` fields are forwarded and not silently dropped (fix `deny_unknown_fields`)
- [ ] OpenAI → Anthropic routing produces correct Anthropic-format request body (bidirectional text)
- [ ] Anthropic → OpenAI response is correctly translated to OpenAI wire format
- [ ] `AnthropicProvider.complete()` returns a valid non-streaming response
- [ ] Tool call round-trip: OpenAI `tools` array translates to Anthropic `tools`; `tool_use` response translates back to OpenAI `tool_calls`
- [ ] All provider errors surface as `CortexError` variants (no raw provider error leaks)
- [ ] Existing integration tests pass; new golden-file tests cover Phase 1 and Phase 2 wire formats
