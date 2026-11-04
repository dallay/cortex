# huginn-ui-events — Specification

## Purpose

Capture non-conversational UI events and notices with bounded retention and deduplication.

## Requirements

### Requirement: EVT-01 — Non-Conversational Events

System notices, toast messages, and non-fatal operational errors MUST be delivered
via dedicated UI events, distinct from session message history. They MUST NOT be
appended to the conversation transcript or serialized into durable session messages.

#### Scenario: EVT-01-A — Toasts outside conversation

- GIVEN an active conversation session
- WHEN a toast notification is emitted (e.g., settings saved, connection updated)
- THEN the notification is displayed outside the chat area
- AND the persistent message log remains unmodified

### Requirement: EVT-02 — Bounded Memory Buffer

The UI event buffer MUST enforce a maximum capacity (e.g. 256 entries).
When full, older events MUST be evicted FIFO.
Clearing or evicting events MUST NOT alter or truncate conversation history.

#### Scenario: EVT-02-A — Buffer eviction

- GIVEN an event buffer at maximum capacity
- WHEN a new UI event arrives
- THEN the oldest event is evicted from the in-memory buffer
- AND reopening the session still recovers all persisted conversation messages and results

### Requirement: EVT-03 — Deduplicated Errors per Operation

Repetitive errors for the same operation MUST be deduplicated. Hash alone MUST NOT
determine identity. Within a 2-second sliding window, identical errors for the same
operation MUST be suppressed from redundant rendering.
Distinct operations MUST NOT suppress each other.

#### Scenario: EVT-03-A — Deduplication window

- GIVEN a visible error from operation A at t=0
- WHEN the same error recurs at t=1s
- THEN the second occurrence is suppressed
- AND at t=2.1s a recurrence is displayed once

#### Scenario: EVT-03-C — Hash collision with distinct message

- GIVEN two different error messages with matching hash in operation A
- WHEN both arrive
- THEN both are displayed and not falsely suppressed

### Requirement: EVT-04 — Finite Retention and Expiration

The deduplication tracker MUST have bounded memory. Expired entries and completed
operations MUST be pruned. Pruning MAY allow a previously seen error to re-render
if triggered again after expiration.

#### Scenario: EVT-04-A — Saturation and expiration

- GIVEN a deduplication capacity limit L
- WHEN errors from L+1 operations arrive and subsequently expire
- THEN total retained memory does not exceed the limit
- AND expired entries are safely reclaimed
