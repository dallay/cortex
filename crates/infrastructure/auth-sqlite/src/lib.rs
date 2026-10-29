use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope,
    ApiKeySubject, ApiKeyTier, ModelId, NewSession, NewUser, PasswordHash, ProviderId, Session,
    SessionId, SessionRepositoryError, SessionRepositoryPort, SpendingBudgetLedgerPort,
    SpendingBudgetReservation, SpendingBudgetReservationError, SpendingReservationOutcome, User,
    UserId, UserRepositoryError, UserRepositoryPort,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, TransactionBehavior};

pub struct SqliteApiKeyRepository {
    conn: Mutex<Connection>,
}

impl SqliteApiKeyRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        if db_path.as_ref().to_str() == Some(":memory:") {
            db_migration::run_on_connection(&mut conn)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ApiKeyRepositoryError> {
        self.conn
            .lock()
            .map_err(|_| ApiKeyRepositoryError::Database("sqlite mutex poisoned".to_string()))
    }

    #[cfg(test)]
    async fn insert_test_key(&self, record: TestApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        self.insert_record_for_test(&record)
    }

    #[cfg(test)]
    fn insert_record_for_test(
        &self,
        record: &TestApiKeyRecord,
    ) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO api_keys (
                id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                revoked_at, expires_at, created_at, last_used_at,
                allowed_models_json, allowed_providers_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.id.to_string(),
                record.label,
                record.key_hash,
                record.key_prefix,
                scopes_to_json(&record.scopes)?,
                record.tier.as_str(),
                bool_to_i64(record.is_active),
                optional_datetime(record.revoked_at),
                optional_datetime(record.expires_at),
                record.created_at.to_rfc3339(),
                optional_datetime(record.last_used_at),
                "[]",
                "[]",
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }

