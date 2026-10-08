# Delta for usage-history

## ADDED Requirements

### Requirement: Usage History Table

The system SHALL create a new `usage_history` SQLite table that stores detailed per-request usage records including all five token dimensions (prompt, completion, cache_read, cache_creation, reasoning), latency metrics, cost estimates, and identifiers for the API key and provider connection used.

The table MUST include these columns: `id` (INTEGER PRIMARY KEY AUTOINCREMENT), `request_id` (TEXT NOT NULL), `provider` (TEXT NOT NULL), `model` (TEXT NOT NULL), `status` (TEXT NOT NULL), `requested_tier` (TEXT), `api_key_id` (TEXT), `connection_id` (TEXT), `tokens_prompt` (INTEGER), `tokens_completion` (INTEGER), `tokens_cache_read` (INTEGER), `tokens_cache_creation` (INTEGER), `tokens_reasoning` (INTEGER), `ttft_ms` (INTEGER), `latency_ms` (INTEGER NOT NULL), `cost_usd` (REAL), `timestamp` (TEXT NOT NULL).

Indexes SHALL be created on: `(request_id)`, `(provider)`, `(model)`, `(timestamp)`, `(api_key_id)`, `(connection_id)`.

#### Scenario: Usage record written on successful request

- GIVEN a routed request completes successfully with all token dimensions populated
- WHEN the `UsageRecorderPort::record()` is called with a complete `UsageEntry`
- THEN the record is persisted to `usage_history` with all fields populated
- AND `cost_usd` reflects the pricing formula result

#### Scenario: Usage record written with null optional fields

- GIVEN a routed request to a provider that does not return cache or reasoning tokens
- WHEN the `UsageRecorderPort::record()` is called with partial token data
- THEN `tokens_cache_read`, `tokens_cache_creation`, and `tokens_reasoning` are stored as NULL
- AND `cost_usd` is calculated from available token fields only

#### Scenario: Missing pricing produces warning and null cost

- GIVEN a request for provider "unknown-provider" with no pricing config entry
- WHEN the use case layer builds the `UsageEntry` before calling `UsageRecorderPort::record()`
- THEN `cost_usd` is set to NULL on the entry that is persisted
- AND a `tracing::warn!` is emitted with field `pricing_missing`
- AND the `usage_cost_unknown_total` metric counter is incremented

### Requirement: UsageRecorderPort Interface

The system SHALL provide a `UsageRecorderPort` trait in `rook-core` with write and read methods. The trait MUST include:

- `record(&self, entry: UsageEntry) -> CortexResult<()>` — writes a single usage entry
- `list(&self, filters: UsageFilters, pagination: Pagination) -> CortexResult<Vec<UsageEntry>>` — paginated list with all fields, ordered by timestamp descending
- `count(&self, filters: UsageFilters) -> CortexResult<u64>` — total matching records for paginated API responses
- `summary(&self, filters: UsageFilters) -> CortexResult<UsageSummary>` — aggregated counts and averages
- `cost_breakdown(&self, filters: UsageFilters) -> CortexResult<CostBreakdown>` — sum of costs grouped by provider, model, and API key

`UsageFilters` MUST support filtering by: `provider` (Option<ProviderId>), `model` (Option<ModelId>), `api_key_id` (Option<ApiKeyId>), `connection_id` (Option<ConnectionId>), `start` (Option<DateTime<Utc>>), `end` (Option<DateTime<Utc>>), `status` (Option<RequestStatus>).

`Pagination` MUST have `offset: u64` and `limit: u64`. Default limit MUST be 100. Maximum limit MUST be 1000.

The port is nullable — when `None`, record calls are silently skipped (fire-and-forget semantics preserved).

#### Scenario: List returns paginated results ordered by timestamp

- GIVEN 250 usage records exist with various providers
- WHEN `list(filters: None, pagination: Pagination { offset: 0, limit: 100 })` is called
- THEN exactly 100 entries are returned
- AND entries are ordered by `timestamp` descending

#### Scenario: Summary aggregates across filtered set

