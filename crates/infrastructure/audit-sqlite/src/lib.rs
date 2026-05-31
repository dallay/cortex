// audit-sqlite — SQLite-backed implementation of AuditPort

use async_trait::async_trait;
use rook_core::{AuditEntry, AuditPort, RequestStatus};
use rusqlite::{params, Connection};
use shared_kernel::{CortexError, CortexResult};
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
            "CREATE TABLE IF NOT EXISTS audit (
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
