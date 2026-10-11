// audit-sqlite — SQLite-backed implementation of AuditPort

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, AuditEntry, AuditPort, CostBreakdown, Pagination, ProviderKind, QuotaObservation,
    QuotaObservationRecorderPort, RequestStatus, UsageEntry, UsageFilters, UsageRecorderPort,
    UsageSummary,
};
use rusqlite::{params, Connection};
use shared_kernel::{ConnectionId, CortexError, CortexResult, ModelId, ProviderId, RequestId};
use std::collections::HashMap;
use std::path::Path;
use tokio::sync::Mutex;

/// SQLite-backed audit log.
///
/// Schema:
///   CREATE TABLE audit (
///     id          INTEGER PRIMARY KEY AUTOINCREMENT,
///     request_id  TEXT NOT NULL,
///     provider    TEXT NOT NULL,
///     model       TEXT NOT NULL,
///     status      TEXT NOT NULL,
///     prompt_tokens      INTEGER,
///     completion_tokens  INTEGER,
///     total_tokens       INTEGER,
///     estimated_cost_usd REAL,
///     latency_ms  INTEGER NOT NULL,
///     timestamp   TEXT NOT NULL
///   );
pub struct SqliteAudit {
    conn: Mutex<Connection>,
}

impl SqliteAudit {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS audit (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id  TEXT NOT NULL,
                provider    TEXT NOT NULL,
                model       TEXT NOT NULL,
                status      TEXT NOT NULL,
                prompt_tokens      INTEGER,
                completion_tokens  INTEGER,
                total_tokens       INTEGER,
                estimated_cost_usd REAL,
                latency_ms  INTEGER NOT NULL,
                timestamp   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_request_id ON audit(request_id);
            CREATE INDEX IF NOT EXISTS idx_audit_provider ON audit(provider);
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit(timestamp);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

#[async_trait]
impl AuditPort for SqliteAudit {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
        let status_str = match entry.status {
            RequestStatus::Success => "success",
            RequestStatus::Failure => "failure",
            RequestStatus::RateLimited => "rate_limited",
            RequestStatus::Timeout => "timeout",
        };

        let (prompt_tokens, completion_tokens, total_tokens, estimated_cost) = entry
            .usage
            .map(|u| {
                (
                    u.prompt_tokens,
                    u.completion_tokens,
                    u.total_tokens,
                    u.estimated_cost_usd,
                )
            })
            .unwrap_or((0, 0, 0, None));

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO audit
             (request_id, provider, model, status,
              prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd,
              latency_ms, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.request_id.to_string(),
                entry.provider.to_string(),
                entry.model.to_string(),
                status_str,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                estimated_cost,
                entry.latency_ms as i64,
                entry.timestamp.to_rfc3339(),
            ],
        )
        .map_err(|e| CortexError::provider(format!("sqlite insert failed: {e}")))?;

        Ok(())
    }
}

/// SQLite-backed usage history repository.
pub struct SqliteUsageRepository {
    conn: Mutex<Connection>,
}

