# Design: dynamic-provider-registry

## Technical Approach

Replace TOML `[[providers]]` as the runtime source with a SQLite-backed dynamic registry. `FallbackRouter` holds `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>` instead of a plain `Vec`. `ManageConnections` calls `refresh_registry()` after every mutating CRUD operation (create/update/delete). At startup, `RookContainer::build` seeds the registry from existing SQLite state before accepting traffic. TOML config loses its `providers` field entirely.

---

## Architecture Decisions

### Decision: RwLock over Mutex for provider list

**Choice**: `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>`
**Alternatives considered**: `Arc<Mutex<Vec<...>>>`, channel-based actor model
**Rationale**: Read-heavy workload (every `select()` call needs the list) but writes are infrequent (only on CRUD-triggered refresh). `RwLock` allows concurrent readers while a writer holds exclusive access. `Mutex` would block all reads during write, which is unnecessary contention for this access pattern.

### Decision: Refresh replaces entire provider list atomically

**Choice**: `replace_all` atomically swaps the inner `Vec`
**Alternatives considered**: Incremental upsert per connection, copy-on-write clone per request
**Rationale**: Simpler correctness — no partial state observable during refresh. Incremental updates would require holding the write lock for the entire duration of building all providers (including network I/O for credential decryption). Atomic swap means the read path sees either the old list or the new list, never a partially-built one.

### Decision: Partial failure survives refresh

**Choice**: If any providers fail to build during refresh, successful ones are installed and failures are logged
**Rationale**: A single corrupt connection should not empty the entire registry. The operator can fix the connection via CRUD without the system going dark.

---

## Data Flow

```
CRUD request (POST/PUT/DELETE /api/providers)
  → ManageConnections.create/update/delete
  → SqliteProviderRepository write
  → ManageConnections.refresh_registry()
  → repo.list() → all active connections
  → for each: decrypt_credentials() → build_provider_from_connection()
  → registry.replace_all(new_providers)
  → router sees updated list on next select()
```

Startup flow:

```
RookContainer::build
  → FallbackRouter::new_empty(strategy)   // empty registry
  → ManageConnections::new(...)
  → manage_connections.refresh_registry() // seed from SQLite
  → router is now populated
```

---

## Module-by-Module Changes

### `crates/domain/rook-core/src/ports.rs`

`ProviderRegistryPort` trait gains three methods:

```rust
// BEFORE
pub trait ProviderRegistryPort: Send + Sync {
    fn providers(&self) -> Vec<ProviderId>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;
}

// AFTER
pub trait ProviderRegistryPort: Send + Sync {
    fn providers(&self) -> Vec<ProviderId>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;
    fn replace_all(&self, providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError>;
    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError>;
    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("provider build failed for '{provider_id}': {reason}")]
    ProviderBuildFailed { provider_id: ProviderId, reason: String },
    #[error("registry locked")]
    RegistryLocked,
}
```

`RegistryError::ProviderBuildFailed` enables detailed error reporting from `refresh_registry`, even though the trait method itself doesn't carry per-provider errors — those are accumulated in `ManageConnections` before calling `replace_all`.

### `crates/application/rook-usecases/src/router_impl.rs`

