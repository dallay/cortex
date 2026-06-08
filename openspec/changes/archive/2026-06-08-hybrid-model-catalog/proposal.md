# Proposal: Hybrid Model Catalog — Expand Static Catalog + Enable Dynamic Discovery

## Intent

Rook's `StaticModelCatalog` currently exposes only 3-5 hardcoded models per provider, making models like `ollamacloud/qwen3-coder-next` unavailable in the dashboard's API key restriction UI. The goal is to expand the catalog to cover all Ollama Cloud models (mirroring OmniRoute's provider registry) and establish the architecture for future dynamic introspection per provider.

**Why**: Users cannot restrict API keys to models they actually want to use. The catalog must reflect the real model zoo available through each provider connection.

## Scope

### In Scope
- Expand `StaticModelCatalog` with all Ollama Cloud models from OmniRoute's registry (8+ models)
- Add `qwen3-coder-next` and related Qwen/Coder models to Ollama Cloud catalog entry
- Design extensible catalog structure (static base + override layer) for future dynamic discovery
- Define the wire format for `GET /api/models` response to support model grouping by provider kind

### Out of Scope
- Runtime `/v1/models` introspection per provider (future work)
- Adding models for providers other than Ollama Cloud (deferred)
- Dashboard UI changes (handled separately)
- Combo execution changes (already implemented in `combo-sqlite`)

## Capabilities

### New Capabilities
- `model-catalog-static`: Expanded static catalog with Ollama Cloud full model list

### Modified Capabilities
- None at spec level — this is a data expansion of an existing capability

## Approach

### Catalog Structure

```
ModelCatalogPort (trait)
 └── StaticModelCatalog (current — hardcoded Vec)
 └── ExpandedCatalog (proposed — static base + runtime overrides)
```

**Phase 1 (this change)**: Expand the hardcoded `catalog()` function in `crates/infrastructure/models-catalog/src/lib.rs` to include all Ollama Cloud models from OmniRoute's `providerRegistry.ts`.

**Phase 2 (future)**: Add a `DynamicModelCatalog` implementation that fetches from provider's `/v1/models` or `/api/tags` endpoints and merges with the static base.

### Ollama Cloud Models to Add

From OmniRoute's `providerRegistry.ts`:
- `deepseek-v4-pro` (reasoning)
- `deepseek-v4-flash` (reasoning)
- `kimi-k2.6`
- `glm-5.1`
- `minimax-m2.7`
- `gemma4:31b`
- `nemotron-3-super`
- `qwen3.5:397b`
- Plus any additional from `ollamaModels.ts`

### API Response Format

`GET /api/models` already exists and returns `ListModelsResponse` with `ProviderModelsGroup[]`. No wire format changes needed — the endpoint will automatically surface the expanded catalog through existing aggregation logic.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/infrastructure/models-catalog/src/lib.rs` | Modified | Expand `catalog()` function with Ollama Cloud models |
| `crates/infrastructure/transport-axum/src/handlers/models.rs` | No change | Already uses `model_catalog.list()` — will automatically surface new entries |
| `openspec/specs/combo-repository/spec.md` | Reference | Already spec'd and implemented — no changes needed |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Catalog drift (OmniRoute adds models) | Medium | Schedule periodic sync from OmniRoute registry |
| Model name format mismatch (`ollamacloud/` prefix) | Low | Use exact IDs from OmniRoute; verify against actual API |
| Memory bloat from excessive model entries | Low | Cap static catalog at ~50 models per provider kind |

## Rollback Plan

- **Revert**: Restore previous `catalog()` array in `StaticModelCatalog` with original 3-5 models per provider
- **No DB migration needed**: Catalog is in-memory, no persistence to revert
- **Rollback command**: `git checkout HEAD -- crates/infrastructure/models-catalog/src/lib.rs`

## Dependencies

- OmniRoute `providerRegistry.ts` as source of truth for Ollama Cloud model IDs
- `ModelCatalogPort` trait in `rook-core/src/ports.rs` (unchanged interface)

## Success Criteria

- [ ] `StaticModelCatalog::list()` returns all Ollama Cloud models including `qwen3-coder-next`
- [ ] `GET /api/models` response includes expanded Ollama Cloud model list grouped by provider
- [ ] No compilation errors in `models-catalog` crate
- [ ] Unit tests pass for `compute_models_by_provider` aggregation logic
- [ ] `just clippy` passes on modified crate
