use std::path::{Path, PathBuf};

use audit_sqlite::SqliteAudit;
use chrono::{DateTime, Utc};
use rook_core::{AuditEntry, AuditPort, ModelId, RequestStatus, TokenUsage};
use rusqlite::Connection;
use shared_kernel::{ComboId, ConnectionId, ProviderId, RequestId};

const CONNECTION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
}

fn audit_store() -> (tempfile::TempDir, PathBuf, SqliteAudit) {
    let directory = tempfile::tempdir().expect("temporary test directory");
    let db_path = directory.path().join("audit.db");
    let store = SqliteAudit::new(&db_path).expect("audit store");
    (directory, db_path, store)
}

fn fixed_timestamp() -> DateTime<Utc> {
    "2026-06-01T11:00:00+00:00"
        .parse::<DateTime<Utc>>()
        .expect("fixed timestamp")
}

fn entry(status: RequestStatus, usage: Option<TokenUsage>) -> AuditEntry {
    AuditEntry {
        request_id: RequestId::new(),
        provider: ProviderId::new("openai-primary"),
        model: ModelId::new("gpt-4o"),
        status,
        usage,
        latency_ms: 42,
        timestamp: fixed_timestamp(),
        combo_id: None,
        combo_step_index: None,
        connection_id: None,
        provider_runtime_id: None,
    }
}

fn token_usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 11,
        completion_tokens: 7,
        total_tokens: 18,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        reasoning_tokens: None,
        estimated_cost_usd: Some(0.25),
    }
}

fn stored_row(db_path: &Path) -> rusqlite::Result<(String, i64, i64, i64, Option<f64>)> {
    let conn = Connection::open(db_path)?;
    conn.query_row(
        "SELECT status, prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd
             FROM audit ORDER BY id LIMIT 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        },
    )
}

#[test]
fn record_persists_token_usage_columns() {
    runtime().block_on(async {
        let (_directory, db_path, store) = audit_store();
        store
            .record(entry(RequestStatus::Success, Some(token_usage())))
            .await
            .expect("record audit entry");

        let (status, prompt, completion, total, cost) =
            stored_row(&db_path).expect("read stored audit row");
        assert_eq!(status, "success");
        assert_eq!(prompt, 11);
        assert_eq!(completion, 7);
        assert_eq!(total, 18);
        assert_eq!(cost, Some(0.25));
    });
}

#[test]
fn record_defaults_token_columns_when_usage_is_absent() {
    runtime().block_on(async {
        let (_directory, db_path, store) = audit_store();
        store
            .record(entry(RequestStatus::Failure, None))
            .await
            .expect("record audit entry");

        let (status, prompt, completion, total, cost) =
            stored_row(&db_path).expect("read stored audit row");
        assert_eq!(status, "failure");
        assert_eq!(prompt, 0);
        assert_eq!(completion, 0);
        assert_eq!(total, 0);
        assert_eq!(cost, None);
    });
}

#[test]
fn record_maps_every_request_status_to_its_stored_string() {
    runtime().block_on(async {
        for (status, expected) in [
            (RequestStatus::Success, "success"),
            (RequestStatus::Failure, "failure"),
            (RequestStatus::RateLimited, "rate_limited"),
            (RequestStatus::Timeout, "timeout"),
        ] {
            let (_directory, db_path, store) = audit_store();
            store
                .record(entry(status, None))
                .await
                .expect("record audit entry");
            let (stored, _, _, _, _) = stored_row(&db_path).expect("read stored audit row");
            assert_eq!(stored, expected);
        }
    });
}

#[test]
fn record_persists_combo_and_connection_context() {
    runtime().block_on(async {
        let (_directory, db_path, store) = audit_store();
        let mut entry = entry(RequestStatus::Success, None);
        entry.combo_id = Some(ComboId::new());
        entry.combo_step_index = Some(2);
        entry.connection_id = Some(ConnectionId::parse_str(CONNECTION_ID).expect("connection id"));
        entry.provider_runtime_id = Some(ProviderId::new("openai-primary"));
        let combo_id = entry.combo_id.expect("combo id").to_string();
        store.record(entry).await.expect("record audit entry");

        let conn = Connection::open(&db_path).expect("open audit database");
        let (stored_combo, step, connection, runtime_id): (
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT combo_id, combo_step_index, connection_id, provider_runtime_id
                     FROM audit ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read stored audit row");
        assert_eq!(stored_combo.as_deref(), Some(combo_id.as_str()));
        assert_eq!(step, Some(2));
        assert_eq!(connection.as_deref(), Some(CONNECTION_ID));
        assert_eq!(runtime_id.as_deref(), Some("openai-primary"));
    });
}

/// Reopening the store must keep the rows that were already persisted.
///
/// A distinct entry is written before the store is reopened, so a
/// `SqliteAudit::new` that dropped and recreated the table would lose it
/// instead of silently keeping an empty table.
#[test]
fn new_preserves_existing_rows_for_an_existing_audit_table() {
    let directory = tempfile::tempdir().expect("temporary test directory");
    let db_path = directory.path().join("audit.db");
    let store = SqliteAudit::new(&db_path).expect("audit store");

    let first = entry(RequestStatus::Failure, None);
    let first_request_id = first.request_id.clone();
    runtime().block_on(async {
        store
            .record(first)
            .await
            .expect("record audit entry before reopening");
    });

    let reopened = SqliteAudit::new(&db_path).expect("reopen audit store");
    let second = entry(RequestStatus::Success, None);
    let second_request_id = second.request_id.clone();
    runtime().block_on(async {
        reopened
            .record(second)
            .await
            .expect("record audit entry after reopening");
    });

    let conn = Connection::open(&db_path).expect("open audit database");
    let mut stmt = conn
        .prepare("SELECT request_id, status FROM audit ORDER BY id")
        .expect("prepare audit row select");
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query audit rows")
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .expect("collect audit rows");

    assert_eq!(
        rows,
        vec![
            (first_request_id.to_string(), "failure".to_string()),
            (second_request_id.to_string(), "success".to_string()),
        ],
        "reopening the audit store must preserve the rows written before the reopen"
    );
}
