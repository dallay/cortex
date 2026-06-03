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
use rook_core::{ModelCatalogEntry, ModelCatalogPort, ProviderKind};

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
    let counts = count_per_kind(&entries);
    for (kind, count) in counts {
        assert!(
            count >= 1,
            "provider kind {kind:?} must have at least one model"
        );
    }
}

fn count_per_kind(entries: &[ModelCatalogEntry]) -> HashMap<ProviderKind, usize> {
    let mut out: HashMap<ProviderKind, usize> = HashMap::new();
    for entry in entries {
        *out.entry(entry.provider_kind).or_insert(0) += 1;
    }
    out
}
