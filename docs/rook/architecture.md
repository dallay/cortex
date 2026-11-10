# Architecture

## Layer Diagram

```text
┌─────────────────────────────────────────────────────────────┐
│                      apps/rook                              │  ← main.rs, config.rs, di.rs
│                  (binary, DI bootstrap)                    │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│               transport-axum (infrastructure)              │  ← HTTP server, route handlers
│            openai_adapter, anthropic_adapter                │  ← wire format ↔ domain model
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│                  rook-usecases (application)                │  ← RouteRequest, FallbackRouter
│       ManageProviders, HealthCheck, ManageConnections       │
└────────────────────────┬────────────────────────────────────┘
                         │
         ┌───────────────┼───────────────┬───────────────┐
         │               │               │               │
┌────────▼──────┐ ┌──────▼──────┐ ┌──────▼──────┐ ┌──────▼───────┐
│providers-openai││providers-  │ │providers-   │ │providers-   │
│providers-     │ │anthropic   │ │ollama       │ │gemini, groq  │
│anthropic      │ │            │ │             │ │              │
└────────────────┘ └────────────┘ └─────────────┘ └──────────────┘
        ↓                ↓               ↓              ↓
┌───────────────────────────────────────────────────────────────┐
│                        rook-core (domain)                    │  ← CompletionRequest/Response,
│                    ports.rs, model.rs                        │    ProviderPort, RouterPort,
└────────────────────────┬────────────────────────────────────┘    CachePort, AuditPort
                         │
┌────────────────────────▼────────────────────────────────────┐
│                    shared-kernel                            │  ← no external deps
│                id.rs, error.rs, time_.rs                   │
└─────────────────────────────────────────────────────────────┘
```

## Crate Responsibilities

### `shared-kernel`

Common types with zero external dependencies.

- `ProviderId`, `ModelId`, `RequestId` — newtype wrappers (prevents mixing at type level)
- `CortexError` — error types: ProviderError, NotFoundError, RateLimitedError, AllProvidersExhaustedError
- `CacheKey` — derived from request ID for caching

### `rook-core`

Domain model and port traits. Completely provider-agnostic.

- **Model** — `CompletionRequest`, `CompletionResponse`, `Message`, `Role`, `TokenUsage`, `StreamChunk`, `HealthStatus`, `AuditEntry`
- **Ports** — capability traits the domain requires but cannot implement:
    - `ProviderPort` — LLM provider capability (complete, stream, health_check)
    - `RouterPort` — provider selection with failure notification
    - `CachePort` — get/set/delete/clear with TTL
    - `AuditPort` — record audit entries
    - `ProviderRepositoryPort` — persisted provider connection storage
    - `ProviderRegistryPort` — runtime provider lookup for health probes
    - `KeyManager` — credential encryption boundary

### `rook-usecases`

Application orchestration.

- **`RouteRequest`** — the main orchestrator: cache → select provider → execute → cache response → audit → handle failure. Supports combo execution for multi-step fallback chains.
- **`FallbackRouter`** — implements RouterPort with three strategies: Priority, RoundRobin, ModelBased. Includes circuit breaker (3 failures → 30s cooldown).
- **`ManageProviders`** — enable/disable providers (interface only for now)
- **`HealthCheck`** — aggregated health status across all providers
- **`ManageConnections`** — runtime-managed provider connection CRUD/test workflow

### Combos (Multi-step Fallback Chains)

Combos provide automatic failover across multiple provider/model pairs. They are managed as first-class domain entities with:

- **Domain model** (`rook-core`) — `Combo`, `ComboStep`, `ComboStrategy`, validation rules
- **Repository** (`combo-sqlite`) — SQLite-backed persistence with name/ID uniqueness
- **API** (`transport-axum/combo_routes`) — CRUD endpoints at `/api/combos`
- **Execution** (`RouteRequest::execute_combo`) — priority-ordered step execution with timeouts, circuit breaker integration, and comprehensive audit/usage tracking
- **Configuration** (`apps/rook/config`) — TOML-based combo definitions seeded at startup

`apps/rook/di` connects `routing.default_combo` with `RouteRequest` after
loading the combos. Startup rejects a malformed UUID, a nonexistent
or invalid default; the check includes combos persisted in SQLite.
`RouteRequest` applies the default only if the request does not select a
combo via `X-Rook-Combo` or a `rook/*` alias. Both execution modes
reuse the existing authorization checks and combo flow.
All steps must be authorized before any upstream call.
The configuration guide includes the DALLAY-710 migration instructions.

