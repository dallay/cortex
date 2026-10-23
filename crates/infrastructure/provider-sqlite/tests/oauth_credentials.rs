use chrono::Utc;
use provider_sqlite::SqliteProviderRepository;
use rook_core::{
    AuthType, ConnectionConfig, ConnectionId, Credentials, EncryptedBlob, ProviderConnection,
    ProviderKind, ProviderRepositoryPort, QuotaWindowThresholds, TestStatus,
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
        provider_kind: ProviderKind::AzureOpenAI,
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

    assert_eq!(loaded.provider_kind, ProviderKind::AzureOpenAI);
    assert_eq!(loaded.credentials, connection.credentials);
}
