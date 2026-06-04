# Proposal: Usage Tracking — Token Counts & Cost Estimation

## Intent

Replace the current basic `audit` log — a single SQLite table with only `prompt/completion/total` token fields and an always-`None` `estimated_cost_usd` — with a structured usage tracking system. The new system records per-request token counts across all five dimensions (prompt, completion, cache read, cache creation, reasoning), the provider/model, the API key, the underlying provider connection, time-to-first-token, latency, and a USD cost estimate. It exposes three read APIs (`GET /api/usage`, `GET /api/usage/summary`, `GET /api/usage/cost`) for dashboard and cost analysis. Fire-and-forget audit call sites get instrumented so dropped records surface as metric counters, not silent gaps.

---

## Scope

### In Scope

- `V2__usage_history.sql` migration creating a new `usage_history` table
- New `UsageRecorderPort` port trait with write + read methods in `rook-core`
- `UsageEntry` domain struct with all 5 token dimensions, `ttft_ms`, `service_tier`, `connection_id`, `api_key_id`
- Cost estimation via a top-level `[pricing]` TOML config table per `<provider>.<model>`
- Propagation of `connection_id` and `api_key_id` through the request flow via existing port methods
- TTFT measurement in `route_request.rs` (use case layer)
- `GET /api/usage`, `GET /api/usage/summary`, `GET /api/usage/cost` handlers and routes
- Retention sweep (startup + every 6 hours)
- Provider adapter extensions for OpenAI (reasoning tokens) and Anthropic (cache tokens)
- Instrumentation: `tracing::warn!` + metric counter for all four fire-and-forget audit call sites

### Out of Scope

- New API-key scopes (`usage:read`, `usage:admin`) — MANAGEMENT auth is sufficient
- Ollama, Gemini, Groq adapter full implementations — they remain stubs; `api_key_id` and `connection_id` will be `None` for these providers until a follow-up
- Backfill of historical `audit` rows into `usage_history`
- Deprecation or removal of the existing `audit` table and `AuditPort`
- Dashboard UI changes

---

## Capabilities

### New Capabilities

- **`usage-history`**: A new SQLite table recording every routed request's token usage, latency, cost, tier, and IDs. Supports paginated list queries, summary aggregations (by model, provider, day), and cost breakdowns. TTL: 90 days default, configurable.

### Modified Capabilities

- **`audit-log`**: Unchanged for writes. Existing rows are preserved; new writes go to `usage_history`. `AuditPort` remains but is considered deprecated. Fire-and-forget call sites getwarn-level instrumentation to detect silent drops.

---

## Approach

### Coexisting table strategy

The existing `audit` table stays writable during the transition. A new `usage_history` table becomes the canonical write target for the new `UsageRecorderPort`. The two tables coexist — no migration of historical `audit` rows, no `DROP`, no `ALTER`. This gives:

- Zero blast radius on existing dashboards or scripts reading `audit`
- A clean schema for the new shape without accumulating `NULL` columns
- A separable follow-up to deprecate `audit` once the new API is stable

The `AuditPort` is not removed. Fire-and-forget audit sites continue writing to `audit`; the new sites write to `usage_history`. At query time, both can be joined or presented separately.

### API key / connection ID propagation

Two propagation gaps identified in `exploration.md:155-167` are closed by this change:

**`api_key_id`** — The authz middleware (`authz.rs:566-630`) already stamps `api_key_id` into the `x-authz-auth-id` request header. A new extractor function reads this header and passes `Option<ApiKeyId>` into `RequestMetadata`. This mirrors the existing pattern for `ApiKeyRestrictions` on `CompletionRequest` (`model.rs:147-151`).

**`connection_id`** — The `ProviderId` is available after `router.select()`. The `ConnectionId` requires a lookup via `ProviderRepositoryPort` using a new `find_connection_id_by_runtime(&ProviderId) -> Option<ConnectionId>` method. TOML providers (legacy) have no row in `provider_connections`; the lookup is best-effort and returns `None` for those cases. Both fields are `Option` on `UsageEntry`.

