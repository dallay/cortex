use huginn_core::connections::*;
use huginn_runtime::connections::HostSecrets;

#[tokio::test]
async fn explicitly_ephemeral_credentials_are_lost_on_restart_and_never_claim_secure_storage() {
    let store = HostSecrets::default();
    assert_eq!(
        store.mode(),
        SecretMode::Secure,
        "secure remains the default; no implicit fallback"
    );
    let reference = SecretReference::new();
    let secret = Secret::new("p03-ephemeral-sentinel".into()).unwrap();
    store
        .write_mode(SecretMode::Ephemeral, &reference, &secret)
        .await
        .unwrap();
    assert_eq!(
        store
            .read_mode(SecretMode::Ephemeral, &reference)
            .await
            .unwrap()
            .expose(),
        secret.expose()
    );
    let reopened = HostSecrets::default();
    assert_eq!(
        reopened
            .read_mode(SecretMode::Ephemeral, &reference)
            .await
            .unwrap_err(),
        ConnectionError::MissingSecret
    );
    store
        .delete_mode(SecretMode::Ephemeral, &reference)
        .await
        .unwrap();
    assert_eq!(
        store
            .read_mode(SecretMode::Ephemeral, &reference)
            .await
            .unwrap_err(),
        ConnectionError::MissingSecret
    );
}
