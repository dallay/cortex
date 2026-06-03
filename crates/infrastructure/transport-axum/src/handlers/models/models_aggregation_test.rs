//! Unit tests for the pure aggregation logic in `models.rs`.
//!
//! These intentionally do not spin up an Axum router — the aggregation
//! is pure data-in / data-out and the handler is a thin async wrapper
//! covered by the integration tests in `tests/api_models_routes.rs`.

use chrono::Utc;
use rook_core::{
    AuthType, ConnectionConfig, ConnectionId, Credentials, EncryptedBlob, ProviderConnection,
    ProviderKind, QuotaWindowThresholds, TestStatus,
};
use shared_kernel::ProviderId;

use super::compute_models_by_provider;
use rook_core::ModelCatalogEntry;

fn make_connection(
    id: &str,
    name: &str,
    kind: ProviderKind,
    runtime_id: &str,
    is_active: bool,
) -> ProviderConnection {
    // Use deterministic UUIDs (fixed string → parse → stable across runs)
    // so failures are reproducible and the test is easy to read.
    let id_uuid = uuid::Uuid::parse_str(&format!(
        "{:0>32}",
        id.chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
    ))
    .unwrap_or_else(|_| uuid::Uuid::new_v4());
    ProviderConnection {
        id: ConnectionId(id_uuid),
        provider_kind: kind,
        provider_runtime_id: ProviderId::new(runtime_id),
        name: name.to_string(),
        priority: 1,
        is_active,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("encrypted".to_string()),
        },
        config: ConnectionConfig {
            max_concurrent: 1,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.5,
                error: 0.9,
            },
            default_model: None,
            base_url: None,
        },
        test_status: TestStatus::NeverTested,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn catalog_with_openai_and_anthropic() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            model_id: "gpt-4o".to_string(),
            provider_kind: ProviderKind::OpenAI,
        },
        ModelCatalogEntry {
            model_id: "gpt-4-turbo".to_string(),
            provider_kind: ProviderKind::OpenAI,
        },
        ModelCatalogEntry {
            model_id: "claude-3-5-sonnet-latest".to_string(),
            provider_kind: ProviderKind::Anthropic,
        },
    ]
}

#[test]
fn empty_connections_yields_empty_response() {
    let groups = compute_models_by_provider(&[], &catalog_with_openai_and_anthropic());
    assert!(groups.is_empty());
}

#[test]
fn empty_catalog_yields_empty_response() {
    let conns = vec![make_connection(
        "c1",
        "OpenAI Primary",
        ProviderKind::OpenAI,
        "openai-primary",
        true,
    )];
    let groups = compute_models_by_provider(&conns, &[]);
    assert!(
        groups.is_empty(),
        "no models in catalog means no group, even for active connection"
    );
}

#[test]
fn inactive_connection_is_filtered_out() {
    let conns = vec![make_connection(
        "c1",
        "OpenAI Primary",
        ProviderKind::OpenAI,
        "openai-primary",
        false,
    )];
    let groups = compute_models_by_provider(&conns, &catalog_with_openai_and_anthropic());
    assert!(groups.is_empty(), "inactive connection must be filtered");
}

#[test]
fn active_connection_with_catalog_match_is_returned() {
    let conns = vec![make_connection(
        "c1",
        "OpenAI Primary",
        ProviderKind::OpenAI,
        "openai-primary",
        true,
    )];
    let groups = compute_models_by_provider(&conns, &catalog_with_openai_and_anthropic());
    assert_eq!(groups.len(), 1);
    let g = &groups[0];
    // provider_id is the rendered UUID — compare with the connection's id
    // rendered the same way the handler renders it.
    assert_eq!(g.provider_id, conns[0].id.to_string());
    assert_eq!(g.provider_name, "OpenAI Primary");
    assert_eq!(g.provider_kind, "openai");
    assert_eq!(g.models, vec!["gpt-4o", "gpt-4-turbo"]);
}

#[test]
fn active_connection_with_no_catalog_match_is_filtered_out() {
    let conns = vec![make_connection(
        "c1",
        "Ollama Local",
        ProviderKind::Ollama,
        "ollama-local",
        true,
    )];
    // Catalog has only openai + anthropic, no ollama
    let groups = compute_models_by_provider(&conns, &catalog_with_openai_and_anthropic());
    assert!(
        groups.is_empty(),
        "connection with empty models must be filtered"
    );
}

#[test]
fn mixed_active_and_inactive_connections() {
    let c1 = make_connection(
        "c1",
        "OpenAI Primary",
        ProviderKind::OpenAI,
        "openai-primary",
        true,
    );
    let c2 = make_connection(
        "c2",
        "Anthropic Primary",
        ProviderKind::Anthropic,
        "anthropic-primary",
        false, // inactive
    );
    let c3 = make_connection(
        "c3",
        "Anthropic Backup",
        ProviderKind::Anthropic,
        "anthropic-backup",
        true,
    );
    let conns = vec![c1.clone(), c2.clone(), c3.clone()];
    let groups = compute_models_by_provider(&conns, &catalog_with_openai_and_anthropic());
    // c1 (openai) and c3 (anthropic) are active → both returned
    // c2 (anthropic) is inactive → dropped
    assert_eq!(groups.len(), 2);
    let ids: Vec<String> = groups.iter().map(|g| g.provider_id.clone()).collect();
    assert!(ids.contains(&c1.id.to_string()));
    assert!(ids.contains(&c3.id.to_string()));
    assert!(!ids.contains(&c2.id.to_string()));
}

#[test]
fn response_is_sorted_by_provider_id_for_determinism() {
    let c3 = make_connection(
        "c3",
        "Anthropic Backup",
        ProviderKind::Anthropic,
        "anthropic-backup",
        true,
    );
    let c1 = make_connection(
        "c1",
        "OpenAI Primary",
        ProviderKind::OpenAI,
        "openai-primary",
        true,
    );
    let c2 = make_connection(
        "c2",
        "Anthropic Primary",
        ProviderKind::Anthropic,
        "anthropic-primary",
        true,
    );
    // Insert in non-sorted order to verify the function sorts.
    let conns = vec![c3.clone(), c1.clone(), c2.clone()];
    let groups = compute_models_by_provider(&conns, &catalog_with_openai_and_anthropic());
    let ids: Vec<String> = groups.iter().map(|g| g.provider_id.clone()).collect();
    let expected: Vec<String> = {
        let mut v = vec![c1.id.to_string(), c2.id.to_string(), c3.id.to_string()];
        v.sort();
        v
    };
    assert_eq!(
        ids, expected,
        "groups must be sorted by provider_id for deterministic responses"
    );
}