impl SqliteUsageRepository {
    pub fn new(db_path: &Path) -> CortexResult<Self> {
        let conn = Connection::open(db_path)
            .map_err(|e| CortexError::provider(format!("sqlite open failed: {e}")))?;
        // Schema (usage_history table and indexes) is managed by db-migration/V2__usage_history.sql.
        // Calling code must run run_startup_migrations() before using the repository.
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| CortexError::provider(format!("sqlite pragma setup failed: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub async fn delete_older_than(&self, retention_days: u32) -> CortexResult<u64> {
        let conn = self.conn.lock().await;
        let mut total_deleted: u64 = 0;

        loop {
            let cutoff = format!("datetime('now', '-' || {} || ' days')", retention_days);
            let batch: Vec<i64> = conn
                .prepare(&format!(
                    "SELECT rowid FROM usage_history WHERE timestamp < {} LIMIT 1000",
                    cutoff
                ))
                .map_err(|e| CortexError::provider(format!("sqlite prepare failed: {e}")))?
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|e| CortexError::provider(format!("sqlite select failed: {e}")))?
                .filter_map(|row| row.ok())
                .collect();

            if batch.is_empty() {
                break;
            }

            let tx = conn
                .unchecked_transaction()
                .map_err(|e| CortexError::provider(format!("sqlite transaction failed: {e}")))?;
            let placeholders = batch.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let deleted = tx
                .execute(
                    &format!("DELETE FROM usage_history WHERE rowid IN ({placeholders})",),
                    rusqlite::params_from_iter(batch.iter()),
                )
                .map_err(|e| CortexError::provider(format!("sqlite delete failed: {e}")))?;
            tx.commit()
                .map_err(|e| CortexError::provider(format!("sqlite commit failed: {e}")))?;

            total_deleted += deleted as u64;

            if deleted < 1000 {
                break;
            }
        }

        Ok(total_deleted)
    }

    /// Delete only expired upstream quota observations; local usage is untouched.
    pub async fn delete_quota_observations_older_than(
        &self,
        retention_days: u32,
    ) -> CortexResult<u64> {
        let conn = self.conn.lock().await;
        let deleted = conn
            .execute(
                "DELETE FROM provider_quota_observations
                 WHERE datetime(observed_at) < datetime('now', '-' || ?1 || ' days')",
                [retention_days],
            )
            .map_err(|e| CortexError::provider(format!("sqlite quota retention failed: {e}")))?;
        Ok(deleted as u64)
    }
}

#[async_trait]
impl QuotaObservationRecorderPort for SqliteUsageRepository {
    async fn record_observation(&self, observation: QuotaObservation) -> CortexResult<()> {
        let reset_after_milliseconds = observation
            .reset_after_milliseconds
            .map(i64::try_from)
            .transpose()
            .map_err(|e| CortexError::provider(format!("invalid quota reset duration: {e}")))?;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO provider_quota_observations (
                provider_kind, provider_runtime_id, connection_id, metric, source,
                scope, window, quota_limit, remaining, reset_at,
                reset_after_milliseconds, observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                observation.provider_kind.as_str(),
                observation.provider.to_string(),
                observation.connection_id.map(|id| id.to_string()),
                observation.metric,
                observation.source,
                observation.scope,
                observation.window,
                observation.limit,
                observation.remaining,
                observation.reset_at.map(|value| value.to_rfc3339()),
                reset_after_milliseconds,
                observation.observed_at.to_rfc3339(),
            ],
        )
        .map_err(|e| CortexError::provider(format!("sqlite quota insert failed: {e}")))?;
        Ok(())
    }

    async fn list_observations(
        &self,
        provider: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT provider_kind, provider_runtime_id, connection_id, metric, source,
                        scope, window, quota_limit, remaining, reset_at,
                        reset_after_milliseconds, observed_at
                 FROM provider_quota_observations
                 WHERE (?1 IS NULL OR provider_runtime_id = ?1)
                 ORDER BY observed_at DESC, id DESC",
            )
            .map_err(|e| CortexError::provider(format!("sqlite quota list prepare failed: {e}")))?;
        let rows = stmt
            .query_map([provider.map(|id| id.to_string())], |row| {
                let provider_kind: String = row.get(0)?;
                let provider_kind =
                    ProviderKind::try_from(provider_kind.as_str()).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            0,
                            "provider_kind".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;
                let connection_id: Option<String> = row.get(2)?;
                let connection_id = connection_id
                    .map(|id| {
                        ConnectionId::parse_str(&id).map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                2,
                                "connection_id".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })
                    })
                    .transpose()?;
                let reset_at: Option<String> = row.get(9)?;
                let reset_at = reset_at
                    .map(|value| {
                        DateTime::parse_from_rfc3339(&value)
                            .map(|date| date.with_timezone(&Utc))
                            .map_err(|_| {
                                rusqlite::Error::InvalidColumnType(
                                    9,
                                    "reset_at".to_string(),
                                    rusqlite::types::Type::Text,
                                )
                            })
                    })
                    .transpose()?;
                let observed_at: String = row.get(11)?;
                let observed_at = DateTime::parse_from_rfc3339(&observed_at)
                    .map(|date| date.with_timezone(&Utc))
                    .map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            11,
                            "observed_at".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;
                let reset_after_milliseconds: Option<i64> = row.get(10)?;
                Ok(QuotaObservation {
                    provider_kind,
                    provider: ProviderId::new(row.get::<_, String>(1)?),
                    connection_id,
                    metric: row.get(3)?,
                    source: row.get(4)?,
                    scope: row.get(5)?,
                    window: row.get(6)?,
                    limit: row.get(7)?,
                    remaining: row.get(8)?,
                    reset_at,
                    reset_after_milliseconds: reset_after_milliseconds
                        .map(|value| {
                            u64::try_from(value).map_err(|_| {
                                rusqlite::Error::InvalidColumnType(
                                    10,
                                    "reset_after_milliseconds".to_string(),
                                    rusqlite::types::Type::Integer,
                                )
                            })
                        })
                        .transpose()?,
                    observed_at,
                })
            })
            .map_err(|e| CortexError::provider(format!("sqlite quota list failed: {e}")))?;
        rows.map(|row| {
            row.map_err(|e| CortexError::provider(format!("sqlite quota row failed: {e}")))
        })
        .collect()
    }
}