The legacy keys `cache.token_cache.mode` and `cache.token_cache.providers`
are accepted only for compatibility and generate warnings if they appear in the
configuration. They do not control token-cache injection or the response cache.

**Execution Flow:**

```
Request with X-Rook-Combo header or default_combo
  → Load combo from repository
  → Sort steps by priority (ascending)
  → For each step:
      → Connection-bound step (connection_id present):
          → Resolve the persisted connection
          → Verify it is active and its runtime matches the step provider_id
          → Resolve the exact adapter for that connection (no substitution)
          → Authorize the effective connection against allowed_providers
          → Skip with a deterministic reason when the connection is unusable
      → Legacy step (no connection_id):
          → Check circuit breaker (skip if open)
          → Check provider availability (skip if not registered)
      → Execute with 10s timeout
      → On success: return immediately
      → On 4xx (except 429): stop, return error
      → On 429/5xx/network: continue to next step
  → If all steps fail: AllProvidersExhausted error
```

**Step identity:**

- `ComboStep.provider_id` holds the logical provider runtime ID when the step sets
  `connection_id`. The persisted connection is the authority: the runtime ID must
  match `connection.provider_runtime_id`, and the adapter ID must equal
  `ProviderId::new(connection.id.to_string())`.
- `allowed_providers` authorizes connections (adapter IDs), not runtime IDs. A key
  allowed for one account cannot execute another account. The check runs before
  any upstream call and returns 403.
- Steps without `connection_id` keep the historical adapter-ID selection and
  authorization behavior. Legacy rows are never re-normalized.
- Audit and usage records carry the effective connection. Audit also stores the
  runtime ID plus the combo ID and step index.

**Timeouts:**

- Per-step timeout: 10 seconds
- Overall combo timeout: 60 seconds

**Progressive combo streaming:**

- Streaming requests use each eligible step's native `ProviderPort::stream`.
  Native streaming capability must be supported; completion-only steps are skipped.
  Non-streaming combo execution is unchanged.
- Selection waits for exactly one domain chunk. Setup failures, an empty upstream,
  early 429/5xx/network errors, and first-chunk timeouts can advance to the next
  step. Non-retryable 4xx errors stop the chain.
- Any successful chunk commits the selected step, including an empty or
  reasoning-only chunk. No later error can switch providers, even if a downstream
  serializer emits no event for that chunk. Later failures become terminal SSE
  errors; EOF without a finish reason is also a failure.
- Chunks retain their order, tool fragments, reasoning where supported by the
  output protocol, and usage. The response model remains the requested alias;
  audit and usage records identify the physical model and effective connection.
- The 10-second step deadline covers stream setup, first output, and the selected
  stream lifetime. The 60-second combo deadline bounds fallback attempts and the
  selected stream. A late timeout ends the stream without fallback.
- The route retains one prefetched chunk and polls upstream only on downstream
  demand. It does not accumulate response text or arguments, create a producer
  task, or introduce a queue. Dropping selection or the returned stream releases
  upstream work, connection admission, and any circuit trial reservation.
- Stream accounting completes at EOF, not at the finish chunk, because usage can
  arrive afterward. After selection, cancellation records failure with any observed
  usage, without marking the provider unhealthy. Cancellation-safe persistence uses
  at most 64 asynchronous accounting jobs with a 10-second persistence deadline.
  Saturation, a missing runtime, or a persistence timeout is reported through
  `combo_stream_accounting_rejected_total` or `combo_stream_accounting_timeout_total`
  and warnings; persistence remains best-effort, as for other audit writes.
  `combo_stream_cancelled_total` counts cancellations after selection.
- Anthropic output retains at most 64 tool indices, one text index, one thinking
  index, and terminal metadata. Tool, text, and thinking blocks have distinct
  output indices. Terminal stops include usage received after the finish chunk.
  The Anthropic provider merges input/cache usage from `message_start` with output
  usage from `message_delta`.

### Upstream Quota Eligibility

Providers report their own quota limits through response headers. Rook captures those
values as `QuotaObservation` records, scoped to the connection that produced the
response. The records stay separate from locally measured usage (`UsageEntry`).

**Read model:**

- `QuotaObservationRecorderPort::latest_observations` (`QuotaObservationQuery`) returns
  the newest observation per connection, metric, scope, window, and source.