### TTFT measurement

TTFT is measured in `route_request.rs` — the use case layer — using `std::time::Instant` when the first successful `StreamChunk` arrives. The provider adapter does not change. This keeps the measurement domain-aligned and avoids coupling adapters to a non-domain concept. The measurement is recorded on the `StreamChunk` or captured in the async stream block before the audit record is written.

### Retention sweep

SQLite has no native TTL. The sweep runs as a `tokio` task:

- First sweep at startup (before the server accepts traffic)
- Subsequent sweeps every 6 hours

Sweep deletes from `usage_history` where `timestamp < now() - retention_days`. The interval and default (90 days) are configurable via `config.usage.retention_days`.

---

## Cost Calculation Design

### Pricing TOML shape

Pricing is a top-level `[pricing]` table in `RookConfig`, keyed by `<provider>.<model>`:

```toml
[pricing]
[pricing.openai.gpt-4o]
prompt_per_million = 2.50
completion_per_million = 10.00

[pricing.openai.gpt-4o-mini]
prompt_per_million = 0.15
completion_per_million = 0.60

[pricing.anthropic.claude-sonnet-4-7]
prompt_per_million = 3.00
completion_per_million = 15.00

[pricing.anthropic.claude-opus-4]
prompt_per_million = 15.00
completion_per_million = 75.00

[pricing.ollama.llama3]
prompt_per_million = 0.0   # local — no cost
completion_per_million = 0.0

[pricing.groq."llama-3.3-70b"]
prompt_per_million = 0.59
completion_per_million = 2.40
```

Cache token pricing follows the provider's actual billing model (cache creation is billed at full prompt price on Anthropic; cache read is discounted). The formula handles missing price entries with a `tracing::warn!` per request and a metric counter, surfacing "cost unknown" in the UI rather than `$0.00`.

### Formula

```
cost_usd =
  (prompt_tokens      * price_prompt_per_token)   +
  (completion_tokens  * price_completion_per_token) +
  (cache_read_tokens  * price_cache_read_per_token)  +
  (cache_creation_tokens * price_cache_creation_per_token)
```

All prices in dollars per million tokens; divided by 1,000,000 for per-token cost.

---

## Migration Strategy

### V2 migration

`V2__usage_history.sql` creates the new table alongside the existing `audit` table. The migration is additive — no existing migration is modified. The `audit_sqlite` crate's inline `CREATE TABLE IF NOT EXISTS audit` block remains; it will no longer receive new writes but stays valid for historical reads.

Schema for `usage_history`:

| Column                  | Type                                | Notes                                          |
|-------------------------|-------------------------------------|------------------------------------------------|
| `id`                    | `INTEGER PRIMARY KEY AUTOINCREMENT` |                                                |
| `request_id`            | `TEXT NOT NULL`                     | from `RequestId`                               |
| `provider`              | `TEXT NOT NULL`                     | e.g. `"openai"`                                |
| `model`                 | `TEXT NOT NULL`                     | e.g. `"gpt-4o"`                                |
| `status`                | `TEXT NOT NULL`                     | `Success \| Failure \| RateLimited \| Timeout` |
| `requested_tier`        | `TEXT`                              | what the client sent; `Standard \| Premium`    |
| `api_key_id`            | `TEXT`                              | `ApiKeyId` as text, nullable                   |
| `connection_id`         | `TEXT`                              | `ConnectionId` as text, nullable               |
| `tokens_prompt`         | `INTEGER`                           |                                                |
| `tokens_completion`     | `INTEGER`                           |                                                |
| `tokens_cache_read`     | `INTEGER`                           |                                                |
| `tokens_cache_creation` | `INTEGER`                           |                                                |
| `tokens_reasoning`      | `INTEGER`                           |                                                |
| `ttft_ms`               | `INTEGER`                           | time to first token in ms                      |
| `latency_ms`            | `INTEGER NOT NULL`                  |                                                |
| `cost_usd`              | `REAL`                              | calculated, nullable if pricing missing        |
| `timestamp`             | `TEXT NOT NULL`                     | ISO 8601                                       |

