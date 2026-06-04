# Technical Design: Usage Tracking — Token Counts & Cost Estimation

This design implements the approved proposal in `openspec/changes/usage-tracking-token-counts-cost-estimation/proposal.md` and satisfies the delta spec in `specs/usage-history/spec.md`. It preserves the legacy `audit` table and `AuditPort` while adding a new canonical `usage_history` table and `UsageRecorderPort` for token/cost analytics.

## § Design Decisions

1. **DD1: Coexist with legacy audit.** Keep `AuditEntry`/`AuditPort` and the `audit` table unchanged (`model.rs:359-406`, `ports.rs:119-122`, `V0__initial.sql:99-115`) and add `usage_history` via `V2__usage_history.sql`. This avoids checksum changes to existing migrations and avoids breaking external readers.
2. **DD2: New usage port, not expanded audit port.** Add `UsageRecorderPort` beside `AuditPort` in `crates/domain/rook-core/src/ports.rs`. The port owns both recording and read APIs because the UI needs list, summary, and cost breakdown from the same store.
3. **DD3: Nullable usage recorder.** `RookUsecases` carries `pub usage_recorder: Option<Arc<dyn UsageRecorderPort>>`; `RouteRequest` receives the same nullable port and silently skips when `None`. This preserves fire-and-forget semantics required by `spec.md:46-48` while using a valid sized Rust trait-object shape.
4. **DD4: Optional token dimensions.** `TokenUsage` keeps required prompt/completion/total counters and adds `Option<u64>` for cache/reasoning dimensions. Providers that cannot report a dimension store `NULL` instead of `0` so dashboards can distinguish “not reported” from “reported zero”.
5. **DD5: UsageEntry stores flattened token fields.** `UsageEntry` stores token fields directly instead of embedding `TokenUsage`, matching the required SQL column names and making aggregation SQL simple.
6. **DD6: `requested_tier` is the requested service tier.** The spec-approved meaning is “what the client asked for”, not provider-billed tier. The field remains nullable because current domain requests do not yet model service tiers for every wire format.
7. **DD7: API key ID is extracted in transport.** `authz.rs` already stamps `x-authz-auth-id`; route handlers read it before constructing `CompletionRequest`, mirroring the existing restrictions extraction in `routes.rs:280-314`.
8. **DD8: Connection ID lookup is best-effort.** `ProviderRepositoryPort::find_connection_id_by_runtime(&ProviderId)` returns `Ok(None)` for TOML/non-CRUD providers. This matches `ARCHITECTURE.md:104-137`, where runtime providers may be registry-backed and provider CRUD can be disabled.
9. **DD9: TTFT belongs in the use case layer.** `route_request.rs` already owns `Instant::now()` and stream completion auditing (`route_request.rs:146-218`), so first-chunk timing is measured there without coupling providers to observability.
10. **DD10: Cost is calculated in application layer.** Provider adapters only parse token usage; `RouteRequest` applies local `[pricing.<provider>.<model>]` configuration so pricing updates do not require provider changes.
11. **DD11: Unknown cost is `NULL`, never `$0`.** Missing pricing emits `tracing::warn!(usage_cost_unknown_total = 1, pricing_missing = true, ...)`, increments `usage_cost_unknown_total`, and stores `None` in `UsageEntry.cost_usd`.
12. **DD12: Retention is repository-owned, task-started.** `SqliteUsageRepository` exposes a concrete `delete_older_than(retention_days: u32)` helper; `apps/rook/src/server.rs` starts the startup/periodic task using app config.
13. **DD13: Management auth is inherited.** `/api/usage*` routes are mounted under `/api/` and `authz.rs:384-401` classifies them as `Management`; no new usage scope is added.
14. **DD14: Pagination total requires a port count helper.** The spec says the list endpoint returns `{ entries, total }`, while `list()` returns only the current page. `UsageRecorderPort::count(filters)` is part of the approved interface so handlers can compute totals without leaking SQLite details or downcasting trait objects.
15. **DD15: Keep migrations additive and ordered.** `V2__usage_history.sql` runs after `V0__initial.sql` and `V1__allowed_models_providers.sql` via existing refinery migration ordering; do not edit `V0` or `V1`.

## § Domain Layer

### UsageEntry struct