- GIVEN 50 usage records for provider "openai" with varying token counts
- WHEN `summary(filters: UsageFilters { provider: Some("openai") })` is called
- THEN the result contains `total_requests: 50`
- AND `total_prompt_tokens`, `total_completion_tokens` are summed across all 50 records
- AND `avg_ttft_ms` and `avg_latency_ms` are computed correctly

#### Scenario: Cost breakdown groups by provider, model, and API key

- GIVEN 100 usage records with varying costs across providers
- WHEN `cost_breakdown(filters: UsageFilters { start: Some(last_7_days) })` is called
- THEN `total_cost_usd` is the sum of all records' cost_usd
- AND `by_provider` maps each ProviderId to its summed cost
- AND `by_model` maps each ModelId to its summed cost
- AND `by_api_key` maps each ApiKeyId to its summed cost

### Requirement: API Key ID Propagation

The system SHALL propagate `api_key_id` from the authz middleware to the request context. The authz middleware stamps `api_key_id` into the `x-authz-auth-id` request header. A new extractor function reads this header and passes `Option<ApiKeyId>` into `RequestMetadata`.

#### Scenario: API key ID extracted from request header

- GIVEN an authenticated request with `x-authz-auth-id: key_abc123` header
- WHEN the request is processed by `route_request.rs`
- THEN `request_metadata.api_key_id` contains `Some(ApiKeyId("key_abc123"))`
- AND the usage record includes the api_key_id

#### Scenario: Missing auth header produces None

- GIVEN a request without the `x-authz-auth-id` header (e.g., internal routing)
- WHEN the request is processed
- THEN `request_metadata.api_key_id` contains `None`
- AND the usage record has `api_key_id` as NULL

### Requirement: Connection ID Propagation

The system SHALL propagate `connection_id` via a new `ProviderRepositoryPort::find_connection_id_by_runtime()` method. The method accepts `ProviderId` and returns `Option<ConnectionId>`. The lookup is best-effort — returns `None` for TOML providers that have no row in `provider_connections`.

#### Scenario: Connection ID found for dynamic provider

- GIVEN a `ConnectionId` exists in `provider_connections` for runtime provider "openai-primary"
- WHEN `find_connection_id_by_runtime(ProviderId("openai-primary"))` is called
- THEN `Some(ConnectionId(...))` is returned

#### Scenario: Connection ID None for TOML provider

- GIVEN provider "ollama-local" is defined in TOML config only
- WHEN `find_connection_id_by_runtime(ProviderId("ollama-local"))` is called
- THEN `None` is returned
- AND no error is raised

### Requirement: TTFT Measurement

The system SHALL measure time-to-first-token (TTFT) in the use case layer (`route_request.rs`). TTFT is measured from request start to the arrival of the first successful `StreamChunk`. The measurement is captured before the audit record is written.

#### Scenario: TTFT recorded for streaming request

- GIVEN a streaming request to OpenAI that receives first chunk after 150ms
- WHEN the first `StreamChunk` arrives at the use case layer
- THEN `ttft_ms: 150` is recorded in the usage entry
- AND `latency_ms` records the total request duration

#### Scenario: TTFT not available for non-streaming

- GIVEN a non-streaming request that completes in 200ms total
- WHEN the response is assembled
- THEN `ttft_ms` is recorded as the total latency (no streaming chunks)
- AND `latency_ms` is 200ms

### Requirement: Retention Sweep

The system SHALL enforce data retention by running a sweep job at startup and every 6 hours thereafter. The sweep deletes from `usage_history` where `timestamp < now() - retention_days`. Default retention is 90 days and is configurable via `config.usage.retention_days`.

#### Scenario: Startup sweep deletes expired records

- GIVEN `usage_history` contains 1000 records, 150 of which are older than 90 days
- WHEN the application starts and the startup sweep runs
- THEN 150 records are deleted
- AND 850 records remain

#### Scenario: Periodic sweep runs every 6 hours

- GIVEN the application has been running for 6 hours
- WHEN the periodic sweep fires
- THEN records older than `now() - retention_days` are deleted
- AND no active records are affected

### Requirement: Fire-and-Forget Instrumentation

The system SHALL instrument all four fire-and-forget audit call sites with `tracing::warn!` and a metric counter. When audit or usage record creation fails silently, the system MUST emit `tracing::warn!` with field `usage_record_failed = true` and increment the `usage_record_failed_total` metric counter.

