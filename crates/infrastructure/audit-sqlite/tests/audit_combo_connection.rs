use audit_sqlite::SqliteAudit;
use chrono::Utc;
use rook_core::{AuditEntry, AuditPort, ModelId, ProviderId, RequestStatus};
use rusqlite::{params, Connection, OptionalExtension};
use shared_kernel::{ConnectionId, RequestId};
use tempfile::NamedTempFile;

#[test]
fn audit_persists_combo_and_effective_connection_identity() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let db = NamedTempFile::new().expect("temporary database");
            db_migration::run_migrations(db.path().to_str().expect("UTF-8 path"))
                .expect("apply migrations");
            let audit = SqliteAudit::new(db.path()).expect("create audit adapter");
            let combo_id = rook_core::ComboId::new();
            let connection_id = ConnectionId::new();
            let request_id = RequestId::new();
            let entry = AuditEntry {
                request_id: request_id.clone(),
                provider: ProviderId::new(connection_id.to_string()),
                model: ModelId::new("gpt-4o"),
                status: RequestStatus::Success,
                usage: None,
                latency_ms: 12,
                timestamp: Utc::now(),
                combo_id: Some(combo_id),
                combo_step_index: Some(1),
                connection_id: Some(connection_id),
                provider_runtime_id: Some(ProviderId::new("openai-primary")),
            };
            audit.record(entry).await.expect("record audit entry");

            let conn = Connection::open(db.path()).expect("open persisted database");
            let row = conn
                .query_row(
                    "SELECT combo_id, combo_step_index, connection_id, provider_runtime_id
                     FROM audit WHERE request_id = ?1",
                    params![request_id.to_string()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .expect("query audit row");
            assert_eq!(
                row,
                Some((
                    combo_id.to_string(),
                    1,
                    connection_id.to_string(),
                    "openai-primary".to_string(),
                ))
            );
        });
}
