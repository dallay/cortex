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
            ApiKeyScope::parse("chat:read").unwrap(),
            ApiKeyScope::parse("chat:write").unwrap(),
        ],
        tier: ApiKeyTier::Pro,
        is_active: true,
        revoked_at: None,
        expires_at: Some(Utc::now() + chrono::Duration::days(30)),
        created_at: Utc::now(),
        last_used_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
    }
}

#[test]
fn api_key_record_response_dto_converts_correctly() {
    let record = test_record();
    let dto = ApiKeyRecordResponseDto::from(&record);

    assert_eq!(dto.id, "key_123");
    assert_eq!(dto.label, "Production Key");
    assert_eq!(dto.key_prefix, "rook_test");
    assert_eq!(
        dto.scopes,
        vec!["chat:read".to_string(), "chat:write".to_string()]
    );
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
        "scopes": ["chat:read", "chat:write"],
        "tier": "enterprise",
        "expiresAt": "2026-06-30T00:00:00Z"
    });

    let dto: CreateApiKeyRequestDto = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(dto.label, "Test Key");
    assert_eq!(dto.scopes, vec!["chat:read".to_string(), "chat:write".to_string()]);
    assert_eq!(dto.tier, "enterprise");
    assert!(dto.expires_at.is_some());
}

#[test]
fn update_api_key_request_deserializes_correctly() {
    let json = serde_json::json!({
        "label": "Updated Label",
        "scopes": ["chat:read"],
        "tier": "free",
        "isActive": false,
        "expiresAt": null
    });

    let dto: UpdateApiKeyRequestDto = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(dto.label, Some("Updated Label".to_string()));
    assert_eq!(dto.scopes, Some(vec!["chat:read".to_string()]));
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

// =============================================================================
// Rotation response serialization
//
// The rotate handler returns a CreateApiKeyResponseDto (same shape as create):
// the freshly-rotated record (with NEW keyPrefix) plus the new plaintext key.
// These tests pin down the wire contract.
// =============================================================================

#[test]
fn rotation_response_uses_new_key_prefix_after_rotate() {
    // Simulate a freshly-rotated record: only keyPrefix changed from the
    // pre-rotation state, every other field is preserved.
    let rotated = ApiKeyRecord {
        id: ApiKeyId::new("key_123"),
        label: "Production Key".to_string(),
        // Same key_hash slot, but the prefix points to the NEW secret.
        // We don't expose the hash, but the prefix must reflect the new key.
        key_hash: "hash_xyz987".to_string(),
        key_prefix: "rk-newpr".to_string(),
        scopes: test_record().scopes,
        tier: test_record().tier,
        is_active: true,
        revoked_at: None,
        expires_at: test_record().expires_at,
        created_at: test_record().created_at,
        last_used_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
    };
    let dto = CreateApiKeyResponseDto {
        key: ApiKeyRecordResponseDto::from(&rotated),
        plaintext_key: "rk-newprefix123abc".to_string(),
    };

    let json = serde_json::to_value(&dto).expect("should serialize");
    assert_eq!(
        json["key"]["keyPrefix"], "rk-newpr",
        "rotated record must surface the new prefix"
    );
    assert_eq!(
        json["plaintextKey"], "rk-newprefix123abc",
        "plaintext key must be returned"
    );
    // keyHash must NEVER leak, even on rotation.
    assert!(
        json["key"]["keyHash"].is_null(),
        "keyHash should not be exposed in rotation response"
    );
    // The original test fixture used "rook_test" as prefix. The rotated
    // record uses a different prefix, proving the response reflects the
    // post-rotation state (not a stale pre-rotation read).
    assert_ne!(
        json["key"]["keyPrefix"], "rook_test",
        "prefix must be the NEW one, not the original"
    );
}

#[test]
fn rotation_response_preserves_metadata_through_serialization() {
    // Pre-rotation record (e.g. as it was right before rotate was called).
    let pre = test_record();
    // Post-rotation record: only key_hash and key_prefix changed; everything
    // else MUST be byte-identical to the pre-rotation state.
    let post = ApiKeyRecord {
        key_hash: "different-hash".to_string(),
        key_prefix: "rk-rotat".to_string(),
        ..pre.clone()
    };

    let pre_dto = ApiKeyRecordResponseDto::from(&pre);
    let post_dto = ApiKeyRecordResponseDto::from(&post);

    // All non-credential fields must round-trip identically.
    assert_eq!(pre_dto.id, post_dto.id);
    assert_eq!(pre_dto.label, post_dto.label);
    assert_eq!(pre_dto.scopes, post_dto.scopes);
    assert_eq!(pre_dto.tier, post_dto.tier);
    assert_eq!(pre_dto.is_active, post_dto.is_active);
    assert_eq!(pre_dto.expires_at, post_dto.expires_at);
    assert_eq!(pre_dto.created_at, post_dto.created_at);
    assert_eq!(pre_dto.allowed_models, post_dto.allowed_models);
    assert_eq!(pre_dto.allowed_providers, post_dto.allowed_providers);

    // The credential-adjacent fields are the only ones that change.
    assert_ne!(pre_dto.key_prefix, post_dto.key_prefix);
}
