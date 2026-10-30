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
    #[serde(default)]
    pub spending_budget: Option<rook_core::SpendingBudgetRequest>,
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
    #[serde(default, deserialize_with = "double_option")]
    pub spending_budget: Option<Option<rook_core::SpendingBudgetRequest>>,
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
    pub spending_budget: Option<rook_core::SpendingBudgetRequest>,
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
            spending_budget: record.spending_budget.as_ref().map(|budget| {
                rook_core::SpendingBudgetRequest {
                    amount_micros_usd: budget.amount_micros_usd,
                    request_limit_micros_usd: budget.request_limit_micros_usd,
                    window: budget.window,
                }
            }),
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

    let spending_budget = req.spending_budget.map(|budget| rook_core::SpendingBudget {
        api_key_id: ApiKeyId::new("pending"),
        amount_micros_usd: budget.amount_micros_usd,
        request_limit_micros_usd: budget.request_limit_micros_usd,
        window: budget.window,
    });
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
        spending_budget,
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

    let spending_budget = req.spending_budget.map(|value| {
        value.map(|budget| rook_core::SpendingBudget {
            api_key_id: key_id.clone(),
            amount_micros_usd: budget.amount_micros_usd,
            request_limit_micros_usd: budget.request_limit_micros_usd,
            window: budget.window,
        })
    });
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
        spending_budget,
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
    use rook_core::ApiKeyRecord;

    fn make_test_record() -> ApiKeyRecord {
        ApiKeyRecord {
            id: ApiKeyId::new("key-123"),
            label: "Test Key".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "rk-test".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Pro,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: Utc::now(),
            last_used_at: None,
            allowed_models: vec![ModelId::new("gpt-4o")],
            allowed_providers: vec![ProviderId::new("openai")],
            spending_budget: None,
        }
    }

    #[test]
    fn test_api_key_record_response_dto_from() {
        let record = make_test_record();
        let dto = ApiKeyRecordResponseDto::from(&record);

        assert_eq!(dto.id, "key-123");
        assert_eq!(dto.label, "Test Key");
        assert_eq!(dto.key_prefix, "rk-test");
        assert_eq!(dto.scopes, vec!["chat:read"]);
        assert_eq!(dto.tier, "pro");
        assert!(dto.is_active);
        assert_eq!(dto.allowed_models, vec!["gpt-4o"]);
        assert_eq!(dto.allowed_providers, vec!["openai"]);
    }

    #[test]
    fn api_key_record_response_dto_preserves_the_spending_budget() {
        let mut record = make_test_record();
        record.spending_budget = Some(rook_core::SpendingBudget {
            api_key_id: ApiKeyId::new("key-123"),
            amount_micros_usd: 4_000_000,
            request_limit_micros_usd: 500_000,
            window: rook_core::SpendingBudgetWindow::Monthly,
        });

        let dto = ApiKeyRecordResponseDto::from(&record);

        assert_eq!(
            dto.spending_budget
                .expect("the budget reaches the response"),
            rook_core::SpendingBudgetRequest {
                amount_micros_usd: 4_000_000,
                request_limit_micros_usd: 500_000,
                window: rook_core::SpendingBudgetWindow::Monthly,
            }
        );
    }

    #[test]
    fn update_request_distinguishes_an_absent_budget_from_an_explicit_clear() {
        let absent: UpdateApiKeyRequestDto = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(absent.spending_budget.is_none());

        let cleared: UpdateApiKeyRequestDto =
            serde_json::from_value(serde_json::json!({ "spendingBudget": null })).unwrap();
        assert!(
            cleared
                .spending_budget
                .is_some_and(|budget| budget.is_none()),
            "an explicit null must clear the budget instead of keeping it"
        );

        let updated: UpdateApiKeyRequestDto = serde_json::from_value(serde_json::json!({
            "spendingBudget": {
                "amountMicrosUsd": 4_000_000,
                "requestLimitMicrosUsd": 500_000,
                "window": "monthly"
            }
        }))
        .unwrap();
        assert_eq!(
            updated.spending_budget.expect("the budget is present"),
            Some(rook_core::SpendingBudgetRequest {
                amount_micros_usd: 4_000_000,
                request_limit_micros_usd: 500_000,
                window: rook_core::SpendingBudgetWindow::Monthly,
            })
        );
    }

    #[test]
    fn test_pagination_params_default() {
        assert_eq!(default_limit(), 20);
    }

    #[test]
    fn test_double_option_deserialization() {
        #[derive(Debug, Deserialize)]
        struct TestStruct {
            #[serde(default, deserialize_with = "double_option")]
            val: Option<Option<String>>,
        }

        // Absent field
        let json_absent = serde_json::json!({});
        let s1: TestStruct = serde_json::from_value(json_absent).unwrap();
        assert_eq!(s1.val, None);

        // Explicit null
        let json_null = serde_json::json!({ "val": null });
        let s2: TestStruct = serde_json::from_value(json_null).unwrap();
        assert_eq!(s2.val, Some(None));

        // Concrete string
        let json_val = serde_json::json!({ "val": "2026-10-15T00:00:00Z" });
        let s3: TestStruct = serde_json::from_value(json_val).unwrap();
        assert_eq!(s3.val, Some(Some("2026-10-15T00:00:00Z".to_string())));
    }

    #[test]
    fn test_map_error_handling() {
        let err_not_found = map_error(rook_usecases::ManageApiKeysError::NotFound(ApiKeyId::new(
            "missing",
        )));
        assert_eq!(err_not_found.status, StatusCode::NOT_FOUND);
        assert_eq!(err_not_found.code, "NOT_FOUND");

        let err_revoked = map_error(rook_usecases::ManageApiKeysError::Revoked(ApiKeyId::new(
            "revoked",
        )));
        assert_eq!(err_revoked.status, StatusCode::CONFLICT);
        assert_eq!(err_revoked.code, "KEY_REVOKED");

        let err_validation = map_error(rook_usecases::ManageApiKeysError::Validation(
            "invalid scope".to_string(),
        ));
        assert_eq!(err_validation.status, StatusCode::BAD_REQUEST);
        assert_eq!(err_validation.code, "VALIDATION_ERROR");

        let err_duplicate = map_error(rook_usecases::ManageApiKeysError::Repository(
            rook_core::ApiKeyRepositoryError::DuplicateHash,
        ));
        assert_eq!(err_duplicate.status, StatusCode::CONFLICT);
        assert_eq!(err_duplicate.code, "CONFLICT");

        let err_repo_not_found = map_error(rook_usecases::ManageApiKeysError::Repository(
            rook_core::ApiKeyRepositoryError::NotFound(ApiKeyId::new("missing-repo")),
        ));
        assert_eq!(err_repo_not_found.status, StatusCode::NOT_FOUND);
        assert_eq!(err_repo_not_found.code, "NOT_FOUND");

        let err_db = map_error(rook_usecases::ManageApiKeysError::Repository(
            rook_core::ApiKeyRepositoryError::Database("sensitive sqlite stacktrace".to_string()),
        ));
        assert_eq!(err_db.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err_db.code, "INTERNAL_ERROR");
        assert_eq!(err_db.message, "internal server error");
        assert!(!err_db.message.contains("sqlite"));
    }
}
