//! Managed-connection quota capture: the canonical-endpoint gate and the
//! runtime/connection identity wrapping, exercised through the same production
//! wiring that `DynamicProviderBuilder` uses.

use std::sync::Arc;

use rook::di::quota_recorder_for_connection;
use rook_core::{
    ConnectionId, ProviderId, ProviderKind, QuotaObservation, QuotaObservationRecorderPort,
};
use shared_kernel::CortexResult;

struct NullRecorder;

#[async_trait::async_trait]
impl QuotaObservationRecorderPort for NullRecorder {
    async fn record_observation(&self, _observation: QuotaObservation) -> CortexResult<()> {
        Ok(())
    }

    async fn list_observations(
        &self,
        _provider: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        Ok(Vec::new())
    }
}

/// Returns whether a connection with this endpoint would capture quota signals.
fn capture_enabled(kind: ProviderKind, base_url: Option<&str>) -> bool {
    quota_recorder_for_connection(
        kind,
        base_url,
        ConnectionId::new(),
        ProviderId::new("probe"),
        Some(Arc::new(NullRecorder)),
    )
    .is_some()
}

#[test]
fn canonical_gate_accepts_default_and_explicit_canonical_urls() {
    for (kind, url) in [
        (ProviderKind::OpenAI, None),
        (ProviderKind::OpenAI, Some("https://api.openai.com")),
        (ProviderKind::OpenAI, Some("https://api.openai.com/")),
        (ProviderKind::OpenAI, Some("https://api.openai.com/v1")),
        (ProviderKind::Anthropic, None),
        (ProviderKind::Anthropic, Some("https://api.anthropic.com")),
        (ProviderKind::Groq, None),
        (ProviderKind::Groq, Some("https://api.groq.com/openai/v1")),
    ] {
        assert!(
            capture_enabled(kind, url),
            "{kind:?} with {url:?} must stay eligible for quota capture"
        );
    }
}

#[test]
fn canonical_gate_rejects_custom_and_non_capturing_endpoints() {
    for (kind, url) in [
        (ProviderKind::OpenAI, Some("https://proxy.internal/v1")),
        (ProviderKind::OpenAI, Some("http://api.openai.com")),
        (ProviderKind::OpenAI, Some("https://api.openai.com:8443")),
        (ProviderKind::OpenAI, Some("https://api.openai.com.evil.test")),
        (ProviderKind::Anthropic, Some("https://api.anthropic.com/v1")),
        (ProviderKind::Groq, Some("https://api.groq.com/v1")),
        (ProviderKind::Ollama, Some("http://localhost:11434")),
        (
            ProviderKind::Gemini,
            Some("https://generativelanguage.googleapis.com"),
        ),
    ] {
        assert!(
            !capture_enabled(kind, url),
            "{kind:?} with {url:?} must not be attributed to the provider"
        );
    }
}

#[test]
fn capture_is_disabled_without_a_recorder_even_for_canonical_endpoints() {
    assert!(
        quota_recorder_for_connection(
            ProviderKind::OpenAI,
            None,
            ConnectionId::new(),
            ProviderId::new("probe"),
            None,
        )
        .is_none(),
        "no recorder means nothing to capture into"
    );
}

/// The recorder enqueues writes onto a bounded background worker, so tests must
/// wait for the queue to drain rather than asserting immediately.
async fn wait_for_observations(
    repository: &audit_sqlite::SqliteUsageRepository,
    provider: &ProviderId,
    expected: usize,
) -> Vec<QuotaObservation> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let rows = repository
            .list_observations(Some(provider.clone()))
            .await
            .unwrap();
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "quota writer did not drain"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn identity_wrapper_restores_runtime_and_connection() {    let db = tempfile::NamedTempFile::new().unwrap();
    db_migration::run_migrations(db.path().to_str().unwrap()).unwrap();
    let repository = Arc::new(audit_sqlite::SqliteUsageRepository::new(db.path()).unwrap());

    let runtime_id = ProviderId::new("openai-primary");
    for _ in 0..2 {
        let connection_id = ConnectionId::new();
        let recorder = quota_recorder_for_connection(
            ProviderKind::OpenAI,
            None,
            connection_id,
            runtime_id.clone(),
            Some(repository.clone() as Arc<dyn QuotaObservationRecorderPort>),
        )
        .expect("canonical connection must be eligible for capture");

        // The provider adapter reports the connection-scoped registry id; the
        // wrapper must restore the managed runtime identity before persisting.
        recorder
            .record_observation(QuotaObservation {
                provider_kind: ProviderKind::OpenAI,
                provider: ProviderId::new(connection_id.to_string()),
                connection_id: None,
                metric: "requests".into(),
                source: "header".into(),
                scope: None,
                window: None,
                limit: Some(100.0),
                remaining: Some(75.0),
                reset_at: None,
                reset_after_milliseconds: None,
                observed_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
    }

    let rows = wait_for_observations(&repository, &runtime_id, 2).await;
    assert_eq!(rows.len(), 2, "both connections must persist under the runtime");
    for row in rows {
        assert_eq!(row.provider, runtime_id);
        assert_eq!(row.provider_kind, ProviderKind::OpenAI);
        assert_eq!(row.metric, "requests");
        assert_eq!(row.limit, Some(100.0));
        assert_eq!(row.remaining, Some(75.0));
        assert!(
            row.connection_id.is_some(),
            "the wrapper must restore the connection identity"
        );
    }
}