    #[cfg(test)]
    fn last_used_at_for_test(
        &self,
        id: &ApiKeyId,
    ) -> Result<Option<DateTime<Utc>>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let value = conn
            .query_row(
                "SELECT last_used_at FROM api_keys WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        value
            .as_deref()
            .map(parse_datetime)
            .transpose()
            .map_err(db_error)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApiKeyRecord {
    id: ApiKeyId,
    label: String,
    key_hash: String,
    key_prefix: String,
    scopes: Vec<ApiKeyScope>,
    tier: ApiKeyTier,
    is_active: bool,
    revoked_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
impl TestApiKeyRecord {
    fn active(id: &str, key_hash: &str) -> Self {
        Self {
            id: ApiKeyId::new(id),
            label: "Production".to_string(),
            key_hash: key_hash.to_string(),
            key_prefix: key_hash.chars().take(8).collect(),
            scopes: vec![ApiKeyScope::parse("chat:read").expect("scope")],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: Utc::now(),
            last_used_at: None,
        }
    }
}

#[async_trait]
impl SpendingBudgetLedgerPort for SqliteApiKeyRepository {
    async fn reserve_spending(
        &self,
        reservation: &SpendingBudgetReservation,
    ) -> Result<SpendingReservationOutcome, SpendingBudgetReservationError> {
        if reservation.window_start >= reservation.window_end
            || reservation.reserved_micros_usd == 0
            || reservation.budget_micros_usd == 0
            || reservation.reserved_micros_usd > reservation.budget_micros_usd
            || reservation.reserved_micros_usd > i64::MAX as u64
            || reservation.budget_micros_usd > i64::MAX as u64
        {
            return Err(SpendingBudgetReservationError::InvalidReservation);
        }

        let mut conn = self.conn.lock().map_err(|_| {
            SpendingBudgetReservationError::Database("sqlite mutex poisoned".to_string())
        })?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(spending_reservation_db_error)?;
        let existing = tx
            .query_row(
                "SELECT window_start, window_end, reserved_micros_usd, budget_micros_usd
                 FROM api_key_spending_reservations
                 WHERE api_key_id = ?1 AND request_id = ?2",
                params![
                    reservation.api_key_id.to_string(),
                    reservation.request_id.to_string()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(spending_reservation_db_error)?;

        let window_start = reservation.window_start.to_rfc3339();
        let window_end = reservation.window_end.to_rfc3339();
        if let Some((existing_start, existing_end, reserved, budget)) = existing {
            let identical = existing_start == window_start
                && existing_end == window_end
                && reserved == reservation.reserved_micros_usd as i64
                && budget == reservation.budget_micros_usd as i64;
            if identical {
                return Ok(SpendingReservationOutcome::Existing);
            }
            return Err(SpendingBudgetReservationError::IdempotencyConflict);
        }

        let (committed, existing_budget): (i64, Option<i64>) = tx
            .query_row(
                "SELECT
                    COALESCE(SUM(CASE
                        WHEN state IN ('reserved', 'unresolved') THEN reserved_micros_usd
                        WHEN state = 'settled' THEN settled_micros_usd
                        ELSE 0 END), 0),
                    MIN(CASE WHEN state != 'released' THEN budget_micros_usd END)
                 FROM api_key_spending_reservations
                 WHERE api_key_id = ?1 AND window_start = ?2 AND window_end = ?3",
                params![reservation.api_key_id.to_string(), window_start, window_end],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(spending_reservation_db_error)?;
        let next = committed
            .checked_add(reservation.reserved_micros_usd as i64)
            .ok_or(SpendingBudgetReservationError::BudgetExceeded)?;
        let effective_budget = existing_budget
            .unwrap_or(reservation.budget_micros_usd as i64)
            .min(reservation.budget_micros_usd as i64);
        if next > effective_budget {
            return Err(SpendingBudgetReservationError::BudgetExceeded);
        }

        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO api_key_spending_reservations (
                api_key_id, request_id, window_start, window_end,
                reserved_micros_usd, budget_micros_usd, state,
                settled_micros_usd, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'reserved', NULL, ?7, ?7)",
            params![
                reservation.api_key_id.to_string(),
                reservation.request_id.to_string(),
                window_start,
                window_end,
                reservation.reserved_micros_usd as i64,
                reservation.budget_micros_usd as i64,
                now,
            ],
        )
        .map_err(spending_reservation_db_error)?;
        tx.commit().map_err(spending_reservation_db_error)?;
        Ok(SpendingReservationOutcome::Reserved)
    }

    async fn settle_spending(
        &self,
        api_key_id: &rook_core::ApiKeyId,
        request_id: &shared_kernel::RequestId,
        cost_micros_usd: Option<u64>,
    ) -> Result<(), SpendingBudgetReservationError> {
        let cost = cost_micros_usd
            .map(|value| {
                i64::try_from(value)
                    .map_err(|_| SpendingBudgetReservationError::CostExceedsReservation)
            })
            .transpose()?;
        let mut conn = self.conn.lock().map_err(|_| {
            SpendingBudgetReservationError::Database("sqlite mutex poisoned".to_string())
        })?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(spending_reservation_db_error)?;
        let row = tx
            .query_row(
                "SELECT state, reserved_micros_usd, settled_micros_usd
                 FROM api_key_spending_reservations WHERE api_key_id = ?1 AND request_id = ?2",
                params![api_key_id.to_string(), request_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(spending_reservation_db_error)?
            .ok_or(SpendingBudgetReservationError::NotFound)?;

        let (state, reserved, settled) = row;
        let (next_state, next_cost) = match (state.as_str(), cost) {
            ("reserved", Some(cost)) if cost <= reserved => ("settled", Some(cost)),
            ("reserved", Some(_)) => {
                return Err(SpendingBudgetReservationError::CostExceedsReservation)
            }
            ("reserved", None) => ("unresolved", None),
            ("settled", Some(cost)) if settled == Some(cost) => return Ok(()),
            ("unresolved", None) => return Ok(()),
            _ => return Err(SpendingBudgetReservationError::InvalidTransition),
        };
        tx.execute(
            "UPDATE api_key_spending_reservations SET state = ?1, settled_micros_usd = ?2, updated_at = ?3
             WHERE api_key_id = ?4 AND request_id = ?5",
            params![next_state, next_cost, Utc::now().to_rfc3339(), api_key_id.to_string(), request_id.to_string()],
        )
        .map_err(spending_reservation_db_error)?;
        tx.commit().map_err(spending_reservation_db_error)?;
        Ok(())
    }

    async fn release_spending_before_upstream(
        &self,
        api_key_id: &rook_core::ApiKeyId,
        request_id: &shared_kernel::RequestId,
    ) -> Result<(), SpendingBudgetReservationError> {
        let conn = self.conn.lock().map_err(|_| {
            SpendingBudgetReservationError::Database("sqlite mutex poisoned".to_string())
        })?;
        let changed = conn
            .execute(
                "UPDATE api_key_spending_reservations SET state = 'released', updated_at = ?1
                 WHERE api_key_id = ?2 AND request_id = ?3 AND state = 'reserved'",
                params![
                    Utc::now().to_rfc3339(),
                    api_key_id.to_string(),
                    request_id.to_string()
                ],
            )
            .map_err(spending_reservation_db_error)?;
        if changed == 1 {
            return Ok(());
        }
        let state = conn
            .query_row(
                "SELECT state FROM api_key_spending_reservations WHERE api_key_id = ?1 AND request_id = ?2",
                params![api_key_id.to_string(), request_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(spending_reservation_db_error)?
            .ok_or(SpendingBudgetReservationError::NotFound)?;
        if state == "released" {
            Ok(())
        } else {
            Err(SpendingBudgetReservationError::InvalidTransition)
        }
    }
}

#[async_trait]
impl ApiKeyRepositoryPort for SqliteApiKeyRepository {
    async fn find_active_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.query_row(
            "SELECT id, label, scopes_json, tier, allowed_models_json, allowed_providers_json, spending_budget_json
             FROM api_keys
             WHERE key_hash = ?1
               AND is_active = 1
               AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > ?2)",
            params![hash, now],
            row_to_subject,
        )
        .optional()
        .map_err(db_error)
    }

    async fn record_last_used(
        &self,
        id: &ApiKeyId,
        used_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute(
                "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                params![used_at.to_rfc3339(), id.to_string()],
            )
            .map_err(db_error)?;
        if rows == 0 {
            return Err(ApiKeyRepositoryError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                        revoked_at, expires_at, created_at, last_used_at,
                        allowed_models_json, allowed_providers_json, spending_budget_json
                 FROM api_keys
                 ORDER BY created_at DESC",
            )
            .map_err(db_error)?;
        let records = stmt
            .query_map([], row_to_record)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(records)
    }

    async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                    revoked_at, expires_at, created_at, last_used_at,
                    allowed_models_json, allowed_providers_json, spending_budget_json
             FROM api_keys
             WHERE id = ?1",
            params![id.to_string()],
            row_to_record,
        )
        .optional()
        .map_err(db_error)
    }

    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO api_keys (
                id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                revoked_at, expires_at, created_at, last_used_at,
                allowed_models_json, allowed_providers_json, spending_budget_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                record.id.to_string(),
                record.label,
                record.key_hash,
                record.key_prefix,
                scopes_to_json(&record.scopes)?,
                record.tier.as_str(),
                bool_to_i64(record.is_active),
                optional_datetime(record.revoked_at),
                optional_datetime(record.expires_at),
                record.created_at.to_rfc3339(),
                optional_datetime(record.last_used_at),
                models_to_json(&record.allowed_models)?,
                providers_to_json(&record.allowed_providers)?,
                record
                    .spending_budget
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))?,
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }

    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute(
                "UPDATE api_keys SET
                    label = ?1,
                    scopes_json = ?2,
                    tier = ?3,
                    is_active = ?4,
                    revoked_at = ?5,
                    expires_at = ?6,
                    last_used_at = ?7,
                    allowed_models_json = ?8,
                    allowed_providers_json = ?9,
                    spending_budget_json = ?10
                 WHERE id = ?11",
                params![
                    record.label,
                    scopes_to_json(&record.scopes)?,
                    record.tier.as_str(),
                    bool_to_i64(record.is_active),
                    optional_datetime(record.revoked_at),
                    optional_datetime(record.expires_at),
                    optional_datetime(record.last_used_at),
                    models_to_json(&record.allowed_models)?,
                    providers_to_json(&record.allowed_providers)?,
                    record
                        .spending_budget
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))?,
                    record.id.to_string(),
                ],
            )
            .map_err(db_error)?;
        if rows == 0 {
            return Err(ApiKeyRepositoryError::NotFound(record.id.clone()));
        }
        Ok(())
    }

    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute(
                "DELETE FROM api_keys WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(db_error)?;
        if rows == 0 {
            return Err(ApiKeyRepositoryError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn revoke(
        &self,
        id: &ApiKeyId,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;

        // First check if the key exists
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?1)",
                params![id.to_string()],
                |row| row.get(0),
            )
            .map_err(db_error)?;

        if !exists {
            return Err(ApiKeyRepositoryError::NotFound(id.clone()));
        }

        // Use COALESCE to preserve original revoked_at if already set (idempotent)
        conn.execute(
            "UPDATE api_keys SET is_active = 0, revoked_at = COALESCE(revoked_at, ?1) WHERE id = ?2",
            params![revoked_at.to_rfc3339(), id.to_string()],
        )
        .map_err(db_error)?;

        Ok(())
    }

    async fn rotate_hash(
        &self,
        id: &ApiKeyId,
        new_hash: &str,
        new_prefix: &str,
    ) -> Result<(), ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute(
                "UPDATE api_keys SET key_hash = ?1, key_prefix = ?2 WHERE id = ?3",
                params![new_hash, new_prefix, id.to_string()],
            )
            .map_err(db_error)?;
        if rows == 0 {
            return Err(ApiKeyRepositoryError::NotFound(id.clone()));
        }
        Ok(())
    }

    async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                    revoked_at, expires_at, created_at, last_used_at,
                    allowed_models_json, allowed_providers_json, spending_budget_json
             FROM api_keys
             WHERE is_active = 1
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2",
            )
            .map_err(db_error)?;
        let records = stmt
            .query_map(params![limit, offset], row_to_record)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(records)
    }

    async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT COUNT(*) FROM api_keys WHERE is_active = 1",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    }
}

