# Design: Fix Provider Empty Models List

## Technical Approach

Inject `Arc<dyn ModelCatalogPort>` into `DynamicProviderBuilder` at construction time. When `ProviderBuilderPort::build()` is called, synchronously query the catalog for models matching the `provider_kind`, then pass that list to `build_provider_from_connection`. The catalog query is a simple in-memory `Vec` filter — no async overhead.

```
di.rs: ManageConnections construction
    └── DynamicProviderBuilder::new(model_catalog: Arc<dyn ModelCatalogPort>)
            │
            ▼
ProviderBuilderPort::build(input: ProviderBuildInput)
    ├── catalog.list() → filter(|e| e.provider_kind == input.provider_kind)
    │                     → collect as Vec<ModelId>
    └── build_provider_from_connection(
            connection_id, kind, credentials, base_url,
            models: Vec<ModelId>   ← NEW parameter
        )
```

## Architecture Decisions

### Decision: Pass models list as parameter to existing function

**Choice**: Modify `build_provider_from_connection` signature to accept `models: Vec<ModelId>`, add parameter to all 6 match arms.

**Alternatives considered**:
- Create a new builder struct that carries catalog reference and calls catalog internally — adds indirection without benefit
- Make `ProviderBuildInput` carry the models list — would require changes in `manage_connections.rs` call sites to populate it, which is the same work but less explicit

**Rationale**: Minimal diff, single place to change, explicit parameter makes it clear models come from caller.

### Decision: Query catalog synchronously in `build()`

**Choice**: Call `model_catalog.list()` synchronously inside the async `build()` method.

**Alternatives considered**:
- Async catalog query — `list()` is already `async` but returns a `Vec` with no await needed in practice (static catalog). Making it sync would require removing `async_trait` from `ModelCatalogPort` which is a larger change.

**Rationale**: The `async fn list()` in `ModelCatalogPort` has a trivial implementation for `StaticModelCatalog` — it just returns `catalog()`. Calling `.await` is negligible overhead and keeps the trait unchanged.

### Decision: Filter catalog by `provider_kind` only, not by connection

**Choice**: Return ALL models for a given `provider_kind` from the catalog, regardless of which connection is being built.

**Alternatives considered**:
- Filter by connection-specific constraints (e.g., API key tier, region) — not currently modeled in the catalog, would require extending `ModelCatalogEntry`

**Rationale**: The catalog is the source of truth for which models exist per provider kind. If a model is in the catalog for `OllamaCloud`, any `OllamaCloud` connection can theoretically serve it. Further filtering (per-connection restrictions) is future work.

## Data Flow

```
RookUsecases::new(..., model_catalog: Arc<dyn ModelCatalogPort>, ...)
    └── ManageConnections::new(..., builder: DynamicProviderBuilder(model_catalog), ...)
            │
            ▼ (on connection activation)
        DynamicProviderBuilder::build(input: ProviderBuildInput)
            │
            ├── let models: Vec<ModelId> = model_catalog.list()
            │       .await
            │       .into_iter()
            │       .filter(|e| e.provider_kind == input.provider_kind)
            │       .map(|e| e.model_id)
            │       .collect();
            │
            └── build_provider_from_connection(
                    &input.connection_id,
                    input.provider_kind,
                    &input.decrypted_credentials,
                    input.base_url,
                    models,          ← injected
                )
                    │
                    ▼ (in each match arm)
                providers_ollama::OllamaProviderConfig {
                    ...
                    models: Vec<ModelId>,  ← used here (previously Vec::new())
                    ...
                }
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `apps/rook/src/di.rs` | Modify | Add `catalog: Arc<dyn ModelCatalogPort>` field to `DynamicProviderBuilder`; update `ManageConnections` construction to pass catalog; add `models` parameter to `build_provider_from_connection`; update all 6 match arms |
| `crates/application/rook-usecases/src/manage_connections.rs` | Modify | Update `DynamicProviderBuilder::new` signature to accept `Arc<dyn ModelCatalogPort>` |
| `crates/application/rook-usecases/tests/manage_connections_test_credentials.rs` | Modify | Update `DynamicProviderBuilder` construction in test to pass mock catalog |

## Interfaces / Contracts

### `DynamicProviderBuilder` struct (di.rs)

```rust
struct DynamicProviderBuilder {
    catalog: Arc<dyn ModelCatalogPort>,  // NEW field
}

impl DynamicProviderBuilder {
    fn new(catalog: Arc<dyn ModelCatalogPort>) -> Self {
        Self { catalog }
    }
}
```

### `build_provider_from_connection` signature (di.rs)

```rust
pub fn build_provider_from_connection(
    connection_id: &ConnectionId,
    kind: ProviderKind,
    credentials: &DecryptedCredentials,
    base_url_override: Option<String>,
    models: Vec<ModelId>,  // NEW parameter
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError>
```

### Updated match arms (di.rs)

All 6 `ProviderKind` match arms change from:
```rust
models: Vec::new(),
```
to:
```rust
models: models.clone(),
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | `DynamicProviderBuilder` passes catalog models to provider config | Test in `manage_connections_test_credentials.rs`: construct builder with mock catalog, call `build()`, verify `OllamaProvider` config has correct models |
| Unit | `supports_model()` returns true for cataloged model | Already covered by existing `provider_supports_model_test` in `manage_connections_test_credentials.rs` — ensure it runs for OllamaCloud |
| Integration | Full request with `ollamacloud/qwen3-coder-next` succeeds | Existing E2E tests (or manual verification with local Rook) |

## Migration / Rollout

No migration required. This change only affects in-memory provider construction at startup/connection activation. No DB schema changes, no feature flags needed.

## Open Questions

- None — the fix is straightforward and low-risk.