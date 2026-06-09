# Tasks: Fix Provider Empty Models List

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Review budget | 400 changed lines (project default) |
| Estimated workload | Low |
| Chained PRs recommended | No |
| Proposed delivery strategy | single-pr |
| Work-unit balance | 3 files, all in same crate (`apps/rook`), minimal logic — single review |

## Phase 1: DI Construction Changes

- [x] 1.1 Add `catalog: Arc<dyn ModelCatalogPort>` field to `DynamicProviderBuilder` struct in `apps/rook/src/di.rs`
- [x] 1.2 Add `catalog` parameter to `DynamicProviderBuilder::new()` constructor
- [x] 1.3 Update `ManageConnections` construction at line ~154 to pass `model_catalog` to `DynamicProviderBuilder::new()`

## Phase 2: Provider Building Signature Update

- [x] 2.1 Add `models: Vec<ModelId>` parameter to `build_provider_from_connection()` signature
- [x] 2.2 Update `ProviderKind::OpenAI` match arm: `models: Vec::new()` → `models: models.clone()`
- [x] 2.3 Update `ProviderKind::Anthropic` match arm: `models: Vec::new()` → `models: models.clone()`
- [x] 2.4 Update `ProviderKind::Ollama` match arm: `models: Vec::new()` → `models: models.clone()`
- [x] 2.5 Update `ProviderKind::OllamaCloud` match arm: `models: Vec::new()` → `models: models.clone()`
- [x] 2.6 Update `ProviderKind::Gemini` match arm: `models: Vec::new()` → `models: models.clone()`
- [x] 2.7 Update `ProviderKind::Groq` match arm: `models: Vec::new()` → `models: models.clone()`

## Phase 3: ProviderBuilderPort Implementation

- [x] 3.1 Update `ProviderBuilderPort::build()` impl in `DynamicProviderBuilder` to:
  - Call `self.catalog.list().await`
  - Filter entries where `provider_kind == input.provider_kind`
  - Collect as `Vec<ModelId>`
  - Pass to `build_provider_from_connection(..., models)`
- [x] 3.2 Update `DynamicProviderBuilder::new()` call site in `di.rs` to pass `catalog.clone()`

## Phase 4: Testing

- [x] 4.1 Run `cargo test -p rook-usecases --lib` — 120 tests passed
- [x] 4.2 Run `cargo test -p providers-ollama --lib` — 11 tests passed
- [x] 4.3 Run `just clippy` — no lints
- [x] 4.4 Run `cargo build` — compilation succeeded

## Phase 5: Verification

- [x] 5.1 Start local Rook with `ollamacloud/qwen3-coder-next` model
- [x] 5.2 Confirm "all providers exhausted" is gone on first request
- [x] 5.3 Confirm health check still returns healthy status