# Proposal: Anthropic SSE Streaming for `/v1/messages`

## Intent

Enable the Anthropic SDK and Claude Code to stream responses through Rook by implementing SSE streaming on the `/v1/messages` endpoint. When `stream: true` is set, Rook must proxy and translate provider streams into the Anthropic message stream format. The non-streaming path remains unaffected.

## Scope

### In Scope

1. **Anthropic SSE adapter** (`anthropic_adapter.rs`) — new `AnthropicStreamChunk` type and `From<StreamChunk>` translation to Anthropic SSE format
2. **Streaming handler** (`routes.rs`) — branch `anthropic_messages` on `stream: true` and delegate to `execute_stream`, then translate chunks to SSE
3. **Anthropic provider streaming** (`providers-anthropic/src/lib.rs`) — implement `stream()` to parse Anthropic's SSE event format into `StreamChunk`; currently a stub
4. **Upstream disconnect handling** — `execute_stream` already propagates provider stream abort; verify it via `AbortHandle` propagation

### Out of Scope

- Changes to `execute_stream` in `route_request.rs` — already handles streaming with correct audit-on-completion
- Non-streaming path — already implemented and must not regress
- OpenAI `/v1/chat/completions` streaming — already working

## Capabilities

### New Capabilities

- `anthropic-streaming`: SSE streaming for Anthropic `/v1/messages` endpoint. Streams `content_block_delta` events per token, final `message_delta` with `stop_reason` and `usage`.

### Modified Capabilities

- None (no existing spec-level behavior changes)

## Approach

### Pattern: Follow OpenAI SSE Streaming

The OpenAI SSE streaming path (`chat_completions_stream` in `routes.rs`) is the reference implementation:

1. Call `execute_stream(req)` → returns `BoxStream<Result<StreamChunk, CortexError>>`
2. Map each `StreamChunk` to the provider's wire format via an adapter `From` impl
3. Wrap in `Sse::new(events)` with `apply_sse_headers`
4. `execute_stream` already audits on stream completion — no additional audit needed

### Anthropic SSE Format

Each chunk maps as follows:

```
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"..."}}
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":123}}
data: [DONE]
```

**Mapping from `StreamChunk`**:

- `delta` field → `text` in `content_block_delta`
- `finish_reason` (when `Some`) → `stop_reason` in `message_delta`
- `usage` (when `Some`) → `output_tokens` in `message_delta` usage

### Provider Implementation

`AnthropicProvider::stream()` currently returns an error stub. It must:

1. POST to Anthropic `/v1/messages` with `stream: true`
2. Parse SSE `text/event-stream` lines
3. Translate `content_block_delta` events → `StreamChunk { delta: text }`
4. Translate `message_stop` / final event → `StreamChunk { finish_reason: Some(Stop), usage }`
5. Propagate upstream errors; `reqwest` must handle disconnect via its `Response` bytes streaming

## Affected Areas

| Area                                        | Impact   | Description                                                         |
|---------------------------------------------|----------|---------------------------------------------------------------------|
| `crates/infrastructure/transport-axum`      | Modified | `anthropic_adapter.rs` (new types), `routes.rs` (streaming handler) |
| `crates/infrastructure/providers-anthropic` | Modified | `stream()` implementation using reqwest SSE parsing                 |

## Risks

| Risk                               | Likelihood | Mitigation                                                                                    |
|------------------------------------|------------|-----------------------------------------------------------------------------------------------|
| Provider doesn't support streaming | Low        | Verify provider config; log and return 501 if unsupported                                     |
| Upstream disconnect not propagated | Medium     | Use reqwest's streaming response with proper error handling; `execute_stream` aborts on error |
| SSE chunk translation mismatch     | Medium     | Add integration test that verifies SSE format end-to-end                                      |

## Rollback Plan

1. Revert `routes.rs` to call `execute(req)` unconditionally (remove streaming branch)
2. Revert `anthropic_adapter.rs` to remove streaming types
3. Revert `providers-anthropic/src/lib.rs` to the error stub
4. No DB migration needed — purely additive change

## Dependencies

- `reqwest` (already in workspace) — for SSE stream parsing
- No new external dependencies

## Success Criteria

- [ ] `POST /v1/messages` with `stream: true` returns `Content-Type: text/event-stream`
- [ ] Each SSE chunk follows Anthropic message stream format (`content_block_delta`, `message_delta`)
- [ ] Final chunk includes `output_tokens` in usage
- [ ] Upstream disconnect cleanly aborts (provider stream error propagates to SSE error event)
- [ ] Streaming requests are audited via existing `execute_stream` audit-on-completion
- [ ] Non-streaming `POST /v1/messages` with `stream: false` returns JSON and is unaffected
- [ ] `cargo test --workspace` passes with no regressions
