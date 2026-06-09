//! Integration tests for `StaticModelCatalog`.
//!
//! These exercise the public API (`ModelCatalogPort::list`) and verify that
//! the catalog:
//!   1. is non-empty
//!   2. covers every known `ProviderKind`
//!   3. has unique model ids per provider kind
//!   4. is stable across calls

use std::collections::{HashMap, HashSet};

use models_catalog::StaticModelCatalog;
use rook_core::{ModelCatalogPort, ProviderKind};

#[tokio::test]
async fn list_is_non_empty() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;
    assert!(
        !entries.is_empty(),
        "catalog must declare at least one model"
    );
}

#[tokio::test]
async fn list_covers_every_known_provider_kind() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;
    let kinds: HashSet<ProviderKind> = entries.iter().map(|e| e.provider_kind).collect();

    for kind in [
        ProviderKind::OpenAI,
        ProviderKind::Anthropic,
        ProviderKind::Ollama,
        ProviderKind::OllamaCloud,
        ProviderKind::Gemini,
        ProviderKind::Groq,
    ] {
        assert!(
            kinds.contains(&kind),
            "missing catalog entries for provider kind {kind:?}"
        );
    }
}

#[tokio::test]
async fn list_has_unique_model_ids_within_a_provider_kind() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;

    let mut by_kind: HashMap<ProviderKind, HashSet<String>> = HashMap::new();
    for entry in &entries {
        by_kind
            .entry(entry.provider_kind)
            .or_default()
            .insert(entry.model_id.clone());
    }

    for (kind, ids) in &by_kind {
        let total_for_kind = entries.iter().filter(|e| e.provider_kind == *kind).count();
        assert_eq!(
            ids.len(),
            total_for_kind,
            "duplicate model_id within provider kind {kind:?}"
        );
    }
}

#[tokio::test]
async fn list_is_stable_across_calls() {
    let catalog = StaticModelCatalog::new();
    let first = catalog.list().await;
    let second = catalog.list().await;
    assert_eq!(first, second, "catalog must be deterministic across calls");
}

#[tokio::test]
async fn list_returns_at_least_one_entry_per_supported_provider_kind() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;
    let kinds_in_catalog: HashSet<ProviderKind> = entries.iter().map(|e| e.provider_kind).collect();

    // Explicitly assert that every known ProviderKind appears at least once.
    // This complements list_covers_every_known_provider_kind by verifying
    // the *count* is non-zero for each kind, not just that the kind is absent.
    for kind in [
        ProviderKind::OpenAI,
        ProviderKind::Anthropic,
        ProviderKind::Ollama,
        ProviderKind::OllamaCloud,
        ProviderKind::Gemini,
        ProviderKind::Groq,
    ] {
        let count = entries.iter().filter(|e| e.provider_kind == kind).count();
        assert!(
            count >= 1,
            "provider kind {kind:?} must have at least one model, but catalog has none"
        );
        // Also verify the kind is in the set (redundant with the above, but
        // documents the intent that kinds_in_catalog is derived correctly).
        assert!(
            kinds_in_catalog.contains(&kind),
            "provider kind {kind:?} must appear in catalog"
        );
    }
}

/// Ollama Cloud models — must include the 9 models from OmniRoute registry.
/// This verifies the expanded catalog is working correctly.
const OLLAMA_CLOUD_EXPECTED_MODEL_IDS: &[&str] = &[
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

#[tokio::test]
async fn ollama_cloud_has_expanded_model_list() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;

    let ollama_cloud_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.provider_kind == ProviderKind::OllamaCloud)
        .collect();

    let actual_ids: HashSet<_> = ollama_cloud_entries
        .iter()
        .map(|e| e.model_id.as_str())
        .collect();

    let expected_ids: HashSet<_> = OLLAMA_CLOUD_EXPECTED_MODEL_IDS.iter().copied().collect();

    assert_eq!(
        actual_ids.len(),
        expected_ids.len(),
        "OllamaCloud must have exactly {} models, got {}",
        expected_ids.len(),
        actual_ids.len()
    );

    for expected_id in OLLAMA_CLOUD_EXPECTED_MODEL_IDS {
        assert!(
            actual_ids.contains(expected_id),
            "missing OllamaCloud model: {expected_id}"
        );
    }
}

#[tokio::test]
async fn ollama_cloud_qwen3_coder_next_is_present() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;

    let has_qwen3_coder = entries.iter().any(|e| {
        e.provider_kind == ProviderKind::OllamaCloud && e.model_id == "ollamacloud/qwen3-coder-next"
    });

    assert!(
        has_qwen3_coder,
        "ollamacloud/qwen3-coder-next must be present in catalog for OpenCode compatibility"
    );
}
