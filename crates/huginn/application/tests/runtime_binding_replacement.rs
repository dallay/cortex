use async_trait::async_trait;
use huginn_application::connections::{ConnectionAccess, ConnectionManager};
use huginn_core::{
    connections::{
        ConnectionCredentials, ConnectionError, ConnectionRecord, ConnectionSettings, Secret,
        SecretMode, SecretReference, SecretStore,
    },
    providers::{ConnectionIdentity, ProviderRegistration},
    CancellationToken,
};
use std::sync::Arc;
use uuid::Uuid;

struct TestSecretStore;
#[async_trait]
impl SecretStore for TestSecretStore {
    fn mode(&self) -> SecretMode {
        SecretMode::Ephemeral
    }

    async fn read(&self, _: &SecretReference) -> Result<Secret, ConnectionError> {
        Err(ConnectionError::MissingSecret)
    }

    async fn write(&self, _: &SecretReference, _: &Secret) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn delete(&self, _: &SecretReference) -> Result<(), ConnectionError> {
        Ok(())
    }
}

#[tokio::test]
async fn replacing_runtime_binding_removes_previous_runtime_secret() {
    let access = ConnectionAccess::new(Arc::new(TestSecretStore));
    let id = Uuid::new_v4();
    let identity = ConnectionIdentity {
        id: id.to_string(),
        adapter: ProviderRegistration::new("test.provider".into(), 1),
        revision: 1,
    };
    let settings = ConnectionSettings {
        provider: "test.provider".into(),
        endpoint: Some("https://example.test".into()),
        model: "test-model".into(),
        requires_secret: true,
        allow_loopback_http: false,
    };

    ConnectionManager::bind_runtime_for_test(
        &access,
        identity.clone(),
        settings.clone(),
        Some(Secret::new("old-secret".into()).unwrap()),
    )
    .unwrap();
    ConnectionManager::bind_record_for_test(
        &access,
        identity.clone(),
        ConnectionRecord {
            id,
            revision: Uuid::new_v4(),
            settings,
            secret: None,
            mode: SecretMode::Ephemeral,
        },
    )
    .unwrap();

    assert!(matches!(
        access.resolve(&identity, CancellationToken::new()).await,
        Err(ConnectionError::MissingSecret)
    ));
}