fn row_to_subject(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKeySubject> {
    let id: String = row.get("id")?;
    let label: String = row.get("label")?;
    let scopes_json: String = row.get("scopes_json")?;
    let tier: String = row.get("tier")?;
    let allowed_models_json: String = row.get("allowed_models_json")?;
    let allowed_providers_json: String = row.get("allowed_providers_json")?;
    let spending_budget_json: Option<String> = row.get("spending_budget_json")?;
    Ok(ApiKeySubject {
        id: ApiKeyId::new(id),
        label,
        scopes: scopes_from_json(&scopes_json).map_err(|error| invalid_data(error.to_string()))?,
        tier: ApiKeyTier::from_str(&tier).map_err(|error| invalid_data(error.to_string()))?,
        allowed_models: models_from_json(&allowed_models_json)
            .map_err(|error| invalid_data(error.to_string()))?,
        allowed_providers: providers_from_json(&allowed_providers_json)
            .map_err(|error| invalid_data(error.to_string()))?,
        spending_budget: spending_budget_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| invalid_data(error.to_string()))?,
    })
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKeyRecord> {
    let id: String = row.get("id")?;
    let label: String = row.get("label")?;
    let key_hash: String = row.get("key_hash")?;
    let key_prefix: String = row.get("key_prefix")?;
    let scopes_json: String = row.get("scopes_json")?;
    let tier: String = row.get("tier")?;
    let is_active: i64 = row.get("is_active")?;
    let revoked_at_str: Option<String> = row.get("revoked_at")?;
    let expires_at_str: Option<String> = row.get("expires_at")?;
    let created_at_str: String = row.get("created_at")?;
    let last_used_at_str: Option<String> = row.get("last_used_at")?;
    let allowed_models_json: String = row.get("allowed_models_json")?;
    let allowed_providers_json: String = row.get("allowed_providers_json")?;
    let spending_budget_json: Option<String> = row.get("spending_budget_json")?;

    let revoked_at = revoked_at_str.map(|s| parse_datetime(&s)).transpose()?;
    let expires_at = expires_at_str.map(|s| parse_datetime(&s)).transpose()?;
    let created_at = parse_datetime(&created_at_str)?;
    let last_used_at = last_used_at_str.map(|s| parse_datetime(&s)).transpose()?;

    Ok(ApiKeyRecord {
        id: ApiKeyId::new(id),
        label,
        key_hash,
        key_prefix,
        scopes: scopes_from_json(&scopes_json).map_err(|error| invalid_data(error.to_string()))?,
        tier: ApiKeyTier::from_str(&tier).map_err(|error| invalid_data(error.to_string()))?,
        is_active: is_active != 0,
        revoked_at,
        expires_at,
        created_at,
        last_used_at,
        allowed_models: models_from_json(&allowed_models_json)
            .map_err(|error| invalid_data(error.to_string()))?,
        allowed_providers: providers_from_json(&allowed_providers_json)
            .map_err(|error| invalid_data(error.to_string()))?,
        spending_budget: spending_budget_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| invalid_data(error.to_string()))?,
    })
}

fn scopes_from_json(value: &str) -> Result<Vec<ApiKeyScope>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|error| error.to_string())?;
    Ok(values
        .iter()
        .map(|scope| ApiKeyScope::parse_lenient(scope))
        .collect())
}

fn scopes_to_json(scopes: &[ApiKeyScope]) -> Result<String, ApiKeyRepositoryError> {
    let values = scopes.iter().map(ApiKeyScope::as_str).collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))
}

fn models_from_json(value: &str) -> Result<Vec<ModelId>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|e| e.to_string())?;
    Ok(values.into_iter().map(ModelId::new).collect())
}

fn models_to_json(models: &[ModelId]) -> Result<String, ApiKeyRepositoryError> {
    let values: Vec<&str> = models.iter().map(|m| m.as_str()).collect();
    serde_json::to_string(&values).map_err(|e| ApiKeyRepositoryError::Database(e.to_string()))
}

fn providers_from_json(value: &str) -> Result<Vec<ProviderId>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|e| e.to_string())?;
    Ok(values.into_iter().map(ProviderId::new).collect())
}

fn providers_to_json(providers: &[ProviderId]) -> Result<String, ApiKeyRepositoryError> {
    let values: Vec<&str> = providers.iter().map(|p| p.as_str()).collect();
    serde_json::to_string(&values).map_err(|e| ApiKeyRepositoryError::Database(e.to_string()))
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn optional_datetime(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|dt| dt.to_rfc3339())
}

// ---------------------------------------------------------------------------
// SqliteUserRepository — user persistence for MANAGEMENT auth
// ---------------------------------------------------------------------------

pub struct SqliteUserRepository {
    conn: Mutex<Connection>,
}

impl SqliteUserRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        if db_path.as_ref().to_str() == Some(":memory:") {
            db_migration::run_on_connection(&mut conn)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, UserRepositoryError> {
        self.conn
            .lock()
            .map_err(|_| UserRepositoryError::Database("sqlite mutex poisoned".to_string()))
    }
}

#[async_trait]
impl UserRepositoryPort for SqliteUserRepository {
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, UserRepositoryError> {
        let conn = self.lock()?;
        let result = conn
            .query_row(
                "SELECT id, username, password_hash, created_at, updated_at
                 FROM users
                 WHERE username = ?1 COLLATE NOCASE",
                params![username],
                row_to_user,
            )
            .optional()
            .map_err(user_db_error)?;
        Ok(result)
    }

    async fn find_by_id(&self, user_id: &UserId) -> Result<Option<User>, UserRepositoryError> {
        let conn = self.lock()?;
        let result = conn
            .query_row(
                "SELECT id, username, password_hash, created_at, updated_at
                 FROM users
                 WHERE id = ?1",
                params![user_id.to_string()],
                row_to_user,
            )
            .optional()
            .map_err(user_db_error)?;
        Ok(result)
    }

