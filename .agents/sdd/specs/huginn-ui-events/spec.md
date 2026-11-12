# huginn-ui-events — Specification

## Purpose

Separate conversation and UI surfaces during emission, projection, and
recovery. This capability is new; `proposal.md` prevails over exploration
suggestions.

## Requirements

### Requirement: EVT-01 — Explicit classification before deduplication

The system MUST distinguish `Conversation`, `Overlay`, `Notification`, `Status`
and `Diagnostic` through explicit methods per category, without a generic
`notice` nor classification by prefixes. Only `Conversation` MUST feed
`conversation`. No UI event MUST become conversation through repetition, dedup
expiry, or synchronization. Notices and diagnostics MUST be observable on
non-conversational surfaces, without a toast layer.

#### Scenario: EVT-01-A — QA of opening and closing without contamination

- GIVEN a conversation whose content and entry count have been recorded
- WHEN each of `/help`, `/providers`, `/login`, `/model` and `/settings` is opened and closed, and repeated after more than 2 seconds
- THEN the conversation content and entry count do not change
- AND help, forms, UI results, and notices are shown outside the conversation
- AND these commands are not sent to the model as prompts

#### Scenario: EVT-01-B — Classification independent of the text

- GIVEN the same text emitted explicitly as `Conversation`, `Notification`, `Status` and `Diagnostic`
- WHEN the four emissions are projected
- THEN only the `Conversation` emission is added to the conversation
- AND the text prefixes do not alter the category

### Requirement: EVT-02 — Conversation and persistence preserved

The system MUST rename `committed` to `conversation` and preserve user/agent
messages and relevant tool results, including tool errors that constitute
conversational results. It MUST NOT deduplicate legitimate conversational
messages. Streaming and synchronization MUST preserve order, content, and the
absence of duplicates caused by reprojection. The UI buffer limit MUST NOT
truncate the persisted session nor its recovery.

#### Scenario: EVT-02-A — Legitimate repetition and streaming

- GIVEN two legitimate identical messages within an interval below 2 seconds and a streaming response
- WHEN the response ends and the same session is synchronized again
- THEN both messages remain in order and a single complete response
- AND the relevant result of a tool remains in the conversation, not converted into a UI notice

#### Scenario: EVT-02-B — Exact limit and recovery

- GIVEN a session with more than 256 persisted events and an empty UI buffer
- WHEN 256 retainable events are incorporated and then one more
- THEN the UI buffer contains exactly 256 events, the most recent ones in order
- AND the oldest event leaves only the UI buffer
- AND reopening the session allows recovering all persisted messages and results, including those before the limit

### Requirement: EVT-03 — Errors deduplicated per operation

The system MUST deduplicate UI errors using the hash, the message equality, and
the operation identity. The hash alone MUST NOT determine equality. With a
controllable clock, an equal error of the same operation MUST be suppressed
during the 2 seconds following its last visible emission; at exactly 2 seconds
it MUST be able to reappear. Suppression MUST NOT extend that window.
Independent operations MUST NOT suppress each other; a new invocation MUST have
a distinct identity. A suppressed error MUST NOT produce another equivalent
notice on a different UI surface. This rule MUST NOT remove conversational
results.

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:e39ffd7ce76aeebb2b39300e0bbeea81eb8006df5c8b9b60efd9a5f082ba1d92 -->

#### Scenario: EVT-03-A — Temporal boundaries without real waits

- GIVEN a visible error of operation A at t=0 and a controlled clock
- WHEN the same error of A arrives at t=1.999 seconds and at t=2 seconds
- THEN the first repetition does not generate a second presentation of the error
- AND the second is presented once outside the conversation
- AND the suppressed repetition has not shifted the expiry

#### Scenario: EVT-03-B — Distinct and interleaved operations

- GIVEN an error M of A visible at t=0
- WHEN at t=0.5 M of B arrives and at t=1 M of A arrives
- THEN M of B is presented because B is independent
- AND M of A stays suppressed even though B was interleaved

<!-- /coderabbit-ai-generated:v1 -->

#### Scenario: EVT-03-C — Collision and different message

- GIVEN two different messages with the same hash within the same operation
- WHEN both are emitted in less than 2 seconds
- THEN both are presented, because their content is not equal

### Requirement: EVT-04 — Bounded UI retention and dedup

The UI buffer MUST retain at most 256 events and evict the oldest ones first.
The dedup state MUST have a documented finite limit of entries and of retained
size, release finished operations and expired records, and not keep secrets.
Exhausting that limit MUST NOT hide new errors: the eviction MAY allow showing
a previously seen error again. The concrete dedup limit is left to the design,
not the obligation to bound it.

#### Scenario: EVT-04-A — Saturation and expiry

- GIVEN the documented dedup limit L and a controlled clock
- WHEN errors from L+1 identities arrive and then their windows expire or their operations finish
- THEN retention never exceeds the documented limit
- AND every new error remains observable
- AND expired or finished records are released without altering the persisted session
