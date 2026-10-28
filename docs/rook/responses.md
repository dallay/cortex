# Responses API: stateless compatibility

`POST /v1/responses` accepts text and client functions. Rook normalizes requests
to the existing domain and uses `RouteRequest`, the OpenAI and Anthropic adapters,
and existing format translation. It does not forward requests to a native
Responses upstream or guarantee equivalence with the complete OpenAI API.

## Authorization and routing

The existing `/v1/*` policy applies: API key when configured, `chat:write` or
`admin` scope, model/provider restrictions, request limits, audit, and usage.
`X-Rook-Combo`, `rook/*` aliases, and the default combo remain supported.
Authorization denial occurs before upstream execution. Client-supplied
`X-Authz-*` headers are not trusted.

Responses enables the optional application `OutputValidation` contract for each
request. The policy validates complete responses, individual chunks, and EOF
before recording success. A local rejection records one failure with observed
identity, tokens, and cost; it does not report provider failure or start fallback.
The previous endpoints do not enable this policy.

If a provider adapter rejects a frame that also contains valid usage, the error
retains that usage for audit and cost accounting. It does not deliver invalid
arguments as a successful chunk. An upstream error before the first chunk
preserves combo fallback; each attempt retains its own identity and outcome.

## Supported requests

Top-level fields: `model`, `input`, `instructions`, `stream`, `max_output_tokens`,
`temperature`, `tools`, `tool_choice`, and `store`.

- `model` is required and cannot be empty.
- `input` is a string or an array containing 1 to 1024 items.
- Messages support `role` values `user`, `assistant`, `system`, and `developer`.
  `type` can be omitted or set to `message`. `content` is a string or `input_text`
  parts; assistant messages also support `output_text` for replay. Only empty
  annotations and logprobs are supported.
- `function_call` requires `call_id`, `name`, and `arguments`, a string containing
  a JSON object. IDs must be unique. Consecutive calls belong to the same assistant
  turn. Assistant text interleaved between pending calls is also grouped before
  their results. Results and messages with other roles delimit turns.
- `function_call_output` requires `call_id` and text `output`. The call must appear
  earlier in `input`; duplicate results are not supported.
- Items can contain `id` and `status` (`completed` or `incomplete`) during replay.
  Rook does not persist those item identities.
- Functions require `type: "function"`, `name`, and `parameters` with
  `type: "object"`. They support `description` and `strict: false`. If `strict` is
  omitted, Rook uses non-strict semantics; it does not promise strict model output
  validation.
- `tool_choice` supports `auto`, `none`, `required`, or
  `{"type":"function","name":"nombre_declarado"}`. `required` needs functions.
- `max_output_tokens` must be positive. `temperature` must be between 0 and 2.
- `store` must be omitted or `false`. Responses always report `store: false`.
  Rook does not store conversations or allow response retrieval by ID.

Example request:

```json
{
  "model": "gpt-4o",
  "input": "Busca el archivo README",
  "store": false,
  "tools": [{
    "type": "function",
    "name": "lookup",
    "description": "Busca un archivo",
    "parameters": {"type": "object", "properties": {"name": {"type": "string"}}},
    "strict": false
  }]
}
```

The response contains a Rook `resp_*` ID, the requested model, `status`, `output`,
and `usage` when reported by the provider. Output items are assistant messages
with `output_text` parts, or `function_call` items with an `fc_*` item ID, upstream
`call_id`, name, and arguments. Complete arguments preserve the JSON object, not
necessarily its original whitespace or property order. If the provider does not
supply a `call_id`, Rook creates one that is stable for that response. It does not
duplicate calls present in both Anthropic domain representations.

The `tool_calls` finish state requires at least one function. A response with
functions and `stop` is rejected before recording success. This validation does
not prevent the incomplete states `length` and `content_filter`.

`length` produces `status: "incomplete"` with reason `max_output_tokens`;
`content_filter` produces `incomplete` with reason `content_filter`. An incomplete
call retains partial arguments without presenting them as valid JSON. Provider
reasoning output is outside this subset.

For OpenAI streaming, `length` and `content_filter` preserve `call_id`, name, and
partial arguments, including empty arguments (`""`) when finish arrives in a
separate chunk, even if usage follows finish. Both `[DONE]` and valid EOF produce
one `response.incomplete`, without `response.failed`. This exception applies only
to the Responses origin; Chat and Messages retain rejection of calls without a
`tool_calls` finish.

## Streaming events

With `stream: true`, each SSE frame contains `event: <type>` and a JSON object in
`data`. `sequence_number` starts at 0 and increments by one. The Chat Completions
`[DONE]` marker is not emitted.

Supported events, in lifecycle order:

1. `response.created` and `response.in_progress`.
2. `response.output_item.added` for each message or function.
3. For text: `response.content_part.added`, `response.output_text.delta` events,
   `response.output_text.done`, and `response.content_part.done`.
4. For functions: `response.function_call_arguments.delta` events and
   `response.function_call_arguments.done`.
5. `response.output_item.done` for each item.
6. Exactly one terminal state: `response.completed`, `response.incomplete`, or
   `response.failed`. A late error emits `error` followed by `response.failed`.