#[async_trait]
impl UsageRecorderPort for SqliteUsageRepository {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO usage_history (
                request_id, provider, model, status, requested_tier, api_key_id, connection_id,
                tokens_prompt, tokens_completion, tokens_cache_read, tokens_cache_creation,
                tokens_reasoning, ttft_ms, latency_ms, cost_usd, timestamp
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                entry.request_id.to_string(),
                entry.provider.to_string(),
                entry.model.to_string(),
                status_to_str(entry.status),
                entry.requested_tier,
                entry.api_key_id.map(|id| id.to_string()),
                entry.connection_id.map(|id| id.to_string()),
                entry.tokens_prompt.map(|value| value as i64),
                entry.tokens_completion.map(|value| value as i64),
                entry.tokens_cache_read.map(|value| value as i64),
                entry.tokens_cache_creation.map(|value| value as i64),
                entry.tokens_reasoning.map(|value| value as i64),
                entry.ttft_ms.map(|value| value as i64),
                entry.latency_ms as i64,
                entry.cost_usd,
                entry.timestamp.to_rfc3339(),
            ],
        )
        .map_err(|e| CortexError::provider(format!("sqlite usage insert failed: {e}")))?;
        Ok(())
    }

    async fn list(
        &self,
        filters: UsageFilters,
        pagination: Pagination,
    ) -> CortexResult<Vec<UsageEntry>> {
        let pagination = pagination.clamped();
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT request_id, provider, model, status, requested_tier, api_key_id, connection_id,
                        tokens_prompt, tokens_completion, tokens_cache_read, tokens_cache_creation,
                        tokens_reasoning, ttft_ms, latency_ms, cost_usd, timestamp
                 FROM usage_history
                 {}
                 ORDER BY timestamp DESC
                 LIMIT ?8 OFFSET ?9",
                usage_filter_where_sql()
            ))
            .map_err(|e| CortexError::provider(format!("sqlite usage list prepare failed: {e}")))?;
        let params = filter_params(&filters);
        let rows = stmt
            .query_map(
                params![
                    params.provider,
                    params.model,
                    params.api_key_id,
                    params.connection_id,
                    params.start,
                    params.end,
                    params.status,
                    pagination.limit as i64,
                    pagination.offset as i64,
                ],
                row_to_usage_entry,
            )
            .map_err(|e| CortexError::provider(format!("sqlite usage list failed: {e}")))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(
                row.map_err(|e| CortexError::provider(format!("sqlite usage row failed: {e}")))?,
            );
        }
        Ok(entries)
    }

    async fn count(&self, filters: UsageFilters) -> CortexResult<u64> {
        let conn = self.conn.lock().await;
        let params = filter_params(&filters);
        let count: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM usage_history {}",
                    usage_filter_where_sql()
                ),
                params![
                    params.provider,
                    params.model,
                    params.api_key_id,
                    params.connection_id,
                    params.start,
                    params.end,
                    params.status,
                ],
                |row| row.get(0),
            )
            .map_err(|e| CortexError::provider(format!("sqlite usage count failed: {e}")))?;
        Ok(count as u64)
    }

    async fn summary(&self, filters: UsageFilters) -> CortexResult<UsageSummary> {
        let conn = self.conn.lock().await;
        let params = filter_params(&filters);
        conn.query_row(
            &format!(
                "SELECT COUNT(*) AS total_requests,
                        COALESCE(SUM(tokens_prompt), 0) AS total_prompt_tokens,
                        COALESCE(SUM(tokens_completion), 0) AS total_completion_tokens,
                        COALESCE(SUM(tokens_cache_read), 0) AS total_cache_read_tokens,
                        COALESCE(SUM(tokens_cache_creation), 0) AS total_cache_creation_tokens,
                        COALESCE(SUM(tokens_reasoning), 0) AS total_reasoning_tokens,
                        AVG(ttft_ms) AS avg_ttft_ms,
                        COALESCE(AVG(latency_ms), 0) AS avg_latency_ms
                 FROM usage_history {}",
                usage_filter_where_sql()
            ),
            params![
                params.provider,
                params.model,
                params.api_key_id,
                params.connection_id,
                params.start,
                params.end,
                params.status,
            ],
            |row| {
                Ok(UsageSummary {
                    total_requests: row.get::<_, i64>(0)? as u64,
                    total_prompt_tokens: row.get::<_, i64>(1)? as u64,
                    total_completion_tokens: row.get::<_, i64>(2)? as u64,
                    total_cache_read_tokens: row.get::<_, i64>(3)? as u64,
                    total_cache_creation_tokens: row.get::<_, i64>(4)? as u64,
                    total_reasoning_tokens: row.get::<_, i64>(5)? as u64,
                    avg_ttft_ms: row.get(6)?,
                    avg_latency_ms: row.get(7)?,
                })
            },
        )
        .map_err(|e| CortexError::provider(format!("sqlite usage summary failed: {e}")))
    }

    async fn cost_breakdown(&self, filters: UsageFilters) -> CortexResult<CostBreakdown> {
        let total_cost_usd = query_total_cost(&self.conn, &filters).await?;
        let by_provider =
            query_cost_group(&self.conn, &filters, "provider", ProviderId::new).await?;
        let by_model = query_cost_group(&self.conn, &filters, "model", ModelId::new).await?;
        let by_api_key =
            query_cost_group(&self.conn, &filters, "api_key_id", ApiKeyId::new).await?;
        Ok(CostBreakdown {
            total_cost_usd,
            by_provider,
            by_model,
            by_api_key,
        })
    }
}

