use chrono::{DateTime, Duration, Utc};
use provider_sqlite::SqliteProviderRepository;
use rook_core::{
    ApiFormat, AuthType, ConnectionConfig, ConnectionId, Credentials, EncryptedBlob, ModelId,
    ProviderConnection, ProviderKind, ProviderRepositoryPort, QuotaWindowThresholds,
    RepositoryError, TestStatus,
};
use shared_kernel::ProviderId;

fn connection() -> ProviderConnection {
    let timestamp = "2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    ProviderConnection {
        id: ConnectionId::parse_str("c56a4180-65aa-42ec-a945-5fd21dec0538").unwrap(),
        provider_kind: ProviderKind::OpenAiCompatible,
        provider_runtime_id: ProviderId::new("compatible-runtime"),
        name: "compatible".into(),
        priority: 5,
        is_active: false,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("encrypted-key".into()),
        },
        config: ConnectionConfig {
            max_concurrent: 3,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: Some(ModelId::new("vendor/model:latest")),
            base_url: Some("https://upstream.example/gateway/v1".into()),
            supported_models: vec![
                ModelId::new("vendor/model:latest"),
                ModelId::new("model-\"quoted\"-\\-雪"),
            ],
            api_format: Some(ApiFormat::OpenAI),
        },
        test_status: TestStatus::Active {
            last_test_at: timestamp,
            latency_ms: 42,
        },
        created_at: timestamp,
        updated_at: timestamp,
    }
}

#[tokio::test]
async fn compatible_model_declarations_round_trip_through_create_find_and_list() {
    let repo = SqliteProviderRepository::new(":memory:").unwrap();
    let original = connection();
    repo.create(&original).await.unwrap();

    let found = repo.find(&original.id).await.unwrap().unwrap();
    let listed = repo.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    for loaded in [found, listed.into_iter().next().unwrap()] {
        assert_eq!(loaded.provider_kind, ProviderKind::OpenAiCompatible);
        assert_eq!(loaded.config, original.config);
        assert_eq!(loaded.credentials, original.credentials);
        assert_eq!(loaded.test_status, original.test_status);
        assert_eq!(loaded.created_at, original.created_at);
        assert_eq!(loaded.updated_at, original.updated_at);
    }
}

#[tokio::test]
async fn updating_model_declarations_preserves_status_credentials_and_optimistic_locking() {
    let repo = SqliteProviderRepository::new(":memory:").unwrap();
    let original = connection();
    repo.create(&original).await.unwrap();
    let mut updated = original.clone();
    updated.config.supported_models = vec![ModelId::new("replacement")];
    updated.config.api_format = Some(ApiFormat::Anthropic);
    updated.updated_at += Duration::seconds(1);
    repo.update(&updated, original.updated_at).await.unwrap();

    let loaded = repo.find(&original.id).await.unwrap().unwrap();
    assert_eq!(loaded.config, updated.config);
    assert_eq!(loaded.credentials, original.credentials);
    assert_eq!(loaded.test_status, original.test_status);
    assert_eq!(loaded.created_at, original.created_at);
    assert_eq!(loaded.updated_at, updated.updated_at);

    let mut stale = original.clone();
    stale.config.supported_models.clear();
    assert_eq!(
        repo.update(&stale, original.updated_at).await,
        Err(RepositoryError::StaleUpdate)
    );
    assert_eq!(
        repo.find(&original.id).await.unwrap().unwrap().config,
        updated.config
    );

    updated.config.supported_models.clear();
    let expected = updated.updated_at;
    updated.updated_at += Duration::seconds(1);
    repo.update(&updated, expected).await.unwrap();
    assert!(repo
        .find(&original.id)
        .await
        .unwrap()
        .unwrap()
        .config
        .supported_models
        .is_empty());
}

#[tokio::test]
async fn absent_api_format_uses_the_provider_default_on_create_and_update() {
    for (kind, expected) in [
        (ProviderKind::Anthropic, ApiFormat::Anthropic),
        (ProviderKind::OpenAI, ApiFormat::OpenAI),
        (ProviderKind::OpenAiCompatible, ApiFormat::OpenAI),
    ] {
        let repo = SqliteProviderRepository::new(":memory:").unwrap();
        let mut original = connection();
        original.provider_kind = kind;
        original.config.api_format = None;
        repo.create(&original).await.unwrap();
        assert_eq!(
            repo.find(&original.id)
                .await
                .unwrap()
                .unwrap()
                .config
                .api_format,
            Some(expected)
        );

        let expected_updated_at = original.updated_at;
        original.updated_at += Duration::seconds(1);
        repo.update(&original, expected_updated_at).await.unwrap();
        assert_eq!(
            repo.find(&original.id)
                .await
                .unwrap()
                .unwrap()
                .config
                .api_format,
            Some(expected)
        );
    }
}

#[tokio::test]
async fn corrupt_model_or_format_columns_return_errors_instead_of_silent_defaults() {
    for (models, format) in [
        ("not json", "openai"),
        ("{}", "openai"),
        ("[42]", "openai"),
        ("[]", "unknown"),
    ] {
        let database = tempfile::NamedTempFile::new().unwrap();
        db_migration::run_migrations(database.path().to_str().unwrap()).unwrap();
        let repo = SqliteProviderRepository::new(database.path()).unwrap();
        let original = connection();
        repo.create(&original).await.unwrap();
        rusqlite::Connection::open(database.path()).unwrap().execute(
            "UPDATE provider_connections SET supported_models = ?1, api_format = ?2 WHERE id = ?3",
            rusqlite::params![models, format, original.id.to_string()],
        ).unwrap();

        assert!(
            matches!(
                repo.find(&original.id).await,
                Err(RepositoryError::Database(_))
            ),
            "{models}, {format}"
        );
        assert!(
            matches!(repo.list().await, Err(RepositoryError::Database(_))),
            "{models}, {format}"
        );
    }
}
