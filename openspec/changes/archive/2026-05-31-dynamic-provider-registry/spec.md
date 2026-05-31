# Dynamic Provider Registry — Specification

> **Purpose**: Define the runtime provider registry as a dynamic, SQLite-backed system with hot-reload capability. Replaces TOML `[[providers]]` as the sole runtime source for request routing.

---

## 1. Change Summary

The runtime provider registry (the `FallbackRouter` and the `ProviderRegistryPort` it implements) becomes dynamic and SQLite-backed. Providers are no longer loaded from TOML at startup — they are persisted in SQLite via the `ProviderConnection` CRUD from the prior change, and the router reads from an in-memory registry that is refreshed on every mutating CRUD operation. TOML `config.toml` retains only infrastructure configuration (server, routing, cache, database, audit, auth).

---

## 2. Scope

### 2.1 What Changes

- **`FallbackRouter.providers`**: Changes from `Vec<Arc<dyn ProviderPort>>` (owned, immutable after construction) to `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>` — thread-safe interior mutability for hot reload.
- **`ProviderRegistryPort` trait**: Three new mutation methods added — `replace_all`, `upsert`, and `remove`.
- **`build_provider_from_connection`**: A new non-private function (in `di.rs` or a dedicated module) that constructs a runtime `Arc<dyn ProviderPort>` from a decrypted `ProviderConnection`. Called by registry refresh.
- **`ManageConnections` CRUD operations**: After every `create`, `update`, and `delete` that succeeds, the registry is synchronously refreshed with the current SQLite state.
- **TOML `config.toml`**: The `providers` top-level field is removed. Infrastructure sections remain.
- **DI bootstrap**: `FallbackRouter` is constructed empty at startup. An initial registry refresh populates it from SQLite.
- **`/health` endpoint**: Unchanged — it calls `HealthCheck` which uses `ProviderRegistryPort`, and that behavior is identical after this change.
- **`/api/providers` CRUD routes**: Unchanged — `ManageConnections` drives them, and they continue to work as specified in `provider-connections-transport/spec.md`.

### 2.2 What Does NOT Change

- `ProviderConnection` aggregate, `ProviderKind`, `ConnectionId`, encryption format — all unchanged from `provider-connections/spec.md`.
- `ProviderRepositoryPort` trait and `SqliteProviderRepository` implementation.
- `ManageConnections` domain logic (validation, encryption, health probe flow).
- HTTP transport layer for CRUD routes.
- `HealthStatus` enum or `/health` response shape.
- `RoutingStrategy` enum or the circuit breaker.
- Encryption key derivation (Argon2id + AES-256-GCM).

---

## 3. TOML Configuration Changes

### 3.1 Removed

```toml
# OLD — removed entirely
[[providers]]
id = "openai-primary"
kind = "openai"
api_key = "${OPENAI_API_KEY}"
models = ["gpt-4o", "gpt-4o-mini"]
timeout_secs = 60
```

### 3.2 What Remains

```toml
# config.toml — infrastructure only

[server]
host = "0.0.0.0"
port = 8080

[routing]
strategy = "priority"   # priority | round-robin | model-based

[cache]
enabled = true
ttl_secs = 300

[database]
db_path = "~/.local/share/cortex/rook/rook.db"

[auth.api_keys]
enabled = true
allow_env_fallback = true

[provider_crud]
enabled = true
```

**`provider_crud` stays**: `provider_crud.enabled` still gates the CRUD HTTP routes. The dynamic registry operates independently of this flag — it is always active.

---

## 4. ProviderRegistryPort — Extended Trait

### 4.1 Extended Interface

The `ProviderRegistryPort` trait gains three methods:

```rust
pub trait ProviderRegistryPort: Send + Sync {
    /// Returns all registered provider IDs (for inspection, not mutation).
    fn providers(&self) -> Vec<ProviderId>;

    /// Returns a provider by ID, or None if not registered.
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;

    /// Replaces the entire in-memory provider set atomically.
    /// Used after a registry refresh triggered by CRUD operations.
    fn replace_all(&self, providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError>;

    /// Upserts a single provider — adds or updates by ProviderId.
    /// Returns an error if the provider could not be constructed.
    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError>;

    /// Removes a provider by ProviderId.
    /// No-op if the provider is not present.
    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError>;
}
```

