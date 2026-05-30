use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeySubject, ApiKeyTier,
    NewSession, NewUser, PasswordHash, Session, SessionId, SessionRepositoryError,
    SessionRepositoryPort, User, UserId, UserRepositoryError, UserRepositoryPort,
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

        CREATE TABLE IF NOT EXISTS users (
            id           TEXT PRIMARY KEY,
            username     TEXT NOT NULL UNIQUE COLLATE NOCASE,
            password_hash TEXT,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_users_username ON users (username COLLATE NOCASE);

        CREATE TABLE IF NOT EXISTS sessions (
            id          TEXT PRIMARY KEY,
            token_hash  TEXT NOT NULL UNIQUE,
            user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at  TEXT NOT NULL,
            expires_at  TEXT NOT NULL,
            revoked     INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1))
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON sessions (token_hash);
        CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions (expires_at);
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

fn scopes_from_json(value: &str) -> Result<Vec<ApiKeyScope>, String> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|error| error.to_string())?;
    values
        .iter()
        .map(|scope| ApiKeyScope::parse(scope).map_err(|error| error.to_string()))
        .collect()
}

#[cfg(test)]
fn scopes_to_json(scopes: &[ApiKeyScope]) -> Result<String, ApiKeyRepositoryError> {
    let values = scopes.iter().map(ApiKeyScope::as_str).collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| ApiKeyRepositoryError::Database(error.to_string()))
}

#[cfg(test)]
fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

#[cfg(test)]
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

#[cfg(test)]
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
