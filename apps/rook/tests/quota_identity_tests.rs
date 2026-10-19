//! Managed-connection quota capture: canonical-endpoint gating and runtime / connection
//! identity wrapping for the managed provider-construction path.

use std::sync::{Arc, Mutex};

use rook::di::quota_recorder_for_connection;
use rook_core::{
    ConnectionId, ProviderId, ProviderKind, QuotaObservation, QuotaObservationRecorderPort,
};
use shared_kernel::CortexResult;

#[derive(Default)]
struct NullRecorder {
    batches: Mutex<Vec<Vec<QuotaObservation>>>,
}

#[async_trait::async_trait]
impl QuotaObservationRecorderPort for NullRecorder {
    async fn record_observation(&self, observation: QuotaObservation) -> CortexResult<()> {
        self.record_observations(vec![observation]).await
    }

    async fn record_observations(&self, observations: Vec<QuotaObservation>) -> CortexResult<()> {
        self.batches.lock().unwrap().push(observations);
        Ok(())
    }

    async fn list_observations(
        &self,
        _provider: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        Ok(self
            .batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect())
    }
}

/// Returns whether a connection with this endpoint would capture quota signals.
fn capture_enabled(kind: ProviderKind, base_url: Option<&str>) -> bool {
    quota_recorder_for_connection(
        kind,
        base_url,
        ConnectionId::new(),
        ProviderId::new("probe"),
        Some(Arc::new(NullRecorder::default())),
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
        (
            ProviderKind::OpenAI,
            Some("https://api.openai.com.evil.test"),
        ),
        (
            ProviderKind::Anthropic,
            Some("https://api.anthropic.com/v1"),
        ),
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
async fn identity_wrapper_restores_runtime_and_connection() {
    let db = tempfile::NamedTempFile::new().unwrap();
    db_migration::run_migrations(db.path().to_str().unwrap()).unwrap();
    let repository = Arc::new(audit_sqlite::SqliteUsageRepository::new(db.path()).unwrap());

    let runtime_id = ProviderId::new("openai-primary");
    let mut expected_connections = Vec::new();
    for connection_id in [ConnectionId::new(), ConnectionId::new()] {
        expected_connections.push(connection_id);
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
    assert_eq!(
        rows.len(),
        2,
        "both connections must persist under the runtime"
    );
    let expected_connections: std::collections::HashSet<_> =
        expected_connections.into_iter().collect();
    let observed_connections: std::collections::HashSet<_> = rows
        .iter()
        .map(|row| row.connection_id.expect("connection identity must persist"))
        .collect();
    assert_eq!(observed_connections, expected_connections);
    for row in rows {
        assert_eq!(row.provider, runtime_id);
        assert_eq!(row.provider_kind, ProviderKind::OpenAI);
        assert_eq!(row.metric, "requests");
        assert_eq!(row.limit, Some(100.0));
        assert_eq!(row.remaining, Some(75.0));
    }
}

#[tokio::test]
async fn identity_wrapper_preserves_batch_boundary_and_stamps_every_observation() {
    let inner = Arc::new(NullRecorder::default());
    let connection_id = ConnectionId::new();
    let observations: Vec<_> = ["requests", "tokens"]
        .into_iter()
        .map(|metric| QuotaObservation {
            provider_kind: ProviderKind::OpenAI,
            provider: ProviderId::new("connection-scoped-id"),
            connection_id: None,
            metric: metric.into(),
            source: "header".into(),
            scope: None,
            window: None,
            limit: Some(100.0),
            remaining: Some(75.0),
            reset_at: None,
            reset_after_milliseconds: None,
            observed_at: chrono::Utc::now(),
        })
        .collect();

    let recorder = quota_recorder_for_connection(
        ProviderKind::OpenAI,
        None,
        connection_id,
        ProviderId::new("openai-primary"),
        Some(inner.clone() as Arc<dyn QuotaObservationRecorderPort>),
    )
    .unwrap();
    recorder.record_observations(observations).await.unwrap();

    let batches = inner.batches.lock().unwrap();
    assert_eq!(
        batches.len(),
        1,
        "one provider response must remain one batch"
    );
    assert_eq!(batches[0].len(), 2);
    for (observation, metric) in batches[0].iter().zip(["requests", "tokens"]) {
        assert_eq!(observation.provider, ProviderId::new("openai-primary"));
        assert_eq!(observation.connection_id, Some(connection_id));
        assert_eq!(observation.metric, metric);
    }
}
