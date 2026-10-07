use agent_core::{AgentError, Result, Session, SessionInfo, SessionStore};
use async_trait::async_trait;
use fs2::FileExt;
use rusqlite::{params, Connection};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct SqliteSessions {
    connection: Mutex<Connection>,
    locks: PathBuf,
}
/// Cross-process advisory lock held for the duration of a conversation.
/// It is released by the operating system even if the process crashes.
pub struct SessionLease {
    _file: File,
}
impl SqliteSessions {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let _ = options.open(path)?;
        let connection = Connection::open(path).map_err(db_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_error)?;
        if version > 1 {
            return Err(AgentError::Session(
                "session database is from a newer version".into(),
            ));
        }
        connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); PRAGMA user_version=1; COMMIT;")
            .map_err(db_error)?;
        let locks = lock_directory()?;
        Ok(Self {
            connection: Mutex::new(connection),
            locks,
        })
    }
    pub fn lease(&self, id: &str) -> Result<SessionLease> {
        if id.len() != 36
            || !id.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        {
            return Err(AgentError::Session("invalid session id".into()));
        }
        let uuid = uuid::Uuid::parse_str(id)
            .map_err(|_| AgentError::Session("invalid session id".into()))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.locks.join(format!("{}.lock", uuid.as_hyphenated())))?;
        file.try_lock_exclusive()
            .map_err(|_| AgentError::Session("session is in use by another process".into()))?;
        Ok(SessionLease { _file: file })
    }
}
fn lock_directory() -> Result<PathBuf> {
    let user = ["USER", "USERNAME", "HOME", "USERPROFILE"]
        .into_iter()
        .find_map(std::env::var_os)
        .ok_or_else(|| AgentError::Session("current user identity is unavailable".into()))?;
    let user_key = user
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    let locks = std::env::temp_dir().join(format!("cortex-agent-session-locks-{user_key:016x}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        match std::fs::DirBuilder::new().mode(0o700).create(&locks) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = std::fs::symlink_metadata(&locks)?;
        if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(AgentError::Session(
                "session lock directory must be a private directory".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&locks)?;
    Ok(locks)
}
fn db_error(error: rusqlite::Error) -> AgentError {
    AgentError::Session(error.to_string())
}
#[async_trait]
impl SessionStore for SqliteSessions {
    async fn save(&self, session: &Session) -> Result<()> {
        let data = serde_json::to_string(session)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| AgentError::Session("database lock poisoned".into()))?;
        connection.execute("INSERT INTO sessions(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data, updated_at=CURRENT_TIMESTAMP",params![session.id,data]).map_err(db_error)?;
        drop(connection);
        Ok(())
    }
    async fn load(&self, id: &str) -> Result<Session> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AgentError::Session("database lock poisoned".into()))?;
        let data: String = connection
            .query_row("SELECT data FROM sessions WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .map_err(db_error)?;
        drop(connection);
        let session: Session = serde_json::from_str(&data)?;
        if session.id != id
            || session.summary_through > session.messages.len()
            || (session.summary_through > 0 && session.summary.is_none())
            || session
                .messages
                .get(session.summary_through)
                .is_some_and(|m| m.role != agent_core::Role::User)
        {
            return Err(AgentError::Session(
                "invalid session history or summary boundary".into(),
            ));
        }
        Ok(session)
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AgentError::Session("database lock poisoned".into()))?;
        let mut statement = connection
            .prepare("SELECT data FROM sessions ORDER BY updated_at DESC,id")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        let data = rows
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(db_error)?;
        drop(statement);
        drop(connection);
        data.into_iter()
            .map(|data| {
                let session: Session = serde_json::from_str(&data)?;
                Ok(SessionInfo {
                    id: session.id,
                    workspace: session.workspace,
                    interrupted: session.interrupted,
                })
            })
            .collect()
    }
}