struct FilterParams {
    provider: Option<String>,
    model: Option<String>,
    api_key_id: Option<String>,
    connection_id: Option<String>,
    start: Option<String>,
    end: Option<String>,
    status: Option<&'static str>,
}

fn filter_params(filters: &UsageFilters) -> FilterParams {
    FilterParams {
        provider: filters.provider.as_ref().map(ToString::to_string),
        model: filters.model.as_ref().map(ToString::to_string),
        api_key_id: filters.api_key_id.as_ref().map(ToString::to_string),
        connection_id: filters.connection_id.as_ref().map(ToString::to_string),
        start: filters.start.map(|dt| dt.to_rfc3339()),
        end: filters.end.map(|dt| dt.to_rfc3339()),
        status: filters.status.map(status_to_str),
    }
}

fn usage_filter_where_sql() -> &'static str {
    "WHERE (?1 IS NULL OR provider = ?1)
       AND (?2 IS NULL OR model = ?2)
       AND (?3 IS NULL OR api_key_id = ?3)
       AND (?4 IS NULL OR connection_id = ?4)
       AND (?5 IS NULL OR timestamp >= ?5)
       AND (?6 IS NULL OR timestamp <= ?6)
       AND (?7 IS NULL OR status = ?7)"
}

fn status_to_str(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Success => "success",
        RequestStatus::Failure => "failure",
        RequestStatus::RateLimited => "rate_limited",
        RequestStatus::Timeout => "timeout",
    }
}