- `audit-sqlite` answers that query with a bounded SQL read, filtered by connection,
  provider, metric, and source.

**Evaluation** (`rook-core::quota::evaluate_connection_quota`):

| State           | Meaning                                                                      | Routing effect                                |
|-----------------|------------------------------------------------------------------------------|-----------------------------------------------|
| `Exhausted`     | Fresh, applicable evidence reports no remaining capacity                     | The connection is skipped before upstream I/O |
| `Available`     | Fresh, applicable evidence reports remaining capacity                        | The connection stays eligible                 |
| `Stale`         | The evidence is older than the freshness window, or its reset instant passed | The connection stays eligible                 |
| `NotApplicable` | The evidence describes a metric that does not apply to requests              | The connection stays eligible                 |
| `Unknown`       | No evidence exists                                                           | The connection stays eligible                 |

**Policy defaults** (`QuotaEligibilityPolicy`):

- Freshness window: 300 seconds.
- Exhaustion threshold: remaining of 0.
- Applicable metrics: `requests`, `tokens`, `input_tokens`, `output_tokens`.

The defaults are a starting point, not a provider contract. Deployments can narrow the
applicable metrics or the freshness window.

**Rules:**

- Only `Exhausted` skips a connection. Every other state fails open, so a missing or
  stale header never removes a working account.
- Only observations captured for that connection count. A provider-scoped observation
  never proves that one connection is exhausted.
- The reset instant comes from `reset_at` or from `reset_after_milliseconds` anchored to
  `observed_at`. A passed reset restores eligibility, and a newer observation with
  remaining capacity overrides an older exhaustion.
- The authorization preflight runs first. A quota decision never relaxes an API key
  restriction.
- A runtime 429 still follows the existing combo fallback rules.
- Skip reasons carry quota provenance only: provider, metric, counters, and timing.
  Credentials never reach the reason string.

### Provider Eligibility Stage

The first selection stage returns an `EligibilityResult` with eligible targets and one
`Rejection` for each rejected target. The public `RejectionReason` enum is closed and
serializes with stable `snake_case` values:

| Reason | Condition |
|---|---|
| `auth_missing` | The provider adapter reports that required authentication is unavailable. |
| `auth_expired` | The persisted connection has `TestStatus::Expired` and its adapter is unavailable. |
| `model_unsupported` | The provider does not support the requested model. |
| `capability_missing` | The provider cannot satisfy a required request capability. |
| `circuit_open` | The provider circuit is open or its half-open trial is already reserved. |
| `quota_exhausted` | Fresh, applicable connection quota evidence proves exhaustion. |
| `key_model_not_allowed` | The request model is outside the API key's model allowlist. |
| `key_provider_not_allowed` | The target is outside the API key's provider allowlist. |
| `connection_disabled` | The persisted provider connection is inactive. |

When a candidate fails more than one check, the evaluator returns one reason in this order:
key model, key provider, connection disabled, missing authentication, expired
authentication, unsupported model, missing capability, open circuit, then exhausted quota.
This makes rejection reporting deterministic.