Indexes: `(request_id)`, `(provider)`, `(model)`, `(timestamp)`, `(api_key_id)`, `(connection_id)`.

### Backfill decision

No backfill. Existing `audit` rows remain in the `audit` table and are not migrated. The `audit` table is preserved as read-only historical record. The `usage_history` table grows from the moment the new code ships. Cost dashboards can query both tables separately during the transition period.

---

## New Port: UsageRecorderPort

### Interface design

```rust
// rook-core/src/ports.rs

pub trait UsageRecorderPort: Send + Sync {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()>;
    async fn list(
        &self,
        filters: UsageFilters,
        pagination: Pagination,
    ) -> CortexResult<Vec<UsageEntry>>;
    async fn summary(&self, filters: UsageFilters) -> CortexResult<UsageSummary>;
    async fn cost_breakdown(
        &self,
        filters: UsageFilters,
    ) -> CortexResult<CostBreakdown>;
}

#[derive(Clone)]
pub struct UsageFilters {
    pub provider: Option<ProviderId>,
    pub model: Option<ModelId>,
    pub api_key_id: Option<ApiKeyId>,
    pub connection_id: Option<ConnectionId>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub status: Option<RequestStatus>,
}

#[derive(Clone)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,
}

pub struct UsageSummary {
    pub total_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_reasoning_tokens: u64,
    pub avg_ttft_ms: f64,
    pub avg_latency_ms: f64,
}

pub struct CostBreakdown {
    pub total_cost_usd: f64,
    pub by_provider: HashMap<ProviderId, f64>,
    pub by_model: HashMap<ModelId, f64>,
    pub by_api_key: HashMap<ApiKeyId, f64>,
}
```

### Query methods

- **`list`**: paginated rows with all fields. Supports filtering by provider, model, api_key_id, connection_id, date range, and status. Ordered by `timestamp` descending.
- **`summary`**: aggregated counts and averages across the filtered set — used by `GET /api/usage/summary`.
- **`cost_breakdown`**: sum of `cost_usd` grouped by provider, model, and API key — used by `GET /api/usage/cost`.

The `UsageRecorderPort` is **optional / nullable** in `RookUsecases` — the same pattern as `manage_connections` and `manage_api_keys` at `rook-usecases/src/lib.rs:46-47`. The port is represented as `Option<Arc<dyn UsageRecorderPort>>`; when `None`, usage recording is silently skipped (fire-and-forget semantics preserved, no request failures).

---

## Affected Areas