fn status_from_str(status: &str) -> rusqlite::Result<RequestStatus> {
    match status {
        "success" => Ok(RequestStatus::Success),
        "failure" => Ok(RequestStatus::Failure),
        "rate_limited" => Ok(RequestStatus::RateLimited),
        "timeout" => Ok(RequestStatus::Timeout),
        _ => Err(rusqlite::Error::InvalidParameterName(format!(
            "invalid usage status: {status}"
        ))),
    }
}

fn optional_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)
        .map(|value| value.map(|value| value as u64))
}

fn row_to_usage_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageEntry> {
    let request_id =
        RequestId(row.get::<_, String>(0)?.parse().map_err(|_| {
            rusqlite::Error::InvalidParameterName("invalid request_id".to_string())
        })?);
    let connection_id = row
        .get::<_, Option<String>>(6)?
        .map(|id| {
            ConnectionId::parse_str(&id).map_err(|_| {
                rusqlite::Error::InvalidParameterName("invalid connection_id".to_string())
            })
        })
        .transpose()?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(15)?)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|_| rusqlite::Error::InvalidParameterName("invalid timestamp".to_string()))?;

    Ok(UsageEntry {
        request_id,
        provider: ProviderId::new(row.get::<_, String>(1)?),
        model: ModelId::new(row.get::<_, String>(2)?),
        status: status_from_str(&row.get::<_, String>(3)?)?,
        requested_tier: row.get(4)?,
        api_key_id: row.get::<_, Option<String>>(5)?.map(ApiKeyId::new),
        connection_id,
        tokens_prompt: optional_u64(row, 7)?,
        tokens_completion: optional_u64(row, 8)?,
        tokens_cache_read: optional_u64(row, 9)?,
        tokens_cache_creation: optional_u64(row, 10)?,
        tokens_reasoning: optional_u64(row, 11)?,
        ttft_ms: optional_u64(row, 12)?,
        latency_ms: row.get::<_, i64>(13)? as u64,
        cost_usd: row.get(14)?,
        timestamp,
    })
}

async fn query_total_cost(conn: &Mutex<Connection>, filters: &UsageFilters) -> CortexResult<f64> {
    let conn = conn.lock().await;
    let params = filter_params(filters);
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM usage_history {}",
            usage_filter_where_sql()
        ),
        params![
            params.provider,
            params.model,
            params.api_key_id,
            params.connection_id,
            params.start,
            params.end,
            params.status,
        ],
        |row| row.get(0),
    )
    .map_err(|e| CortexError::provider(format!("sqlite usage cost total failed: {e}")))
}

