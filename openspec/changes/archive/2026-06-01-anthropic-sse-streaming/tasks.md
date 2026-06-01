# Tasks: Anthropic SSE Streaming for `/v1/messages`

## Review Workload Forecast

| Field                   | Value       |
|-------------------------|-------------|
| Estimated changed lines | 250–400     |
| 400-line budget risk    | Medium      |
| Chained PRs recommended | No          |
| Suggested split         | Single PR   |
| Delivery strategy       | ask-on-risk |
| Chain strategy          | pending     |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Medium

## Phase 1: SSE Event Types (anthropic_adapter.rs)

- [x] 1.1 Add `AnthropicSseEvent` enum with variants: `ContentBlockDelta`, `MessageDelta`, `Error`
- [x] 1.2 Add `AnthropicTextDelta` struct with `delta_type` and `text` fields
- [x] 1.3 Add `AnthropicMessageDeltaDetails` struct with `stop_reason` and optional `stop_sequence`
- [x] 1.4 Add `AnthropicMessageDeltaUsage` struct with `output_tokens` and optional `input_tokens`
- [x] 1.5 Add `AnthropicErrorEvent` and `AnthropicErrorBody` structs for error SSE events
- [x] 1.6 Verify all types derive `Debug` and implement `Serialize`

## Phase 2: StreamChunk → AnthropicSseEvent Mapping

- [x] 2.1 Implement `From<&StreamChunk>` for `AnthropicSseEvent`:
    - If `finish_reason` is `None`: return `ContentBlockDelta { index: 0, delta: AnthropicTextDelta { delta_type: "text_delta".to_string(), text: delta } }`
    - If `finish_reason` is `Some`: return `MessageDelta { delta: AnthropicStopReason, usage: AnthropicUsage }`
- [ ] 2.2 Add unit test: text chunk maps to `content_block_delta` with correct index and text
- [ ] 2.3 Add unit test: final chunk (with `finish_reason`) maps to `message_delta` with stop_reason and usage

## Phase 3: Streaming Handler (routes.rs)

- [x] 3.1 Modify `anthropic_messages` to check `req.stream == Some(true)` and branch to `anthropic_messages_stream`
- [x] 3.2 Add `anthropic_messages_stream` handler with signature matching `chat_completions_stream` pattern
- [x] 3.3 In handler: call `usecases.route_request.execute_stream(req)`
- [x] 3.4 Map each `StreamChunk` to `AnthropicSseEvent`, serialize to JSON, wrap in `Event::default().data(...)`
- [x] 3.5 On error: yield error SSE event via `openai_error_event` pattern adapted for Anthropic
- [x] 3.6 Chain `[DONE]` sentinel at stream end
- [x] 3.7 Apply SSE headers via `apply_sse_headers` function (reuse from OpenAI path)
- [ ] 3.8 Add unit test: `stream: true` returns `text/event-stream` Content-Type
- [ ] 3.9 Add unit test: `stream: false` returns JSON (regression test)

## Phase 4: Provider Stream Implementation (providers-anthropic/src/lib.rs)

- [x] 4.1 In `AnthropicProvider::stream()`: construct reqwest POST request with `stream: true` to `/v1/messages`
- [x] 4.2 Parse SSE bytes stream from response body
- [x] 4.3 For each SSE event `data: {...}` line: parse JSON and extract `type` field
- [x] 4.4 Map `content_block_delta` → `StreamChunk { delta: Some(text), finish_reason: None, usage: None }`
- [x] 4.5 Map `message_stop` or `message_delta` → `StreamChunk { delta: None, finish_reason: Some(...), usage: Some(...) }`
- [x] 4.6 Handle upstream errors → return `Err(CortexError::upstream_error(...))`
- [x] 4.7 Return `BoxStream` with proper error handling

## Phase 5: Integration Testing

- [ ] 5.1 Add integration test: `POST /v1/messages` with `stream: true` returns valid SSE events
- [ ] 5.2 Verify `content_block_delta` events appear for each token
- [ ] 5.3 Verify `message_delta` event appears with `stop_reason` and `usage.output_tokens`
- [ ] 5.4 Verify stream ends after `message_delta` event (no explicit `[DONE]` — stream terminates naturally per design.md §Contract)
- [ ] 5.5 Add integration test: non-streaming path still returns JSON (regression)
- [ ] 5.6 Add integration test: provider error yields error SSE event

## Implementation Order

1. **Phase 1 → 2**: Adapter types first — `routes.rs` and `providers-anthropic` both depend on `AnthropicSseEvent`
2. **Phase 3**: Routes handler — depends on adapter types and `From<StreamChunk>` impl
3. **Phase 4**: Provider implementation — can be tested independently once interface is correct
4. **Phase 5**: Integration tests — run after all pieces wired together

## Dependencies

- `anthropic_adapter.rs` types → `routes.rs` handler and `providers-anthropic/src/lib.rs`
- `From<StreamChunk>` impl → routes handler mapping logic
- `apply_sse_headers` → already exists, reuse in routes handler
- `openai_error_event` → adapt pattern for Anthropic errors in routes handler
