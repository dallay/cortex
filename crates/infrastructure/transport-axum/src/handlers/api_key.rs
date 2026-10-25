use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::{Json, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Deserializes `Option<Option<T>>` so that:
///   - absent field  → `None`      (no change intended)
///   - `null` value  → `Some(None)` (explicit clear)
///   - concrete value → `Some(Some(v))` (new value)
///
/// Use with `#[serde(default, deserialize_with = "double_option")]`.
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

use rook_core::{ApiKeyId, ApiKeyScope, ApiKeyTier, ModelId, ProviderId};
use rook_usecases::{CreateApiKeyRequest, UpdateApiKeyRequest};

use crate::api_key_dto::ListApiKeysResponseDto;
use crate::HttpError;

type Usecases = Arc<rook_usecases::RookUsecases>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyRequestDto {
    pub label: String,
    pub scopes: Vec<String>,
    pub tier: String,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default)]
    pub allowed_providers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiKeyRequestDto {
    pub label: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub tier: Option<String>,
    pub is_active: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub expires_at: Option<Option<DateTime<Utc>>>,
    pub allowed_models: Option<Vec<String>>,
    pub allowed_providers: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyRecordResponseDto {
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub tier: String,
    pub is_active: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
}

impl From<&rook_core::ApiKeyRecord> for ApiKeyRecordResponseDto {
    fn from(record: &rook_core::ApiKeyRecord) -> Self {
        Self {
            id: record.id.to_string(),
            label: record.label.clone(),
            key_prefix: record.key_prefix.clone(),
            scopes: record
                .scopes
                .iter()
                .map(|s| s.as_str().to_string())
                .collect(),
            tier: record.tier.as_str().to_string(),
            is_active: record.is_active,
            revoked_at: record.revoked_at,
            expires_at: record.expires_at,
            created_at: record.created_at,
            last_used_at: record.last_used_at,
            allowed_models: record
                .allowed_models
                .iter()
                .map(|m| m.as_str().to_string())
                .collect(),
            allowed_providers: record
                .allowed_providers
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyResponseDto {
    pub key: ApiKeyRecordResponseDto,
    pub plaintext_key: String,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

pub async fn list_api_keys(
    State(usecases): State<Usecases>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<ListApiKeysResponseDto<ApiKeyRecordResponseDto>>, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let limit = pagination.limit.clamp(1, 100);
    let offset = pagination.offset.max(0);

    let (records, total) = mak.list_paginated(limit, offset).await.map_err(map_error)?;

    Ok(Json(ListApiKeysResponseDto::new(
        records.iter().map(ApiKeyRecordResponseDto::from).collect(),
        total,
        limit,
        offset,
    )))
}

pub async fn create_api_key(
    State(usecases): State<Usecases>,
    Json(req): Json<CreateApiKeyRequestDto>,
) -> Result<(StatusCode, Json<CreateApiKeyResponseDto>), HttpError> {
    let mak = manage_api_keys(&usecases)?;

    // Parse scopes
    let mut scopes = Vec::new();
    for scope_str in &req.scopes {
        let parsed = ApiKeyScope::parse(scope_str).map_err(|e| HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: e.to_string(),
        })?;
        scopes.push(parsed);
    }

    // Parse tier
    let tier = ApiKeyTier::from_str(&req.tier).map_err(|e| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: e.to_string(),
    })?;

    let domain_req = CreateApiKeyRequest {
        label: req.label,
        scopes,
        tier,
        expires_at: req.expires_at,
        allowed_models: req.allowed_models.into_iter().map(ModelId::new).collect(),
        allowed_providers: req
            .allowed_providers
            .into_iter()
            .map(ProviderId::new)
            .collect(),
    };

    let (record, plaintext_key) = mak.create(domain_req).await.map_err(map_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponseDto {
            key: ApiKeyRecordResponseDto::from(&record),
            plaintext_key,
        }),
    ))
}

pub async fn get_api_key(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyRecordResponseDto>, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let key_id = ApiKeyId::new(id);
    let record = mak
        .get(&key_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("API key not found"))?;

    Ok(Json(ApiKeyRecordResponseDto::from(&record)))
}

pub async fn update_api_key(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequestDto>,
) -> Result<Json<ApiKeyRecordResponseDto>, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let key_id = ApiKeyId::new(id);

    // Parse scopes if present
    let scopes = match req.scopes {
        Some(scopes_vec) => {
            let mut parsed_scopes = Vec::new();
            for s in &scopes_vec {
                let parsed = ApiKeyScope::parse(s).map_err(|e| HttpError {
                    status: StatusCode::BAD_REQUEST,
                    code: "VALIDATION_ERROR",
                    message: e.to_string(),
                })?;
                parsed_scopes.push(parsed);
            }
            Some(parsed_scopes)
        }
        None => None,
    };

    // Parse tier if present
    let tier = match req.tier {
        Some(t_str) => Some(ApiKeyTier::from_str(&t_str).map_err(|e| HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: e.to_string(),
        })?),
        None => None,
    };

    let domain_req = UpdateApiKeyRequest {
        label: req.label,
        scopes,
        tier,
        is_active: req.is_active,
        expires_at: req.expires_at,
        allowed_models: req
            .allowed_models
            .map(|v| v.into_iter().map(ModelId::new).collect()),
        allowed_providers: req
            .allowed_providers
            .map(|v| v.into_iter().map(ProviderId::new).collect()),
    };

    let record = mak.update(&key_id, domain_req).await.map_err(map_error)?;

    Ok(Json(ApiKeyRecordResponseDto::from(&record)))
}