#### Scenario: Audit failure emits warning and increments metric

- GIVEN an audit call site where the underlying record operation silently fails
- WHEN the fire-and-forget path completes
- THEN `tracing::warn!` is emitted with `usage_record_failed = true` and relevant context
- AND the metric counter `usage_record_failed_total` is incremented

---

## MODIFIED Requirements

### Requirement: TokenUsage Extended

The existing `TokenUsage` struct in `rook-core` is extended with three new fields: `cache_read_tokens: Option<u64>`, `cache_creation_tokens: Option<u64>`, and `reasoning_tokens: Option<u64>`. These fields are nullable — they are `None` for providers that do not return them.

(Previously: TokenUsage had only `prompt_tokens`, `completion_tokens`, and `total_tokens` — all required u64 fields)

#### Scenario: OpenAI returns reasoning tokens

- GIVEN a request to OpenAI with `reasoning_tokens` in the response
- WHEN the `OpenAIUsage` is parsed
- THEN `tokens_reasoning: Some(1500)` is set on the `TokenUsage`

#### Scenario: Ollama returns no cache tokens

- GIVEN a request to Ollama that returns only `prompt_eval_count` and `eval_count`
- WHEN the token usage is parsed
- THEN `tokens_cache_read: None` and `tokens_cache_creation: None` and `tokens_reasoning: None`

---

## REMOVED Requirements

(None — the existing `audit` table and `AuditPort` remain as deprecated but preserved.)

---

## Provider Token Field Mapping

| Provider  | prompt_tokens     | completion_tokens    | cache_read                          | cache_creation              | reasoning                                  |
|-----------|-------------------|----------------------|-------------------------------------|-----------------------------|--------------------------------------------|
| OpenAI    | prompt_tokens     | completion_tokens    | prompt_tokens_details.cached_tokens | None                        | completion_tokens_details.reasoning_tokens |
| Anthropic | input_tokens      | output_tokens        | cache_read_input_tokens             | cache_creation_input_tokens | —                                          |
| Ollama    | prompt_eval_count | eval_count           | None                                | None                        | None                                       |
| Gemini    | promptTokenCount  | candidatesTokenCount | None                                | None                        | None                                       |
| Groq      | prompt_tokens     | completion_tokens    | —                                   | —                           | —                                          |

---

## Cost Calculation

```
cost_usd = (prompt_tokens * price_prompt_per_token) + (completion_tokens * price_completion_per_token) + (cache_read_tokens * price_cache_read_per_token) + (cache_creation_tokens * price_cache_creation_per_token)
```

Prices from `[pricing.<provider>.<model>]` TOML config (dollars per million). Model IDs containing dots MUST be quoted in TOML table paths, for example `[pricing.groq."llama-3.3-70b"]`. When no pricing entry exists: `cost_usd` is NULL, `tracing::warn!` emitted, `usage_cost_unknown_total` incremented.

---

## API Endpoints

| Method | Path                 | Purpose                                    |
|--------|----------------------|--------------------------------------------|
| `GET`  | `/api/usage`         | List usage entries (paginated)             |
| `GET`  | `/api/usage/summary` | Aggregated counts and averages             |
| `GET`  | `/api/usage/cost`    | Cost breakdown by provider, model, API key |

Always-on, MANAGEMENT auth required.

#### Scenario: GET /api/usage returns paginated entries

- GIVEN 500 usage records exist
- WHEN client calls `GET /api/usage?limit=50&offset=0&provider=openai`
- THEN response is `200 OK` with JSON: `{ "entries": [...], "total": 500 }`

#### Scenario: GET /api/usage/summary returns aggregated data

- GIVEN usage records with varying token counts
- WHEN client calls `GET /api/usage/summary?provider=anthropic`
- THEN response includes `total_requests`, `total_prompt_tokens`, `avg_ttft_ms`, `avg_latency_ms`

#### Scenario: GET /api/usage/cost returns cost breakdown

- GIVEN usage records with calculated costs
- WHEN client calls `GET /api/usage/cost?start=2026-06-01T00:00:00Z`
- THEN response includes `total_cost_usd`, `by_provider`, `by_model`, `by_api_key`
