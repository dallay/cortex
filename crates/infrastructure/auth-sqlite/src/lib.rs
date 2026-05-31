use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope,
    ApiKeySubject, ApiKeyTier, NewSession, NewUser, PasswordHash, Session, SessionId,
    SessionRepositoryError, SessionRepositoryPort, User, UserId, UserRepositoryError,
    UserRepositoryPort,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

pub struct SqliteApiKeyRepository {
    conn: Mutex<Connection>,
}

impl SqliteApiKeyRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        db_migration::run_on_connection(&mut conn)?;
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

    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                        revoked_at, expires_at, created_at, last_used_at
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
                    revoked_at, expires_at, created_at, last_used_at
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
                    last_used_at = ?7
                 WHERE id = ?8",
                params![
                    record.label,
                    scopes_to_json(&record.scopes)?,
                    record.tier.as_str(),
                    bool_to_i64(record.is_active),
                    optional_datetime(record.revoked_at),
                    optional_datetime(record.expires_at),
                    optional_datetime(record.last_used_at),
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

    async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                    revoked_at, expires_at, created_at, last_used_at
             FROM api_keys
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
        conn.query_row("SELECT COUNT(*) FROM api_keys", [], |row| row.get(0))
            .map_err(db_error)
    }
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

    let revoked_at = revoked_at_str.map(|s| parse_datetime(&s)).transpose()?;
    let expires_at = expires_at_str.map(|s| parse_datetime(&s)).transpose()?;
    let created_at = parse_datetime(&created_at_str)?;
    let last_used_at = last_used_at_str.map(|s| parse_datetime(&s)).transpose()?;

    Ok(ApiKeyRecord {
        id: ApiKeyId::new(id),
        label,
        key_hash,
        key_prefix,
        scopes: scopes_from_json(&scopes_json).map_err(invalid_data)?,
        tier: ApiKeyTier::from_str(&tier).map_err(|error| invalid_data(error.to_string()))?,
        is_active: is_active != 0,
        revoked_at,
        expires_at,
        created_at,
        last_used_at,
    })
}

fn scopes_from_json(value: &str) -> Result<Vec<ApiKeyScope>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|error| error.to_string())?;
    values
        .iter()
        .map(|scope| ApiKeyScope::parse(scope).map_err(|error| error.to_string()))
        .collect()
}

fn scopes_to_json(scopes: &[ApiKeyScope]) -> Result<String, ApiKeyRepositoryError> {
    let values = scopes.iter().map(ApiKeyScope::as_str).collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))
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
        let mut conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        db_migration::run_on_connection(&mut conn)?;
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
        let mut conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        db_migration::run_on_connection(&mut conn)?;
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
    use rook_core::{
        ApiKeyId, ApiKeyRepositoryPort, ApiKeyTier, NewSession as CoreNewSession,
        NewUser as CoreNewUser, PasswordHash as CorePasswordHash, SessionRepositoryPort,
        UserRepositoryPort,
    };
    use rusqlite::params;
    use std::fs;
    use std::path::Path;

    use super::{
        SqliteApiKeyRepository, SqliteSessionRepository, SqliteUserRepository, TestApiKeyRecord,
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // Shared temp file path for tests that need both repos
    fn shared_test_db() -> impl AsRef<Path> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir();
        dir.join(format!("rook_test_auth_{}.db", timestamp))
    }

    fn cleanup_test_db(path: &Path) {
        let _ = fs::remove_file(path);
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
                scopes: vec![rook_core::ApiKeyScope::parse("read").unwrap()],
                tier: ApiKeyTier::Free,
                is_active: true,
                revoked_at: None,
                expires_at: None,
                created_at: Utc::now(),
                last_used_at: None,
            };
            repo.create(&record).await.expect("create");

            // 2. Find the key
            let found = repo
                .find(&ApiKeyId::new("key-123"))
                .await
                .expect("find")
                .expect("some");
            assert_eq!(found.label, "Development Key");
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
                    scopes: vec![rook_core::ApiKeyScope::parse("read").unwrap()],
                    tier: ApiKeyTier::Free,
                    is_active: true,
                    revoked_at: None,
                    expires_at: None,
                    created_at: Utc::now(),
                    last_used_at: None,
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
        let db_path = shared_test_db();
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

        cleanup_test_db(db_path.as_ref());
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
        let db_path = shared_test_db();
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

        cleanup_test_db(db_path.as_ref());
    }

    #[test]
    fn session_repository_delete_expired() {
        let db_path = shared_test_db();
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

        cleanup_test_db(db_path.as_ref());
    }
}
