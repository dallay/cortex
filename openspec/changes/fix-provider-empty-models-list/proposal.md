# Proposal: Fix Provider Empty Models List — "All Providers Exhausted"

## Intent

On the first request to an Ollama Cloud provider (e.g., `ollamacloud/qwen3-coder-next`), Rook returns "all providers exhausted" even though the health check shows the provider is healthy with ~1000ms latency. The root cause: `build_provider_from_connection` constructs all providers with `models: Vec::new()` (empty list), causing `supports_model()` to always return `false`. The router then discards all providers as unavailable.

## Scope

### In Scope
- Modify `DynamicProviderBuilder` to receive `Arc<dyn ModelCatalogPort>` at construction time
- Modify `build_provider_from_connection` signature to accept `models: Vec<ModelId>` 
- Update all 6 `ProviderKind` match arms in `build_provider_from_connection` to use the passed models list instead of `Vec::new()`
- Update `ManageConnections::new` to pass the model catalog to `DynamicProviderBuilder`
- Add a unit test that verifies `supports_model()` returns `true` when model is in the catalog

### Out of Scope
- Dynamic model discovery from provider APIs (future work — requires separate spec)
- Changes to `ProviderPort` trait or other provider implementations
- Dashboard changes

## Approach

Inject the `ModelCatalogPort` into `DynamicProviderBuilder` at construction time. When `ProviderBuilderPort::build()` is called, query the catalog for models matching the `provider_kind`, then pass that list to `build_provider_from_connection`.

```
ManageConnections (at construction)
  └── builder: DynamicProviderBuilder(model_catalog: Arc<dyn ModelCatalogPort>)
        └── build(input: ProviderBuildInput)
              ├── catalog.list() → filter by provider_kind → Vec<ModelId>
              └── build_provider_from_connection(..., models: Vec<ModelId>)
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `apps/rook/src/di.rs` | Modified | Add `model_catalog` field to `DynamicProviderBuilder`; update `ManageConnections` construction and `build_provider_from_connection` signature |
| `crates/application/rook-usecases/src/manage_connections.rs` | Modified | `ProviderBuilderPort::build` receives catalog-backed models |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Breaking `ProviderBuilderPort` impl if signature changes incorrectly | Low | Only add field to struct, don't change interface method signature |
| `model_catalog` not available in all construction paths | Low | `DynamicProviderBuilder` always gets it in DI; verify at construction |

## Rollback Plan

Revert the 3 files changed (`di.rs`, `manage_connections.rs` test, `manage_connections.rs` impl). No DB migration needed — this only changes in-memory provider construction.

## Dependencies

- `ModelCatalogPort` already exists in `rook-core` and is already injected into `RookUsecases` — no new trait needed

## Success Criteria

- [ ] `cargo test -p rook-usecases --lib` passes
- [ ] `cargo test -p providers-ollama --lib` passes  
- [ ] A request with model `ollamacloud/qwen3-coder-next` to an active OllamaCloud connection does NOT return "all providers exhausted"
- [ ] Health check still returns healthy status (no regression)
- [ ] All 6 provider kinds (`OpenAI`, `Anthropic`, `Ollama`, `OllamaCloud`, `Gemini`, `Groq`) are constructed with non-empty models list when models exist in catalog