# Design: Hybrid Model Catalog — Ollama Cloud Expansion

## Technical Approach

Expand the `StaticModelCatalog` to include all Ollama Cloud models from OmniRoute's `providerRegistry.ts`. The change is purely data expansion — no architectural changes.

## Architecture Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Model ID format | `ollamacloud/{model-name}` | Consistent with OmniRoute's `ollamacloud` alias; prefixed to disambiguate from local Ollama |
| ProviderKind | `ProviderKind::OllamaCloud` | Already exists; no new enum variant needed |
| Data organization | Grouped by `provider_kind` in arrays | Mirrors existing pattern; easy to scan and maintain |

## Data Flow

```
catalog() → Vec<ModelCatalogEntry>
                ├── ProviderKind::OpenAI  → [gpt-4o, gpt-4-turbo, ...]
                ├── ProviderKind::Anthropic → [claude-3-5-sonnet-latest, ...]
                ├── ProviderKind::Ollama → [llama3.2, mistral, qwen2.5]
                ├── ProviderKind::OllamaCloud → [ollamacloud/deepseek-v4-pro, ...]  ← EXPANDED
                ├── ProviderKind::Gemini → [gemini-1.5-pro, ...]
                └── ProviderKind::Groq → [llama-3.1-70b-versatile, ...]
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/infrastructure/models-catalog/src/lib.rs` | Modify | Replace `ollama_cloud` array with full OmniRoute model list |

## Current Code Structure

```rust
fn catalog() -> Vec<ModelCatalogEntry> {
    let ollama_cloud = ["llama3.2", "mistral", "qwen2.5"];  // placeholder
    // ...
    for m in ollama_cloud {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::OllamaCloud,
        });
    }
}
```

## What to Add

Replace the `ollama_cloud` array with the full list from OmniRoute's `ollama-cloud` provider entry:

```rust
let ollama_cloud = [
    "ollamacloud/deepseek-v4-pro",
    "ollamacloud/deepseek-v4-flash",
    "ollamacloud/kimi-k2.6",
    "ollamacloud/glm-5.1",
    "ollamacloud/minimax-m2.7",
    "ollamacloud/gemma4:31b",
    "ollamacloud/nemotron-3-super",
    "ollamacloud/qwen3.5:397b",
    "ollamacloud/qwen3-coder-next",
];
```

Model ID is prefixed with `ollamacloud/` to match OmniRoute's convention and distinguish from local Ollama models in API key restriction UI.

## Interfaces / Contracts

No interface changes. `ModelCatalogEntry` struct remains:
```rust
pub struct ModelCatalogEntry {
    pub model_id: String,
    pub provider_kind: ProviderKind,
}
```

## Testing Strategy

| Layer | What | How |
|-------|------|-----|
| Unit | `catalog()` returns correct count | `cargo test -p models-catalog` |
| Integration | API key restriction UI renders all Ollama Cloud models | Browser test via Playwright |

## Migration / Rollout

No migration needed — this is a pure data expansion with no schema or behavior change.

## Open Questions

None.

## Summary

- **Approach**: Replace placeholder `ollama_cloud` array with full OmniRoute model list
- **Key Decisions**: Prefix all model IDs with `ollamacloud/` for disambiguation
- **Files Affected**: 1 file modified
- **Testing**: Unit test + integration via Playwright