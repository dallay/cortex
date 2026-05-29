use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::ports::ProviderRepositoryPort;
use rook_core::provider_connection::{
    AuthType, ConnectionConfig, Credentials, EncryptedBlob, ProviderConnection, ProviderKind,
    QuotaWindowThresholds, RepositoryError, TestStatus,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use shared_kernel::{ConnectionId, ModelId, ProviderId};

use super::migration;

pub struct SqliteProviderRepository {
    conn: Mutex<Connection>,
}

impl SqliteProviderRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        migration::run(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
        self.conn
            .lock()
            .map_err(|_| RepositoryError::Database("sqlite mutex poisoned".to_string()))
    }
}

#[async_trait]
impl ProviderRepositoryPort for SqliteProviderRepository {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT_PROVIDER_CONNECTIONS}{ORDER_PROVIDER_CONNECTIONS}"
            ))
            .map_err(db_error)?;
        let rows = stmt.query_map([], row_to_connection).map_err(db_error)?;
        let mut connections = Vec::new();
        for row in rows {
            connections.push(row.map_err(db_error)?);
        }
        Ok(connections)
    }

    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        let conn = self.lock()?;
        conn.query_row(
            &format!("{SELECT_PROVIDER_CONNECTIONS} WHERE id = ?1"),
            params![id.to_string()],
            row_to_connection,
        )
        .optional()
        .map_err(db_error)
    }

    async fn create(&self, conn_model: &ProviderConnection) -> Result<(), RepositoryError> {
        validate_credential_shape(conn_model)?;

        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(db_error)?;

        if tx
            .query_row(
                "SELECT 1 FROM provider_connections WHERE id = ?1",
                params![conn_model.id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?
            .is_some()
        {
            return Err(RepositoryError::DuplicateId(conn_model.id));
        }

        if tx
            .query_row(
                "SELECT 1 FROM provider_connections WHERE provider_kind = ?1 AND name = ?2",
                params![conn_model.provider_kind.as_str(), conn_model.name],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?
            .is_some()
        {
            return Err(duplicate_connection(conn_model));
        }

        tx.execute(INSERT_PROVIDER_CONNECTION, provider_params(conn_model))
            .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }

    async fn update(
        &self,
        conn_model: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        validate_credential_shape(conn_model)?;

        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(db_error)?;
        let rows = tx
            .execute(
                UPDATE_PROVIDER_CONNECTION,
                update_params(conn_model, expected_updated_at),
            )
            .map_err(|error| update_error(error, conn_model))?;

        if rows == 0 {
            let exists = tx
                .query_row(
                    "SELECT 1 FROM provider_connections WHERE id = ?1",
                    params![conn_model.id.to_string()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(db_error)?
                .is_some();
            return if exists {
                Err(RepositoryError::StaleUpdate)
            } else {
                Err(RepositoryError::NotFound(conn_model.id))
            };
        }

        tx.commit().map_err(db_error)
    }

    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(db_error)?;
        let rows = tx
            .execute(
                "DELETE FROM provider_connections WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        if rows == 0 {
            return Err(RepositoryError::NotFound(*id));
        }
        Ok(())
    }
}

const SELECT_PROVIDER_CONNECTIONS: &str = "
    SELECT id, provider_kind, provider_runtime_id, name, auth_type,
           priority, is_active, api_key_ct, oauth_email_ct, access_token_ct,
           refresh_token_ct, scope_ct, id_token_ct, project_id_ct, expires_at,
           max_concurrent, quota_warning, quota_error, default_model,
           test_status, test_latency_ms, test_error, test_expires_at, last_test_at,
           created_at, updated_at
    FROM provider_connections";

const ORDER_PROVIDER_CONNECTIONS: &str = " ORDER BY priority ASC, created_at DESC";

const INSERT_PROVIDER_CONNECTION: &str = "
    INSERT INTO provider_connections (
        id, provider_kind, provider_runtime_id, name, auth_type,
        priority, is_active,
        api_key_ct, oauth_email_ct, access_token_ct,
        refresh_token_ct, scope_ct, id_token_ct, project_id_ct, expires_at,
        max_concurrent, quota_warning, quota_error, default_model,
        test_status, test_latency_ms, test_error, test_expires_at, last_test_at,
        created_at, updated_at
    ) VALUES (
        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
        ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
    )";

const UPDATE_PROVIDER_CONNECTION: &str = "
    UPDATE provider_connections SET
        provider_kind = ?2,
        provider_runtime_id = ?3,
        name = ?4,
        auth_type = ?5,
        priority = ?6,
        is_active = ?7,
        api_key_ct = ?8,
        oauth_email_ct = ?9,
        access_token_ct = ?10,
        refresh_token_ct = ?11,
        scope_ct = ?12,
        id_token_ct = ?13,
        project_id_ct = ?14,
        expires_at = ?15,
        max_concurrent = ?16,
        quota_warning = ?17,
        quota_error = ?18,
        default_model = ?19,
        test_status = ?20,
        test_latency_ms = ?21,
        test_error = ?22,
        test_expires_at = ?23,
        last_test_at = ?24,
        updated_at = ?26
    WHERE id = ?1 AND updated_at = ?27";

fn provider_params(
    conn: &ProviderConnection,
) -> rusqlite::ParamsFromIter<Vec<rusqlite::types::Value>> {
    let mut values = common_values(conn);
    values.push(conn.created_at.to_rfc3339().into());
    values.push(conn.updated_at.to_rfc3339().into());
    rusqlite::params_from_iter(values)
}

fn update_params(
    conn: &ProviderConnection,
    expected_updated_at: DateTime<Utc>,
) -> rusqlite::ParamsFromIter<Vec<rusqlite::types::Value>> {
    let mut values = common_values(conn);
    values.push(conn.created_at.to_rfc3339().into());
    values.push(conn.updated_at.to_rfc3339().into());
    values.push(expected_updated_at.to_rfc3339().into());
    rusqlite::params_from_iter(values)
}

fn common_values(conn: &ProviderConnection) -> Vec<rusqlite::types::Value> {
    vec![
        conn.id.to_string().into(),
        conn.provider_kind.as_str().to_string().into(),
        conn.provider_runtime_id.to_string().into(),
        conn.name.clone().into(),
        auth_type_str(conn.auth_type).to_string().into(),
        i64::from(conn.priority).into(),
        i64::from(conn.is_active).into(),
        optional_string(api_key_ct(&conn.credentials)),
        optional_string(oauth_email_ct(&conn.credentials)),
        optional_string(access_token_ct(&conn.credentials)),
        optional_string(refresh_token_ct(&conn.credentials)),
        optional_string(scope_ct(&conn.credentials)),
        optional_string(id_token_ct(&conn.credentials)),
        optional_string(project_id_ct(&conn.credentials)),
        conn.expires_at()
            .map_or(rusqlite::types::Value::Null, Into::into),
        i64::from(conn.config.max_concurrent).into(),
        f64::from(conn.config.quota_window_thresholds.warning).into(),
        f64::from(conn.config.quota_window_thresholds.error).into(),
        optional_string(conn.config.default_model.as_ref().map(ToString::to_string)),
        serialize_test_status(&conn.test_status).to_string().into(),
        conn.test_status
            .latency_ms()
            .map(|ms| ms as i64)
            .map_or(rusqlite::types::Value::Null, Into::into),
        optional_string(conn.test_status.error_msg()),
        conn.test_status
            .expires_at()
            .map_or(rusqlite::types::Value::Null, Into::into),
        optional_string(conn.test_status.last_test_at().map(|dt| dt.to_rfc3339())),
    ]
}

fn optional_string(value: Option<String>) -> rusqlite::types::Value {
    value.map_or(rusqlite::types::Value::Null, Into::into)
}

fn validate_credential_shape(conn: &ProviderConnection) -> Result<(), RepositoryError> {
    match (&conn.auth_type, &conn.credentials) {
        (AuthType::ApiKey, Credentials::ApiKey { api_key }) if !api_key.as_str().is_empty() => {
            Ok(())
        }
        (
            AuthType::OAuth,
            Credentials::OAuth {
                email,
                access_token,
                refresh_token,
                scope,
                id_token,
                project_id,
                expires_at: _,
            },
        ) if [
            email.as_str(),
            access_token.as_str(),
            refresh_token.as_str(),
            scope.as_str(),
            id_token.as_str(),
            project_id.as_str(),
        ]
        .iter()
        .all(|value| !value.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(RepositoryError::Database(
            "provider connection credential shape does not match auth_type".to_string(),
        )),
    }
}

fn row_to_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderConnection> {
    let auth_type = match row.get::<_, String>("auth_type")?.as_str() {
        "apiKey" => AuthType::ApiKey,
        "oauth" => AuthType::OAuth,
        _ => return Err(invalid_data("invalid auth_type")),
    };
    let provider_kind = row.get::<_, String>("provider_kind")?;
    let test_status = row.get::<_, String>("test_status")?;
    let created_at = row.get::<_, String>("created_at")?;
    let updated_at = row.get::<_, String>("updated_at")?;

    let credentials = match auth_type {
        AuthType::ApiKey => Credentials::ApiKey {
            api_key: EncryptedBlob(required_string(row, "api_key_ct")?),
        },
        AuthType::OAuth => Credentials::OAuth {
            email: EncryptedBlob(required_string(row, "oauth_email_ct")?),
            access_token: EncryptedBlob(required_string(row, "access_token_ct")?),
            refresh_token: EncryptedBlob(required_string(row, "refresh_token_ct")?),
            expires_at: required_i64(row, "expires_at")?,
            scope: EncryptedBlob(required_string(row, "scope_ct")?),
            id_token: EncryptedBlob(required_string(row, "id_token_ct")?),
            project_id: EncryptedBlob(required_string(row, "project_id_ct")?),
        },
    };

    Ok(ProviderConnection {
        id: ConnectionId::parse_str(&row.get::<_, String>("id")?)
            .map_err(|_| invalid_data("invalid connection id"))?,
        provider_kind: ProviderKind::try_from(provider_kind.as_str())
            .map_err(|_| invalid_data("invalid provider kind"))?,
        provider_runtime_id: ProviderId::new(row.get::<_, String>("provider_runtime_id")?),
        name: row.get("name")?,
        priority: row.get::<_, i64>("priority")? as u8,
        is_active: row.get("is_active")?,
        auth_type,
        credentials,
        config: ConnectionConfig {
            max_concurrent: row.get::<_, i64>("max_concurrent")? as u32,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: row.get::<_, f64>("quota_warning")? as f32,
                error: row.get::<_, f64>("quota_error")? as f32,
            },
            default_model: row
                .get::<_, Option<String>>("default_model")?
                .map(ModelId::new),
        },
        test_status: parse_test_status(
            test_status.as_str(),
            row.get::<_, Option<i64>>("test_latency_ms")?
                .map(|ms| ms as u64),
            row.get("test_error")?,
            row.get("test_expires_at")?,
            row.get::<_, Option<String>>("last_test_at")?.as_deref(),
        )?,
        created_at: parse_datetime(&created_at)?,
        updated_at: parse_datetime(&updated_at)?,
    })
}

fn required_string(row: &rusqlite::Row<'_>, column: &str) -> rusqlite::Result<String> {
    row.get::<_, Option<String>>(column)?
        .ok_or_else(|| invalid_data("missing required credential column"))
}

fn required_i64(row: &rusqlite::Row<'_>, column: &str) -> rusqlite::Result<i64> {
    row.get::<_, Option<i64>>(column)?
        .ok_or_else(|| invalid_data("missing required integer column"))
}

fn parse_datetime(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| invalid_data("invalid timestamp"))
}

fn parse_test_status(
    status: &str,
    latency_ms: Option<u64>,
    error: Option<String>,
    expires_at: Option<i64>,
    last_test_at: Option<&str>,
) -> rusqlite::Result<TestStatus> {
    // For non-NeverTested states, last_test_at is required and must parse correctly
    let parsed_last_test_at: Option<DateTime<Utc>> = match last_test_at {
        Some(value) => {
            let dt = DateTime::parse_from_rfc3339(value)
                .map_err(|_| invalid_data("invalid last_test_at timestamp"))?;
            Some(dt.with_timezone(&Utc))
        }
        None if status == "neverTested" => None,
        None => return Err(invalid_data("missing last_test_at for non-NeverTested status")),
    };

    match status {
        "active" => Ok(TestStatus::Active {
            last_test_at: parsed_last_test_at
                .ok_or_else(|| invalid_data("last_test_at required for active status"))?,
            latency_ms: latency_ms.unwrap_or_default(),
        }),
        "unhealthy" => Ok(TestStatus::Unhealthy {
            last_test_at: parsed_last_test_at
                .ok_or_else(|| invalid_data("last_test_at required for unhealthy status"))?,
            error: error.unwrap_or_default(),
        }),
        "expired" => Ok(TestStatus::Expired {
            last_test_at: parsed_last_test_at
                .ok_or_else(|| invalid_data("last_test_at required for expired status"))?,
            expires_at: expires_at.unwrap_or_default(),
        }),
        "unknown" => Ok(TestStatus::Unknown {
            last_test_at: parsed_last_test_at
                .ok_or_else(|| invalid_data("last_test_at required for unknown status"))?,
            reason: error.unwrap_or_else(|| "health_check_not_supported".to_string()),
        }),
        _ => Ok(TestStatus::NeverTested),
    }
}

fn auth_type_str(auth_type: AuthType) -> &'static str {
    match auth_type {
        AuthType::ApiKey => "apiKey",
        AuthType::OAuth => "oauth",
    }
}

fn serialize_test_status(status: &TestStatus) -> &'static str {
    match status {
        TestStatus::NeverTested => "neverTested",
        TestStatus::Active { .. } => "active",
        TestStatus::Unhealthy { .. } => "unhealthy",
        TestStatus::Expired { .. } => "expired",
        TestStatus::Unknown { .. } => "unknown",
    }
}