The candidate set contains registered router providers. A persisted inactive connection is rejected
as `connection_disabled` only while its adapter remains registered, such as during a registry refresh
race. A connection already removed from the router is not a candidate. Only eligible registered
targets reach priority, round-robin, or weighted selection.
Managed adapters expose their connection identity through `ProviderPort::connection_id()`. A
connection-bound adapter with no persisted routing-state row is rejected as `connection_disabled`.
An unmanaged provider remains unbound even if its ID looks like a UUID that matches a connection ID
or runtime ID. Unbound non-UUID provider IDs may still match an active runtime ID for compatibility.
Each managed adapter also carries a persisted adapter-configuration revision. The revision changes
when adapter inputs change, including concurrency, endpoint, model declarations, or API format. It
does not change for name, priority, active state, quota thresholds, health-test status, OAuth token,
expiry, email, scope, or ID-token state, or re-encryption of semantically unchanged API-key plaintext.
OAuth project and client configuration changes increment the revision. A revision mismatch is rejected
as `connection_disabled` until the registry contains an adapter built from the current configuration.
Revision comparison uses the same normalization as credential persistence. A supplied replacement can
recover a stored credential that the key manager cannot decrypt.
After budget admission, Rook rechecks the selected managed connection's active state before upstream
I/O. It uses the same credential-free, bounded routing-state query. A cancellation-safe reservation
guard releases the durable budget if the request is canceled during that check or if the check rejects
the connection or its revision.
Cancellation cleanup uses at most 32 concurrent release tasks. If all slots are occupied, Rook keeps
the reservation charged and logs the saturation instead of spawning unbounded cleanup work.
The use case also confirms that the selected target appeared in the evaluated candidate snapshot;
this prevents a concurrent registry refresh from routing through a target whose quota or connection
state was not checked. It also confirms the selected adapter is the same instance evaluated for that
target. Combo and combo-stream paths use the credential-free state projection and recheck managed
adapter identity and revision before upstream I/O.
The selector retains each target's configured weight and cannot reintroduce a rejected target.
The routing stage reads a credential-free candidate-specific projection from
`ProviderRepositoryPort::list_routing_state_for_candidates`. SQLite returns only rows that match
registered target IDs and the active runtime row with lowest priority, then lowest ID. The use case and
repository reject more than 4,000 distinct target IDs; the use case rejects before starting the
query. The query runs in `spawn_blocking` behind a single-slot gate and has a 250 ms deadline. Rook
fails closed if it cannot read connection state within the deadline. A repository that does not
implement a credential-free routing-state projection fails closed; the default port does not call
credential-bearing `list()`. The SQLite query does not load credential columns.
The stage resolves an explicitly connection-bound target by exact connection UUID. For unbound,
non-UUID target IDs, it can resolve an active provider runtime ID. The runtime lookup uses priority
then connection ID to choose one deterministic row, so quota evidence remains scoped to the selected
connection.
If the connection-state read fails, the router stops before upstream I/O. Quota read failures remain
eligible under the existing bounded fail-open policy.

An expired OAuth access token remains eligible when the adapter is available, so the existing
request-time refresh path can run. Expired status rejects only when the adapter is also unavailable.
A quota read error, timeout, stale observation, missing observation, or saturated 1,000-series query
does not reject a candidate. A capped result remains unknown even if the returned observations appear
exhausted. The router queries each
connection separately under one shared 250 ms quota deadline, so one connection cannot consume
another connection's series limit. Candidates rejected by key, connection, authentication, model, or
capability filters do not trigger quota reads. The router refreshes circuit state after these reads
because a cooldown can expire while connection or quota data is loading. It evaluates each quota
result after the read completes so a reset that passes during the query restores eligibility.
If every candidate is rejected by the API key's provider allowlist, the request remains forbidden.
If a key-authorized candidate fails another eligibility check, the router returns
all-providers-exhausted when no candidate remains eligible.

### `transport-axum`

HTTP transport layer. All wire-format logic lives here.

- **`routes.rs`** — axum router assembling the client-facing `/v1` routes together with the auth, bootstrap, resilience, API-key, rate-limit, usage and cache routers
- **`openai_adapter.rs`** — OpenAI wire format ↔ domain model translation
- **`anthropic_adapter.rs`** — Anthropic `/v1/messages` wire format ↔ domain model
- **`responses_routes.rs`, `responses_adapter.rs`, `responses_stream.rs`** — the stateless `/v1/responses` surface, its request conversion and its streaming event encoder
- **`provider_routes.rs`** — `/api/providers` CRUD endpoints, mounted only when enabled
- **`oauth_routes.rs`, `combo_routes.rs`, `alias_routes.rs`, `models_routes.rs`** — dedicated routers for OAuth connections, combos, combo/model aliases and the model catalog
- **`provider_dto.rs`** — provider connection JSON DTOs; responses always include `credentials: {}`

### Provider crates (`providers-openai`, `providers-anthropic`, `providers-ollama`, `providers-gemini`, `providers-groq`)

Each implements `ProviderPort` for a specific API. All share the same structure:

- Config struct (id, api_key, base_url, models list, timeout_secs)
- `new()` → `Arc<Self>`
- `is_available()` — synchronous check (e.g., non-empty API key)
- `health_check()` — async, returns `HealthStatus` with latency
- `complete()` — makes the actual API call via `reqwest::Client`
- `stream()` — implemented in OpenAI, Anthropic (text + tool use), Ollama; stub in Gemini, Groq

### `cache-memory`

`DashMap`-based in-memory cache with TTL support. Implements `CachePort`.

### `audit-sqlite`

SQLite-backed audit log. Implements `AuditPort`. Auto-creates schema on init with indexes on `request_id`, `provider`, `timestamp`.

### `encryption-inmemory`

AES-256-GCM credential encryption with Argon2id key derivation. Implements `KeyManager`.