`FallbackRouter` field changes from owned `Vec` to shared interior mutability:

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
    providers: Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>,  // shared, thread-safe
    strategy: RoutingStrategy,
    circuits: DashMap<ProviderId, CircuitState>,
    round_robin_index: RwLock<usize>,
}
```

New constructor for empty startup:

```rust
impl FallbackRouter {
    /// Constructs a router with no providers.
    /// Providers are added by initial refresh from SQLite.
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

`ProviderRegistryPort` impl updated — every method that reads `providers` acquires the read guard; mutation methods acquire write guard:

```rust
impl ProviderRegistryPort for FallbackRouter {
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.read().unwrap().iter().map(|p| p.id().clone()).collect()
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.providers
            .read()
            .unwrap()
            .iter()
            .find(|p| p.id() == id)
            .cloned()
    }

    fn replace_all(&self, new_providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        let mut guard = self
            .providers
            .write()
            .map_err(|_| RegistryError::RegistryLocked)?;
        *guard = new_providers;
        Ok(())
    }

    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        let mut guard = self
            .providers
            .write()
            .map_err(|_| RegistryError::RegistryLocked)?;
        if let Some(existing) = guard.iter_mut().find(|p| p.id() == provider.id()) {
            *existing = provider;
        } else {
            guard.push(provider);
        }
        Ok(())
    }

    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError> {
        let mut guard = self
            .providers
            .write()
            .map_err(|_| RegistryError::RegistryLocked)?;
        guard.retain(|p| p.id() != id);
        Ok(())
    }
}
```

`available_providers` helper (used by `select`) acquires read lock:

```rust
fn available_providers<'a>(&'a self, model: &ModelId) -> Vec<&'a Arc<dyn ProviderPort>> {
    self.providers
        .read()
        .unwrap()
        .iter()
        .filter(|p| {
            let id = p.id();
            let circuit = self.circuits.get(id).map(|s| s.clone()).unwrap_or_default();
            !circuit.is_open() && p.supported_models().contains(model)
        })
        .collect()
}
```

`RouterPort::providers()` also updated to read through the lock. The existing `new(providers, strategy)` constructor is kept for tests only (`#[cfg(test)]`).

### `crates/application/rook-usecases/src/manage_connections.rs`

`ManageConnectionsError` gains:

```rust
pub enum ManageConnectionsError {
    // ... existing variants ...
    #[error("registry update failed: {0}")]
    RegistryUpdateFailed(String),  // wraps RegistryError on refresh failure
}
```

`refresh_registry` private method added:

```rust
impl ManageConnections {
    // ... existing methods unchanged ...

    async fn refresh_registry(&self) -> ManageConnectionsResult<()> {
        let connections = self.repo.list().await?;

        let mut new_providers: Vec<Arc<dyn ProviderPort>> = Vec::new();
        let mut errors: Vec<ProviderBuildError> = Vec::new();

        for conn in connections {
            if !conn.is_active {
                continue;
            }

            let decrypted = match self.decrypt_credentials(&conn.credentials) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(
                        connection_id = %conn.id,
                        "failed to decrypt credentials during registry refresh: {e}"
                    );
                    continue;
                }
            };

            let base_url = conn.config.base_url.clone();

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
                "registry refresh completed with {} build failures"
            );
        }

        Ok(())
    }

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

`create`, `update`, and `delete` each call `refresh_registry()` after the SQLite write:

```rust
pub async fn create(...) -> ManageConnectionsResult<ProviderConnection> {
    // ... validate, build conn, write to repo ...
    self.repo.create(&conn).await?;
    self.refresh_registry().await?;  // refresh after write
    Ok(conn)
}

pub async fn update(...) -> ManageConnectionsResult<ProviderConnection> {
    // ... validate, build updated conn, write to repo with expected_updated_at ...
    self.repo.update(&updated, request.expected_updated_at).await?;
    self.refresh_registry().await?;  // refresh after write
    Ok(updated)
}

pub async fn delete(&self, id: &ConnectionId) -> ManageConnectionsResult<()> {
    self.repo.delete(id).await?;
    self.refresh_registry().await?;  // refresh after write
    Ok(())
}
```

### `crates/domain/rook-core/src/provider_connection.rs`

`ConnectionConfig` gains `base_url`:

```rust
// BEFORE
pub struct ConnectionConfig {
    pub max_concurrent: u32,
    pub quota_window_thresholds: QuotaWindowThresholds,
    pub default_model: Option<ModelId>,
}

// AFTER
pub struct ConnectionConfig {
    pub max_concurrent: u32,
    pub quota_window_thresholds: QuotaWindowThresholds,
    pub default_model: Option<ModelId>,
    pub base_url: Option<String>,  // NEW — optional override for provider's default base URL
}
```

### `crates/domain/rook-core/src/decrypted_credentials.rs` (new file)

`DecryptedCredentials` enum for use during registry refresh:

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

### `apps/rook/src/di.rs`

Removed: `providers: Vec<ProviderConfig>` from config, `build_provider()` TOML-based private function, all `ProviderConfig` handling.

Added: `build_provider_from_connection` public function, initial bootstrap refresh call.

`RookContainer::build` changes:

```rust
pub fn build(config: &RookConfig) -> anyhow::Result<Self> {
    // 1. Cache
    let cache: Arc<dyn CachePort> = if config.cache.enabled {
        Arc::new(InMemoryCache::new(config.cache.ttl()))
    } else {
        Arc::new(NoOpCache)
    };

    // 2. Audit
    let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.database.db_path)?);

    // 3. Router — constructed EMPTY, populated by initial refresh
    let strategy: RoutingStrategy = config.routing.strategy.into();
    let fallback_router: Arc<FallbackRouter> = Arc::new(FallbackRouter::new_empty(strategy));
    let router: Arc<dyn RouterPort> = fallback_router.clone();
    let registry: Arc<dyn ProviderRegistryPort> = fallback_router;

    // 4. Provider CRUD — always constructed (registry is always active)
    let passphrase = required_env("ENCRYPTION_PASSPHRASE")?;
    let salt = required_env("ENCRYPTION_SALT")?;
    let key_manager = Arc::new(
        AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
            .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
    );
    let repo: Arc<dyn ProviderRepositoryPort> =
        Arc::new(SqliteProviderRepository::new(&config.database.db_path)?);
    let manage_connections = ManageConnections::new(repo, registry.clone(), key_manager);

    // 5. Initial registry bootstrap from SQLite
    manage_connections.refresh_registry().await
        .map_err(|e| anyhow::anyhow!("failed to seed provider registry from SQLite: {e}"))?;

    // 6. Use cases
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

