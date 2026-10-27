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

`apps/rook/di` conecta `routing.default_combo` con `RouteRequest` después de
cargar los combos. El arranque rechaza un UUID mal formado, un default
inexistente o inválido; la comprobación incluye los combos persistidos en SQLite.
`RouteRequest` aplica el default únicamente si la solicitud no selecciona un
combo mediante `X-Rook-Combo` ni un alias `rook/*`. Ambas modalidades de ejecución
reutilizan las comprobaciones de autorización y el flujo existente de combos.
Todos los pasos deben estar autorizados antes de cualquier llamada upstream.
La guía de configuración incluye las instrucciones de migración de DALLAY-710.

Las claves heredadas `cache.token_cache.mode` y `cache.token_cache.providers`
solo se aceptan por compatibilidad y generan advertencias si aparecen en la
configuración. No controlan inyección de token cache ni la caché de respuestas.

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

| State | Meaning | Routing effect |
| -- | -- | -- |
| `Exhausted` | Fresh, applicable evidence reports no remaining capacity | The connection is skipped before upstream I/O |
| `Available` | Fresh, applicable evidence reports remaining capacity | The connection stays eligible |
| `Stale` | The evidence is older than the freshness window, or its reset instant passed | The connection stays eligible |
| `NotApplicable` | The evidence describes a metric that does not apply to requests | The connection stays eligible |
| `Unknown` | No evidence exists | The connection stays eligible |

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

### `transport-axum`

HTTP transport layer. All wire-format logic lives here.

- **`routes.rs`** — axum router with four endpoints
- **`openai_adapter.rs`** — OpenAI wire format ↔ domain model translation
- **`anthropic_adapter.rs`** — Anthropic `/v1/messages` wire format ↔ domain model
- **`provider_routes.rs`** — `/api/providers` CRUD endpoints, mounted only when enabled
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