    async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
        let conn = self.lock()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .map_err(user_db_error)?;
        Ok(count > 0)
    }

    async fn create(&self, user: &NewUser) -> Result<User, UserRepositoryError> {
        let conn = self.lock()?;
        let id = UserId::new();
        let now = Utc::now();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                user.username,
                user.password_hash,
                now.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )
        .map_err(user_db_error)?;
        Ok(User {
            id,
            username: user.username.clone(),
            password_hash: user.password_hash.clone(),
            created_at: now,
            updated_at: now,
        })
    }

    async fn update_password_hash(
        &self,
        user_id: &UserId,
        hash: &PasswordHash,
    ) -> Result<(), UserRepositoryError> {
        let conn = self.lock()?;
        let now = Utc::now();
        let rows = conn
            .execute(
                "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3",
                params![hash.as_str(), now.to_rfc3339(), user_id.to_string()],
            )
            .map_err(user_db_error)?;
        if rows == 0 {
            return Err(UserRepositoryError::NotFound(user_id.clone()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SqliteSessionRepository — session persistence for MANAGEMENT auth
// ---------------------------------------------------------------------------

pub struct SqliteSessionRepository {
    conn: Mutex<Connection>,
}

impl SqliteSessionRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        if db_path.as_ref().to_str() == Some(":memory:") {
            db_migration::run_on_connection(&mut conn)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, SessionRepositoryError> {
        self.conn
            .lock()
            .map_err(|_| SessionRepositoryError::Database("sqlite mutex poisoned".to_string()))
    }
}

#[async_trait]
impl SessionRepositoryPort for SqliteSessionRepository {
    async fn create(
        &self,
        session: &NewSession,
        token_hash: &str,
    ) -> Result<Session, SessionRepositoryError> {
        let conn = self.lock()?;
        let id = SessionId::new();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(24);
        conn.execute(
            "INSERT INTO sessions (id, token_hash, user_id, created_at, expires_at, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                id.to_string(),
                token_hash,
                session.user_id.to_string(),
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )
        .map_err(session_db_error)?;
        Ok(Session {
            id,
            token_hash: token_hash.to_string(),
            user_id: session.user_id.clone(),
            created_at: now,
            expires_at,
            revoked: false,
        })
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let result = conn
            .query_row(
                "SELECT id, token_hash, user_id, created_at, expires_at, revoked
                 FROM sessions
                 WHERE token_hash = ?1
                   AND revoked = 0
                   AND expires_at > ?2",
                params![token_hash, now],
                row_to_session,
            )
            .optional()
            .map_err(session_db_error)?;
        Ok(result)
    }

    async fn revoke(&self, session_id: &SessionId) -> Result<(), SessionRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute(
                "UPDATE sessions SET revoked = 1 WHERE id = ?1",
                params![session_id.to_string()],
            )
            .map_err(session_db_error)?;
        if rows == 0 {
            return Err(SessionRepositoryError::NotFound(session_id.clone()));
        }
        Ok(())
    }

    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let count = conn
            .execute("DELETE FROM sessions WHERE expires_at < ?1", params![now])
            .map_err(session_db_error)?;
        Ok(count as u64)
    }
}

// ---------------------------------------------------------------------------
// Row parsing helpers
// ---------------------------------------------------------------------------

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let id_str: String = row.get("id")?;
    let username: String = row.get("username")?;
    let password_hash: Option<String> = row.get("password_hash")?;
    let created_at_str: String = row.get("created_at")?;
    let updated_at_str: String = row.get("updated_at")?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))?;
    Ok(User {
        id: UserId::parse_str(&id_str).map_err(|_| invalid_data("invalid user id"))?,
        username,
        password_hash,
        created_at,
        updated_at,
    })
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let id_str: String = row.get("id")?;
    let token_hash: String = row.get("token_hash")?;
    let user_id_str: String = row.get("user_id")?;
    let created_at_str: String = row.get("created_at")?;
    let expires_at_str: String = row.get("expires_at")?;
    let revoked: i64 = row.get("revoked")?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))?;
    let expires_at = DateTime::parse_from_rfc3339(&expires_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))?;
    Ok(Session {
        id: SessionId::parse_str(&id_str).map_err(|_| invalid_data("invalid session id"))?,
        token_hash,
        user_id: UserId::parse_str(&user_id_str).map_err(|_| invalid_data("invalid user id"))?,
        created_at,
        expires_at,
        revoked: revoked != 0,
    })
}

fn invalid_data(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

fn parse_datetime(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))
}

fn user_db_error(error: rusqlite::Error) -> UserRepositoryError {
    match error {
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if sqlite_error.code == ErrorCode::ConstraintViolation =>
        {
            UserRepositoryError::DuplicateUsername
        }
        other => UserRepositoryError::Database(other.to_string()),
    }
}

fn session_db_error(error: rusqlite::Error) -> SessionRepositoryError {
    SessionRepositoryError::Database(error.to_string())
}

fn spending_reservation_db_error(error: rusqlite::Error) -> SpendingBudgetReservationError {
    SpendingBudgetReservationError::Database(error.to_string())
}