### 4.2 RegistryError

```rust
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("provider build failed for '{provider_id}': {reason}")]
    ProviderBuildFailed { provider_id: ProviderId, reason: String },
    #[error("registry locked")]
    RegistryLocked,
}
```

### 4.3 replace_all Atomicity

`replace_all` replaces the entire inner `Vec` in a single write transaction on the `RwLock`. All reads (via `providers()` and `get()`) remain lock-free on the read path — they acquire the RwLock read guard only, never blocking each other. Writes acquire the write lock exclusively.

---

## 5. FallbackRouter Implementation Changes

### 5.1 Field Change

```rust
// BEFORE
pub struct FallbackRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    strategy: RoutingStrategy,
    circuits: DashMap<ProviderId, CircuitState>,
    round_robin_index: RwLock<usize>,
}

// AFTER
pub struct FallbackRouter {
    providers: Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>,  // Changed from Vec to Arc<RwLock<Vec>>
    strategy: RoutingStrategy,
    circuits: DashMap<ProviderId, CircuitState>,
    round_robin_index: RwLock<usize>,
}
```

### 5.2 Constructor

```rust
impl FallbackRouter {
    /// Creates a router with an empty provider set.
    /// Populated by a post-startup initial refresh from SQLite.
    pub fn new_empty(strategy: RoutingStrategy) -> Self {
        Self {
            providers: Arc::new(RwLock::new(Vec::new())),
            strategy,
            circuits: DashMap::new(),
            round_robin_index: RwLock::new(0),
        }
    }
}
```

`FallbackRouter::new` is retained (for tests) but marked `#[cfg(test)]`.

### 5.3 ProviderRegistryPort Implementation

```rust
impl ProviderRegistryPort for FallbackRouter {
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.read().unwrap().iter().map(|p| p.id().clone()).collect()
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.providers.read().unwrap().iter().find(|p| p.id() == id).cloned()
    }

    fn replace_all(&self, new_providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        let mut guard = self.providers.write().map_err(|_| RegistryError::RegistryLocked)?;
        *guard = new_providers;
        Ok(())
    }

    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        let mut guard = self.providers.write().map_err(|_| RegistryError::RegistryLocked)?;
        if let Some(existing) = guard.iter_mut().find(|p| p.id() == provider.id()) {
            *existing = provider;
        } else {
            guard.push(provider);
        }
        Ok(())
    }

    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError> {
        let mut guard = self.providers.write().map_err(|_| RegistryError::RegistryLocked)?;
        guard.retain(|p| p.id() != id);
        Ok(())
    }
}
```

### 5.4 available_providers Adjusted

```rust
fn available_providers<'a>(&'a self, model: &ModelId) -> Vec<&'a Arc<dyn ProviderPort>> {
    self.providers.read().unwrap()
        .iter()
        .filter(|p| {
            let id = p.id();
            let circuit = self.circuits.get(id).map(|s| s.clone()).unwrap_or_default();
            !circuit.is_open() && p.supports_model(model)
        })
        .collect()
}
```

---

## 6. build_provider_from_connection

This function converts a decrypted `ProviderConnection` into a runtime `Arc<dyn ProviderPort>`. It lives in `apps/rook/src/di.rs` (or a dedicated `provider-builder.rs` module within the app) and is called during registry refresh.

### 6.1 Signature

```rust
pub fn build_provider_from_connection(
    conn: &ProviderConnection,
    decrypted_credentials: &DecryptedCredentials,
    base_url_override: Option<String>,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError>
```

### 6.2 DecryptedCredentials

```rust
pub enum DecryptedCredentials {
    ApiKey { api_key: String },
    OAuth {
        email: String,
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        scope: String,
        id_token: String,
        project_id: String,
    },
}
```

### 6.3 base_url Override Rules

| ProviderKind | base_url required? | Default if not provided     |
|--------------|--------------------|-----------------------------|
| `openai`     | No                 | `https://api.openai.com`    |
| `anthropic`  | No                 | `https://api.anthropic.com` |
| `ollama`     | **Yes**            | N/A — error if missing      |
| `gemini`     | No                 | Uses provider's own default |
| `groq`       | No                 | `https://api.groq.com`      |

If `base_url_override` is provided, it is used regardless of provider kind.