`build_provider_from_connection` — full implementation:

```rust
/// Builds a runtime provider from a decrypted ProviderConnection.
/// Returns an error for ollama if base_url is not provided.
pub fn build_provider_from_connection(
    conn: &ProviderConnection,
    decrypted: &DecryptedCredentials,
    base_url_override: Option<String>,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError> {
    let models: Vec<ModelId> = conn
        .config
        .default_model
        .iter()
        .cloned()
        .collect();

    match conn.provider_kind {
        ProviderKind::OpenAI => {
            let DecryptedCredentials::ApiKey { api_key } = decrypted else {
                return Err(ProviderBuildError::ConstructionFailed(
                    "OpenAI requires ApiKey credentials".to_string(),
                ));
            };
            let base_url = base_url_override
                .unwrap_or_else(|| "https://api.openai.com".to_string());
            OpenAIProvider::new(providers_openai::OpenAIProviderConfig {
                id: conn.provider_runtime_id.clone(),
                api_key: api_key.clone(),
                base_url,
                models,
                timeout_secs: 60,
            })
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
            .map(|p| Arc::new(p) as Arc<dyn ProviderPort>)
        }
        ProviderKind::Anthropic => {
            let DecryptedCredentials::ApiKey { api_key } = decrypted else {
                return Err(ProviderBuildError::ConstructionFailed(
                    "Anthropic requires ApiKey credentials".to_string(),
                ));
            };
            let base_url = base_url_override
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            AnthropicProvider::new(providers_anthropic::AnthropicProviderConfig {
                id: conn.provider_runtime_id.clone(),
                api_key: api_key.clone(),
                base_url,
                models,
                timeout_secs: 60,
            })
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
            .map(|p| p as Arc<dyn ProviderPort>)
        }
        ProviderKind::Ollama => {
            let base_url = base_url_override
                .ok_or(ProviderBuildError::OllamaRequiresBaseUrl)?;
            OllamaProvider::new(providers_ollama::OllamaProviderConfig {
                id: conn.provider_runtime_id.clone(),
                base_url,
                models,
                timeout_secs: 300,
            })
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
            .map(|p| p as Arc<dyn ProviderPort>)
        }
        ProviderKind::Gemini => {
            let DecryptedCredentials::ApiKey { api_key } = decrypted else {
                return Err(ProviderBuildError::ConstructionFailed(
                    "Gemini requires ApiKey credentials".to_string(),
                ));
            };
            // No base_url override for Gemini (uses provider's own default)
            GeminiProvider::new(providers_gemini::GeminiProviderConfig {
                id: conn.provider_runtime_id.clone(),
                api_key: api_key.clone(),
                models,
                timeout_secs: 60,
            })
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
            .map(|p| p as Arc<dyn ProviderPort>)
        }
        ProviderKind::Groq => {
            let DecryptedCredentials::ApiKey { api_key } = decrypted else {
                return Err(ProviderBuildError::ConstructionFailed(
                    "Groq requires ApiKey credentials".to_string(),
                ));
            };
            let base_url = base_url_override
                .unwrap_or_else(|| "https://api.groq.com".to_string());
            GroqProvider::new(providers_groq::GroqProviderConfig {
                id: conn.provider_runtime_id.clone(),
                api_key: api_key.clone(),
                base_url,
                models,
                timeout_secs: 60,
            })
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))
            .map(|p| p as Arc<dyn ProviderPort>)
        }
    }
}
```