### `provider-sqlite`

SQLite-backed provider connection repository. Implements `ProviderRepositoryPort`.

### `apps/rook`

Binary crate. Assembles all infrastructure.

- **`config.rs`** — loads `RookConfig` from TOML, expands `~` in paths, expands `${ENV_VAR}` in api_key
- **`di.rs`** — `RookContainer::build()` — builds all providers, cache, audit, router, usecases. Single place where all crates are assembled.
- **`server.rs`** — axum server bootstrap with graceful shutdown
- **`main.rs`** — init tracing → load config → build container → start server

## Key Abstractions

### ProviderPort

```rust
#[async_trait]
pub trait ProviderPort: Send + Sync + 'static {
    fn id(&self) -> &ProviderId;
    fn supported_models(&self) -> &[ModelId];
    fn is_available(&self) -> bool;
    async fn health_check(&self) -> HealthStatus;
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse>;
    async fn stream(&self, req: &CompletionRequest) -> CortexResult<BoxStream<'_, CortexResult<StreamChunk>>>;
}
```

### RouterPort

```rust
#[async_trait]
pub trait RouterPort: Send + Sync {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>>;
    async fn on_failure(&self, provider: &ProviderId, error: &CortexError);
    fn providers(&self) -> Vec<ProviderId>;
}
```

### CachePort

```rust
#[async_trait]
pub trait CachePort: Send + Sync {
    async fn get(&self, key: &CacheKey) -> CortexResult<Option<CompletionResponse>>;
    async fn set(&self, key: &CacheKey, value: &CompletionResponse, ttl: Duration) -> CortexResult<()>;
    async fn delete(&self, key: &CacheKey) -> CortexResult<()>;
    async fn clear(&self) -> CortexResult<()>;
}
```

### AuditPort

```rust
#[async_trait]
pub trait AuditPort: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()>;
}
```

## Data Flow

```
Client HTTP Request
        │
        ▼
transport-axum/routes.rs ─── OpenAI/Anthropic adapter (wire format → domain)
        │
        ▼
rook-usecases/RouteRequest::execute(req)
        │
        ├─ CachePort::get(cache_key)      ← TTL cache (DashMap)
        │
        ▼
FallbackRouter::select(req)        ← circuit breaker + strategy
        │
        ▼
ProviderPort::complete(req)       ← actual API call (reqwest)
        │
        ├─ on success:
        │   ├─ CachePort::set(cache_key, resp, ttl)
        │   └─ AuditPort::record(success entry)
        │
        └─ on failure:
            ├─ RouterPort::on_failure(provider_id, error)  ← circuit breaker
            └─ AuditPort::record(failure entry)
        │
        ▼
transport-axum ─── domain response → wire format
        │
        ▼
Client HTTP Response
```

## Configuration Flow

```
rook.toml file
    │
    ▼
config::RookConfig::load()         ← toml::from_str + path expansion
    │
    ├─ Expands ~ in database.db_path to $HOME
    ├─ Expands ${ENV_VAR} in provider.api_key
    │
    ▼
di::RookContainer::build(&config) ← assembles all infrastructure
    │
    ├─ build_provider(pc) per provider  ← maps config.kind → provider impl
    ├─ InMemoryCache or NoOpCache
    ├─ SqliteAudit::new(database.db_path)
    ├─ FallbackRouter::new(providers, strategy)
    ├─ If auth.api_keys.enabled:
    │   ├─ require API_KEY_HASH_SECRET
    │   ├─ SqliteApiKeyRepository(database.db_path)
    │   └─ AuthenticateClientApi
    ├─ If provider_crud.enabled:
    │   ├─ require ENCRYPTION_PASSPHRASE and ENCRYPTION_SALT
    │   ├─ AesGcmKeyManager
    │   └─ SqliteProviderRepository(database.db_path)
    │
    ▼
RookUsecases { route_request, manage_providers, health_check, authenticate_client_api, manage_connections }
    │
    ▼
transport_axum::router(usecases, authz_config)  ← axum Router with routes + state
```

## Provider Registry Lifecycle

`ManageConnections::refresh_registry()` builds active SQLite provider connections and atomically replaces the runtime registry. It runs at startup and after connection create/update/delete when `provider_crud.enabled` is true; the registry starts empty.

## Observability

`tracing` + `tracing-subscriber` with env-filter. Structured JSON logs to stdout. Metrics via `metrics` crate (labels: provider, model, status).