| Area                                                                      | Impact   | Description                                                                                                                                                                         |
|---------------------------------------------------------------------------|----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/domain/rook-core/src/model.rs`                                    | Modified | New `UsageEntry` struct, `UsageFilters`, `Pagination`, `UsageSummary`, `CostBreakdown` types; cache/reasoning/ttft fields added to `TokenUsage`                                     |
| `crates/domain/rook-core/src/ports.rs`                                    | Modified | New `UsageRecorderPort` trait; `ProviderRepositoryPort` gains `find_connection_id_by_runtime`                                                                                       |
| `crates/application/rook-usecases/src/route_request.rs`                   | Modified | `connection_id`/`api_key_id` propagated; TTFT captured at first chunk; four audit call sites instrumented with warn + metric                                                        |
| `crates/infrastructure/audit-sqlite/src/lib.rs`                           | Modified | V2 migration runs; `usage_history` table created; `SqliteUsageRepository` implements `UsageRecorderPort`                                                                            |
| `crates/infrastructure/db-migration/src/migrations/V2__usage_history.sql` | New      | Creates `usage_history` table + indexes                                                                                                                                             |
| `crates/infrastructure/transport-axum/src/handlers/usage.rs`              | New      | `GET /usage`, `/usage/summary`, `/usage/cost` handlers                                                                                                                              |
| `crates/infrastructure/transport-axum/src/routes.rs`                      | Modified | New `usage_routes()` builder merged into router                                                                                                                                     |
| `crates/infrastructure/transport-axum/src/authz.rs`                       | Modified | New extractor for `x-authz-auth-id` header → `Option<ApiKeyId>`                                                                                                                     |
| `crates/infrastructure/providers-openai/src/provider.rs`                  | Modified | `OpenAIUsage` extended with `prompt_tokens_details.cached_tokens` and `completion_tokens_details.reasoning_tokens`; `stream_options.include_usage: true` added to streaming request |
| `crates/infrastructure/providers-anthropic/src/lib.rs`                    | Modified | `AnthropicNonStreamUsage` and `AnthropicMessageDeltaUsage` extended with `cache_creation_input_tokens` and `cache_read_input_tokens`                                                |
| `crates/infrastructure/providers-ollama/src/lib.rs`                       | Modified | Stub replaced with partial `TokenUsage` from `prompt_eval_count`/`eval_count` (no cache/reasoning tokens)                                                                           |
| `crates/infrastructure/providers-gemini/src/lib.rs`                       | Modified | Stub replaced with partial `TokenUsage` from `usageMetadata`                                                                                                                        |
| `crates/infrastructure/providers-groq/src/lib.rs`                         | Modified | Stub replaced with partial `TokenUsage` (OpenAI-compatible format; reuse OpenAI parser)                                                                                             |
| `apps/rook/src/di.rs`                                                     | Modified | Wire `SqliteUsageRepository` as `Arc<dyn UsageRecorderPort>`; wire extractor for `api_key_id`                                                                                       |
| `apps/rook/src/config.rs`                                                 | Modified | New `[usage]` section (retention_days) and `[pricing.<provider>.<model>]` section                                                                                                   |
| `docs/configuration.md`                                                   | Modified | Document `[usage]` and `[pricing]` sections                                                                                                                                         |

---

## Risks

| #   | Risk                                                                                | Likelihood | Mitigation                                                                                                                         |
|-----|-------------------------------------------------------------------------------------|------------|------------------------------------------------------------------------------------------------------------------------------------|
| R1  | Fire-and-forget audit silently drops records under DB pressure                      | High       | `tracing::warn!` + `usage_record_failed_total` metric counter on all four call sites                                               |
| R2  | Ollama/Gemini/Groq stubs return all-NULL token fields                               | High       | Document in spec which providers return complete vs partial; `connection_id` and `api_key_id` are `None` until adapters are filled |
| R3  | Anthropic streaming does not return cache token fields on `message_delta`           | Medium     | Pin `anthropic-version` header; document cache tokens as best-effort on streaming; non-streaming path returns full data            |
| R4  | `connection_id` not available for TOML providers (no row in `provider_connections`) | Medium     | Field is `Option<ConnectionId>`; lookup is best-effort; `None` is acceptable                                                       |
| R5  | `api_key_id` not propagated to `CompletionRequest` today                            | Medium     | New extractor reads `x-authz-auth-id` header; mirrors existing `ApiKeyRestrictions` pattern; add test to assert header presence    |
| R6  | `service_tier` semantics ambiguous (requested vs billed)                            | Medium     | Decision G4: record **requested tier** (what client sent in the request)                                                           |
| R7  | Missing pricing config silently produces `$0.00` cost                               | High       | `tracing::warn!` + `usage_cost_unknown_total` counter per request; UI surfaces "cost unknown"                                      |
| R8  | Retention sweep never runs if process does not restart                              | Low        | Startup sweep + periodic 6h sweep; both fire even if startup sweep was delayed                                                     |
| R9  | Spec conflicts with `api-key-scopes-and-restrictions` (defers audit changes)        | Low        | That change is archived and complete; no active conflict                                                                           |
| R10 | `authz.rs` header pipeline change breaks `api_key_id` propagation                   | Low        | New extractor is a dedicated function, not inline; test coverage added                                                             |

---

## Open Questions Resolved

| #   | Question                                   | Resolution                                                                                |
|-----|--------------------------------------------|-------------------------------------------------------------------------------------------|
| G1  | Replace or coexist `audit`?                | **Coexist** — keep `audit` deprecated, add `usage_history` as canonical new table         |
| G2  | New port or extend `AuditPort`?            | **New `UsageRecorderPort`** with write + read methods; `AuditPort` unchanged              |
| G3  | Pricing config location?                   | **Top-level `[pricing]` TOML** per `<provider>.<model>`                                   |
| G4  | `service_tier` semantics?                  | **Record requested tier** — what client sent in the request                               |
| G5  | TTFT measurement layer?                    | **Use case** (`route_request.rs`) — `Instant::now()` at first chunk arrival               |
| G6  | Token field completeness across providers? | **v1 ships full schema; Anthropic + OpenAI return full data; Ollama/Gemini/Groq partial** |
| G7  | Retention enforcement?                     | **Sweep job at startup + every 6h**                                                       |
| G8  | `/api/usage*` routes always-on?            | **Always-on** — no config toggle                                                          |
| G9  | New API-key scope for usage?               | **No** — MANAGEMENT auth suffices                                                         |
| G10 | Backfill existing `audit` rows?            | **No** — leave existing rows; new writes go to `usage_history`                            |

---

## Next Phase Input for sdd-spec

`sdd-spec` must nail down the following before implementation:

- **Exact SQL column definitions and types** for `usage_history` — confirm `requested_tier` as `TEXT NOT NULL` or `TEXT`, nullable — and whether `ttft_ms` should be `INTEGER` or `REAL`
- **`UsageRecorderPort` method signatures** — confirm `list` returns `Vec<UsageEntry>` or `impl Stream`; confirm `summary` and `cost_breakdown` are separate methods or can be merged
- **`UsageEntry` field list** — confirm which token fields are required vs optional per provider; confirm `api_key_id` and `connection_id` are `Option` everywhere
- **`ProviderRepositoryPort::find_connection_id_by_runtime`** — exact signature and whether it returns `Option<ConnectionId>` or `CortexResult<Option<ConnectionId>>`
- **Which provider parses which token fields** — explicit mapping: OpenAI → `reasoning_tokens`; Anthropic → `cache_read` + `cache_creation`; Ollama → `prompt_eval_count`/`eval_count` only; Gemini → `promptTokenCount`/`candidatesTokenCount`; Groq → OpenAI-compatible
- **`TokenUsage` or new struct** for carrying all 5 token dimensions through the response — whether to extend the existing `TokenUsage` or introduce a `TokenUsageDetail` struct alongside it
- **`RetentionConfig`** — exact field name (`retention_days: u32`), default of 90, whether it lives in `UsageConfig` or as a top-level config field
- **Retention sweep timing** — confirm 6h interval; whether it should be configurable via `[usage].sweep_interval_hours`
- **Metric counter names** — agree on `usage_record_failed_total` and `usage_cost_unknown_total` as the metric names to emit
- **`Pagination` defaults** — default `limit` (e.g. 100) and max `limit` (e.g. 1000)
- **Route path definitions** — confirm `/api/usage`, `/api/usage/summary`, `/api/usage/cost` vs alternative paths like `/api/v1/usage/*`
- **OpenAPI / `ApiResponse` format** — confirm what the list response wraps (e.g. `{ entries: [...], total: u64 }`) and what error shape is returned on `usage_record_failed`
