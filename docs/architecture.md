# Architecture

## Layer Diagram

```
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
- `NuxaError` — error types: ProviderError, NotFoundError, RateLimitedError, AllProvidersExhaustedError
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
- **`RouteRequest`** — the main orchestrator: cache → select provider → execute → cache response → audit → handle failure
- **`FallbackRouter`** — implements RouterPort with three strategies: Priority, RoundRobin, ModelBased. Includes circuit breaker (3 failures → 30s cooldown).
- **`ManageProviders`** — enable/disable providers (interface only for now)
- **`HealthCheck`** — aggregated health status across all providers
- **`ManageConnections`** — runtime-managed provider connection CRUD/test workflow

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
- `stream()` — stub in all providers except OpenAI (not yet implemented)

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
    async fn complete(&self, req: &CompletionRequest) -> NuxaResult<CompletionResponse>;
    async fn stream(&self, req: &CompletionRequest) -> NuxaResult<BoxStream<'_, NuxaResult<StreamChunk>>>;
}
```

### RouterPort
```rust
#[async_trait]
pub trait RouterPort: Send + Sync {
    async fn select(&self, req: &CompletionRequest) -> NuxaResult<Arc<dyn ProviderPort>>;
    async fn on_failure(&self, provider: &ProviderId, error: &NuxaError);
    fn providers(&self) -> Vec<ProviderId>;
}
```

### CachePort
```rust
#[async_trait]
pub trait CachePort: Send + Sync {
    async fn get(&self, key: &CacheKey) -> NuxaResult<Option<CompletionResponse>>;
    async fn set(&self, key: &CacheKey, value: &CompletionResponse, ttl: Duration) -> NuxaResult<()>;
    async fn delete(&self, key: &CacheKey) -> NuxaResult<()>;
    async fn clear(&self) -> NuxaResult<()>;
}
```

### AuditPort
```rust
#[async_trait]
pub trait AuditPort: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> NuxaResult<()>;
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

## Provider CRUD Limitation

Provider CRUD is an administrative storage and health-test surface in v1. SQLite provider connections are not hot-registered into the request router; TOML providers continue to serve completion traffic.

## Observability

`tracing` + `tracing-subscriber` with env-filter. Structured JSON logs to stdout. Metrics via `metrics` crate (labels: provider, model, status).
