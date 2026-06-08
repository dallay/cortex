# Delta Spec: Hybrid Model Catalog — Expand Static Catalog

## Change

`2026-06-08-hybrid-model-catalog`

## MODIFIED Requirements

### Requirement: Ollama Cloud Model Catalog Coverage

The `StaticModelCatalog` catalog entry for `ProviderKind::OllamaCloud` SHALL include all models available via the Ollama Cloud API, enabling the dashboard's API key restriction UI to surface the full model zoo.

The catalog SHALL use the `ollamacloud/{model-name}` ID format for each entry, matching the provider's `alias` prefix. Each entry SHALL contain only `model_id` and `provider_kind: ProviderKind::OllamaCloud`.

(Previously: only3 hardcoded models: `llama3.2`, `mistral`, `qwen2.5`)

#### Scenario: Full Ollama Cloud catalog in list response

- GIVEN a client calls `StaticModelCatalog::list()`
- WHEN the catalog is queried
- THEN the result SHALL include exactly 8 `ModelCatalogEntry` records for `ProviderKind::OllamaCloud`
- AND each entry SHALL have `provider_kind: ProviderKind::OllamaCloud`
- AND each entry SHALL have `model_id` prefixed with `ollamacloud/`

#### Scenario: Specific models present

- GIVEN the expanded catalog
- WHEN `ollamacloud/qwen3-coder-next` is requested
- THEN it SHALL appear in the list with `provider_kind: ProviderKind::OllamaCloud`
- AND `ollamacloud/deepseek-v4-pro` SHALL be present
- AND `ollamacloud/minimax-m2.7` SHALL be present

#### Scenario: GET /api/models surfaces expanded catalog

- GIVEN the expanded `StaticModelCatalog`
- WHEN a client calls `GET /api/models`
- THEN the response SHALL group models by provider
- AND the Ollama Cloud group SHALL contain 8 models
- AND the `ollamacloud/qwen3-coder-next` model SHALL be present in that group

## ADDED Requirements

### Requirement: Ollama Cloud Model List

The `catalog()` function in `crates/infrastructure/models-catalog/src/lib.rs` SHALL return the following8 Ollama Cloud models:

| model_id | display name |
|---|---|
| `ollamacloud/deepseek-v4-pro` | DeepSeek V4 Pro |
| `ollamacloud/deepseek-v4-flash` | DeepSeek V4 Flash |
| `ollamacloud/kimi-k2.6` | Kimi K2.6 |
| `ollamacloud/glm-5.1` | GLM 5.1 |
| `ollamacloud/minimax-m2.7` | MiniMax M2.7 |
| `ollamacloud/gemma4:31b` | Gemma 4 31B |
| `ollamacloud/nemotron-3-super` | NVIDIA Nemotron 3 Super |
| `ollamacloud/qwen3.5:397b` | Qwen 3.5 397B |

Source: OmniRoute `providerRegistry.ts` → `ollama-cloud` entry (lines 2958–2980).

#### Scenario: Catalog returns8 Ollama Cloud entries

- GIVEN the `catalog()` function is called
- WHEN the vector is filtered to `ProviderKind::OllamaCloud`
- THEN exactly 8 entries SHALL be returned
- AND each `model_id` SHALL match the `ollamacloud/{name}` format above

## Success Criteria

- [ ] `StaticModelCatalog::list()` returns 8 `ModelCatalogEntry` records for `ProviderKind::OllamaCloud`
- [ ] `ollamacloud/qwen3-coder-next` is present in the catalog (via `ollamacloud/` prefix)
- [ ] `GET /api/models` response includes the8 expanded Ollama Cloud models grouped by provider
- [ ] `cargo check -p models-catalog` passes with no errors
- [ ] `just clippy -p models-catalog` passes with no warnings