fn db_error(error: rusqlite::Error) -> ApiKeyRepositoryError {
    match error {
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if sqlite_error.code == ErrorCode::ConstraintViolation =>
        {
            ApiKeyRepositoryError::DuplicateHash
        }
        other => ApiKeyRepositoryError::Database(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use rook_core::{
        ApiKeyId, ApiKeyRepositoryPort, ApiKeyScope, ApiKeyTier, NewSession as CoreNewSession,
        NewUser as CoreNewUser, PasswordHash as CorePasswordHash, SessionRepositoryPort,
        SpendingBudgetLedgerPort, UserRepositoryPort,
    };
    use rusqlite::params;
    use std::path::PathBuf;

    use super::{
        SqliteApiKeyRepository, SqliteSessionRepository, SqliteUserRepository, TestApiKeyRecord,
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // Each test owns an isolated directory. Keep the directory alive while the
    // SQLite repositories use the database, then let TempDir remove only files
    // created by this test.
    fn shared_test_db() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("temporary test directory");
        let path = directory.path().join("auth.db");
        db_migration::run_migrations(&path.to_string_lossy()).expect("migrations");
        (directory, path)
    }

    #[test]
    fn spending_reservation_rejects_total_budget_overrun() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let api_key_id = ApiKeyId::new("budget-key");
            let window_start = Utc::with_ymd_and_hms(&Utc, 2026, 10, 28, 0, 0, 0)
                .single()
                .expect("window start");
            let window_end = Utc
                .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                .single()
                .expect("window end");
            let first = rook_core::SpendingBudgetReservation {
                api_key_id: api_key_id.clone(),
                request_id: shared_kernel::RequestId::new(),
                window_start,
                window_end,
                reserved_micros_usd: 60,
                budget_micros_usd: 100,
            };
            repo.reserve_spending(&first)
                .await
                .expect("first reservation");

            let second = rook_core::SpendingBudgetReservation {
                request_id: shared_kernel::RequestId::new(),
                reserved_micros_usd: 41,
                ..first
            };
            let result = repo.reserve_spending(&second).await;

            assert!(matches!(
                result,
                Err(rook_core::SpendingBudgetReservationError::BudgetExceeded)
            ));
        });
    }

    #[test]
    fn spending_reservation_is_idempotent_and_rejects_changed_parameters() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let reservation = rook_core::SpendingBudgetReservation {
                api_key_id: ApiKeyId::new("budget-key"),
                request_id: shared_kernel::RequestId::new(),
                window_start: Utc
                    .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
                    .single()
                    .unwrap(),
                window_end: Utc
                    .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                    .single()
                    .unwrap(),
                reserved_micros_usd: 60,
                budget_micros_usd: 100,
            };
            assert_eq!(
                repo.reserve_spending(&reservation).await.expect("reserve"),
                rook_core::SpendingReservationOutcome::Reserved
            );
            assert_eq!(
                repo.reserve_spending(&reservation)
                    .await
                    .expect("idempotent replay"),
                rook_core::SpendingReservationOutcome::Existing
            );

            let conflicting = rook_core::SpendingBudgetReservation {
                reserved_micros_usd: 61,
                ..reservation
            };
            assert!(matches!(
                repo.reserve_spending(&conflicting).await,
                Err(rook_core::SpendingBudgetReservationError::IdempotencyConflict)
            ));
        });
    }

    #[test]
    fn unknown_cost_keeps_reservation_and_release_only_works_before_settlement() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let api_key_id = ApiKeyId::new("budget-key");
            let request_id = shared_kernel::RequestId::new();
            let reservation = rook_core::SpendingBudgetReservation {
                api_key_id: api_key_id.clone(),
                request_id: request_id.clone(),
                window_start: Utc
                    .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
                    .single()
                    .unwrap(),
                window_end: Utc
                    .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                    .single()
                    .unwrap(),
                reserved_micros_usd: 60,
                budget_micros_usd: 100,
            };
            repo.reserve_spending(&reservation).await.expect("reserve");
            repo.settle_spending(&api_key_id, &request_id, None)
                .await
                .expect("unknown cost");
            repo.settle_spending(&api_key_id, &request_id, None)
                .await
                .expect("unknown cost replay");
            assert!(matches!(
                repo.release_spending_before_upstream(&api_key_id, &request_id)
                    .await,
                Err(rook_core::SpendingBudgetReservationError::InvalidTransition)
            ));

            let second = rook_core::SpendingBudgetReservation {
                request_id: shared_kernel::RequestId::new(),
                ..reservation
            };
            assert!(matches!(
                repo.reserve_spending(&second).await,
                Err(rook_core::SpendingBudgetReservationError::BudgetExceeded)
            ));
        });
    }

    #[test]
    fn concurrent_reservations_cannot_exceed_window_budget() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let db_path = directory.path().join("budget.db");
        db_migration::run_migrations(&db_path.to_string_lossy()).expect("migrations");
        let window_start = Utc
            .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
            .single()
            .unwrap();
        let window_end = Utc
            .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
            .single()
            .unwrap();
        let api_key_id = ApiKeyId::new("budget-key");

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let db_path = db_path.clone();
                let api_key_id = api_key_id.clone();
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .expect("runtime");
                    runtime.block_on(async move {
                        let repo = SqliteApiKeyRepository::new(db_path).expect("repo");
                        let reservation = rook_core::SpendingBudgetReservation {
                            api_key_id,
                            request_id: shared_kernel::RequestId::new(),
                            window_start,
                            window_end,
                            reserved_micros_usd: 60,
                            budget_micros_usd: 100,
                        };
                        repo.reserve_spending(&reservation).await
                    })
                })
            })
            .collect();

        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread"))
            .collect();
        let admitted = results.iter().filter(|result| result.is_ok()).count();
        let rejected = results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(rook_core::SpendingBudgetReservationError::BudgetExceeded)
                )
            })
            .count();
        assert_eq!(admitted, 1);
        assert_eq!(rejected, 7);
    }

    #[test]
    fn settled_cost_releases_unused_reservation_but_rejects_cost_above_cap() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let api_key_id = ApiKeyId::new("budget-key");
            let reservation = rook_core::SpendingBudgetReservation {
                api_key_id: api_key_id.clone(),
                request_id: shared_kernel::RequestId::new(),
                window_start: Utc
                    .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
                    .single()
                    .unwrap(),
                window_end: Utc
                    .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                    .single()
                    .unwrap(),
                reserved_micros_usd: 60,
                budget_micros_usd: 100,
            };
            repo.reserve_spending(&reservation).await.expect("reserve");
            assert!(matches!(
                repo.settle_spending(&api_key_id, &reservation.request_id, Some(61))
                    .await,
                Err(rook_core::SpendingBudgetReservationError::CostExceedsReservation)
            ));
            repo.settle_spending(&api_key_id, &reservation.request_id, Some(40))
                .await
                .expect("settle");

            let second = rook_core::SpendingBudgetReservation {
                request_id: shared_kernel::RequestId::new(),
                reserved_micros_usd: 60,
                ..reservation.clone()
            };
            assert_eq!(
                repo.reserve_spending(&second)
                    .await
                    .expect("reused headroom"),
                rook_core::SpendingReservationOutcome::Reserved
            );
            assert!(matches!(
                repo.release_spending_before_upstream(&api_key_id, &reservation.request_id)
                    .await,
                Err(rook_core::SpendingBudgetReservationError::InvalidTransition)
            ));
        });
    }

    #[test]
    fn release_before_upstream_is_idempotent_and_frees_headroom() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let api_key_id = ApiKeyId::new("budget-key");
            let reservation = rook_core::SpendingBudgetReservation {
                api_key_id: api_key_id.clone(),
                request_id: shared_kernel::RequestId::new(),
                window_start: Utc
                    .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
                    .single()
                    .unwrap(),
                window_end: Utc
                    .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                    .single()
                    .unwrap(),
                reserved_micros_usd: 60,
                budget_micros_usd: 100,
            };
            repo.reserve_spending(&reservation).await.expect("reserve");
            repo.release_spending_before_upstream(&api_key_id, &reservation.request_id)
                .await
                .expect("release");
            repo.release_spending_before_upstream(&api_key_id, &reservation.request_id)
                .await
                .expect("release replay");
            let next = rook_core::SpendingBudgetReservation {
                request_id: shared_kernel::RequestId::new(),
                ..reservation
            };
            assert_eq!(
                repo.reserve_spending(&next)
                    .await
                    .expect("reserve freed amount"),
                rook_core::SpendingReservationOutcome::Reserved
            );
        });
    }

    #[test]
    fn spending_reservation_survives_repository_reopen() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let db_path = directory.path().join("budget.db");
        db_migration::run_migrations(&db_path.to_string_lossy()).expect("migrations");
        let reservation = rook_core::SpendingBudgetReservation {
            api_key_id: ApiKeyId::new("budget-key"),
            request_id: shared_kernel::RequestId::new(),
            window_start: Utc
                .with_ymd_and_hms(2026, 10, 28, 0, 0, 0)
                .single()
                .unwrap(),
            window_end: Utc
                .with_ymd_and_hms(2026, 10, 29, 0, 0, 0)
                .single()
                .unwrap(),
            reserved_micros_usd: 60,
            budget_micros_usd: 100,
        };
        runtime().block_on(async {
            let first_repo = SqliteApiKeyRepository::new(&db_path).expect("first repo");
            first_repo
                .reserve_spending(&reservation)
                .await
                .expect("reserve");
            drop(first_repo);

            let reopened_repo = SqliteApiKeyRepository::new(&db_path).expect("reopened repo");
            let second = rook_core::SpendingBudgetReservation {
                request_id: shared_kernel::RequestId::new(),
                reserved_micros_usd: 41,
                ..reservation
            };
            assert!(matches!(
                reopened_repo.reserve_spending(&second).await,
                Err(rook_core::SpendingBudgetReservationError::BudgetExceeded)
            ));
        });
    }

    #[test]
    fn finds_only_active_unrevoked_unexpired_api_keys_by_hash() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let mut active = TestApiKeyRecord::active("active", "hash-active");
            active
                .scopes
                .push(rook_core::ApiKeyScope::parse("chat:write").expect("scope"));
            active.tier = ApiKeyTier::Pro;
            active.expires_at = Some(Utc::now() + Duration::days(1));
            repo.insert_test_key(active).await.expect("insert active");
            let mut revoked = TestApiKeyRecord::active("revoked", "hash-revoked");
            revoked.label = "Revoked".to_string();
            revoked.revoked_at = Some(Utc::now());
            repo.insert_test_key(revoked).await.expect("insert revoked");

            let active = repo
                .find_active_by_hash("hash-active")
                .await
                .expect("find active")
                .expect("active subject");
            assert_eq!(active.id, ApiKeyId::new("active"));
            assert_eq!(active.tier, ApiKeyTier::Pro);
            assert_eq!(active.scopes[0].as_str(), "chat:read");
            assert!(repo
                .find_active_by_hash("hash-revoked")
                .await
                .expect("find revoked")
                .is_none());
        });
    }

    #[test]
    fn record_last_used_updates_timestamp() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let id = ApiKeyId::new("active");
            repo.insert_test_key(TestApiKeyRecord::active(id.as_str(), "hash-active"))
                .await
                .expect("insert");
            let used_at = Utc::now();

            repo.record_last_used(&id, used_at)
                .await
                .expect("record last used");

            let stored = repo.last_used_at_for_test(&id).expect("last used");
            assert!(stored.is_some(), "last_used should be recorded");
            // Fuzzy comparison: SQLite datetime storage may truncate microseconds
            let diff = (stored.unwrap() - used_at).num_milliseconds().abs();
            assert!(
                diff < 1000,
                "last_used should be within 1 second of used_at"
            );
        });
    }

    #[test]
    fn api_key_repository_crud() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");

            // 1. Create a key
            let record = rook_core::ApiKeyRecord {
                id: ApiKeyId::new("key-123"),
                label: "Development Key".to_string(),
                key_hash: "hash-123".to_string(),
                key_prefix: "rk-123".to_string(),
                scopes: vec![rook_core::ApiKeyScope::parse("chat:read").unwrap()],
                tier: ApiKeyTier::Free,
                is_active: true,
                revoked_at: None,
                expires_at: None,
                created_at: Utc::now(),
                last_used_at: None,
                allowed_models: vec![],
                allowed_providers: vec![],
                spending_budget: Some(rook_core::SpendingBudget {
                    api_key_id: ApiKeyId::new("key-123"),
                    amount_micros_usd: 25_000_000,
                    request_limit_micros_usd: 2_000_000,
                    window: rook_core::SpendingBudgetWindow::Monthly,
                }),
            };
            repo.create(&record).await.expect("create");

            // 2. Find the key
            let found = repo
                .find(&ApiKeyId::new("key-123"))
                .await
                .expect("find")
                .expect("some");
            assert_eq!(found.label, "Development Key");
            assert_eq!(found.spending_budget, record.spending_budget);
            let authenticated = repo
                .find_active_by_hash("hash-123")
                .await
                .expect("find active by hash")
                .expect("active subject");
            assert_eq!(authenticated.spending_budget, record.spending_budget);
            assert_eq!(found.key_hash, "hash-123");
            assert!(found.is_active);

            // 3. List keys
            let list = repo.list().await.expect("list");
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, ApiKeyId::new("key-123"));

            // 4. Update the key
            let mut updated = found;
            updated.label = "Production Key".to_string();
            updated.tier = ApiKeyTier::Enterprise;
            updated.is_active = false;
            repo.update(&updated).await.expect("update");

            let found_updated = repo
                .find(&ApiKeyId::new("key-123"))
                .await
                .expect("find")
                .expect("some");
            assert_eq!(found_updated.label, "Production Key");
            assert_eq!(found_updated.tier, ApiKeyTier::Enterprise);
            assert!(!found_updated.is_active);

            // 5. Delete the key
            repo.delete(&ApiKeyId::new("key-123"))
                .await
                .expect("delete");
            let found_deleted = repo.find(&ApiKeyId::new("key-123")).await.expect("find");
            assert!(found_deleted.is_none());
        });
    }

    #[test]
    fn revoke_sets_is_active_false_and_revoked_at() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let record = TestApiKeyRecord::active("revoke-test", "hash-revoke");
            repo.insert_test_key(record).await.expect("insert");

            repo.revoke(&ApiKeyId::new("revoke-test"), Utc::now())
                .await
                .expect("revoke");

            let found = repo
                .find(&ApiKeyId::new("revoke-test"))
                .await
                .expect("find")
                .expect("some");
            assert!(!found.is_active);
            assert!(found.revoked_at.is_some());
        });
    }

    #[test]
    fn revoke_idempotent() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let record = TestApiKeyRecord::active("idempotent-test", "hash-idempotent");
            repo.insert_test_key(record).await.expect("insert");

            // Revoke twice
            repo.revoke(&ApiKeyId::new("idempotent-test"), Utc::now())
                .await
                .expect("first revoke");
            let second = repo
                .revoke(&ApiKeyId::new("idempotent-test"), Utc::now())
                .await;
            assert!(second.is_ok(), "second revoke should not error");
        });
    }

    #[test]
    fn list_paginated_returns_correct_slice() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");

            // Insert 5 keys
            for i in 0..5 {
                let record = rook_core::ApiKeyRecord {
                    id: ApiKeyId::new(format!("key-{}", i)),
                    label: format!("Key {}", i),
                    key_hash: format!("hash-{}", i),
                    key_prefix: format!("rk-{}", i),
                    scopes: vec![rook_core::ApiKeyScope::parse("chat:read").unwrap()],
                    tier: ApiKeyTier::Free,
                    is_active: true,
                    revoked_at: None,
                    expires_at: None,
                    created_at: Utc::now(),
                    last_used_at: None,
                    allowed_models: vec![],
                    allowed_providers: vec![],
                    spending_budget: None,
                };
                repo.create(&record).await.expect("create");
            }

            // Get first page (limit 2, offset 0)
            let page1 = repo.list_paginated(2, 0).await.expect("list paginated");
            assert_eq!(page1.len(), 2);

            // Get second page (limit 2, offset 2)
            let page2 = repo.list_paginated(2, 2).await.expect("list paginated");
            assert_eq!(page2.len(), 2);

            // Get total count
            let total = repo.count().await.expect("count");
            assert_eq!(total, 5);
        });
    }

    // =============================================================================
    // SqliteUserRepository tests
    // =============================================================================

    #[test]
    fn user_repository_create_and_find_by_username() {
        runtime().block_on(async {
            let repo = SqliteUserRepository::new(":memory:").expect("repo");
            let new_user = CoreNewUser {
                username: "admin".to_string(),
                password_hash: None,
            };
            let created = repo.create(&new_user).await.expect("create user");
            assert_eq!(created.username, "admin");
            assert!(created.password_hash.is_none());

            let found = repo
                .find_by_username("admin")
                .await
                .expect("find by username")
                .expect("user should exist");
            assert_eq!(found.username, "admin");
        });
    }

    #[test]
    fn user_repository_find_by_username_case_insensitive() {
        runtime().block_on(async {
            let repo = SqliteUserRepository::new(":memory:").expect("repo");
            let new_user = CoreNewUser {
                username: "Admin".to_string(),
                password_hash: None,
            };
            repo.create(&new_user).await.expect("create user");

            let found = repo
                .find_by_username("ADMIN")
                .await
                .expect("find by username")
                .expect("user should exist");
            assert_eq!(found.username, "Admin");
        });
    }

    #[test]
    fn user_repository_create_duplicate_username() {
        runtime().block_on(async {
            let repo = SqliteUserRepository::new(":memory:").expect("repo");
            let new_user = CoreNewUser {
                username: "admin".to_string(),
                password_hash: None,
            };
            repo.create(&new_user).await.expect("create user");

            let duplicate_user = CoreNewUser {
                username: "admin".to_string(),
                password_hash: Some("hash".to_string()),
            };
            let result = repo.create(&duplicate_user).await;
            // First call should get DuplicateUsername, but the constraint might cause Database error
            // depending on SQLite version. Both are acceptable behaviors.
            match result {
                Err(rook_core::UserRepositoryError::DuplicateUsername) => {}
                Err(rook_core::UserRepositoryError::Database(_)) => {}
                other => panic!("expected DuplicateUsername or Database, got {:?}", other),
            }
        });
    }

    #[test]
    fn user_repository_update_password_hash() {
        runtime().block_on(async {
            let repo = SqliteUserRepository::new(":memory:").expect("repo");
            let created = repo
                .create(&CoreNewUser {
                    username: "admin".to_string(),
                    password_hash: None,
                })
                .await
                .expect("create user");

            let hash = CorePasswordHash::from("$argon2id$hash".to_string());
            repo.update_password_hash(&created.id, &hash)
                .await
                .expect("update password hash");

            let found = repo
                .find_by_id(&created.id)
                .await
                .expect("find by id")
                .expect("user should exist");
            assert_eq!(found.password_hash.as_deref(), Some("$argon2id$hash"));
        });
    }

    // =============================================================================
    // SqliteSessionRepository tests
    // =============================================================================

    #[test]
    fn session_repository_create_and_find() {
        let (_db_dir, db_path) = shared_test_db();
        let user_repo = SqliteUserRepository::new(&db_path).expect("repo");
        let session_repo = SqliteSessionRepository::new(&db_path).expect("repo");

        runtime().block_on(async {
            let user = user_repo
                .create(&CoreNewUser {
                    username: "admin".to_string(),
                    password_hash: None,
                })
                .await
                .expect("create user");

            let session = session_repo
                .create(
                    &CoreNewSession {
                        user_id: user.id.clone(),
                        token: vec![0u8; 32],
                    },
                    "token_hash_abc123",
                )
                .await
                .expect("create session");

            let found = session_repo
                .find_by_token_hash("token_hash_abc123")
                .await
                .expect("find by token hash")
                .expect("session should exist");
            assert_eq!(found.id, session.id);
            assert_eq!(found.user_id, user.id);
            assert!(!found.revoked);
        });
    }

    #[test]
    fn session_repository_find_by_token_hash_not_found() {
        runtime().block_on(async {
            let repo = SqliteSessionRepository::new(":memory:").expect("repo");
            let result = repo
                .find_by_token_hash("nonexistent")
                .await
                .expect("find by token hash");
            assert!(result.is_none());
        });
    }

    #[test]
    fn session_repository_revoke() {
        let (_db_dir, db_path) = shared_test_db();
        let user_repo = SqliteUserRepository::new(&db_path).expect("repo");
        let session_repo = SqliteSessionRepository::new(&db_path).expect("repo");

        runtime().block_on(async {
            let user = user_repo
                .create(&CoreNewUser {
                    username: "admin".to_string(),
                    password_hash: None,
                })
                .await
                .expect("create user");

            let session = session_repo
                .create(
                    &CoreNewSession {
                        user_id: user.id.clone(),
                        token: vec![0u8; 32],
                    },
                    "token_hash_revoked",
                )
                .await
                .expect("create session");

            session_repo
                .revoke(&session.id)
                .await
                .expect("revoke session");

            let found = session_repo
                .find_by_token_hash("token_hash_revoked")
                .await
                .expect("find by token hash");
            assert!(found.is_none());
        });
    }

    #[test]
    fn session_repository_delete_expired() {
        let (_db_dir, db_path) = shared_test_db();
        let user_repo = SqliteUserRepository::new(&db_path).expect("repo");
        let session_repo = SqliteSessionRepository::new(&db_path).expect("repo");

        runtime().block_on(async {
            let user = user_repo
                .create(&CoreNewUser {
                    username: "admin".to_string(),
                    password_hash: None,
                })
                .await
                .expect("create user");

            // Create a session directly in DB with expired time
            {
                let conn = session_repo.lock().expect("lock");
                let past = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
                conn.execute(
                    "INSERT INTO sessions (id, token_hash, user_id, created_at, expires_at, revoked)
                     VALUES ('expired-session-id', 'expired_token_hash', ?1, ?2, ?3, 0)",
                    params![user.id.to_string(), past.clone(), past],
                )
                .expect("insert expired session");
            }

            let count = session_repo.delete_expired().await.expect("delete expired");
            assert_eq!(count, 1);

            // Verify the expired session is gone
            let found = session_repo
                .find_by_token_hash("expired_token_hash")
                .await
                .expect("find by token hash");
            assert!(found.is_none());
        });
    }

    #[test]
    fn create_api_key_with_allowed_models_and_providers_persists_correctly() {
        use rook_core::{ModelId, ProviderId};
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");

            let record = rook_core::ApiKeyRecord {
                id: ApiKeyId::new("key-restricted"),
                label: "Restricted Key".to_string(),
                key_hash: "hash-restricted".to_string(),
                key_prefix: "rk-rest".to_string(),
                scopes: vec![rook_core::ApiKeyScope::parse("chat:read").unwrap()],
                tier: ApiKeyTier::Free,
                is_active: true,
                revoked_at: None,
                expires_at: None,
                created_at: Utc::now(),
                last_used_at: None,
                allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("claude-3")],
                allowed_providers: vec![ProviderId::new("openai")],
                spending_budget: None,
            };
            repo.create(&record).await.expect("create");

            let found = repo
                .find(&ApiKeyId::new("key-restricted"))
                .await
                .expect("find")
                .expect("some");

            assert_eq!(found.allowed_models.len(), 2);
            assert_eq!(found.allowed_models[0].as_str(), "gpt-4");
            assert_eq!(found.allowed_models[1].as_str(), "claude-3");
            assert_eq!(found.allowed_providers.len(), 1);
            assert_eq!(found.allowed_providers[0].as_str(), "openai");
        });
    }

    #[test]
    fn update_api_key_restrictions_persists_correctly() {
        use rook_core::{ModelId, ProviderId};
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");

            // Start with empty restrictions
            let record = rook_core::ApiKeyRecord {
                id: ApiKeyId::new("key-update-test"),
                label: "Update Test".to_string(),
                key_hash: "hash-update".to_string(),
                key_prefix: "rk-upd".to_string(),
                scopes: vec![rook_core::ApiKeyScope::parse("chat:read").unwrap()],
                tier: ApiKeyTier::Free,
                is_active: true,
                revoked_at: None,
                expires_at: None,
                created_at: Utc::now(),
                last_used_at: None,
                allowed_models: vec![],
                allowed_providers: vec![],
                spending_budget: None,
            };
            repo.create(&record).await.expect("create");

            // Update with restrictions
            let mut updated = record.clone();
            updated.allowed_models = vec![ModelId::new("gpt-4o")];
            updated.allowed_providers = vec![ProviderId::new("anthropic")];
            repo.update(&updated).await.expect("update");

            let found = repo
                .find(&ApiKeyId::new("key-update-test"))
                .await
                .expect("find")
                .expect("some");
            assert_eq!(found.allowed_models.len(), 1);
            assert_eq!(found.allowed_models[0].as_str(), "gpt-4o");
            assert_eq!(found.allowed_providers[0].as_str(), "anthropic");

            // Clear restrictions
            let mut cleared = found;
            cleared.allowed_models = vec![];
            cleared.allowed_providers = vec![];
            repo.update(&cleared).await.expect("update clear");

            let refound = repo
                .find(&ApiKeyId::new("key-update-test"))
                .await
                .expect("find")
                .expect("some");
            assert!(refound.allowed_models.is_empty(), "should be cleared");
            assert!(refound.allowed_providers.is_empty(), "should be cleared");
        });
    }

    #[test]
    fn empty_restrictions_means_unrestricted() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");

            let record = rook_core::ApiKeyRecord {
                id: ApiKeyId::new("key-empty-restrictions"),
                label: "Unrestricted".to_string(),
                key_hash: "hash-unrestricted".to_string(),
                key_prefix: "rk-unr".to_string(),
                scopes: vec![rook_core::ApiKeyScope::parse("chat:write").unwrap()],
                tier: ApiKeyTier::Pro,
                is_active: true,
                revoked_at: None,
                expires_at: None,
                created_at: Utc::now(),
                last_used_at: None,
                allowed_models: vec![],
                allowed_providers: vec![],
                spending_budget: None,
            };
            repo.create(&record).await.expect("create");

            let found = repo
                .find(&ApiKeyId::new("key-empty-restrictions"))
                .await
                .expect("find")
                .expect("some");

            assert!(found.allowed_models.is_empty(), "empty = unrestricted");
            assert!(found.allowed_providers.is_empty(), "empty = unrestricted");
        });
    }

    // Regression: scopes_from_json used to call ApiKeyScope::parse (strict) which
    // rejected pre-#83 legacy scopes ('read', 'write') and broke reads of any
    // existing key. parse_lenient accepts unknown strings and logs a warning.
    #[test]
    fn read_key_with_legacy_scope_string_is_preserved() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            // Use the test helper and override scopes to a legacy 'read' value.
            let mut record = TestApiKeyRecord::active("legacy_key_id", "hash:legacy");
            record.label = "Legacy Key".to_string();
            record.scopes = vec![ApiKeyScope::parse_lenient("read")];
            repo.insert_test_key(record)
                .await
                .expect("insert legacy key");

            // Re-fetch — this exercises the scopes_from_json read path.
            let result = repo
                .find_active_by_hash("hash:legacy")
                .await
                .expect("query")
                .expect("legacy key must be readable");

            assert_eq!(result.label, "Legacy Key");
            assert_eq!(result.scopes.len(), 1);
            assert_eq!(result.scopes[0].as_str(), "read");
        });
    }

    // =============================================================================
    // rotate_hash tests — TDD: pin the storage-layer rotation contract.
    //
    // The whole point of rotation is that the OLD hash no longer matches a
    // row in api_keys, and the NEW hash now does. We assert both directions.
    // =============================================================================

    #[test]
    fn rotate_hash_replaces_hash_and_prefix() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let id = ApiKeyId::new("rotate-1");
            repo.insert_test_key(TestApiKeyRecord::active(id.as_str(), "hash-original"))
                .await
                .expect("insert");

            repo.rotate_hash(&id, "hash-rotated", "rk-rotat")
                .await
                .expect("rotate_hash");

            // 1. The OLD hash no longer authenticates.
            let old_lookup = repo
                .find_active_by_hash("hash-original")
                .await
                .expect("find old");
            assert!(
                old_lookup.is_none(),
                "old hash must NOT resolve to any active row after rotation"
            );

            // 2. The NEW hash authenticates and returns the same id.
            let new_lookup = repo
                .find_active_by_hash("hash-rotated")
                .await
                .expect("find new")
                .expect("new hash must resolve");
            assert_eq!(new_lookup.id, id, "rotated row must keep its id");

            // 3. The stored prefix was updated.
            let stored = repo.find(&id).await.expect("find").expect("some");
            assert_eq!(stored.key_hash, "hash-rotated");
            assert_eq!(stored.key_prefix, "rk-rotat");
        });
    }

    #[test]
    fn rotate_hash_returns_not_found_for_unknown_id() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let result = repo
                .rotate_hash(&ApiKeyId::new("does-not-exist"), "h", "p")
                .await;
            match result {
                Err(rook_core::ApiKeyRepositoryError::NotFound(id)) => {
                    assert_eq!(id, ApiKeyId::new("does-not-exist"));
                }
                other => panic!("expected NotFound, got {:?}", other),
            }
        });
    }

    #[test]
    fn rotate_hash_preserves_all_other_fields() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let id = ApiKeyId::new("rotate-preserve");
            let mut record = TestApiKeyRecord::active(id.as_str(), "hash-orig");
            record.label = "Preserved Label".to_string();
            record.tier = ApiKeyTier::Enterprise;
            record.expires_at = Some(Utc::now() + Duration::days(30));
            let original_created_at = record.created_at;
            repo.insert_test_key(record).await.expect("insert");

            // Manually set a last_used_at via the standard update path to
            // ensure it survives rotation. (No public last_used setter on
            // TestApiKeyRecord, so we update it through the public repo API.)
            repo.record_last_used(&id, Utc::now())
                .await
                .expect("record last used");

            repo.rotate_hash(&id, "hash-new", "rk-newpr")
                .await
                .expect("rotate");

            let stored = repo.find(&id).await.expect("find").expect("some");
            assert_eq!(stored.key_hash, "hash-new");
            assert_eq!(stored.key_prefix, "rk-newpr");
            assert_eq!(stored.label, "Preserved Label", "label preserved");
            assert_eq!(stored.tier, ApiKeyTier::Enterprise, "tier preserved");
            assert!(stored.expires_at.is_some(), "expires_at preserved (some)");
            assert_eq!(
                stored.created_at, original_created_at,
                "created_at MUST NOT change on rotation"
            );
            assert!(
                stored.last_used_at.is_some(),
                "last_used_at preserved through rotation"
            );
            assert!(stored.is_active, "key remains active");
            assert!(stored.revoked_at.is_none(), "key remains unrevoked");
        });
    }
}
