use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeySubject, ApiKeyTier,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

pub struct SqliteApiKeyRepository {
    conn: Mutex<Connection>,
}

impl SqliteApiKeyRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        run_migration(&conn)?;
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
                revoked_at, expires_at, created_at, last_used_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            scopes: vec![ApiKeyScope::parse("read").expect("scope")],
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
impl ApiKeyRepositoryPort for SqliteApiKeyRepository {
    async fn find_active_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.query_row(
            "SELECT id, label, scopes_json, tier
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
}

fn run_migration(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS api_keys (
            id           TEXT PRIMARY KEY,
            label        TEXT NOT NULL,
            key_hash     TEXT NOT NULL UNIQUE,
            key_prefix   TEXT NOT NULL,
            scopes_json  TEXT NOT NULL,
            tier         TEXT NOT NULL CHECK (tier IN ('free', 'pro', 'enterprise')),
            is_active    INTEGER NOT NULL CHECK (is_active IN (0, 1)),
            revoked_at   TEXT,
            expires_at   TEXT,
            created_at   TEXT NOT NULL,
            last_used_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_api_keys_active ON api_keys (is_active);
        CREATE INDEX IF NOT EXISTS idx_api_keys_revoked_at ON api_keys (revoked_at);
        CREATE INDEX IF NOT EXISTS idx_api_keys_expires_at ON api_keys (expires_at);
        "#,
    )?;
    Ok(())
}

fn row_to_subject(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKeySubject> {
    let id: String = row.get("id")?;
    let label: String = row.get("label")?;
    let scopes_json: String = row.get("scopes_json")?;
    let tier: String = row.get("tier")?;
    Ok(ApiKeySubject {
        id: ApiKeyId::new(id),
        label,
        scopes: scopes_from_json(&scopes_json).map_err(invalid_data)?,
        tier: ApiKeyTier::from_str(&tier).map_err(|error| invalid_data(error.to_string()))?,
    })
}

#[cfg(test)]
fn scopes_to_json(scopes: &[ApiKeyScope]) -> Result<String, ApiKeyRepositoryError> {
    let values = scopes.iter().map(ApiKeyScope::as_str).collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))
}

fn scopes_from_json(value: &str) -> Result<Vec<ApiKeyScope>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|error| error.to_string())?;
    values
        .iter()
        .map(|scope| ApiKeyScope::parse(scope).map_err(|error| error.to_string()))
        .collect()
}

#[cfg(test)]
fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

#[cfg(test)]
fn optional_datetime(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
fn parse_datetime(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid datetime"))
}

fn invalid_data(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
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
    use chrono::{Duration, Utc};
    use rook_core::{ApiKeyId, ApiKeyRepositoryPort, ApiKeyTier};

    use super::{SqliteApiKeyRepository, TestApiKeyRecord};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    #[test]
    fn finds_only_active_unrevoked_unexpired_api_keys_by_hash() {
        runtime().block_on(async {
            let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
            let mut active = TestApiKeyRecord::active("active", "hash-active");
            active
                .scopes
                .push(rook_core::ApiKeyScope::parse("write").expect("scope"));
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
            assert_eq!(active.scopes[0].as_str(), "read");
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

            assert_eq!(
                repo.last_used_at_for_test(&id).expect("last used"),
                Some(used_at)
            );
        });
    }
}