`ProviderBuildError` lives in `di.rs` or a dedicated module:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderBuildError {
    #[error("unsupported provider kind: '{0}'")]
    UnsupportedKind(String),
    #[error("ollama provider requires base_url")]
    OllamaRequiresBaseUrl,
    #[error("provider construction failed: {0}")]
    ConstructionFailed(String),
}
```

### `apps/rook/src/config.rs`

`RookConfig.providers: Vec<ProviderConfig>` removed. `ProviderConfig` struct removed.

```rust
// BEFORE
pub struct RookConfig {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub cache: CacheConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub provider_crud: ProviderCrudConfig,
    pub providers: Vec<ProviderConfig>,  // REMOVED
}

// AFTER
pub struct RookConfig {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub cache: CacheConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub provider_crud: ProviderCrudConfig,
    // no providers field
}
```

`load()` no longer expands `${VAR}` in provider api_keys (no longer relevant since no providers). `ProviderConfig` struct removed entirely.

---

## File Changes

| File                                                         | Action | Description                                                                                                                    |
|--------------------------------------------------------------|--------|--------------------------------------------------------------------------------------------------------------------------------|
| `crates/domain/rook-core/src/ports.rs`                       | Modify | Add `replace_all`, `upsert`, `remove` to `ProviderRegistryPort`; add `RegistryError`                                           |
| `crates/domain/rook-core/src/provider_connection.rs`         | Modify | Add `base_url: Option<String>` to `ConnectionConfig`                                                                           |
| `crates/domain/rook-core/src/decrypted_credentials.rs`       | Create | New module: `DecryptedCredentials` enum                                                                                        |
| `crates/application/rook-usecases/src/router_impl.rs`        | Modify | `providers` field → `Arc<RwLock<Vec<...>>>`, all lock patterns updated                                                         |
| `crates/application/rook-usecases/src/manage_connections.rs` | Modify | Add `RegistryUpdateFailed` error, `refresh_registry()` method, call it from create/update/delete                               |
| `crates/application/rook-usecases/src/lib.rs`                | Modify | Re-export `RegistryError` from `rook_core`                                                                                     |
| `apps/rook/src/di.rs`                                        | Modify | Remove TOML provider build; add `build_provider_from_connection` and `ProviderBuildError`; add initial `refresh_registry` call |
| `apps/rook/src/config.rs`                                    | Modify | Remove `providers: Vec<ProviderConfig>` from `RookConfig`; remove `ProviderConfig` struct                                      |

---

## Testing Strategy

### Unit Tests

**`router_impl.rs`** (existing test module extended):

- `fallback_router_new_empty_creates_empty_registry`: verifies `providers()` returns empty after `new_empty`
- `provider_registry_replace_all_atomic`: calls `replace_all([p1, p2])` on router created via `new_empty`, verifies both `providers()` and `get()` return correct results
- `provider_registry_upsert_adds_new_provider`: verifies upserting a new provider adds it to the list
- `provider_registry_upsert_updates_existing_provider`: verifies upserting with same ID updates, not duplicates
- `provider_registry_remove_eliminates_provider`: verifies remove leaves the provider out of `providers()`
- `available_providers_excludes_circuit_open_providers`: existing circuit breaker test still valid
- `select_reads_from_locked_provider_list`: concurrent reads during `replace_all` — stress test with multiple readers and one writer

**`manage_connections.rs`** (test module extended):

- `refresh_registry_skips_inactive_connections`: repo returns mixed active/inactive, verify only active are in registry
- `refresh_registry_decrypts_and_builds_provider`: mock repo returns a connection, mock key_manager returns plaintext, verify correct provider kind built
- `refresh_registry_partial_failure_keeps_valid_providers`: two connections, one decrypt fails, verify the other is still added via `replace_all`
- `refresh_registry_all_failures_results_in_empty_registry`: verify `replace_all([])` is called when all builds fail
- `create_calls_refresh_after_write`: mock repo, verify `refresh_registry` called once after `repo.create`
- `update_calls_refresh_after_write`: mock repo, verify `refresh_registry` called once after `repo.update`
- `delete_calls_refresh_after_write`: mock repo, verify `refresh_registry` called once after `repo.delete`

**`di.rs`** (new test module):

- `build_provider_from_connection_openai_uses_default_base_url`: no override, verify default URL used
- `build_provider_from_connection_openai_uses_override`: override provided, verify it takes precedence
- `build_provider_from_connection_ollama_requires_base_url`: no override, verify `ProviderBuildError::OllamaRequiresBaseUrl`
- `build_provider_from_connection_ollama_uses_override`: override provided, verify ollama builds successfully
- `build_provider_from_connection_unknown_kind`: verify `ProviderBuildError::UnsupportedKind`

### Integration Tests

- **Refresh chain**: `POST /api/providers` creates a connection, then `GET /health` shows the new provider — validates the full refresh chain end-to-end
- **Startup with empty SQLite**: start with empty DB, verify `/health` returns empty provider list (not an error)
- **Refresh on delete**: create two providers via CRUD, `DELETE` one, verify `/health` shows only the remaining one
- **Persistence**: restart the server, verify registry still reflects SQLite state (no TOML re-read)
- **Partial failure**: add two connections, corrupt one credential in SQLite directly, verify the other still works

---

## Error Handling

### `ProviderBuildError` variants

| Variant                      | When triggered                                                     | Effect on refresh                                    |
|------------------------------|--------------------------------------------------------------------|------------------------------------------------------|
| `UnsupportedKind(String)`    | Unknown `ProviderKind` (exhaustive enum — should not occur)        | Provider skipped, logged at ERROR with connection_id |
| `OllamaRequiresBaseUrl`      | `provider_kind = "ollama"` and no `base_url` in `ConnectionConfig` | Provider skipped, logged at ERROR with connection_id |
| `ConstructionFailed(String)` | Provider constructor returns `Err`                                 | Provider skipped, logged at ERROR with connection_id |

### `refresh_registry` failure modes

1. **Decryption failure**: logs error, skips connection, continues. Connection remains in SQLite unaltered.
2. **Build failure for some providers**: logs each failure at ERROR, calls `replace_all` with successful providers only. Returns `Ok(())`.
3. **All providers fail**: calls `replace_all([])` (empty registry). Router returns `all_providers_exhausted` on next request. Operator must fix connections via CRUD.
4. **`replace_all` lock poisoning** (poisoned mutex): returns `RegistryError::RegistryLocked`, wrapped as `ManageConnectionsError::RegistryUpdateFailed`. This aborts the CRUD operation but does not crash the process.

### CRUD operations after `refresh_registry` failure

If `refresh_registry()` fails after a SQLite write has committed, the connection is persisted but the registry may be out of sync with SQLite. This is logged as a critical error. A subsequent CRUD operation will retry the refresh and bring the registry back in sync.

---

## Open Questions

- [ ] **OAuth provider support**: OpenAI/Anthropic/Groq with OAuth credentials currently fail at build time with `ConstructionFailed`. Should we add `ProviderBuildError::OAuthNotSupported` and block OAuth credential types for these providers at creation time? Or allow creation and fail at build time?
- [ ] **Startup failure on empty registry**: If `refresh_registry()` fails at startup, the process currently fails with `anyhow::anyhow!`. Should there be a startup mode that allows the server to start with an empty registry for migration scenarios?
- [ ] **Connection priority ordering**: `refresh_registry` builds providers in `repo.list()` order. Should the registry be sorted by `priority` field before calling `replace_all` so that higher-priority providers appear first in the list?