Add near the existing audit section in `crates/domain/rook-core/src/model.rs` after `AuditEntry` (`model.rs:359-406`) so request status and token usage types are in scope. Import `ConnectionId` at `model.rs:7` alongside `ModelId`, `ProviderId`, and `RequestId`, and import/re-export `ApiKeyId` if not already in `lib.rs`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub request_id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub status: RequestStatus,
    pub requested_tier: Option<String>,
    pub api_key_id: Option<ApiKeyId>,
    pub connection_id: Option<ConnectionId>,
    pub tokens_prompt: Option<u64>,
    pub tokens_completion: Option<u64>,
    pub tokens_cache_read: Option<u64>,
    pub tokens_cache_creation: Option<u64>,
    pub tokens_reasoning: Option<u64>,
    pub ttft_ms: Option<u64>,
    pub latency_ms: u64,
    pub cost_usd: Option<f64>,
    pub timestamp: DateTime<Utc>,
}
```

Field count: 16 domain fields. The SQLite table has 17 columns because it adds storage-only `id INTEGER PRIMARY KEY AUTOINCREMENT`; do not put `id` on `UsageEntry` unless the UI later requires row identity.

`Option` fields and reasons:

- `requested_tier`: nullable because current `CompletionRequest` has no service-tier field and not all providers accept one.
- `api_key_id`: nullable for internal calls, env fallback keys, or middleware bypass tests.
- `connection_id`: nullable for TOML providers and when `provider_crud.enabled = false`.
- `tokens_*`: nullable when provider did not report the field. Prompt/completion are still nullable in `UsageEntry` because failures and stub providers can record no usage.
- `ttft_ms`: nullable for failures before a first chunk; non-streaming success sets it equal to `latency_ms` per `spec.md:120-125`.
- `cost_usd`: nullable when pricing is missing.

Delta from `AuditEntry` at `model.rs:359-406`:

```rust
// AuditEntry today: 7 fields, embeds Option<TokenUsage>, no identity metadata.
pub struct AuditEntry {
    pub request_id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub status: RequestStatus,
    pub usage: Option<TokenUsage>,
    pub latency_ms: u64,
    pub timestamp: DateTime<Utc>,
}

// UsageEntry: flattened token columns + requested_tier + api_key_id + connection_id + ttft + cost.
```

### Extended TokenUsage

Modify `TokenUsage` at `crates/domain/rook-core/src/model.rs:286-293`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    /// Deprecated for provider adapters; usage cost is calculated in RouteRequest.
    pub estimated_cost_usd: Option<f64>,
}
```

All existing tests that construct `TokenUsage` must add `None` for the three new fields. Keep `estimated_cost_usd` for compatibility with `AuditEntry` and transport responses, but new usage cost uses `UsageEntry.cost_usd`.

### UsageFilters, Pagination, UsageSummary, CostBreakdown

Place these in `crates/domain/rook-core/src/model.rs` after `UsageEntry`, and export them from `rook-core/src/lib.rs`.

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageFilters {
    pub provider: Option<ProviderId>,
    pub model: Option<ModelId>,
    pub api_key_id: Option<ApiKeyId>,
    pub connection_id: Option<ConnectionId>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub status: Option<RequestStatus>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { offset: 0, limit: 100 }
    }
}

impl Pagination {
    pub const DEFAULT_LIMIT: u64 = 100;
    pub const MAX_LIMIT: u64 = 1000;

    pub fn clamped(self) -> Self {
        Self { offset: self.offset, limit: self.limit.min(Self::MAX_LIMIT) }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_reasoning_tokens: u64,
    pub avg_ttft_ms: Option<f64>,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub total_cost_usd: f64,
    pub by_provider: HashMap<ProviderId, f64>,
    pub by_model: HashMap<ModelId, f64>,
    pub by_api_key: HashMap<ApiKeyId, f64>,
}
```

`avg_ttft_ms` is `Option<f64>` because all matching rows may have `NULL` `ttft_ms`.

## § Port Layer

### UsageRecorderPort trait

Add to `crates/domain/rook-core/src/ports.rs` after `AuditPort` (`ports.rs:119-122`). Imports: add `UsageEntry`, `UsageFilters`, `Pagination`, `UsageSummary`, `CostBreakdown` to the `super::{...}` list.

```rust
#[async_trait]
pub trait UsageRecorderPort: Send + Sync {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()>;

    async fn list(
        &self,
        filters: UsageFilters,
        pagination: Pagination,
    ) -> CortexResult<Vec<UsageEntry>>;

