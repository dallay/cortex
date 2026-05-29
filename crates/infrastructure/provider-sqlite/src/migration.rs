use rusqlite::Connection;

const SCHEMA_VERSION: i64 = 1;

pub fn run(conn: &Connection) -> anyhow::Result<()> {
    // Read current schema version
    let current_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);

    // Only run migration if older than current version
    if current_version >= SCHEMA_VERSION {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS provider_connections (
            id                  TEXT PRIMARY KEY,
            provider_kind       TEXT    NOT NULL,
            provider_runtime_id TEXT    NOT NULL,
            name                TEXT    NOT NULL,
            auth_type           TEXT    NOT NULL CHECK (auth_type IN ('apiKey', 'oauth')),
            priority            INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),
            is_active           INTEGER NOT NULL CHECK (is_active IN (0, 1)),

            api_key_ct          TEXT,
            oauth_email_ct      TEXT,
            access_token_ct     TEXT,
            refresh_token_ct    TEXT,
            scope_ct            TEXT,
            id_token_ct         TEXT,
            project_id_ct       TEXT,
            expires_at          INTEGER,  -- RFC3339 ISO8601 for OAuth expiry (epoch seconds, compared to Utc::now().timestamp())

            max_concurrent      INTEGER NOT NULL CHECK (max_concurrent >= 1),
            quota_warning       REAL    NOT NULL,
            quota_error         REAL    NOT NULL,
            default_model       TEXT,

            test_status         TEXT    NOT NULL,
            test_latency_ms     INTEGER,
            test_error          TEXT,
            test_expires_at     INTEGER,  -- epoch seconds for TestStatus::Expired
            last_test_at        TEXT,      -- RFC3339 ISO8601 for last test timestamp

            created_at          TEXT    NOT NULL,  -- RFC3339 ISO8601
            updated_at          TEXT    NOT NULL,  -- RFC3339 ISO8601

            UNIQUE (provider_kind, name)
        );

        CREATE INDEX IF NOT EXISTS idx_pc_provider_kind ON provider_connections (provider_kind);
        CREATE INDEX IF NOT EXISTS idx_pc_runtime_id ON provider_connections (provider_runtime_id);
        CREATE INDEX IF NOT EXISTS idx_pc_active ON provider_connections (is_active) WHERE is_active = 1;
        CREATE INDEX IF NOT EXISTS idx_pc_priority_created ON provider_connections (priority ASC, created_at DESC);
        "#,
    )?;

    // Update schema version after successful execution
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    Ok(())
}
