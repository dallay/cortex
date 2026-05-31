// Integration tests for API Key CRUD routes and DTOs

use chrono::Utc;
use rook_core::{ApiKeyId, ApiKeyRecord, ApiKeyScope, ApiKeyTier};
use transport_axum::handlers::api_key::{
    ApiKeyRecordResponseDto, CreateApiKeyRequestDto, CreateApiKeyResponseDto,
    UpdateApiKeyRequestDto,
};

fn test_record() -> ApiKeyRecord {
    ApiKeyRecord {
        id: ApiKeyId::new("key_123"),
        label: "Production Key".to_string(),
        key_hash: "hash_abc123".to_string(),
        key_prefix: "rook_test".to_string(),
        scopes: vec![
            ApiKeyScope::parse("read").unwrap(),
            ApiKeyScope::parse("write").unwrap(),
        ],
        tier: ApiKeyTier::Pro,
        is_active: true,
        revoked_at: None,
        expires_at: Some(Utc::now() + chrono::Duration::days(30)),
        created_at: Utc::now(),
        last_used_at: None,
    }
}

#[test]
fn api_key_record_response_dto_converts_correctly() {
    let record = test_record();
    let dto = ApiKeyRecordResponseDto::from(&record);

    assert_eq!(dto.id, "key_123");
    assert_eq!(dto.label, "Production Key");
    assert_eq!(dto.key_prefix, "rook_test");
    assert_eq!(dto.scopes, vec!["read".to_string(), "write".to_string()]);
    assert_eq!(dto.tier, "pro");
    assert!(dto.is_active);
    assert!(dto.expires_at.is_some());
    assert_eq!(dto.created_at, record.created_at);
    assert_eq!(dto.last_used_at, None);
}

#[test]
fn create_api_key_request_deserializes_correctly() {
    let json = serde_json::json!({
        "label": "Test Key",
        "scopes": ["read", "write"],
        "tier": "enterprise",
        "expiresAt": "2026-06-30T00:00:00Z"
    });

    let dto: CreateApiKeyRequestDto = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(dto.label, "Test Key");
    assert_eq!(dto.scopes, vec!["read".to_string(), "write".to_string()]);
    assert_eq!(dto.tier, "enterprise");
    assert!(dto.expires_at.is_some());
}

#[test]
fn update_api_key_request_deserializes_correctly() {
    let json = serde_json::json!({
        "label": "Updated Label",
        "scopes": ["read"],
        "tier": "free",
        "isActive": false,
        "expiresAt": null
    });

    let dto: UpdateApiKeyRequestDto = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(dto.label, Some("Updated Label".to_string()));
    assert_eq!(dto.scopes, Some(vec!["read".to_string()]));
    assert_eq!(dto.tier, Some("free".to_string()));
    assert_eq!(dto.is_active, Some(false));
    assert_eq!(dto.expires_at, Some(None));
}

#[test]
fn create_api_key_response_serializes_correctly() {
    let record = test_record();
    let dto = CreateApiKeyResponseDto {
        key: ApiKeyRecordResponseDto::from(&record),
        plaintext_key: "rook_test_123456789".to_string(),
    };

    let json = serde_json::to_value(&dto).expect("should serialize");
    assert_eq!(json["plaintextKey"], "rook_test_123456789");
    // Ensure keyHash is not exposed in response
    assert!(
        json["key"]["keyHash"].is_null(),
        "keyHash should not be exposed"
    );
    assert_eq!(json["key"]["id"], "key_123");
    assert_eq!(json["key"]["tier"], "pro");
}