Deltas are delivered progressively. The `done` events and terminal snapshot wait
for EOF to include usage that follows upstream finish. EOF without finish, invalid
complete arguments, and late errors produce failure, never success or a provider
switch. Combo fallback can occur only before selecting the first upstream chunk.

Function names can be fragmented before the first argument. Once a function is
announced, its provider cannot change its name or identity. An empty name fragment
is neutral. Anthropic terminal identity summaries are tolerated. If an Anthropic
block receives no `input_json_delta`, its initial `input` object is delivered once
at block closure. If deltas arrive, they replace the initial object; `{}` is not
prepended. Empty OpenAI arguments are not converted to `{}`. Sparse upstream
indices are translated to consecutive output indices.

When upstream finish arrives, validation checks the complete state before
delivering that frame's content. If rejected, neither its deltas nor its content
appear in the terminal snapshot. Deltas delivered in earlier frames remain.
Valid usage from the rejected frame is retained through `StreamValidationError`;
usage from later frames is not read. This local rejection does not permit
fallback after upstream selection.

## Anthropic usage snapshots and selection

For Responses only, Anthropic `message_start` usage is forwarded immediately as
a metadata-only domain chunk. This successful chunk commits provider selection
before the first content delta. An error, EOF, or cancellation after this chunk
retains observed input and cache tokens for accounting; it does not permit
fallback to another provider. Later usage snapshots inherit omitted input and
cache fields from the latest accepted snapshot, not the original snapshot.
Explicit zero values replace previous counters and remain zero when omitted
later. Chat Completions and Messages keep their legacy behavior.

After a recognized finish, a `message_delta` with absent or null `stop_reason`
updates usage only. It does not replay tool identities or finish state. An
explicit unknown `stop_reason` remains a provider validation error, including
after a recognized text or tool finish. Its valid usage snapshot remains
available for failed-response accounting. Content after finish is still rejected.

For Responses OpenAI streams, an explicit unknown `finish_reason` is also a
provider validation error, even in an empty usage frame after a recognized
finish. The rejected frame is not delivered, but its valid usage is retained.
Absent or null finish reasons remain neutral usage updates.

For Anthropic Responses, an omitted tool-use ID remains absent until
the Responses adapter generates a unique `call_id`. Generated IDs can be replayed
with function results. Explicit null, empty, or invalid IDs are rejected, not
replaced, in both nonstream and streaming responses. Rejection retains observed
usage and does not record success or permit fallback after provider selection.
Non-string IDs, including arrays and objects, also remain invalid rather than
being converted to strings or treated as omissions. Nonstream rejection retains
the response's observed usage and does not trigger combo fallback.

For OpenAI Responses, omitted tool-call IDs also generate unique, replayable IDs
in nonstream and streaming responses. Explicit empty or invalid IDs do not
generate replacements. A null ID in an OpenAI stream delta is a neutral
continuation field; it does not change an existing identity. A null ID in a
nonstream tool call is invalid. Chat Completions and Messages retain their
previous ID validation and normalization.

## Limits and explicit rejections

Unknown fields are rejected in every input object. Hosted tools (`web_search`,
`file_search`, etc.), files, images, audio, `previous_response_id`, `conversation`,
`background`, `store: true`, `strict: true`, structured output formats, and
reasoning controls are not supported. These requests return HTTP 400 before
calling the provider.

The adapter supports up to 64 declared functions and 64 output calls, 1 MiB of
aggregate arguments, and 4 MiB of output text plus arguments. The encoder retains
bounded memory for the final snapshot; it does not create a queue or producer
task. Existing HTTP limits remain in place. Validation and encoding use two
independent bounded accumulators per Responses stream. Cancellation accounting
reuses the existing limit of 64 tasks and a 10-second timeout; saturation is
reported through metrics and logs without creating unbounded work.

Input errors use an `error` object with `type`, `code`, `message`, and `param`.
HTTP 413 and 415 are preserved for invalid size and media type. Authorization and
limits retain existing middleware errors. Upstream errors do not expose their
original message to the client. Failure before opening SSE returns an HTTP error;
later failure uses the events above.

## Reproducible SDK evidence

Reference version: **`openai-python==2.11.0`**.

```sh
cargo test -p transport-axum --lib responses_
cargo test -p transport-axum --test gateway_e2e responses::
uv run scripts/verify_responses_sdk.py
```

The script generates fixtures from the real router with mock OpenAI/Anthropic
providers. It validates responses and every event against strict SDK models.
It also checks `responses.create` and accumulation through `responses.stream`.
Cases include Anthropic initial objects without deltas, empty names, interleaved
SSE output replay, and terminal frames with invalid arguments. Additional cases
verify generated OpenAI call IDs and rejected invalid ID types, including
retained usage for standard routes and combos. Both consumers
receive SSE `error` and `response.failed` events. In this version, these events do
not raise `APIError`: the SDK reader does so only when JSON contains a top-level
`error` field. A real HTTP 502 fixture verifies `APIError` separately. The script
requires no credentials or external services.

Pinned SDK limitation: `responses.stream().get_final_response()` retains only
`response.completed`. For `response.incomplete`, validation uses the
`responses.create(stream=True)` event consumer and its terminal response. Clients
must handle incomplete and failed states explicitly. These checks do not establish
compatibility with every coding-agent client or other SDK versions.
