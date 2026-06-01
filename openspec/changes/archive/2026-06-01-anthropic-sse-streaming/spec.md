# Anthropic SSE Streaming Specification

## Purpose

Enable streaming responses for Anthropic `/v1/messages` endpoint by translating provider streams into the Anthropic Server-Sent Events (SSE) message stream format. The router proxies and translates provider streams in real-time, yielding token-by-token SSE chunks compatible with the Anthropic SDK and Claude Code.

## Requirements

### Requirement: Streaming Endpoint Returns SSE Content-Type

The system SHALL return `Content-Type: text/event-stream` when `POST /v1/messages` is called with `stream: true` in the request body.

#### Scenario: Streaming request returns SSE content type

- GIVEN a `POST /v1/messages` request with `stream: true`
- WHEN the request passes routing and provider selection
- THEN the response SHALL have `Content-Type: text/event-stream` header
- AND the response status code SHALL be `200 OK`

#### Scenario: Non-streaming request returns JSON content type

- GIVEN a `POST /v1/messages` request with `stream: false` or omitted
- WHEN the request is processed
- THEN the response SHALL have `Content-Type: application/json` header (unchanged behavior)

### Requirement: Each SSE Chunk Follows Anthropic Message Stream Format

For each stream chunk from the provider, the system SHALL emit a properly formatted SSE event matching the Anthropic message stream specification.

#### Scenario: Text delta chunk format

- GIVEN a `StreamChunk` with `delta` field containing text
- WHEN the chunk is translated to SSE
- THEN the system SHALL emit: `data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"..."}}`
- AND the `index` field SHALL be `0` (single content block)
- AND the `text` field SHALL contain the token text from the delta

#### Scenario: Final message delta chunk format

- GIVEN a `StreamChunk` where `finish_reason` is `Some(StopReason)`
- WHEN the chunk is translated to SSE
- THEN the system SHALL emit: `data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":N}}`
- AND `stop_reason` SHALL be `"end_turn"`
- AND `usage.output_tokens` SHALL contain the output token count

#### Scenario: Done marker after final chunk

- GIVEN the stream has completed all chunks including usage
- WHEN the final SSE event has been sent
- THEN the system SHALL emit: `data: [DONE]`
- AND the stream SHALL be closed

### Requirement: Upstream Disconnect Aborts Provider Request

If the client disconnects before the stream completes, the system SHALL abort the upstream provider request cleanly without leaking resources.

#### Scenario: Client disconnect propagates as stream error

- GIVEN a streaming request is in progress
- WHEN the client closes the connection
- THEN the provider stream SHALL be aborted via `AbortHandle`
- AND no further SSE events SHALL be sent
- AND the audit entry SHALL be recorded with partial usage if available

#### Scenario: Provider error during stream

- GIVEN a streaming request is in progress
- WHEN the upstream provider returns an error
- THEN the system SHALL emit an SSE error chunk: `data: {"type":"error","error":{"type":"...","message":"..."}}`
- AND the stream SHALL be closed
- AND the audit entry SHALL record the error

### Requirement: Streaming Requests Are Audited

The system SHALL record an audit entry for every streaming request after the stream completes or aborts.

#### Scenario: Successful stream audit

- GIVEN a streaming request completes successfully
- WHEN the final SSE event including usage has been sent
- THEN `execute_stream` SHALL record an audit entry with the full request and response metadata
- AND the audit entry SHALL include `output_tokens` from the final usage chunk

#### Scenario: Aborted stream audit

- GIVEN a streaming request is aborted due to disconnect or error
- WHEN the stream terminates
- THEN `execute_stream` SHALL record an audit entry with available data
- AND the audit entry SHALL reflect the incomplete nature of the interaction

### Requirement: Non-Streaming Path Remains Unaffected

The system SHALL NOT modify the existing non-streaming `POST /v1/messages` behavior.

#### Scenario: Non-streaming request uses existing handler

- GIVEN a `POST /v1/messages` request with `stream: false`
- WHEN the request is processed
- THEN the route handler SHALL call `execute(req)` (not `execute_stream`)
- AND `AnthropicMessagesResponse` SHALL be returned as JSON
- AND no SSE formatting SHALL be applied

### Requirement: Provider Stream Implementation

The `AnthropicProvider::stream()` method SHALL translate Anthropic's SSE format into `StreamChunk` events.

#### Scenario: Provider parses content_block_delta events

- GIVEN the upstream Anthropic API returns an SSE event `data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}`
- WHEN the provider parses this event
- THEN it SHALL return `StreamChunk { delta: Some("hello".to_string()), finish_reason: None, usage: None }`

#### Scenario: Provider parses message_stop event with usage

- GIVEN the upstream Anthropic API returns an SSE event containing final usage
- WHEN the provider parses this event
- THEN it SHALL return `StreamChunk { delta: None, finish_reason: Some(StopReason::EndTurn), usage: Some(Usage { output_tokens: N }) }`

#### Scenario: Provider handles upstream errors

- GIVEN the upstream Anthropic API returns an error or disconnects unexpectedly
- WHEN the provider encounters this condition
- THEN it SHALL return `Err(CortexError::UpstreamError(...))`
- AND the error SHALL propagate to the SSE error handler

## Acceptance Criteria

| ID   | Criterion                                                                   | Verified By            |
|------|-----------------------------------------------------------------------------|------------------------|
| AC-1 | POST /v1/messages with stream: true returns Content-Type: text/event-stream | Unit test in routes    |
| AC-2 | Each SSE chunk follows Anthropic message stream format                      | Unit test in adapter   |
| AC-3 | Final chunk includes output_tokens in usage                                 | Integration test       |
| AC-4 | Upstream disconnect cleanly aborts provider request                         | Unit test with mock    |
| AC-5 | Streaming requests are audited via execute_stream                           | Unit test in usecases  |
| AC-6 | Non-streaming POST /v1/messages with stream: false returns JSON             | Unit test (regression) |
| AC-7 | cargo test --workspace passes with no regressions                           | CI                     |
