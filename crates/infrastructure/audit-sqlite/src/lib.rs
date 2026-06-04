// audit-sqlite — SQLite-backed implementation of AuditPort

use async_trait::async_trait;
use rook_core::{
    ApiKeyId, AuditEntry, AuditPort, CostBreakdown, Pagination, RequestStatus, UsageEntry,
    UsageFilters, UsageRecorderPort, UsageSummary,
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
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS usage_history (
                id                      INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id              TEXT NOT NULL,
                provider                TEXT NOT NULL,
                model                   TEXT NOT NULL,
                status                  TEXT NOT NULL,
                requested_tier          TEXT,
                api_key_id              TEXT,
                connection_id           TEXT,
                tokens_prompt           INTEGER,
                tokens_completion       INTEGER,
                tokens_cache_read       INTEGER,
                tokens_cache_creation   INTEGER,
                tokens_reasoning        INTEGER,
                ttft_ms                 INTEGER,
                latency_ms              INTEGER NOT NULL,
                cost_usd                REAL,
                timestamp               TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_history_request_id ON usage_history(request_id);
            CREATE INDEX IF NOT EXISTS idx_usage_history_provider ON usage_history(provider);
            CREATE INDEX IF NOT EXISTS idx_usage_history_model ON usage_history(model);
            CREATE INDEX IF NOT EXISTS idx_usage_history_timestamp ON usage_history(timestamp);
            CREATE INDEX IF NOT EXISTS idx_usage_history_api_key_id ON usage_history(api_key_id);
            CREATE INDEX IF NOT EXISTS idx_usage_history_connection_id ON usage_history(connection_id);",
        )
        .map_err(|e| CortexError::provider(format!("sqlite schema init failed: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub async fn delete_older_than(&self, retention_days: u32) -> CortexResult<u64> {
        let conn = self.conn.lock().await;
        let rows = conn
            .execute(
                "DELETE FROM usage_history WHERE timestamp < datetime('now', '-' || ?1 || ' days')",
                params![retention_days],
            )
            .map_err(|e| CortexError::provider(format!("sqlite delete failed: {e}")))?;
        Ok(rows as u64)
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
        SqliteUsageRepository::new(Path::new(":memory:")).expect("usage repo")
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
}