/// Revoke an API key (soft delete). Sets is_active=false and revoked_at=now.
pub async fn revoke_api_key(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let key_id = ApiKeyId::new(id);
    mak.revoke(&key_id).await.map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Rotate an API key: generate a new `rk-*` secret, atomically replace the
/// stored hash and prefix, and return the new raw key exactly once. The old
/// key is invalidated by the hash replacement and can no longer authenticate.
/// All other fields (label, scopes, tier, restrictions, created_at,
/// last_used_at, expires_at) are preserved.
///
/// Returns:
/// - `200 OK` with `{ key, plaintextKey }` (plaintextKey is shown only this once)
/// - `404 NOT_FOUND` if the id does not exist
/// - `409 CONFLICT` (`KEY_REVOKED`) if the key is revoked
pub async fn rotate_api_key(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<CreateApiKeyResponseDto>, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let key_id = ApiKeyId::new(id);
    let (record, plaintext_key) = mak.rotate(&key_id).await.map_err(map_error)?;
    Ok(Json(CreateApiKeyResponseDto {
        key: ApiKeyRecordResponseDto::from(&record),
        plaintext_key,
    }))
}

fn manage_api_keys(
    usecases: &rook_usecases::RookUsecases,
) -> Result<&rook_usecases::ManageApiKeys, HttpError> {
    usecases.manage_api_keys.as_ref().ok_or_else(|| HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: "API key management is not enabled".to_string(),
    })
}

fn map_error(error: rook_usecases::ManageApiKeysError) -> HttpError {
    match error {
        rook_usecases::ManageApiKeysError::NotFound(_) => not_found("API key not found"),
        rook_usecases::ManageApiKeysError::Revoked(_) => HttpError {
            status: StatusCode::CONFLICT,
            code: "KEY_REVOKED",
            message: "API key has been revoked".to_string(),
        },
        rook_usecases::ManageApiKeysError::Repository(
            rook_core::ApiKeyRepositoryError::NotFound(_),
        ) => not_found("API key not found"),
        rook_usecases::ManageApiKeysError::Repository(
            rook_core::ApiKeyRepositoryError::DuplicateHash,
        ) => HttpError {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: "API key conflict".to_string(),
        },
        rook_usecases::ManageApiKeysError::Repository(_) => internal_error(),
        rook_usecases::ManageApiKeysError::Validation(msg) => HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: msg,
        },
    }
}

fn not_found(message: &str) -> HttpError {
    HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: message.to_string(),
    }
}