async fn query_cost_group<K, F>(
    conn: &Mutex<Connection>,
    filters: &UsageFilters,
    column: &str,
    key: F,
) -> CortexResult<HashMap<K, f64>>
where
    K: Eq + std::hash::Hash,
    F: Fn(String) -> K,
{
    let sql = format!(
        "SELECT {column}, COALESCE(SUM(cost_usd), 0) FROM usage_history {}
         AND {column} IS NOT NULL GROUP BY {column}",
        usage_filter_where_sql()
    );
    let conn = conn.lock().await;
    let params = filter_params(filters);
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        CortexError::provider(format!("sqlite usage cost group prepare failed: {e}"))
    })?;
    let rows = stmt
        .query_map(
            params![
                params.provider,
                params.model,
                params.api_key_id,
                params.connection_id,
                params.start,
                params.end,
                params.status,
            ],
            |row| Ok((key(row.get::<_, String>(0)?), row.get::<_, f64>(1)?)),
        )
        .map_err(|e| CortexError::provider(format!("sqlite usage cost group failed: {e}")))?;

    let mut groups = HashMap::new();
    for row in rows {
        let (group_key, cost) =
            row.map_err(|e| CortexError::provider(format!("sqlite usage cost row failed: {e}")))?;
        groups.insert(group_key, cost);
    }
    Ok(groups)
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    fn usage_repo() -> SqliteUsageRepository {
        static TEST_DB_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = TEST_DB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("audit_sqlite_test_{}.db", n));
        // Remove any stale file from a previous run — ignore errors if it doesn't exist
        let _ = std::fs::remove_file(&db_path);
        let db_path_str = db_path.to_string_lossy().to_string();
        db_migration::run_migrations(&db_path_str).expect("migrations");
        let repo = SqliteUsageRepository::new(&db_path).expect("usage repo");
        // Leak the path so the file persists for the duration of the test process.
        // This avoids the NamedTempFile auto-delete-on-drop issue.
        Box::leak(db_path_str.into_boxed_str());
        repo
    }

    fn timestamp(day: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, day, 12, 0, 0)
            .single()
            .expect("timestamp")
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON * 10.0);
    }

    fn entry(provider: &str, model: &str, day: u32, cost_usd: Option<f64>) -> UsageEntry {
        UsageEntry {
            request_id: RequestId::new(),
            provider: ProviderId::new(provider),
            model: ModelId::new(model),
            status: RequestStatus::Success,
            requested_tier: Some("premium".to_string()),
            api_key_id: Some(ApiKeyId::new("key_123")),
            connection_id: Some(ConnectionId::new()),
            tokens_prompt: Some(100),
            tokens_completion: Some(50),
            tokens_cache_read: Some(25),
            tokens_cache_creation: Some(10),
            tokens_reasoning: Some(5),
            ttft_ms: Some(150),
            latency_ms: 400,
            cost_usd,
            timestamp: timestamp(day),
        }
    }

    #[test]
    fn usage_record_and_list_round_trips_complete_records() {
        runtime().block_on(async {
            let repo = usage_repo();
            let saved = entry("openai", "gpt-4o", 1, Some(0.0123));
            let request_id = saved.request_id.clone();
            let connection_id = saved.connection_id;

            repo.record(saved).await.expect("record");

            let rows = repo
                .list(UsageFilters::default(), Pagination::default())
                .await
                .expect("list");
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.request_id, request_id);
            assert_eq!(row.provider, ProviderId::new("openai"));
            assert_eq!(row.model, ModelId::new("gpt-4o"));
            assert_eq!(row.status, RequestStatus::Success);
            assert_eq!(row.requested_tier.as_deref(), Some("premium"));
            assert_eq!(
                row.api_key_id.as_ref().map(ApiKeyId::as_str),
                Some("key_123")
            );
            assert_eq!(row.connection_id, connection_id);
            assert_eq!(row.tokens_prompt, Some(100));
            assert_eq!(row.tokens_completion, Some(50));
            assert_eq!(row.tokens_cache_read, Some(25));
            assert_eq!(row.tokens_cache_creation, Some(10));
            assert_eq!(row.tokens_reasoning, Some(5));
            assert_eq!(row.ttft_ms, Some(150));
            assert_eq!(row.latency_ms, 400);
            assert_eq!(row.cost_usd, Some(0.0123));
            assert_eq!(row.timestamp, timestamp(1));
        });
    }

    #[test]
    fn usage_record_preserves_null_optional_token_fields() {
        runtime().block_on(async {
            let repo = usage_repo();
            let mut saved = entry("ollama", "llama3", 1, Some(0.001));
            saved.requested_tier = None;
            saved.api_key_id = None;
            saved.connection_id = None;
            saved.tokens_cache_read = None;
            saved.tokens_cache_creation = None;
            saved.tokens_reasoning = None;
            saved.ttft_ms = None;

            repo.record(saved).await.expect("record");
            let rows = repo
                .list(UsageFilters::default(), Pagination::default())
                .await
                .expect("list");

            assert_eq!(rows[0].requested_tier, None);
            assert_eq!(rows[0].api_key_id, None);
            assert_eq!(rows[0].connection_id, None);
            assert_eq!(rows[0].tokens_cache_read, None);
            assert_eq!(rows[0].tokens_cache_creation, None);
            assert_eq!(rows[0].tokens_reasoning, None);
            assert_eq!(rows[0].ttft_ms, None);
        });
    }

    #[test]
    fn usage_list_orders_by_timestamp_desc_and_paginates() {
        runtime().block_on(async {
            let repo = usage_repo();
            for day in 1..=5 {
                repo.record(entry("openai", "gpt-4o", day, Some(day as f64)))
                    .await
                    .expect("record");
            }

            let rows = repo
                .list(
                    UsageFilters::default(),
                    Pagination {
                        offset: 1,
                        limit: 2,
                    },
                )
                .await
                .expect("list");

            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].timestamp, timestamp(4));
            assert_eq!(rows[1].timestamp, timestamp(3));
        });
    }

    #[test]
    fn usage_count_and_summary_apply_filters() {
        runtime().block_on(async {
            let repo = usage_repo();
            let mut openai = entry("openai", "gpt-4o", 1, Some(0.10));
            openai.tokens_prompt = Some(200);
            openai.tokens_completion = Some(80);
            openai.tokens_cache_read = Some(30);
            openai.tokens_cache_creation = Some(20);
            openai.tokens_reasoning = Some(10);
            openai.ttft_ms = Some(100);
            openai.latency_ms = 300;
            repo.record(openai).await.expect("openai 1");

            let mut second = entry("openai", "gpt-4o-mini", 2, Some(0.20));
            second.tokens_prompt = Some(300);
            second.tokens_completion = Some(120);
            second.ttft_ms = Some(200);
            second.latency_ms = 500;
            repo.record(second).await.expect("openai 2");

            repo.record(entry("anthropic", "claude", 3, Some(0.30)))
                .await
                .expect("anthropic");

            let filters = UsageFilters {
                provider: Some(ProviderId::new("openai")),
                start: Some(timestamp(1)),
                end: Some(timestamp(2)),
                ..UsageFilters::default()
            };

            assert_eq!(repo.count(filters.clone()).await.expect("count"), 2);
            let UsageSummary {
                total_requests,
                total_prompt_tokens,
                total_completion_tokens,
                total_cache_read_tokens,
                total_cache_creation_tokens,
                total_reasoning_tokens,
                avg_ttft_ms,
                avg_latency_ms,
            } = repo.summary(filters).await.expect("summary");

            assert_eq!(total_requests, 2);
            assert_eq!(total_prompt_tokens, 500);
            assert_eq!(total_completion_tokens, 200);
            assert_eq!(total_cache_read_tokens, 55);
            assert_eq!(total_cache_creation_tokens, 30);
            assert_eq!(total_reasoning_tokens, 15);
            assert_eq!(avg_ttft_ms, Some(150.0));
            assert_eq!(avg_latency_ms, 400.0);
        });
    }

    #[test]
    fn usage_cost_breakdown_groups_by_provider_model_and_api_key() {
        runtime().block_on(async {
            let repo = usage_repo();
            let mut first = entry("openai", "gpt-4o", 1, Some(0.10));
            first.api_key_id = Some(ApiKeyId::new("key_a"));
            repo.record(first).await.expect("first");

            let mut second = entry("openai", "gpt-4o", 2, Some(0.20));
            second.api_key_id = Some(ApiKeyId::new("key_a"));
            repo.record(second).await.expect("second");

            let mut third = entry("anthropic", "claude", 3, Some(0.30));
            third.api_key_id = Some(ApiKeyId::new("key_b"));
            repo.record(third).await.expect("third");

            let mut unknown_cost = entry("openai", "gpt-4o-mini", 4, None);
            unknown_cost.api_key_id = None;
            repo.record(unknown_cost).await.expect("unknown cost");

            let CostBreakdown {
                total_cost_usd,
                by_provider,
                by_model,
                by_api_key,
            } = repo
                .cost_breakdown(UsageFilters::default())
                .await
                .expect("cost");

            assert_eq!(total_cost_usd, 0.60);
            assert_float_eq(
                *by_provider.get(&ProviderId::new("openai")).expect("openai"),
                0.30,
            );
            assert_float_eq(
                *by_provider
                    .get(&ProviderId::new("anthropic"))
                    .expect("anthropic"),
                0.30,
            );
            assert_float_eq(
                *by_model.get(&ModelId::new("gpt-4o")).expect("gpt-4o"),
                0.30,
            );
            assert_float_eq(
                *by_model.get(&ModelId::new("claude")).expect("claude"),
                0.30,
            );
            assert_float_eq(
                *by_api_key.get(&ApiKeyId::new("key_a")).expect("key_a"),
                0.30,
            );
            assert_float_eq(
                *by_api_key.get(&ApiKeyId::new("key_b")).expect("key_b"),
                0.30,
            );
            assert!(!by_api_key.contains_key(&ApiKeyId::new("")));
        });
    }

    #[test]
    fn usage_delete_older_than_removes_expired_rows() {
        runtime().block_on(async {
            let repo = usage_repo();
            let mut expired = entry("openai", "gpt-4o", 1, Some(0.10));
            expired.timestamp = Utc::now() - Duration::days(120);
            repo.record(expired).await.expect("expired");

            let mut retained = entry("openai", "gpt-4o", 2, Some(0.20));
            retained.timestamp = Utc::now() - Duration::days(30);
            repo.record(retained).await.expect("retained");

            let deleted = repo.delete_older_than(90).await.expect("delete");
            assert_eq!(deleted, 1);
            assert_eq!(repo.count(UsageFilters::default()).await.expect("count"), 1);
        });
    }

    fn quota_observation(provider: &str, connection_id: Option<ConnectionId>) -> QuotaObservation {
        QuotaObservation {
            provider_kind: ProviderKind::OpenAI,
            provider: ProviderId::new(provider),
            connection_id,
            metric: "requests".to_string(),
            source: "x-ratelimit-remaining-requests".to_string(),
            scope: Some("project".to_string()),
            window: Some("1m".to_string()),
            limit: Some(100.0),
            remaining: Some(73.0),
            reset_at: Some(timestamp(3)),
            // 7.66 seconds as reported by fractional-duration headers such as Groq's.
            reset_after_milliseconds: Some(7_660),
            observed_at: timestamp(2),
        }
    }

    #[test]
    fn quota_observation_round_trips_values_nulls_and_runtime_identity() {
        runtime().block_on(async {
            let repo = usage_repo();
            let connection = ConnectionId::new();
            let mut expected = quota_observation("openai-runtime-a", Some(connection));
            expected.scope = None;
            expected.remaining = None;
            repo.record_observation(expected.clone())
                .await
                .expect("record quota");
            repo.record_observation(quota_observation("openai-runtime-b", None))
                .await
                .expect("record second runtime");

            let rows = repo.list_observations(None).await.expect("list quota");
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1], expected);
            assert_eq!(rows[1].provider, ProviderId::new("openai-runtime-a"));
            assert_eq!(rows[1].connection_id, Some(connection));
            assert_eq!(rows[1].scope, None);
            assert_eq!(rows[1].remaining, None);
            assert_eq!(rows[1].reset_at, Some(timestamp(3)));
            assert_eq!(rows[1].reset_after_milliseconds, Some(7_660));

            let filtered = repo
                .list_observations(Some(ProviderId::new("openai-runtime-b")))
                .await
                .expect("filter runtime");
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].provider, ProviderId::new("openai-runtime-b"));
        });
    }

    #[test]
    fn v6_migration_adds_isolated_quota_storage() {
        runtime().block_on(async {
            let repo = usage_repo();
            let conn = repo.conn.lock().await;
            let table_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table' AND name = 'provider_quota_observations'",
                    [],
                    |row| row.get(0),
                )
                .expect("quota table exists");
            let usage_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table' AND name = 'usage_history'",
                    [],
                    |row| row.get(0),
                )
                .expect("usage table exists");
            assert_eq!(table_count, 1);
            assert_eq!(usage_count, 1);
        });
    }

    #[test]
    fn quota_retention_deletes_only_expired_quota_rows() {
        runtime().block_on(async {
            let repo = usage_repo();
            repo.record_observation(quota_observation("openai", None))
                .await
                .expect("old quota");
            let mut recent_quota = quota_observation("openai", None);
            recent_quota.observed_at = Utc::now();
            repo.record_observation(recent_quota)
                .await
                .expect("recent quota");

            let mut old_usage = entry("openai", "gpt-4o", 1, Some(0.1));
            old_usage.timestamp = Utc::now() - Duration::days(120);
            repo.record(old_usage).await.expect("old usage");

            assert_eq!(
                repo.delete_quota_observations_older_than(90).await.unwrap(),
                1
            );
            assert_eq!(repo.list_observations(None).await.unwrap().len(), 1);
            assert_eq!(repo.count(UsageFilters::default()).await.unwrap(), 1);
        });
    }
}