### 6.4 ProviderBuildError

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderBuildError {
    #[error("unsupported provider kind: '{kind}'")]
    UnsupportedKind(String),
    #[error("ollama provider requires base_url")]
    OllamaRequiresBaseUrl,
    #[error("provider construction failed: {0}")]
    ConstructionFailed(String),
}
```

### 6.5 Construction Per Kind

**openai**:

```rust
OpenAIProvider::new(OpenAIProviderConfig {
    id: conn.provider_runtime_id.clone(),
    api_key: decrypted.api_key,
    base_url: base_url_override.unwrap_or_else(|| "https://api.openai.com".to_string()),
    models: conn.config.default_model.as_ref().map(|m| vec![m.clone()]).unwrap_or_default(),
    timeout_secs: 60,
}).map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
```

**anthropic**: analogous, default `https://api.anthropic.com`.

**ollama**:

```rust
OllamaProvider::new(OllamaProviderConfig {
    id: conn.provider_runtime_id.clone(),
    base_url: base_url_override.ok_or(ProviderBuildError::OllamaRequiresBaseUrl)?,
    models: conn.config.default_model.as_ref().map(|m| vec![m.clone()]).unwrap_or_default(),
    timeout_secs: 300,
}).map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
```

**gemini**: uses `providers_gemini::GeminiProvider::new` with default base_url (provider's own default).

**groq**: analogous, default `https://api.groq.com`.

---

## 7. Registry Refresh — Data Flow

### 7.1 Trigger

Every mutating `ManageConnections` operation (`create`, `update`, `delete`) triggers a **synchronous registry refresh** after the SQLite write commits successfully.

```rust
impl ManageConnections {
    pub async fn create(...) -> ManageConnectionsResult<ProviderConnection> {
        // ... SQLite write ...
        self.repo.create(&conn).await?;
        self.refresh_registry().await?;  // <-- added
        Ok(conn)
    }

    pub async fn update(...) -> ManageConnectionsResult<ProviderConnection> {
        // ... SQLite write with optimistic lock ...
        self.repo.update(&updated, request.expected_updated_at).await?;
        self.refresh_registry().await?;  // <-- added
        Ok(updated)
    }

    pub async fn delete(...) -> ManageConnectionsResult<()> {
        self.repo.delete(id).await?;
        self.refresh_registry().await?;  // <-- added
        Ok(())
    }
}
```

### 7.2 refresh_registry Implementation

```rust
impl ManageConnections {
    async fn refresh_registry(&self) -> ManageConnectionsResult<()> {
        let connections = self.repo.list().await?;
        let mut new_providers: Vec<Arc<dyn ProviderPort>> = Vec::new();
        let mut errors: Vec<ProviderBuildError> = Vec::new();

        for conn in connections {
            if !conn.is_active {
                continue;  // skip inactive connections
            }

            let decrypted = self.decrypt_credentials(&conn.credentials)?;
            let base_url = conn.config.base_url.clone();  // from ConnectionConfig

            match build_provider_from_connection(&conn, &decrypted, base_url) {
                Ok(provider) => new_providers.push(provider),
                Err(e) => {
                    tracing::error!(
                        connection_id = %conn.id,
                        provider_runtime_id = %conn.provider_runtime_id,
                        error = %e,
                        "failed to build provider during registry refresh"
                    );
                    errors.push(e);
                }
            }
        }

        self.registry.replace_all(new_providers).map_err(|e| {
            ManageConnectionsError::RegistryUpdateFailed(e.to_string())
        })?;

        if !errors.is_empty() {
            tracing::warn!(
                count = errors.len(),
                "registry refresh completed with {} build failures",
            );
        }

        Ok(())
    }
}
```

### 7.3 DecryptCredentials

```rust
impl ManageConnections {
    fn decrypt_credentials(&self, creds: &Credentials) -> ManageConnectionsResult<DecryptedCredentials> {
        match creds {
            Credentials::ApiKey { api_key } => Ok(DecryptedCredentials::ApiKey {
                api_key: self.key_manager.decrypt(api_key.as_str())
                    .map_err(|e| ManageConnectionsError::Encryption(e))?,
            }),
            Credentials::OAuth { email, access_token, refresh_token, expires_at, scope, id_token, project_id } => {
                Ok(DecryptedCredentials::OAuth {
                    email: self.key_manager.decrypt(email.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                    access_token: self.key_manager.decrypt(access_token.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                    refresh_token: self.key_manager.decrypt(refresh_token.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                    expires_at: *expires_at,
                    scope: self.key_manager.decrypt(scope.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                    id_token: self.key_manager.decrypt(id_token.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                    project_id: self.key_manager.decrypt(project_id.as_str())
                        .map_err(|e| ManageConnectionsError::Encryption(e))?,
                })
            }
        }
    }
}
```

### 7.4 ConnectionConfig.base_url

The `ConnectionConfig` struct (from `provider-connections/spec.md`) is extended to include an optional `base_url` field:

```rust
pub struct ConnectionConfig {
    pub max_concurrent: u32,
    pub quota_window_thresholds: QuotaWindowThresholds,
    pub default_model: Option<ModelId>,
    pub base_url: Option<String>,  // NEW — optional override for provider's base URL
}
```

When not set, each provider kind uses its hardcoded defaults (see section 6.3).

### 7.5 Initial Bootstrap Refresh

At startup, after DI is constructed and all components are wired:

```rust
// In RookContainer::build, after ManageConnections is constructed
if let Some(ref mc) = manage_connections {
    // Initial refresh populates the registry from existing SQLite state
    mc.refresh_registry().await
        .map_err(|e| anyhow::anyhow!("failed to seed provider registry from SQLite: {e}"))?;
}
```

---

## 8. Error Handling During Refresh

### 8.1 Partial Failure Policy

The refresh loop processes connections sequentially. A failure to build a single provider **does not abort** the refresh of other providers. After the loop:

1. Successfully built providers are installed via `replace_all`.
2. All build errors are logged at `ERROR` level with `connection_id` and `provider_runtime_id`.
3. A summary `WARN` log reports the count of failures.
4. `refresh_registry()` returns `Ok(())` even if some providers failed — the registry is not left empty if at least one provider succeeded.

### 8.2 All Providers Fail

If zero providers succeed, `replace_all([])` is called (empty registry). Subsequent requests return `CortexError::all_providers_exhausted()`. The system is not crashed — it remains operational but will reject requests until the registry is repaired via CRUD.

### 8.3 Encryption Failure

If decryption fails for a connection's credentials, the provider is skipped with an encryption error logged, and the refresh continues. The connection is NOT modified — it remains in SQLite and can be corrected via `PUT /api/providers/:id`.

---

## 9. Backwards Compatibility

### 9.1 `/health` Endpoint

Unchanged. `HealthCheck` holds `Arc<dyn ProviderRegistryPort>` and calls `providers()` to build per-provider health status. Since `providers()` now reads from `Arc<RwLock<Vec<...>>>`, behavior is identical.

### 9.2 `/api/providers` CRUD Routes

Unchanged. The CRUD routes are driven by `ManageConnections`, which calls `registry.get()` for health probes during `test()`. Since `registry.get()` now acquires a read lock on the dynamic registry, behavior is identical to the prior implementation.

### 9.3 HealthCheck Response Shape

The `/health` response JSON structure is unchanged. It still returns `healthy: bool`, `latency_ms: u64`, `last_error: Option<String>` per provider.

---

## 10. Migration: TOML to SQLite

This spec does NOT cover the migration script itself (out of scope per design decisions). The migration:

1. Reads `[[providers]]` from the existing `config.toml`.
2. Creates `ProviderConnection` rows in SQLite for each TOML provider.
3. Is a **one-time offline step** that runs before `rook` binary starts with the new binary.
4. Is documented as a separate runbook alongside the spec.

After migration completes, the TOML `providers` section is removed from `config.toml` and the infrastructure-only config is deployed.

---

## 11. ManageConnections Error Variants

The `ManageConnectionsError` enum is extended with one new variant:

```rust
pub enum ManageConnectionsError {
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("encryption error: {0}")]
    Encryption(#[from] CredentialEncryptionError),
    #[error("provider runtime not found: {0}")]
    ProviderRuntimeNotFound(ProviderId),
    #[error("registry update failed: {0}")]
    RegistryUpdateFailed(String),  // NEW — wraps RegistryError on refresh failure
}
```

---

## 12. DI Changes

### 12.1 RookContainer::build

```rust
pub fn build(config: &RookConfig) -> anyhow::Result<Self> {
    // 1. Infrastructure only (no TOML providers)
    let cache: Arc<dyn CachePort> = if config.cache.enabled {
        Arc::new(InMemoryCache::new(config.cache.ttl()))
    } else {
        Arc::new(NoOpCache)
    };

    // 2. Audit
    let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.database.db_path)?);

    // 3. Router — constructed EMPTY, will be populated by initial refresh
    let strategy: RoutingStrategy = config.routing.strategy.into();
    let fallback_router: Arc<FallbackRouter> = Arc::new(FallbackRouter::new_empty(strategy));
    let router: Arc<dyn RouterPort> = fallback_router.clone();
    let registry: Arc<dyn ProviderRegistryPort> = fallback_router;

    // 4. Provider CRUD (always enabled — no feature gate for registry)
    let passphrase = required_env("ENCRYPTION_PASSPHRASE")?;
    let salt = required_env("ENCRYPTION_SALT")?;
    let key_manager = Arc::new(
        AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
            .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
    );
    let repo: Arc<dyn ProviderRepositoryPort> =
        Arc::new(SqliteProviderRepository::new(&config.database.db_path)?);

    // 5. ManageConnections — drives CRUD AND registry refresh
    let manage_connections = ManageConnections::new(repo, registry.clone(), key_manager);

    // 6. Initial registry bootstrap from SQLite
    manage_connections.refresh_registry().await
        .map_err(|e| anyhow::anyhow!("failed to seed provider registry from SQLite: {e}"))?;

    // 7. Use cases
    let usecases = Arc::new(RookUsecases {
        route_request: RouteRequest::new(router.clone(), cache.clone(), audit.clone()),
        manage_providers: ManageProviders::new(router.clone()),
        health_check: HealthCheck::new(registry),
        authenticate_client_api: None,
        manage_connections: Some(manage_connections),
    });

    Ok(Self { usecases, authz_config })
}
```

### 12.2 Removed Config Fields

`ProviderConfig` struct is removed from `config.rs`. `RookConfig.providers: Vec<ProviderConfig>` is removed. The `build_provider()` function (the TOML-based one) is removed from `di.rs`.

---

## 13. Requirements

### R1: Dynamic Registry Reads

The router (`FallbackRouter`) MUST read from an in-memory `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>` for every `select()` and `get()` call. The TOML `[[providers]]` section MUST NOT be consulted at runtime for routing decisions.

### R2: CRUD Triggers Refresh

Every `create`, `update`, and `delete` on `ManageConnections` MUST synchronously call `refresh_registry()` after the SQLite write commits. The refresh MUST rebuild the full provider list from current SQLite state and call `registry.replace_all()`.

### R3: base_url Optionality

`base_url` in `ConnectionConfig` is optional. When absent for `openai`, `anthropic`, and `groq`, the provider's hardcoded default is used. When absent for `ollama`, the provider build MUST fail with `ProviderBuildError::OllamaRequiresBaseUrl`.

### R4: Inactive Connections Skipped

During refresh, connections with `is_active = false` are excluded from the registry and do not become routable.

### R5: Encryption Errors Do Not Crash

A decryption failure during refresh logs an error and skips that connection without aborting the refresh. The connection remains in SQLite.

### R6: Partial Failure Survives

If some providers fail to build during refresh, successfully built providers are installed via `replace_all`. Only a total failure (0 providers) results in an empty registry.

### R7: /health Unchanged

The `/health` endpoint MUST continue to return the same JSON structure with `healthy`, `latency_ms`, and `last_error` fields per provider.

### R8: No Provider CRUD Feature Gate for Registry

The dynamic registry is always active. It does not depend on `provider_crud.enabled`. The `ManageConnections` is always constructed in `RookContainer::build`.

---

## 14. Scenarios

### S-REG-01: Registry populated on startup

- **Given** SQLite contains 2 active `ProviderConnection` rows
- **When** `rook` starts and `RookContainer::build` runs
- **Then** `manage_connections.refresh_registry()` is called
- **And** the `FallbackRouter` has 2 providers registered
- **And** routing requests to `gpt-4o` hits the `openai` provider

### S-REG-02: Create triggers refresh

- **Given** the registry has 1 provider (from startup)
- **When** `POST /api/providers` creates a new connection
- **Then** after the SQLite insert, `refresh_registry()` runs
- **And** the registry now has 2 providers
- **And** routing to the new provider's model succeeds

### S-REG-03: Update triggers refresh

- **Given** 2 providers are registered
- **When** `PUT /api/providers/:id` updates a connection's `priority`
- **Then** after the SQLite update, `refresh_registry()` runs
- **And** the registry reflects the current SQLite state

### S-REG-04: Delete triggers refresh

- **Given** 2 providers are registered
- **When** `DELETE /api/providers/:id` removes a connection
- **Then** after the SQLite delete, `refresh_registry()` runs
- **And** the registry now has 1 provider

### S-REG-05: Inactive connection not registered

- **Given** a `ProviderConnection` row has `is_active = false`
- **When** `refresh_registry()` runs
- **Then** that provider is not added to the router's provider list
- **And** routing does not use that provider

### S-REG-06: Provider build failure during refresh — partial

- **Given** 2 connections exist, one with corrupt credentials
- **When** `refresh_registry()` runs
- **Then** the valid connection's provider is added to the registry
- **And** the corrupt connection logs an error and is skipped
- **And** the registry has 1 provider

### S-REG-07: All providers fail to build

- **Given** all `ProviderConnection` rows have invalid credentials
- **When** `refresh_registry()` runs
- **Then** `replace_all([])` is called with an empty vec
- **And** subsequent requests return `all_providers_exhausted`

### S-REG-08: ollama missing base_url

- **Given** a connection has `provider_kind = "ollama"` with no `base_url` in config
- **When** `build_provider_from_connection` is called
- **Then** `ProviderBuildError::OllamaRequiresBaseUrl` is returned
- **And** the connection is skipped in refresh with an error log

### S-REG-09: OAuth expiry checked before probe

- **Given** a connection has OAuth credentials where `expires_at` is in the past
- **When** `POST /api/providers/:id/test` is called
- **Then** `status: "expired"` is returned without calling the runtime provider
- **And** the runtime registry is NOT consulted

### S-REG-10: Health endpoint unchanged

- **Given** the registry has 2 providers
- **When** a client calls `GET /health`
- **Then** response includes both providers with `healthy: true` or `false`
- **And** response shape matches existing contract

---

## 15. Acceptance Criteria

| #    | Criterion                                                                                  | Validation                                              |
|------|--------------------------------------------------------------------------------------------|---------------------------------------------------------|
| AC1  | `FallbackRouter.providers` is `Arc<RwLock<Vec<...>>>`                                      | Code review                                             |
| AC2  | `ProviderRegistryPort` has `replace_all`, `upsert`, `remove` methods                       | Trait compilation                                       |
| AC3  | `FallbackRouter::new_empty` exists and creates empty registry                              | Unit test                                               |
| AC4  | `ManageConnections.create/update/delete` each call `refresh_registry()` after SQLite write | Code review                                             |
| AC5  | Registry refresh skips `is_active = false` connections                                     | Scenario S-REG-05                                       |
| AC6  | Registry refresh skips providers that fail to build, logs errors, continues                | Scenario S-REG-06                                       |
| AC7  | All providers fail → empty registry, `all_providers_exhausted` on requests                 | Scenario S-REG-07                                       |
| AC8  | `ollama` without `base_url` returns `ProviderBuildError::OllamaRequiresBaseUrl`            | Scenario S-REG-08                                       |
| AC9  | `/health` response shape unchanged                                                         | Integration test                                        |
| AC10 | TOML `config.toml` has no `[[providers]]` section in deployed config                       | Config review                                           |
| AC11 | `build_provider` from TOML removed from `di.rs`                                            | Code review                                             |
| AC12 | Initial bootstrap refresh on startup populates registry from SQLite                        | Scenario S-REG-01                                       |
| AC13 | Workspace tests pass                                                                       | `cargo test --workspace`                                |
| AC14 | Clippy passes with no warnings                                                             | `cargo clippy --workspace --all-targets -- -D warnings` |

---

## 16. Out of Scope For This Change

- Migration script (TOML → SQLite seeding) — separate work item.
- Hot reloading of a single connection without a full refresh — future work.
- Provider model list read from runtime (currently static from `default_model` in config).
- Health check intervals or background refresh scheduling.
- Dynamic provider kind registration (unknown kinds still rejected per `provider-connections/spec.md`).