    async fn count(&self, filters: UsageFilters) -> CortexResult<u64>;

    async fn summary(&self, filters: UsageFilters) -> CortexResult<UsageSummary>;

    async fn cost_breakdown(
        &self,
        filters: UsageFilters,
    ) -> CortexResult<CostBreakdown>;
}
```

The spec requires these 5 methods. `count` exists to implement the specified list response `{ entries, total }` without leaking SQLite details into transport or requiring handlers to downcast the trait object.

Nullable wiring shape in `RookUsecases`:

```rust
pub struct RookUsecases {
    pub route_request: RouteRequest,
    // ... existing fields at lib.rs:42-57
    pub usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
}
```

And `RouteRequest`:

```rust
pub struct RouteRequest {
    router: Arc<dyn RouterPort>,
    cache: Arc<dyn CachePort>,
    audit: Arc<dyn AuditPort>,
    usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
    provider_repository: Option<Arc<dyn ProviderRepositoryPort>>,
    pricing: Arc<PricingConfig>,
    format_translator: Arc<dyn FormatTranslatorPort>,
}
```

### ProviderRepositoryPort new method

Modify `crates/domain/rook-core/src/ports.rs:160-171`:

```rust
#[async_trait]
pub trait ProviderRepositoryPort: Send + Sync {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError>;
    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError>;
    async fn find_connection_id_by_runtime(
        &self,
        provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, RepositoryError>;
    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError>;
    async fn update(
        &self,
        conn: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError>;
}
```

Implementation in `crates/infrastructure/provider-sqlite/src/repository.rs` uses the existing `provider_runtime_id` column selected at `repository.rs:157-164`:

```rust
async fn find_connection_id_by_runtime(
    &self,
    provider: &ProviderId,
) -> Result<Option<ConnectionId>, RepositoryError> {
    let conn = self.lock()?;
    conn.query_row(
        "SELECT id FROM provider_connections WHERE provider_runtime_id = ?1 AND is_active = 1 ORDER BY priority ASC, created_at DESC LIMIT 1",
        params![provider.to_string()],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(db_error)?
    .map(|id| ConnectionId::parse_str(&id).map_err(|e| RepositoryError::Database(e.to_string())))
    .transpose()
}
```

## § Infrastructure Layer

### V2 migration

Create `crates/infrastructure/db-migration/src/migrations/V2__usage_history.sql`. This runs after existing `V0__initial.sql` (`audit` table at `V0:99-115`) and `V1`; no existing migration is modified.

```sql
-- =============================================================================
-- usage_history table
-- =============================================================================
CREATE TABLE usage_history (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id              TEXT NOT NULL,
    provider                TEXT NOT NULL,
    model                   TEXT NOT NULL,
    status                  TEXT NOT NULL,
    requested_tier          TEXT,
    api_key_id              TEXT,
    connection_id           TEXT,
    tokens_prompt           INTEGER,
    tokens_completion       INTEGER,
    tokens_cache_read       INTEGER,
    tokens_cache_creation   INTEGER,
    tokens_reasoning        INTEGER,
    ttft_ms                 INTEGER,
    latency_ms              INTEGER NOT NULL,
    cost_usd                REAL,
    timestamp               TEXT NOT NULL
);

CREATE INDEX idx_usage_history_request_id ON usage_history(request_id);
CREATE INDEX idx_usage_history_provider ON usage_history(provider);
CREATE INDEX idx_usage_history_model ON usage_history(model);
CREATE INDEX idx_usage_history_timestamp ON usage_history(timestamp);
CREATE INDEX idx_usage_history_api_key_id ON usage_history(api_key_id);
CREATE INDEX idx_usage_history_connection_id ON usage_history(connection_id);
```

### SqliteUsageRepository

Modify `crates/infrastructure/audit-sqlite/src/lib.rs`; keep `SqliteAudit` unchanged and add `SqliteUsageRepository` below it. Constructor mirrors `SqliteAudit::new(db_path: impl AsRef<Path>) -> anyhow::Result<Self>` at `lib.rs:30-55`, but design requires exact public signature:

```rust
pub struct SqliteUsageRepository {
    conn: Mutex<Connection>,
}

impl SqliteUsageRepository {
    pub fn new(db_path: &Path) -> CortexResult<Self>;

    async fn execute_with_options(
        &self,
        base_sql: &str,
        filters: &UsageFilters,
        pagination: Option<Pagination>,
    ) -> CortexResult<Vec<UsageEntry>>;

    pub async fn delete_older_than(&self, retention_days: u32) -> CortexResult<u64>;
}
```

`new()` opens SQLite, sets `PRAGMA foreign_keys = ON`, and relies on startup migrations from `apps/rook/src/di.rs:43-47`; it may also run `CREATE TABLE IF NOT EXISTS usage_history` defensively only if the SQL exactly matches `V2`.

`record()` SQL:

```sql
INSERT INTO usage_history (
    request_id, provider, model, status, requested_tier, api_key_id, connection_id,
    tokens_prompt, tokens_completion, tokens_cache_read, tokens_cache_creation,
    tokens_reasoning, ttft_ms, latency_ms, cost_usd, timestamp
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
```

`list()` SQL assembled by `execute_with_options()`:

```sql
SELECT request_id, provider, model, status, requested_tier, api_key_id, connection_id,
       tokens_prompt, tokens_completion, tokens_cache_read, tokens_cache_creation,
       tokens_reasoning, ttft_ms, latency_ms, cost_usd, timestamp
FROM usage_history
WHERE (?1 IS NULL OR provider = ?1)
  AND (?2 IS NULL OR model = ?2)
  AND (?3 IS NULL OR api_key_id = ?3)
  AND (?4 IS NULL OR connection_id = ?4)
  AND (?5 IS NULL OR timestamp >= ?5)
  AND (?6 IS NULL OR timestamp <= ?6)
  AND (?7 IS NULL OR status = ?7)
ORDER BY timestamp DESC
LIMIT ?8 OFFSET ?9
```

`count()` SQL:

```sql
SELECT COUNT(*) FROM usage_history
WHERE (?1 IS NULL OR provider = ?1)
  AND (?2 IS NULL OR model = ?2)
  AND (?3 IS NULL OR api_key_id = ?3)
  AND (?4 IS NULL OR connection_id = ?4)
  AND (?5 IS NULL OR timestamp >= ?5)
  AND (?6 IS NULL OR timestamp <= ?6)
  AND (?7 IS NULL OR status = ?7)
```

`summary()` SQL:

```sql
SELECT COUNT(*) AS total_requests,
       COALESCE(SUM(tokens_prompt), 0) AS total_prompt_tokens,
       COALESCE(SUM(tokens_completion), 0) AS total_completion_tokens,
       COALESCE(SUM(tokens_cache_read), 0) AS total_cache_read_tokens,
       COALESCE(SUM(tokens_cache_creation), 0) AS total_cache_creation_tokens,
       COALESCE(SUM(tokens_reasoning), 0) AS total_reasoning_tokens,
       AVG(ttft_ms) AS avg_ttft_ms,
       COALESCE(AVG(latency_ms), 0) AS avg_latency_ms
FROM usage_history
WHERE ...same filters...
```

`cost_breakdown()` runs four queries using the same filter builder:

```sql
SELECT COALESCE(SUM(cost_usd), 0) FROM usage_history WHERE ...same filters...;
SELECT provider, COALESCE(SUM(cost_usd), 0) FROM usage_history WHERE ...same filters... GROUP BY provider;
SELECT model, COALESCE(SUM(cost_usd), 0) FROM usage_history WHERE ...same filters... GROUP BY model;
SELECT api_key_id, COALESCE(SUM(cost_usd), 0) FROM usage_history WHERE ...same filters... AND api_key_id IS NOT NULL GROUP BY api_key_id;
```

`delete_older_than()` SQL:

```sql
DELETE FROM usage_history
WHERE timestamp < datetime('now', '-' || ?1 || ' days')
```

### Retention sweep task

Implement in `apps/rook/src/server.rs` or new `apps/rook/src/usage_retention.rs`. Preferred new module keeps server bootstrap readable:

```rust
pub async fn run_startup_usage_retention_sweep(
    usage_repo: &SqliteUsageRepository,
    retention_days: u32,
) -> CortexResult<u64>;

pub fn spawn_periodic_usage_retention_sweep(
    usage_repo: Arc<SqliteUsageRepository>,
    retention_days: u32,
    sweep_interval_hours: u32,
) -> tokio::task::JoinHandle<()>;
```

Behavior:

1. Before binding/serving in `server.rs:28-31`, await `run_startup_usage_retention_sweep(...)` so expired rows are deleted before traffic is accepted.
2. After the startup sweep succeeds (or after logging and deciding to continue on failure), call `spawn_periodic_usage_retention_sweep(...)`.
3. The periodic task loops on `tokio::time::interval(Duration::from_secs(sweep_interval_hours as u64 * 3600))`.
4. Log failures with `tracing::warn!(error = %e, "usage retention sweep failed")`.

Exact delete SQL uses configured retention, default 90:

```sql
DELETE FROM usage_history WHERE timestamp < datetime('now', '-90 days')
```

Implementation should parameterize `90` with `retention_days`.

## § Use Case Layer

### Propagation: api_key_id and connection_id

Domain metadata change in `crates/domain/rook-core/src/model.rs:164-172`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub origin: String,
    pub cacheable: bool,
    pub priority: u8,
    pub api_key_id: Option<ApiKeyId>,
    pub requested_tier: Option<String>,
}
```

Transport route handlers (`routes.rs:315-356` OpenAI, `routes.rs:604-646` Anthropic) call `authz::extract_api_key_id(&request)` before `CompletionRequest` is constructed. Because current handlers accept `HeaderMap`, design implementation either changes extractor to accept `&HeaderMap` or changes handler extractor to `Request`; approved requirement says `fn extract_api_key_id(req: &Request) -> Option<ApiKeyId>`, so the handler should use `Request` extraction and split body after reading headers.

Connection lookup in `RouteRequest` after provider selection (`route_request.rs:73-75`, `155-156`):

```rust
let connection_id = if let Some(repo) = self.provider_repository.as_ref() {
    match repo.find_connection_id_by_runtime(&provider_id).await {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(provider = %provider_id, error = %error, "failed to resolve provider connection id");
            None
        }
    }
} else {
    None
};
```

### TTFT measurement

Modify the streaming block at `crates/application/rook-usecases/src/route_request.rs:170-218`.

Current block stores `final_usage` only. New delta:

```rust
let stream = async_stream::try_stream! {
    let mut final_usage: Option<TokenUsage> = None;
    let mut ttft_ms: Option<u64> = None;

    while let Some(chunk) = upstream.next().await {
        match chunk {
            Ok(chunk) => {
                if ttft_ms.is_none() {
                    ttft_ms = Some(start.elapsed().as_millis() as u64);
                }
                if chunk.usage.is_some() {
                    final_usage = chunk.usage.clone();
                }
                yield chunk;
            }
            Err(error) => {
                // failure UsageEntry uses ttft_ms captured so far, or None
            }
        }
    }

    let latency_ms = start.elapsed().as_millis() as u64;
    let usage_entry = build_usage_entry(
        &request_id,
        &provider_id,
        &model,
        RequestStatus::Success,
        final_usage.as_ref(),
        ttft_ms,
        latency_ms,
        api_key_id.clone(),
        connection_id.clone(),
    );
}
```

Non-streaming success sets `ttft_ms = Some(latency_ms)` immediately after `provider.complete()` returns (`route_request.rs:91-94`). Failures before usage availability set `ttft_ms = None`.

### Cost calculation

Add immutable pricing to `RouteRequest` constructor:

```rust
pricing: Arc<PricingConfig>
```

Helper signature in `route_request.rs`:

```rust
fn estimate_cost_usd(
    pricing: &PricingConfig,
    provider: &ProviderId,
    model: &ModelId,
    usage: Option<&TokenUsage>,
) -> Option<f64>;
```

Formula:

```rust
let cost_usd =
    (prompt_tokens as f64 * entry.prompt_per_million / 1_000_000.0) +
    (completion_tokens as f64 * entry.completion_per_million / 1_000_000.0) +
    (cache_read_tokens as f64 * entry.cache_read_per_million.unwrap_or(entry.prompt_per_million) / 1_000_000.0) +
    (cache_creation_tokens as f64 * entry.cache_creation_per_million.unwrap_or(entry.prompt_per_million) / 1_000_000.0);
```

Missing pricing:

```rust
tracing::warn!(
    usage_cost_unknown_total = 1,
    pricing_missing = true,
    provider = %provider,
    model = %model,
    "usage cost unavailable because pricing entry is missing"
);
metrics::counter!("usage_cost_unknown_total").increment(1);
None
```

### Fire-and-forget instrumentation

Instrument four audit/usage record call sites in `route_request.rs`: non-streaming success (`110-119`), streaming failure (`193-202`), streaming success (`208-217`), non-streaming failure (`239-242`). Use this pattern for both `AuditPort` and `UsageRecorderPort` failures:

```rust
if let Err(error) = usage.record(entry).await {
    tracing::warn!(
        usage_record_failed = true,
        request_id = %request_id,
        provider = %provider_id,
        error = %error,
        "failed to record usage entry"
    );
    metrics::counter!("usage_record_failed_total").increment(1);
}
```

For legacy audit failures, update the existing `tracing::warn!(error = %audit_err, "failed to record audit entry")` calls to include the same metric field and counter.

## § Provider Adapters

### OpenAI

File: `crates/infrastructure/providers-openai/src/provider.rs`.

Current `OpenAIUsage` is at `provider.rs:160`. Extend:

```rust
#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<OpenAIPromptTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<OpenAICompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAIPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenAICompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}
```

Mapping at non-streaming `provider.rs:296` and streaming `provider.rs:370`:

```rust
cache_read_tokens: usage.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
cache_creation_tokens: None,
reasoning_tokens: usage.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
```

Add request body field to `OpenAIRequest` near `provider.rs:117-128`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
stream_options: Option<OpenAIStreamOptions>,

#[derive(Debug, Serialize)]
struct OpenAIStreamOptions { include_usage: bool }
```

When `stream == true`, set `stream_options: Some(OpenAIStreamOptions { include_usage: true })`.

### Anthropic

File: `crates/infrastructure/providers-anthropic/src/lib.rs`.

Extend current structs at `lib.rs:35` and `lib.rs:112`:

```rust
#[derive(Debug, Deserialize)]
struct AnthropicNonStreamUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaUsage {
    output_tokens: u32,
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}
```

Mapping at `lib.rs:315` and `lib.rs:429`:

```rust
cache_read_tokens: usage.cache_read_input_tokens,
cache_creation_tokens: usage.cache_creation_input_tokens,
reasoning_tokens: None,
```

### Ollama

File: `crates/infrastructure/providers-ollama/src/lib.rs` currently stubbed per `exploration.md:137-139`. Replace usage stub parsing from Ollama JSON:

```rust
#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}
```

Mapping:

```rust
let prompt = parsed.prompt_eval_count.unwrap_or(0);
let completion = parsed.eval_count.unwrap_or(0);
TokenUsage {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: prompt + completion,
    cache_read_tokens: None,
    cache_creation_tokens: None,
    reasoning_tokens: None,
    estimated_cost_usd: None,
}
```

### Gemini

File: `crates/infrastructure/providers-gemini/src/lib.rs` currently stubbed. Parse:

```rust
#[derive(Debug, Deserialize)]
struct GeminiResponse {
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: Option<u32>,
}
```

No Gemini cache token support in this API. Mapping sets cache/reasoning fields to `None`.

### Groq

File: `crates/infrastructure/providers-groq/src/lib.rs` currently stubbed. Groq is OpenAI-compatible for `usage`; reuse the same shape:

```rust
#[derive(Debug, Deserialize)]
struct GroqUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}
```

Mapping sets cache/reasoning fields to `None`.

## § Transport Layer

### handlers/usage.rs

Create `crates/infrastructure/transport-axum/src/handlers/usage.rs` and export it from `handlers/mod.rs`.

Request query structs:

```rust
#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_id: Option<String>,
    pub connection_id: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub status: Option<RequestStatus>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct UsageListResponse {
    pub entries: Vec<UsageEntry>,
    pub total: u64,
}
```

Handlers:

```rust
pub async fn list_usage(
    State(usecases): State<Arc<RookUsecases>>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageListResponse>, HttpError>;

pub async fn usage_summary(
    State(usecases): State<Arc<RookUsecases>>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageSummary>, HttpError>;

pub async fn usage_cost(
    State(usecases): State<Arc<RookUsecases>>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<CostBreakdown>, HttpError>;
```

Behavior:

- `GET /api/usage` calls `port.list(filters, pagination)` and `port.count(filters)` and returns `{ entries, total }`.
- `GET /api/usage/summary` calls `port.summary(filters)`.
- `GET /api/usage/cost` calls `port.cost_breakdown(filters)`.
- If `usage_recorder` is `None`, return `503` with `USAGE_RECORDER_UNAVAILABLE`. Routes are always mounted; this preserves the approved always-on `/api/usage*` contract while surfacing unavailable storage explicitly.

Query mapping:

```rust
provider -> ProviderId::new(provider)
model -> ModelId::new(model)
api_key_id -> ApiKeyId::new(api_key_id)
connection_id -> ConnectionId::parse_str(&connection_id)
status -> RequestStatus via serde rename_all snake_case
limit -> Pagination::clamped(), default 100, max 1000
```

### routes.rs update

Modify imports at `crates/infrastructure/transport-axum/src/routes.rs:6-12` to include `Query` only in the new handler file, not here. Add merge near `routes.rs:65-80`:

```rust
router = router.merge(usage_routes(usecases.clone()));
```

Add route builder near `routes.rs:700-734`:

```rust
fn usage_routes(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/usage", get(handlers::usage::list_usage))
        .route("/api/usage/summary", get(handlers::usage::usage_summary))
        .route("/api/usage/cost", get(handlers::usage::usage_cost))
        .with_state(usecases)
}
```

Auth: no custom middleware. `authz.rs:384-401` classifies `/api/usage*` as `Management` automatically; GET routes do not need CSRF.

### authz.rs update

Add to `crates/infrastructure/transport-axum/src/authz.rs` near `extract_api_key(headers)` (`authz.rs:566-630` in exploration):

```rust
pub fn extract_api_key_id(req: &Request) -> Option<ApiKeyId> {
    req.headers()
        .get("x-authz-auth-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| *value != "public")
        .map(ApiKeyId::new)
}
```

Route handlers must call this after authz middleware stamps trusted headers and before `CompletionRequest` is constructed. Also add a helper variant if handler signatures remain `HeaderMap`:

```rust
pub fn extract_api_key_id_from_headers(headers: &HeaderMap) -> Option<ApiKeyId>;
```

## § Config Layer

### UsageConfig

Modify `apps/rook/src/config.rs:10-24`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct RookConfig {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub cache: CacheConfig,
    #[serde(default)] pub database: DatabaseConfig,
    #[serde(default)] pub auth: AuthConfig,
    #[serde(default)] pub provider_crud: ProviderCrudConfig,
    #[serde(default)] pub rate_limiting: RateLimiterConfig,
    #[serde(default)] pub usage: UsageConfig,
    #[serde(default)] pub pricing: PricingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageConfig {
    #[serde(default = "default_usage_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_usage_sweep_interval_hours")]
    pub sweep_interval_hours: u32,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self { retention_days: 90, sweep_interval_hours: 6 }
    }
}

fn default_usage_retention_days() -> u32 { 90 }
fn default_usage_sweep_interval_hours() -> u32 { 6 }
```

### PricingConfig

Top-level TOML uses nested maps, because TOML cannot directly deserialize `HashMap<(ProviderId, ModelId), PricingEntry>` from `[pricing.openai.gpt-4o]`. Store nested and expose lookup that acts like a tuple-key map.

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PricingConfig {
    #[serde(flatten)]
    pub providers: HashMap<String, HashMap<String, PricingEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PricingEntry {
    pub prompt_per_million: f64,
    pub completion_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: Option<f64>,
    #[serde(default)]
    pub cache_creation_per_million: Option<f64>,
}

impl PricingConfig {
    pub fn get(&self, provider: &ProviderId, model: &ModelId) -> Option<&PricingEntry> {
        self.providers
            .get(provider.as_str())
            .and_then(|models| models.get(model.as_str()))
    }
}
```

Approved conceptual model is “Top-level: `HashMap<(ProviderId, ModelId), PricingEntry>`”; this design uses a TOML-compatible nested representation with an equivalent lookup key.

Example TOML:

```toml
[usage]
retention_days = 90
sweep_interval_hours = 6

[pricing.openai.gpt-4o]
prompt_per_million = 2.50
completion_per_million = 10.00
cache_read_per_million = 2.50
cache_creation_per_million = 2.50

# Quote model path segments that contain dots, otherwise TOML splits them into nested tables.
[pricing.groq."llama-3.3-70b"]
prompt_per_million = 0.59
completion_per_million = 2.40
```

## § DI Wiring

### di.rs changes

Modify `apps/rook/src/di.rs`.

Imports:

```rust
use audit_sqlite::{SqliteAudit, SqliteUsageRepository};
use rook_core::{AuditPort, UsageRecorderPort, ProviderRepositoryPort, ...};
```

Build usage repository alongside audit near `di.rs:70`:

```rust
let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.database.db_path)?);
let sqlite_usage = Arc::new(SqliteUsageRepository::new(Path::new(&config.database.db_path))?);
let usage_recorder: Option<Arc<dyn UsageRecorderPort>> = Some(sqlite_usage.clone());
```

Provider repo must be shared with `RouteRequest` for connection lookup. Build it once before `manage_connections`:

```rust
let provider_repo: Arc<dyn ProviderRepositoryPort> =
    Arc::new(SqliteProviderRepository::new(&config.database.db_path)?);
let provider_repository_for_usage: Option<Arc<dyn ProviderRepositoryPort>> =
    if config.provider_crud.enabled { Some(provider_repo.clone()) } else { None };
```

Pass into `ManageConnections::new(provider_repo.clone(), ...)` instead of constructing a second repository. Pass into `RouteRequest::new(...)`:

```rust
RouteRequest::new(
    router.clone(),
    cache.clone(),
    audit.clone(),
    usage_recorder.clone(),
    provider_repository_for_usage,
    Arc::new(config.pricing.clone()),
    format_registry.clone(),
)
```

Pass `usage_recorder.clone()` into `RookUsecases::new` after existing optional fields. The existing optional pattern is at `rook-usecases/src/lib.rs:45-47` for `authenticate_client_api`, `manage_connections`, and `manage_api_keys`.

Store concrete `sqlite_usage` on `RookContainer` so `server.rs` can start retention:

```rust
pub usage_repository: Arc<SqliteUsageRepository>,
pub usage_config: UsageConfig,
```

Then `server.rs:12-31` awaits `run_startup_usage_retention_sweep(&container.usage_repository, container.usage_config.retention_days)` before binding the listener, and only then calls `spawn_periodic_usage_retention_sweep(container.usage_repository.clone(), container.usage_config.retention_days, container.usage_config.sweep_interval_hours)`.

## § Implementation Order

1. **Domain model:** extend `TokenUsage`, add `UsageEntry`, `UsageFilters`, `Pagination`, `UsageSummary`, `CostBreakdown`, and export all new types.
2. **Ports:** add `UsageRecorderPort` and `ProviderRepositoryPort::find_connection_id_by_runtime`; update all test fakes that implement `ProviderRepositoryPort`.
3. **Migration:** add `V2__usage_history.sql`; run migration tests/build to confirm refinery embeds it after V0/V1.
4. **SQLite usage repository:** add `SqliteUsageRepository`, row mapping, filter SQL helper, `record`, `list`, `count`, `summary`, `cost_breakdown`, and `delete_older_than`.
5. **Provider repository lookup:** implement runtime provider-to-connection lookup in `provider-sqlite` using `provider_runtime_id` and active priority ordering.
6. **Config:** add `UsageConfig`, `PricingConfig`, `PricingEntry`, defaults, and lookup method.
7. **DI:** instantiate and share `SqliteUsageRepository`, nullable `UsageRecorderPort`, provider repository, pricing config, and update `RookUsecases::new`/`RouteRequest::new` signatures.
8. **Use case recording:** add `UsageEntry` construction helpers, api key/connection propagation, TTFT measurement, cost calculation, and failure instrumentation in the four existing audit call sites.
9. **Transport auth extraction:** add `authz::extract_api_key_id(req: &Request)` and route handler changes to populate `RequestMetadata.api_key_id` before `CompletionRequest` is constructed.
10. **Provider adapters:** update OpenAI and Anthropic parsing first, then implement partial token parsing for Ollama, Gemini, and Groq.
11. **Usage handlers/routes:** create `handlers/usage.rs`, export module, add `usage_routes`, and always merge routes so unavailable storage returns the specified `503 USAGE_RECORDER_UNAVAILABLE` instead of `404`.
12. **Retention task:** add awaited startup sweep plus spawned periodic sweep in `server.rs` or `usage_retention.rs` using configured retention and interval.
13. **Tests and verification:** run targeted unit tests for domain constructors/serialization, SQLite repository CRUD/aggregation, provider parsing, route handler query parsing, and `cargo test -p rook-usecases -p audit-sqlite -p transport-axum`.
14. **Documentation follow-up:** update `docs/configuration.md` with `[usage]` and `[pricing]` only after implementation confirms exact config deserialization.
