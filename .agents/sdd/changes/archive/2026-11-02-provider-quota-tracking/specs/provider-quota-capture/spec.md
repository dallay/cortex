# Provider Quota Capture Specification

## Purpose

Defines how upstream provider quota signals are captured from real HTTP responses and persisted separately from local usage history. This is the capture half (U1–U3) of the provider quota capability; the endpoint contract lives in `provider-usage-quota` and the presentation contract in `providers-ui`.

## Requirements

### Requirement: Separate Upstream Observations from Local Usage

The system MUST persist upstream-reported quota observations in a store distinct from `usage_history`. Local request/token/cost records MUST NOT be rewritten, merged, or backfilled from observations, and observations MUST NOT be derived from local usage, cost, or 429 frequency.

#### Scenario: Observation is stored independently

- GIVEN a provider response carries supported quota headers
- WHEN the response is processed
- THEN an observation row is persisted in `provider_quota_observations`
- AND `usage_history` is unchanged by that write

#### Scenario: Local usage does not produce observations

- GIVEN recorded local usage exists but no upstream headers were captured
- WHEN quota data is queried
- THEN no observation is synthesized from the local records

### Requirement: Capture Only From Canonical Provider Endpoints

The system MUST enable capture only when the effective base URL is the provider's canonical endpoint, whether implicit (no override) or explicitly configured. Custom proxies, non-HTTPS endpoints, explicit ports, and foreign hosts MUST disable capture so third-party rate-limit headers are never attributed to the provider.

#### Scenario: Default endpoint captures

- GIVEN a managed connection for OpenAI, Anthropic, or Groq with no base URL override
- WHEN a response is received
- THEN capture is enabled

#### Scenario: Explicit canonical endpoint captures

- GIVEN a managed connection whose stored base URL is the provider's canonical endpoint
- WHEN a response is received
- THEN capture is enabled
- AND trailing slashes and the OpenAI `/v1` suffix do not change the decision

#### Scenario: Custom endpoint does not capture

- GIVEN a managed connection pointed at a proxy, mock, plain HTTP, an explicit port, or a look-alike host
- WHEN a response is received
- THEN capture is disabled and no observation is persisted

#### Scenario: Non-capturing providers

- GIVEN the provider is Ollama, Ollama Cloud, or Gemini
- WHEN a response is received
- THEN no quota observation is persisted

### Requirement: Record Identity and Provenance

Each observation MUST retain the provider family, the managed runtime identity, and the originating connection. The recorder MUST restore the runtime identity even when the adapter reports a connection-scoped identifier.

#### Scenario: Two connections share one runtime

- GIVEN one managed runtime with two connections
- WHEN both capture observations
- THEN both rows carry the same runtime identity and their own connection identity

#### Scenario: Provenance is preserved

- GIVEN an observation was captured
- WHEN it is retrieved
- THEN its metric, source, `observed_at`, and available scope/window/reset metadata are intact

### Requirement: Capture Must Not Alter Provider Outcomes

Recording MUST be best-effort and MUST NOT change the provider response, its status, its body, or its error classification. A failing or unavailable recorder MUST NOT surface an error to the caller.

#### Scenario: Recorder failure is swallowed

- GIVEN the recorder rejects the write
- WHEN a provider response is processed
- THEN the provider result is returned unchanged
- AND a warning is logged

#### Scenario: Capture on error responses

- GIVEN the provider returns a rate-limited or otherwise failing HTTP status carrying quota headers
- WHEN the response is processed
- THEN headers are captured before the body is consumed
- AND the original error is still returned

### Requirement: Bounded, Non-Blocking Persistence

Persistence MUST NOT block the request path. Writes MUST be queued with a bounded capacity; when the queue is full or closed, the incoming batch MUST be dropped, counted, and logged rather than delaying or failing the request.

#### Scenario: Queue saturation

- GIVEN the write queue is at capacity
- WHEN a new observation batch arrives
- THEN the batch is dropped
- AND the drop counter is incremented
- AND the provider response is unaffected

#### Scenario: Shutdown drains accepted batches

- GIVEN the repository is closing
- WHEN the last sender is released
- THEN accepted batches are drained and the writer is joined

### Requirement: Retention Bounded by Batches

Observation retention MUST delete expired rows in bounded batches inside a transaction, following the same pattern as `usage_history` retention, so large expirations do not hold a long write lock.

#### Scenario: Large expiry set

- GIVEN more expired observations than one batch
- WHEN retention runs
- THEN rows are deleted in bounded batches
- AND recent observations and unrelated `usage_history` rows are preserved

### Requirement: Migration Must Not Collide

The observation store MUST be introduced by a migration whose version is unique in the embedded migration set. Duplicate versions MUST fail review, because the migration runner rejects them at startup.

#### Scenario: Unique version

- GIVEN the embedded migrations directory
- WHEN migrations are loaded
- THEN exactly one migration defines each version
