//! Integration tests for `StaticModelCatalog`.
//!
//! These exercise the public API (`ModelCatalogPort::list`) and verify that
//! the catalog:
//!   1. is non-empty
//!   2. covers every declared provider
//!   3. has unique model ids per provider
//!   4. is stable across calls

use std::collections::{HashMap, HashSet};

use models_catalog::StaticModelCatalog;
use rook_core::ModelCatalogPort;

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
async fn list_covers_every_declared_provider() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;
    let providers: HashSet<&str> = entries.iter().map(|e| e.provider_id.as_str()).collect();

    for provider in [
        "openai",
        "anthropic",
        "ollama",
        "ollama-cloud",
        "gemini",
        "groq",
    ] {
        assert!(
            providers.contains(provider),
            "missing catalog entries for provider {provider:?}"
        );
    }
}

#[tokio::test]
async fn list_has_unique_model_ids_within_a_provider() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;

    let mut by_provider: HashMap<&str, HashSet<String>> = HashMap::new();
    for entry in &entries {
        by_provider
            .entry(entry.provider_id.as_str())
            .or_default()
            .insert(entry.model_id.clone());
    }

    for (provider, ids) in &by_provider {
        let total_for_provider = entries
            .iter()
            .filter(|e| e.provider_id == *provider)
            .count();
        assert_eq!(
            ids.len(),
            total_for_provider,
            "duplicate model_id within provider {provider:?}"
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
async fn list_returns_at_least_one_entry_per_catalogued_provider() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;
    let providers_in_catalog: HashSet<&str> =
        entries.iter().map(|e| e.provider_id.as_str()).collect();

    // Every catalogued provider appears at least once: not just present in
    // the set, but with a non-zero model count.
    for provider in [
        "openai",
        "anthropic",
        "ollama",
        "ollama-cloud",
        "gemini",
        "groq",
    ] {
        let count = entries.iter().filter(|e| e.provider_id == provider).count();
        assert!(
            count >= 1,
            "provider {provider:?} must have at least one model, but catalog has none"
        );
        assert!(
            providers_in_catalog.contains(provider),
            "provider {provider:?} must appear in catalog"
        );
    }
}

/// Ollama Cloud models — must include the 9 models from the reference-gateway registry.
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
        .filter(|e| e.provider_id == "ollama-cloud")
        .collect();

    let actual_ids: HashSet<_> = ollama_cloud_entries
        .iter()
        .map(|e| e.model_id.as_str())
        .collect();

    let expected_ids: HashSet<_> = OLLAMA_CLOUD_EXPECTED_MODEL_IDS.iter().copied().collect();

    assert_eq!(
        actual_ids.len(),
        expected_ids.len(),
        "Ollama Cloud must have exactly {} models, got {}",
        expected_ids.len(),
        actual_ids.len()
    );

    for expected_id in OLLAMA_CLOUD_EXPECTED_MODEL_IDS {
        assert!(
            actual_ids.contains(expected_id),
            "missing Ollama Cloud model: {expected_id}"
        );
    }
}

#[tokio::test]
async fn ollama_cloud_qwen3_coder_next_is_present() {
    let catalog = StaticModelCatalog::new();
    let entries = catalog.list().await;

    let has_qwen3_coder = entries
        .iter()
        .any(|e| e.provider_id == "ollama-cloud" && e.model_id == "ollamacloud/qwen3-coder-next");

    assert!(
        has_qwen3_coder,
        "ollamacloud/qwen3-coder-next must be present in catalog for OpenCode compatibility"
    );
}
