# Tasks: hybrid-model-catalog

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~30 |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | auto-chain |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Expand StaticModelCatalog with 9 Ollama Cloud models | PR 1 | Single PR, no base branch complexity |

## Phase 1: Core Implementation

- [x] 1.1 Replace `ollama_cloud` placeholder array in `catalog()` with full model list: `["deepseek-v4-pro", "deepseek-v4-flash", "kimi-k2.6", "glm-5.1", "minimax-m2.7", "gemma4:31b", "nemotron-3-super", "qwen3.5:397b", "qwen3-coder-next"]`
- [x] 1.2 Update `model_id` construction in the `ollama_cloud` loop to use `ollamacloud/{name}` prefix via `format!("ollamacloud/{}", m)`
- [x] 1.3 Verify `ModelCatalogEntry` struct has no `name` or `description` fields (design intent is captured in `model_id` only — no struct change needed)

## Phase 2: Verification

- [x] 2.1 Run `cargo check -p models-catalog` to verify compilation
- [x] 2.2 Run `just clippy` to verify no warnings
- [x] 2.3 Run `cargo test -p models-catalog` to verify unit tests pass
