# Exploration: Usage Tracking — Token Counts & Cost Estimation

> **GitHub issue**: [#41](https://github.com/dallay/cortex/issues/41)
> **Change slug**: `usage-tracking-token-counts-cost-estimation`
> **Phase**: 1 — Explore
> **Mode**: openspec

---

## § Intent

Replace the current basic `audit` log (one SQLite table, captures only `prompt/completion/total` tokens and an always-`None` `estimated_cost_usd`) with a structured **usage tracking** system. The new system records per-request token counts across all four dimensions (prompt, completion, cache read, cache creation, reasoning), the provider/model, the API key, the underlying provider connection, time-to-first-token, latency, and a USD cost estimate. It exposes `GET /api/usage`, `GET /api/usage/summary`, and `GET /api/usage/cost` for dashboarding and cost analysis.

This exploration establishes what exists today, what is missing, and which decisions the next phase (`sdd-propose`) must surface for the user.

---

## § Archive Verification

The repo carries 8 archived changes. The two most likely to overlap with #41 were checked in depth:

- `2026-05-31-dynamic-provider-registry` — adds the `ProviderRegistryPort` and a SQLite-backed `provider_connections` table. **No usage tracking** in scope.
- `2026-05-31-provider-connections` — same as above; explicitly notes "quotaWindowThresholds: Read-only monitoring only. Values are persisted and returned via API but no automatic enforcement."
- `2026-06-01-anthropic-sse-streaming` and `2026-06-01-format-translation-layer` — wire up SSE/usage on streaming responses. The `format-translation-layer/exploration.md:235` explicitly says **"Cost estimation (`estimated_cost_usd`) is always `None`. This is a placeholder in both providers. Not a blocker for #40 but a visible gap in production observability."** This is the seam the current change picks up.
- `2026-06-02-api-key-scopes-and-restrictions` (`proposal.md:32`) explicitly defers: **"Audit log changes tracking `api_key_id` or scope-violation events. Deferred to a follow-up."** That follow-up is what we are now scoping.
- `2026-06-03-per-client-rate-limiting` — adds per-key rate limits and `GET /api/rate-limits/:scope/:target/status`. Adjacent but distinct from usage history.

A targeted search (`UsageRecorder`, `usage_history`, `UsageEntry`, `cost_breakdown`, `estimated_cost`) across `openspec/` produced **only one substantive match** (the format-translation-layer quote above) plus 4 doc references to `usage` that are all about the `TokenUsage` payload on responses, not persistent tracking.

**Verdict**: no prior change overlaps. The change folder at `openspec/changes/usage-tracking-token-counts-cost-estimation/` exists but is empty — the spec has not been started. The precedent from issue #45 (the `dynamic-provider-registry` change was already on main and got reopened) does not apply here.

---

## § Current State

### 1. Audit log — what exists today

**Schema** (created at startup and persisted via `V0__initial.sql`):

```sql
-- crates/infrastructure/db-migration/src/migrations/V0__initial.sql:99-115
CREATE TABLE audit
(
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id         TEXT    NOT NULL,
    provider           TEXT    NOT NULL,
    model              TEXT    NOT NULL,
    status             TEXT    NOT NULL,
    prompt_tokens      INTEGER,
    completion_tokens  INTEGER,
    total_tokens       INTEGER,
    estimated_cost_usd REAL,
    latency_ms         INTEGER NOT NULL,
    timestamp          TEXT    NOT NULL
);
CREATE INDEX idx_audit_request_id ON audit (request_id);
CREATE INDEX idx_audit_provider ON audit (provider);
CREATE INDEX idx_audit_timestamp ON audit (timestamp);
```

The same `CREATE TABLE IF NOT EXISTS audit` block is duplicated in `crates/infrastructure/audit-sqlite/src/lib.rs:33-50` and is reached only on first open if the migration did not run.

**Port trait and domain types** — `crates/domain/rook-core/src/ports.rs:119-122` and `crates/domain/rook-core/src/model.rs:350-406`:

```rust
// ports.rs:119
pub trait AuditPort: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()>;
}

// model.rs:350
pub enum RequestStatus { Success, Failure, RateLimited, Timeout }

// model.rs:359
pub struct AuditEntry {
    pub request_id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub status: RequestStatus,
    pub usage: Option<TokenUsage>,
    pub latency_ms: u64,
    pub timestamp: DateTime<Utc>,
}

// model.rs:286
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub estimated_cost_usd: Option<f64>,   // always None in practice
}
```

**Adapter** — `SqliteAudit` at `crates/infrastructure/audit-sqlite/src/lib.rs:30-103`. Uses a `tokio::sync::Mutex<Connection>`. No `tests/` directory — verify-report mentions 21 tests, but they are not in the crate today; the references in `archive/2026-06-03-per-client-rate-limiting/verify-report.md:46` are stale.

**DI wiring** — `apps/rook/src/di.rs:70`:

```rust
let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new( & config.database.db_path) ? );
```

**Audit call sites in the use case layer** — all in `crates/application/rook-usecases/src/route_request.rs`:

| Line    | Path                  | When                                | Carries                                 |
|---------|-----------------------|-------------------------------------|-----------------------------------------|
| 110–119 | non-streaming success | after `provider.complete()` returns | `usage` from response, `latency_ms`     |
| 239–243 | non-streaming failure | on `provider.complete()` error      | `usage: None`, `latency_ms`             |
| 193–202 | streaming failure     | on per-chunk `Err` from upstream    | `usage: None`, `latency_ms`             |
| 208–217 | streaming success     | after upstream stream closes        | `final_usage` (last seen `chunk.usage`) |

All four call sites are **fire-and-forget**: failures are logged with `tracing::warn!` and never propagate to the client. This is the right pattern for observability but means a noisy DB can silently drop records.

### 2. Token extraction per provider

#### `providers-openai/src/provider.rs`

- **Non-streaming** (`complete()`, line 240–304): deserializes `OpenAIResponse { usage: OpenAIUsage { prompt_tokens, completion_tokens, total_tokens } }` (line 159–164) and builds a domain `TokenUsage` with `estimated_cost_usd: None` (line 296–301). The `OpenAIUsage` struct has no cache or reasoning fields.
- **Streaming** (`stream()`, line 306–404): deserializes `OpenAIStreamResponse { usage: Option<OpenAIUsage> }` (line 166–173) and exposes the final-chunk usage via `StreamChunk.usage: Option<TokenUsage>` (line 370–387). The `stream_options.include_usage` is **not** sent in the request body (`OpenAIRequest` at line 117–128 has no `stream_options`), so usage in streaming responses depends entirely on the OpenAI endpoint's default behavior. Cache tokens (`cache_read_input_tokens`) and reasoning tokens (`o1`-style) are not captured.

#### `providers-anthropic/src/lib.rs`

- **Non-streaming** (`complete()`, line 241–324): deserializes `AnthropicNonStreamResponse { usage: AnthropicNonStreamUsage { input_tokens, output_tokens } }` (line 17–38). The current `AnthropicNonStreamUsage` struct **only has `input_tokens` and `output_tokens`** — it does not parse `cache_creation_input_tokens` or `cache_read_input_tokens` even though Anthropic returns both. These are dropped on the floor.
- **Streaming** (`stream()`, line 326–465): deserializes `AnthropicStreamEvent::MessageDelta { usage: AnthropicMessageDeltaUsage }` (line 72–77 and 111–116). The current `AnthropicMessageDeltaUsage` only has `output_tokens` and `Option<input_tokens>`. Cache token fields are not parsed.

The Anthropic Messages API today returns:

```json
{
  "usage": {
    "input_tokens": 100,
    "output_tokens": 50,
    "cache_creation_input_tokens": 0,
    "cache_read_input_tokens": 0
  }
}
```

in both streaming `message_delta` and non-streaming responses, but the current adapter drops the cache fields. Reasoning tokens (only relevant for `o1`/future Anthropic reasoning models) are not a current Anthropic concept; they apply to OpenAI's `o1-*` and `o3-*` family.

#### `providers-ollama`, `providers-gemini`, `providers-groq`

All three are 72-line stubs at `crates/infrastructure/providers-{ollama,gemini,groq}/src/lib.rs`. They implement `ProviderPort` with empty bodies for `complete()` and `stream()` — neither path returns a `TokenUsage` at all today. The Ollama local API does return token counts, and Gemini returns `usageMetadata` with `promptTokenCount`/`candidatesTokenCount`. None of that is captured. **They will need a full implementation if we want parity with the OpenAI/Anthropic adapters.**

### 3. ID types — all exist today

| Type           | Path                                       | Shape                                                                                                               |
|----------------|--------------------------------------------|---------------------------------------------------------------------------------------------------------------------|
| `RequestId`    | `crates/domain/shared-kernel/src/id.rs:69` | `struct RequestId(pub Uuid)`                                                                                        |
| `ProviderId`   | `crates/domain/shared-kernel/src/id.rs:7`  | `struct ProviderId(pub SmolStr)`                                                                                    |
| `ModelId`      | `crates/domain/shared-kernel/src/id.rs:38` | `struct ModelId(pub SmolStr)`                                                                                       |
| `ConnectionId` | `crates/domain/shared-kernel/src/id.rs:92` | `struct ConnectionId(pub Uuid)` — distinct from `ProviderId`; the storage identifier for a `ProviderConnection` row |
| `ApiKeyId`     | `crates/domain/rook-core/src/api_key.rs:8` | `struct ApiKeyId(SmolStr)`                                                                                          |

`ProviderId` and `ConnectionId` are deliberately separate: the same runtime provider can be backed by multiple connection rows (e.g. two OpenAI accounts for failover). At audit time we want the connection, not the runtime ID, because cost and quota attribution are per-account.

**All five types are in the codebase. No new type definitions needed.**

### 4. Where the new fields are *not* today — propagation gaps

The `UsageEntry` in the spec adds five fields that are **not** on `AuditEntry` and not threaded through the request flow:

1. **`connection_id: ConnectionId`** — The `ProviderId` is available at `route_request.rs:75, 156` after `router.select()`. The `ConnectionId` is **not**. Mapping `ProviderId → ConnectionId` requires a lookup through `ProviderRepositoryPort` (`crates/domain/rook-core/src/ports.rs:161`). A near-zero-cost option: add a new `find_id_by_runtime(&ProviderId) -> Option<ConnectionId>` port method (or a `try_from` on `ProviderRegistryPort`). Need to verify which runtime providers are CRUD-backed vs TOML — per `AGENTS.md` §"Quirks & Gotchas" / `docs/architecture.md` §"Provider CRUD Limitation", **TOML providers have no row in `provider_connections`**, so the lookup is best-effort (`Option<ConnectionId>`).

2. **`api_key_id: Option<ApiKeyId>`** — Available at the authz layer (`crates/infrastructure/transport-axum/src/authz.rs:566-630` builds `Subject { id: api_key_subject.id, ... }` and stamps it into `x-authz-auth-id` at line 462). But the `Subject` is **not** propagated into `CompletionRequest` / `RouteRequest::execute`. The transport adapters (`openai_adapter.rs`, `anthropic_adapter.rs`) construct `CompletionRequest` with no auth context. Needs a new field on `RequestMetadata` (or a sibling `RequestAuth { api_key_id, connection_id }`) and a new extractor at the route handlers that reads the stamped header.

3. **`tokens_cache_read` / `tokens_cache_creation`** — Not parsed by any provider today (Anthropic drops them on the floor — see §2). The `TokenUsage` domain struct (line 286) has no fields for them; the `audit` table (V0) has no columns; the `UsageEntry` will need both.

4. **`tokens_reasoning`** — Not parsed by any provider. OpenAI's `o1-*`/`o3-*` family returns this in `usage.completion_tokens_details.reasoning_tokens`. Anthropic does not currently expose reasoning tokens in the public Messages API. Needs provider adapter work AND a new field on `TokenUsage` (or a parallel `TokenUsageDetails` struct).

5. **`ttft_ms: u32`** — Not measured anywhere. The first `StreamChunk` is yielded from the provider's stream, but `route_request.rs:176-218` does not capture a timestamp when the first chunk arrives. The adapter layer would need to stamp the first-chunk instant.

6. **`service_tier`** — The spec lists `Standard | Premium`. Neither `RequestStatus` nor any provider response field tracks this. OpenAI's `/v1/chat/completions` accepts a `service_tier` request field and returns the actual tier in the response. Anthropic does not currently expose it. **Unknown: should the field reflect what the client requested, what the provider billed, or both? Need user decision.**

### 5. Config layer

`apps/rook/src/config.rs:10-24` defines `RookConfig` with these top-level sections: `server`, `routing`, `cache`, `database`, `auth`, `provider_crud`, `rate_limiting`. **There is no `[pricing]` or `[usage]` section today.**

`docs/configuration.md:13-58` shows the same schema. A grep for `pricing|price_per|cost_per|usd_per|per_million|1M` across the docs returned **zero matches**.

The pricing data must therefore be designed from scratch. Two natural locations:

- **Top-level `[pricing]` TOML table** with per-model entries. Self-contained, easy to read, but disconnected from the per-provider connection config in `provider_connections` (managed via API, not TOML).
- **Per-connection pricing** stored in the `provider_connections` table. Tracks pricing alongside the connection, but expands the connection schema and pricing changes for existing providers require an API call.

Provider TOML config no longer exists (per `docs/configuration.md:60-61`: "Provider configuration is no longer in TOML. Providers are managed dynamically via the Provider CRUD API"), so pricing cannot live under the (gone) `[providers.*]` block.

### 6. API surface — precedent for `GET /api/usage*`

The `transport-axum` crate follows a tight pattern for admin endpoints:

- Each domain area has a `crates/infrastructure/transport-axum/src/handlers/{name}.rs` module (e.g. `api_key.rs:1`, `rate_limits.rs:1`).
- Each module exposes a `pub type {Name}Store` if it owns persistent state, plus async handler functions.
- `routes.rs:700-734` defines small `fn {name}_routes(...) -> Router` builders that mount the routes. `Router::merge()` happens conditionally in `router()` at `routes.rs:65-80` based on whether the relevant usecase/store is enabled.

Concrete examples relevant to the new endpoints:

- `routes.rs:700-715` — `api_key_routes`: GET/POST `/api/api-keys`, GET/PUT/DELETE `/api/api-keys/{id}`, POST `/api/api-keys/{id}/rotate`.
- `routes.rs:717-734` — `rate_limits_routes`: GET/POST `/api/rate-limits`, PUT/DELETE `/api/rate-limits/{id}`, GET `/api/rate-limits/{scope}/{target}/status`.

**All `/api/*` (non-bootstrap) routes are `AuthTier::Management`** (see `authz.rs:384-401` `classify_route`). They require session-based auth and CSRF for state-changing methods. The new `GET /api/usage*` endpoints inherit that automatically.

A precedent for the *path shape* — `/api/rate-limits/:scope/:target/status` — exists for the read-only "give me current state" use case. The new `/api/usage/summary` and `/api/usage/cost` follow the same pattern.

### 7. Streaming audit-on-completion — pattern is already correct

`route_request.rs:170-218` shows the streaming audit pattern is **already aligned** with what the new design needs:

```rust
let stream = async_stream::try_stream! {
    let mut final_usage: Option<TokenUsage> = None;
    while let Some(chunk) = upstream.next().await {
        match chunk {
            Ok(chunk) => {
                if chunk.usage.is_some() { final_usage = chunk.usage.clone(); }
                yield chunk;
            }
            Err(error) => { /* ... record failure entry ... */ }
        }
    }
    // Stream closed successfully — record success entry
    let entry = AuditEntry::success(..., final_usage, ...);
    audit.record(entry).await;
};
```

Implications for the new design:

- The `final_usage` slot is **the only** natural place to enrich with cache/reasoning tokens and `ttft_ms` from the final SSE chunk. The provider adapter must populate them before the chunk is yielded.
- Recording the audit row happens **after** the client has started receiving the stream (the audit is inside the `try_stream!` body but after `yield chunk`). Latency-sensitive clients see no added tail latency.
- The streaming success path does not need to change semantically — only the `AuditEntry` shape and the `TokenUsage` shape inside it.

### 8. Migrations and DB conventions

`crates/infrastructure/db-migration/src/lib.rs:1-36` uses `refinery` with embedded migrations. Current migrations: `V0__initial.sql`, `V1__allowed_models_providers.sql`. The convention is `V{n}__{name}.sql`. A new `V2__usage_history.sql` will be required to introduce the new `usage_history` table (or to extend `audit` with the additional columns). Existing migrations must remain byte-for-byte unchanged (refinery verifies checksums).

---

## § Gaps & Open Questions

These are decisions the user must make before `sdd-propose` can produce a tight spec. The exploration surfaces them but does not pre-decide.

### G1. Replace or extend the existing `audit` table?

The current `audit` table is referenced by the existing `AuditPort` and a single test in `archive/2026-06-03-per-client-rate-limiting/verify-report.md` (the test itself is not in the crate today). Two viable approaches:

- **(a) Replace**: drop `audit`, create `usage_history` with the new shape. Single source of truth, simpler code, but breaks the historical 10-column shape and any external scripts that read it.
- **(b) Coexist**: keep `audit` for backward compat (deprecated), add `usage_history` as the new canonical table. The new `UsageRecorderPort` writes to `usage_history`; the old `AuditPort` keeps writing to `audit`. Two write paths during the migration.
- **(c) Extend**: `ALTER TABLE audit ADD COLUMN` to add the new columns. Keeps the table but the column count goes from 10 to 19 and the schema gets unwieldy. The retention policy is hard to apply (would need a `WHERE status = 'X'` sweep).

**User decision needed**: replace vs coexist vs extend.

### G2. New `UsageRecorderPort` or extend `AuditPort`?

Three viable designs:

- **(a) New port** `UsageRecorderPort` in `rook-core/ports.rs` with `record(UsageEntry)`, `list(filters)`, `summary(...)`, `cost(...)`. `AuditPort` stays as-is for the legacy `audit_log` (or is removed if G1 = replace).
- **(b) Extend `AuditPort`** with `record(UsageEntry)` (replaces `record(AuditEntry)`) and add the query methods. Single port, but the name "Audit" no longer matches the responsibility.
- **(c) Two ports**: `UsageRecorderPort` for writes, `UsageQueryPort` for reads (CQRS-style). Clean separation but two types where one might do.

**User decision needed**: shape of the port boundary.

### G3. Pricing config — TOML top-level or per-connection?

See §5. The simplest option that respects the current "no provider TOML" rule is a new `[pricing]` top-level table. But the model catalog is also moving toward per-connection (cf. `models-catalog`). Pricing could follow.

**User decision needed**: where does pricing live? Per-model map under `[pricing.<provider>.<model>]` is the safe default; per-connection columns in `provider_connections` is the most flexible.

### G4. `service_tier` semantics

The spec lists `Standard | Premium` without saying which value gets recorded. OpenAI's `service_tier` is a request field the client sets and the response echoes. If we record what the client asked for, that's an input. If we record what the provider billed, that's an output. They can differ.

**User decision needed**: is `service_tier` the requested tier, the billed tier, or both (as separate fields)?

### G5. `ttft_ms` — measure where?

Three options for time-to-first-token:

- **(a) In the use case**: `route_request.rs` captures `Instant::now()` when the first `Ok(chunk)` arrives and computes the delta. No adapter changes.
- **(b) In the provider adapter**: the adapter stamps `ttft_ms` on the first `StreamChunk`. More accurate but couples the adapter to a non-domain concept.
- **(c) In the transport**: a separate middleware records it from the response stream. Cleanest separation but requires streaming-aware middleware.

**User decision needed**: which layer owns `ttft_ms`?

### G6. Cache and reasoning token support — required for v1 or follow-up?

Adding `cache_creation_input_tokens` and `cache_read_input_tokens` to the Anthropic adapter is mechanical (extend `AnthropicNonStreamUsage` and `AnthropicMessageDeltaUsage`). Adding `reasoning_tokens` to the OpenAI adapter requires parsing `usage.completion_tokens_details.reasoning_tokens`. The Ollama/Gemini/Groq stubs need full implementations to return any `TokenUsage` at all.

**User decision needed**: which token types are in scope for v1? If the change ships a new schema but the adapters cannot populate the new fields, the dashboard will show zeroes — which is fine if that's a documented milestone.

### G7. Retention policy implementation

"Default 90 days, configurable" is mentioned. SQLite has no native TTL. Options:

- **(a) Sweep job** — a `tokio` task at startup that deletes rows older than the retention window. Simple, no schema change, but the sweep runs only while the process is up.
- **(b) Lazy sweep** — every read query has a `WHERE timestamp > now() - retention` predicate. No background work, but every API call pays the cost.
- **(c) Per-connection pruning** — at request time, delete one or two old rows. Spreads the cost but unpredictable.

**User decision needed**: how is the retention policy enforced?

### G8. Are `/api/usage*` routes always-on or feature-gated?

The current `audit-sqlite` crate is always wired (`di.rs:70`). The `api_keys` admin API is gated by `config.auth.api_keys.enabled`; the `rate-limits` admin API is gated by `config.rate_limiting.enabled`. The new `usage*` endpoints could follow either pattern. The table itself is always on (audit happens on every request); the read API might want its own toggle.

**User decision needed**: do `GET /api/usage`, `/api/usage/summary`, `/api/usage/cost` get a config flag, or are they always on?

### G9. Auth on the new read endpoints

`/api/*` routes already require session-based MANAGEMENT auth (`authz.rs:384-401`). With a session cookie + admin scope. Is that enough, or do we want a new `usage:read` API-key scope?

**User decision needed**: do `usage:read` and `usage:admin` API-key scopes need to be added to `KnownScope`? The previous change explicitly avoided this.

### G10. Backfill of historical `audit` rows

If G1 = replace, what happens to existing rows? `DROP TABLE audit` loses them. `RENAME TABLE audit TO usage_history` + `ALTER TABLE usage_history ADD COLUMN` keeps them but with `NULL` values for the new columns. An `INSERT INTO usage_history (...) SELECT ... FROM audit` backfill is doable but requires a default for every new column.

**User decision needed**: do existing rows get migrated, dropped, or left in the legacy `audit` table?

---

## § Affected Areas

| Path                                                                | Why it's affected                                                                                                                                                                                                                                                                                     |
|---------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/domain/rook-core/src/model.rs`                              | Add `UsageEntry` (or extend `AuditEntry`); add cache/reasoning/ttft fields to `TokenUsage`; add `service_tier` to `RequestStatus` or as new field. Lines 286–293 (`TokenUsage`), 350–406 (`RequestStatus`, `AuditEntry`).                                                                             |
| `crates/domain/rook-core/src/ports.rs`                              | Add `UsageRecorderPort` (or extend `AuditPort` at lines 119–122). Add `list`, `summary`, `cost` methods.                                                                                                                                                                                              |
| `crates/domain/shared-kernel/src/id.rs`                             | No changes — all five ID types already exist.                                                                                                                                                                                                                                                         |
| `crates/infrastructure/audit-sqlite/src/lib.rs`                     | Replace the `CREATE TABLE` block (lines 33–50) and the INSERT statement (lines 81–99) with the new schema and the new columns. **OR** add a new crate `usage-sqlite` and split the responsibility.                                                                                                    |
| `crates/infrastructure/db-migration/src/migrations/V0__initial.sql` | No edits (migration is frozen). Add `V2__usage_history.sql`.                                                                                                                                                                                                                                          |
| `crates/application/rook-usecases/src/route_request.rs`             | Thread `api_key_id`, `connection_id` through the request flow. The four audit call sites (lines 110, 193, 208, 239) need to populate the new fields. Streaming flow (lines 170–218) needs `ttft_ms` capture.                                                                                          |
| `crates/infrastructure/transport-axum/src/openai_adapter.rs`        | No code change (adapters don't construct `UsageEntry` directly), but the new fields must propagate through the request.                                                                                                                                                                               |
| `crates/infrastructure/transport-axum/src/anthropic_adapter.rs`     | Same.                                                                                                                                                                                                                                                                                                 |
| `crates/infrastructure/transport-axum/src/routes.rs`                | Add a `usage_routes(...)` function (pattern at lines 700–734) and merge it into the router (line 65–80).                                                                                                                                                                                              |
| `crates/infrastructure/transport-axum/src/handlers/`                | New `usage.rs` module following the `rate_limits.rs` pattern at lines 1–206.                                                                                                                                                                                                                          |
| `crates/infrastructure/transport-axum/src/authz.rs`                 | No code change unless a new `usage:read` scope is added (G9).                                                                                                                                                                                                                                         |
| `crates/infrastructure/providers-anthropic/src/lib.rs`              | Extend `AnthropicNonStreamUsage` (lines 34–38) and `AnthropicMessageDeltaUsage` (lines 111–116) to parse `cache_creation_input_tokens` and `cache_read_input_tokens`. Update the `TokenUsage` constructors (lines 315–321, 429–437).                                                                  |
| `crates/infrastructure/providers-openai/src/provider.rs`            | Extend `OpenAIUsage` (lines 159–164) to parse `prompt_tokens_details.cached_tokens` and `completion_tokens_details.reasoning_tokens`. The OpenAI streaming request must set `stream_options: { include_usage: true }` to guarantee the final chunk has `usage`.                                       |
| `crates/infrastructure/providers-ollama/src/lib.rs`                 | Stub today (72 lines). Add non-streaming + streaming `complete()` and `stream()` that build `TokenUsage` from Ollama's `prompt_eval_count`/`eval_count` fields.                                                                                                                                       |
| `crates/infrastructure/providers-gemini/src/lib.rs`                 | Stub today. Add parsing of `usageMetadata` (`promptTokenCount`, `candidatesTokenCount`, `cachedContentTokenCount`).                                                                                                                                                                                   |
| `crates/infrastructure/providers-groq/src/lib.rs`                   | Stub today. Groq uses OpenAI-compatible format, so the OpenAI parser can be reused.                                                                                                                                                                                                                   |
| `apps/rook/src/di.rs`                                               | Wire the new repository (line 70) and propagate `api_key_id`/`connection_id` extractors to the use case. Lines 178–199 (`RookUsecases::new`) need new dependencies.                                                                                                                                   |
| `apps/rook/src/config.rs`                                           | Add a `UsageConfig` (retention, possibly pricing map) under `RookConfig`. Lines 10–24.                                                                                                                                                                                                                |
| `docs/configuration.md`                                             | Document the new `[usage]` and `[pricing]` sections. Lines 13–58.                                                                                                                                                                                                                                     |
| `openspec/specs/`                                                   | No prior spec exists. The previous change (`2026-06-02-api-key-scopes-and-restrictions`) noted "Audit log changes deferred to a follow-up" — that follow-up needs its own spec set, e.g. `usage-tracking-domain`, `usage-tracking-repository`, `usage-tracking-transport`, `usage-tracking-usecases`. |
| `openspec/ARCHITECTURE.md`                                          | Document the new ports, the new table, and the read API.                                                                                                                                                                                                                                              |
| `apps/rook/dashboard/`                                              | (If a usage dashboard is in scope.) The Vue dashboard already has a directory; `api_keys` is a precedent.                                                                                                                                                                                             |

---

## § Risks

| #   | Risk                                                                                                                                                                                                                                                                             | Severity | Mitigation                                                                                                                                                                                                        |
|-----|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| R1  | **Silent data loss from fire-and-forget audit.** The four call sites log failures but never propagate them. Under DB pressure, usage records can drop with no client-visible signal.                                                                                             | High     | Add a counter / tracing event for `usage_record_failed_total` so dashboards can detect the drop. Optionally retry with backoff.                                                                                   |
| R2  | **Provider stubs (Ollama/Gemini/Groq) cannot populate any new field.** The new schema will hold NULLs for the stub providers until the adapters are filled in.                                                                                                                   | High     | Ship the schema + UI in the same change, but document which providers return complete vs partial records.                                                                                                         |
| R3  | **Anthropic cache tokens require API or schema change.** If the current Anthropic API version does not return cache tokens on streaming (only non-streaming), the spec field is half-populated.                                                                                  | Medium   | Verify against the live Anthropic API and pin the `anthropic-version` header. Note in the spec that cache tokens are best-effort on streaming.                                                                    |
| R4  | **`connection_id` is not currently threaded through the use case.** The `ProviderId → ConnectionId` mapping requires either a new port method or a new field on `ProviderRegistryPort`. TOML providers (legacy) have no row in `provider_connections` so the mapping is partial. | Medium   | Add `find_connection_id_by_runtime(&ProviderId) -> Option<ConnectionId>` to `ProviderRepositoryPort` and treat the field as `Option<ConnectionId>` (matches the spec's `api_key_id: Option<ApiKeyId>` precedent). |
| R5  | **`api_key_id` is currently not in `CompletionRequest`.** The authz middleware knows it (stamps it into `x-authz-auth-id`) but the route handlers do not pass it down. Requires adding a field to `RequestMetadata` and a new extractor.                                         | Medium   | Mirror the pattern used for `ApiKeyRestrictions` (a sibling struct on `CompletionRequest` at `model.rs:147-151`).                                                                                                 |
| R6  | **`service_tier` semantics unclear.** Spec lists `Standard or Premium` without saying what the value means.                                                                                                                                                                      | Medium   | G4 is on the open-questions list.                                                                                                                                                                                 |
| R7  | **Cost estimation can be wildly wrong if the pricing config is missing.** A single missing `1M token price` field gives `0.0` cost with no warning.                                                                                                                              | High     | If pricing is missing for a model, log a `tracing::warn!` per request and emit a metric counter. UI surfaces "cost unknown" rather than `$0.00`.                                                                  |
| R8  | **Retention sweep at startup is not durable.** If the process never restarts, the table grows unbounded.                                                                                                                                                                         | Low      | Add a periodic background sweep (every 6h) in addition to the startup sweep.                                                                                                                                      |
| R9  | **Spec conflicts with the in-progress `api-key-scopes-and-restrictions` change.** That change's `proposal.md:32` explicitly defers audit-log changes; this change picks them up. Verify the previous change is fully archived before starting.                                   | Low      | `archive/2026-06-02-api-key-scopes-and-restrictions/` is present and complete. No conflict.                                                                                                                       |
| R10 | **`authz.rs` middleware stamping changes.** Any modification to the `x-authz-auth-id` header pipeline could break the api-key-id propagation.                                                                                                                                    | Low      | Read the new value in a dedicated extractor function, not inline. Add a test that asserts the header is present for authenticated requests.                                                                       |

---

## § Recommended Next Phase input

`sdd-propose` should be focused on the **architectural shape** and the **10 open questions** above. The concrete suggestions to push to the user (in order of impact, lowest-risk first):

1. **Resolve G1 (replace / extend / coexist)**. This single decision shapes the migration file count, the port trait, and the backfill question. Recommend **coexist** for v1: keep the old `audit` table writeable, add a new `usage_history` table for the new shape, deprecate `audit` in a follow-up. Lower blast radius; both can be queried during the transition.

2. **Resolve G2 (port shape)**. Recommend a new `UsageRecorderPort` with `record`, `list_paginated`, `summary`, `cost_breakdown`. `AuditPort` stays as-is until deprecation. Keep the two ports separate because the new one has query responsibility the old one does not.

3. **Resolve G4 (`service_tier`)** and **G5 (`ttft_ms` owner)**. Both are small but block spec wording.

4. **Resolve G6 (token types in v1)**. Recommend a "v1 ships full schema, Anthropic + OpenAI return full data, Ollama/Gemini/Groq return partial until later changes land." Document the partial state.

5. **Resolve G3 (pricing config)**, **G7 (retention)**, **G8 (always-on vs gated)**, **G9 (new scope or not)**, **G10 (backfill)**. These can be decided in the proposal itself with the user's quick yes/no on each.

6. The proposal should explicitly name the **V2 migration** (`V2__usage_history.sql`) and the **new crate decision** (extend `audit-sqlite` or add `usage-sqlite`). Recommend extending `audit-sqlite` (rename in a follow-up) so the DI wiring stays one line.

7. The proposal should call out that **the `UsageRecorderPort` must be optional / nullable in `RookUsecases`** to keep `RookUsecases::new`'s 13-arg constructor from growing another required dep — the same pattern used for `manage_connections` and `manage_api_keys` (see `rook-usecases/src/lib.rs:46-47`).

Once the user has answered the open questions, `sdd-propose` can produce a tight proposal and `sdd-spec` can write 4 spec files (`usage-tracking-domain`, `-repository`, `-usecases`, `-transport`) plus a `usage-tracking-dashboard` delta if the Vue work is in scope.