fn internal_error() -> HttpError {
    HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "INTERNAL_ERROR",
        message: "internal server error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cortex_test_support::FakeApiKeyRepository;
    use encryption_inmemory::Argon2idHasher;

    #[tokio::test]
    async fn test_create_and_get_api_key() {
        let (usecases, _) = setup_common();

        // 1. Create API key
        let create_req = CreateApiKeyRequestDto {
            label: "Test Key".to_string(),
            scopes: vec!["chat:read".to_string(), "chat:write".to_string()],
            tier: "free".to_string(),
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };

        let (status, Json(created_resp)) = create_api_key(State(usecases.clone()), Json(create_req))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert!(!created_resp.plaintext_key.is_empty());
        assert_eq!(created_resp.key.label, "Test Key");

        let key_id = created_resp.key.id;

        // 2. Get API key
        let Json(get_resp) = get_api_key(State(usecases.clone()), Path(key_id.clone()))
            .await
            .unwrap();
        assert_eq!(get_resp.id, key_id);
        assert_eq!(get_resp.label, "Test Key");

        // 3. Update API key
        let update_req = UpdateApiKeyRequestDto {
            label: Some("Updated Key".to_string()),
            scopes: None,
            tier: None,
            is_active: None,
            expires_at: None,
            allowed_models: None,
            allowed_providers: None,
        };
        let Json(updated_resp) = update_api_key(State(usecases.clone()), Path(key_id.clone()), Json(update_req))
            .await
            .unwrap();
        assert_eq!(updated_resp.label, "Updated Key");

        // 4. Rotate API key
        let Json(rotated_resp) = rotate_api_key(State(usecases.clone()), Path(key_id.clone()))
            .await
            .unwrap();
        assert!(!rotated_resp.plaintext_key.is_empty());

        // 5. Revoke API key
        let revoke_status = revoke_api_key(State(usecases.clone()), Path(key_id.clone()))
            .await
            .unwrap();
        assert_eq!(revoke_status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_create_api_key_invalid_scope_and_tier() {
        let (usecases, _) = setup_common();

        // Invalid scope
        let invalid_scope_req = CreateApiKeyRequestDto {
            label: "Test".to_string(),
            scopes: vec!["invalid:scope".to_string()],
            tier: "free".to_string(),
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let scope_err = create_api_key(State(usecases.clone()), Json(invalid_scope_req))
            .await
            .unwrap_err();
        assert_eq!(scope_err.status, StatusCode::BAD_REQUEST);

        // Invalid tier
        let invalid_tier_req = CreateApiKeyRequestDto {
            label: "Test".to_string(),
            scopes: vec!["chat:read".to_string()],
            tier: "invalid_tier".to_string(),
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let tier_err = create_api_key(State(usecases.clone()), Json(invalid_tier_req))
            .await
            .unwrap_err();
        assert_eq!(tier_err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_api_key_not_found() {
        let (usecases, _) = setup_common();

        let err = get_api_key(State(usecases), Path("nonexistent_id".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    fn setup_common() -> (
        Arc<rook_usecases::RookUsecases>,
        Arc<FakeApiKeyRepository>,
    ) {
        #[derive(Default)]
        struct DummyUserRepo;
        #[async_trait]
        impl rook_core::UserRepositoryPort for DummyUserRepo {
            async fn find_by_username(&self, _: &str) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn find_by_id(&self, _: &rook_core::UserId) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> { Ok(false) }
            async fn create(&self, _: &rook_core::NewUser) -> Result<rook_core::User, rook_core::UserRepositoryError> { Err(rook_core::UserRepositoryError::DuplicateUsername) }
            async fn update_password_hash(&self, _: &rook_core::UserId, _: &rook_core::PasswordHash) -> Result<(), rook_core::UserRepositoryError> { Ok(()) }
        }

        let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(DummyUserRepo);
        let hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(Argon2idHasher::new());
        let api_key_repo = Arc::new(FakeApiKeyRepository::default());

        let usecases = crate::bootstrap_helpers::make_test_bootstrap_usecases(
            user_repo.clone(),
            hasher.clone(),
            api_key_repo.clone(),
            rook_usecases::auth::BootstrapStatus::new(user_repo.clone()),
            rook_usecases::auth::SetAdminPassword::new(user_repo, hasher),
            None,
        );

        (usecases, api_key_repo)
    }
}
