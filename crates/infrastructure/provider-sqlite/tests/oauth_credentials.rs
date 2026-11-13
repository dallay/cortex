use chrono::Utc;
use provider_sqlite::SqliteProviderRepository;
use rook_core::{
    AuthType, ConnectionConfig, ConnectionId, Credentials, EncryptedBlob, ProviderConnection,
    ProviderRepositoryPort, QuotaWindowThresholds, RepositoryError, TestStatus,
};
use shared_kernel::ProviderId;

#[tokio::test]
async fn oauth_client_configuration_round_trips_through_v6_columns() {
    let database = tempfile::NamedTempFile::new().expect("temporary database");
    db_migration::run_migrations(database.path().to_str().expect("database path"))
        .expect("apply migrations");
    let repo = SqliteProviderRepository::new(database.path()).expect("repository");
    let connection = ProviderConnection {
        id: ConnectionId::new(),
        provider: rook_core::descriptor_by_id("azure-openai").expect("descriptor"),
        provider_runtime_id: ProviderId::new("azure-test"),
        name: "Azure test".to_string(),
        priority: 10,
        is_active: true,
        auth_type: AuthType::OAuth,
        credentials: Credentials::OAuth {
            email: EncryptedBlob("encrypted-email".to_string()),
            access_token: EncryptedBlob("encrypted-access".to_string()),
            refresh_token: EncryptedBlob("encrypted-refresh".to_string()),
            expires_at: Utc::now().timestamp() + 3600,
            scope: EncryptedBlob("encrypted-scope".to_string()),
            id_token: EncryptedBlob("encrypted-id-token".to_string()),
            project_id: EncryptedBlob("encrypted-project".to_string()),
            oauth_client_id: "client-id".to_string(),
            oauth_client_secret: Some(EncryptedBlob("encrypted-client-secret".to_string())),
            oauth_tenant_id: Some("tenant-guid".to_string()),
        },
        config: ConnectionConfig {
            max_concurrent: 10,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.8,
                error: 0.95,
            },
            default_model: Some(shared_kernel::ModelId::new("deployment")),
            base_url: Some("https://resource.openai.azure.com".to_string()),
            api_format: None,
            supported_models: Vec::new(),
        },
        adapter_revision: 1,
        test_status: TestStatus::NeverTested,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    repo.create(&connection).await.expect("persist connection");
    let loaded = repo
        .find(&connection.id)
        .await
        .expect("load connection")
        .expect("connection exists");

    assert_eq!(loaded.provider.id, "azure-openai");
    assert_eq!(loaded.credentials, connection.credentials);
}

#[tokio::test]
async fn create_rejects_names_colliding_with_a_legacy_alias_row() {
    let temp_file = tempfile::Builder::new()
        .prefix("rook-provider-alias-unique-")
        .suffix(".sqlite")
        .tempfile()
        .expect("temporary database");
    let path = temp_file.path();
    {
        let mut conn = rusqlite::Connection::open(path).expect("database");
        db_migration::run_on_connection(&mut conn).expect("migrations");
        // A pre-canonicalization row carrying the legacy serde spelling.
        conn.execute(
            "INSERT INTO provider_connections (
                id, provider_kind, provider_runtime_id, name, auth_type,
                priority, is_active, api_key_ct, max_concurrent, quota_warning,
                quota_error, test_status, created_at, updated_at
            ) VALUES (?1, 'ollamacloud', 'runtime-legacy', 'shared-name', 'apiKey',
                5, 1, 'enc:v1:legacy', 1, 0.7, 0.9, 'neverTested',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            rusqlite::params![shared_kernel::ConnectionId::new().to_string()],
        )
        .expect("legacy row");
    }

    let repo = SqliteProviderRepository::new(path).expect("repo");
    let canonical = ProviderConnection {
        id: shared_kernel::ConnectionId::new(),
        provider: rook_core::descriptor_by_id("ollama-cloud").expect("ollama-cloud descriptor"),
        provider_runtime_id: shared_kernel::ProviderId::new("runtime-new"),
        name: "shared-name".to_string(),
        priority: 1,
        is_active: true,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("enc:v1:new".to_string()),
        },
        config: ConnectionConfig {
            max_concurrent: 1,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: None,
            base_url: None,
            api_format: None,
            supported_models: Vec::new(),
        },
        adapter_revision: 1,
        test_status: TestStatus::NeverTested,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let result = repo.create(&canonical).await;
    assert!(
        matches!(result, Err(RepositoryError::DuplicateConnection(_))),
        "an alias row must collide with the canonical spelling"
    );
}