fn api_key_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::ApiKey { api_key } => Some(api_key.as_str().to_string()),
        Credentials::OAuth { .. } => None,
    }
}

fn oauth_email_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { email, .. } => Some(email.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn access_token_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { access_token, .. } => Some(access_token.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn refresh_token_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { refresh_token, .. } => Some(refresh_token.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn scope_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { scope, .. } => Some(scope.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn id_token_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { id_token, .. } => Some(id_token.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn project_id_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth { project_id, .. } => Some(project_id.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

trait CredentialsExt {
    fn expires_at(&self) -> Option<i64>;
}

impl CredentialsExt for ProviderConnection {
    fn expires_at(&self) -> Option<i64> {
        match self.credentials {
            Credentials::OAuth { expires_at, .. } => Some(expires_at),
            Credentials::ApiKey { .. } => None,
        }
    }
}

fn duplicate_connection(conn: &ProviderConnection) -> RepositoryError {
    RepositoryError::DuplicateConnection(format!("{}/{}", conn.provider_kind.as_str(), conn.name))
}

fn update_error(error: rusqlite::Error, conn: &ProviderConnection) -> RepositoryError {
    match &error {
        rusqlite::Error::SqliteFailure(sqlite_error, Some(message))
            if sqlite_error.code == ErrorCode::ConstraintViolation
                && message.contains("provider_connections.provider_kind")
                && message.contains("provider_connections.name") =>
        {
            duplicate_connection(conn)
        }
        _ => db_error(error),
    }
}

fn invalid_data(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_string())
}

fn db_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn connection(name: &str, priority: u8, created_at: DateTime<Utc>) -> ProviderConnection {
        ProviderConnection {
            id: ConnectionId::new(),
            provider_kind: ProviderKind::OpenAI,
            provider_runtime_id: ProviderId::new(format!("runtime-{name}")),
            name: name.to_string(),
            priority,
            is_active: true,
            auth_type: AuthType::ApiKey,
            credentials: Credentials::ApiKey {
                api_key: EncryptedBlob(format!("enc:v1:{name}")),
            },
            config: ConnectionConfig {
                max_concurrent: 1,
                quota_window_thresholds: QuotaWindowThresholds {
                    warning: 0.7,
                    error: 0.9,
                },
                default_model: None,
            },
            test_status: TestStatus::NeverTested,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn list_orders_by_priority_then_newest_created_at() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let low = connection("low", 2, timestamp("2026-01-01T00:00:00Z"));
            let old_high = connection("old-high", 1, timestamp("2026-01-02T00:00:00Z"));
            let new_high = connection("new-high", 1, timestamp("2026-01-03T00:00:00Z"));
            repo.create(&low).await.expect("low");
            repo.create(&old_high).await.expect("old");
            repo.create(&new_high).await.expect("new");

            let names = repo
                .list()
                .await
                .expect("list")
                .into_iter()
                .map(|conn| conn.name)
                .collect::<Vec<_>>();
            assert_eq!(names, vec!["new-high", "old-high", "low"]);
        });
    }

    #[test]
    fn update_uses_expected_updated_at_and_reports_stale() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let mut conn = connection("primary", 1, timestamp("2026-01-01T00:00:00Z"));
            repo.create(&conn).await.expect("create");
            conn.name = "changed".to_string();
            conn.updated_at = timestamp("2026-01-02T00:00:00Z");
            let result = repo.update(&conn, timestamp("2026-01-03T00:00:00Z")).await;
            assert_eq!(result, Err(RepositoryError::StaleUpdate));
        });
    }

    #[test]
    fn update_duplicate_name_reports_duplicate_connection() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let existing = connection("existing", 1, timestamp("2026-01-01T00:00:00Z"));
            let mut duplicate = connection("duplicate", 2, timestamp("2026-01-02T00:00:00Z"));
            repo.create(&existing).await.expect("existing");
            repo.create(&duplicate).await.expect("duplicate");

            let expected = duplicate.updated_at;
            duplicate.name = existing.name.clone();
            duplicate.updated_at = timestamp("2026-01-03T00:00:00Z");

            assert!(matches!(
                repo.update(&duplicate, expected).await,
                Err(RepositoryError::DuplicateConnection(_))
            ));
        });
    }
}
