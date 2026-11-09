use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::ports::ProviderRepositoryPort;
use rook_core::provider_connection::{
    AuthType, ConnectionConfig, Credentials, EncryptedBlob, ProviderConnection,
    ProviderConnectionRoutingState, QuotaWindowThresholds, RepositoryError, TestStatus,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use shared_kernel::{ConnectionId, ModelId, ProviderId};
use tokio::sync::Semaphore;

pub struct SqliteProviderRepository {
    conn: Arc<Mutex<Connection>>,
    routing_state_gate: Arc<Semaphore>,
}

impl SqliteProviderRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(&db_path)?;
        db_migration::connection::configure_connection(&conn)?;
        if db_path.as_ref().to_str() == Some(":memory:") {
            db_migration::run_on_connection(&mut conn)?;
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            routing_state_gate: Arc::new(Semaphore::new(1)),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
        lock_connection(&self.conn)
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

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<ProviderConnectionRoutingState>, RepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(SELECT_PROVIDER_ROUTING_STATE)
            .map_err(db_error)?;
        let rows = stmt.query_map([], row_to_routing_state).map_err(db_error)?;
        let mut states = Vec::new();
        for row in rows {
            states.push(row.map_err(db_error)?);
        }
        Ok(states)
    }

    async fn list_routing_state_for_candidates(
        &self,
        targets: &[ProviderId],
    ) -> Result<Vec<ProviderConnectionRoutingState>, RepositoryError> {
        let targets: Vec<_> = targets
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if targets.len() > rook_core::MAX_PROVIDER_ROUTING_STATE_TARGET_IDS {
            return Err(RepositoryError::Database(format!(
                "routing-state target limit exceeded: {} > {}",
                targets.len(),
                rook_core::MAX_PROVIDER_ROUTING_STATE_TARGET_IDS
            )));
        }
        let permit = self
            .routing_state_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RepositoryError::Database("routing-state read gate closed".to_string()))?;
        let connection = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let conn = lock_connection(&connection)?;
            routing_state_for_candidates(&conn, &targets)
        })
        .await
        .map_err(|error| {
            RepositoryError::Database(format!("routing-state query task failed: {error}"))
        })?
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

    async fn find_connection_id_by_runtime(
        &self,
        provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, RepositoryError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id
             FROM provider_connections
             WHERE provider_runtime_id = ?1 AND is_active = 1
             ORDER BY priority ASC, created_at DESC
             LIMIT 1",
            params![provider.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
        .map(|id| {
            ConnectionId::parse_str(&id)
                .map_err(|error| RepositoryError::Database(error.to_string()))
        })
        .transpose()
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

        if has_duplicate_name(&tx, conn_model)? {
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
        if has_duplicate_name(&tx, conn_model)? {
            return Err(duplicate_connection(conn_model));
        }
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
           max_concurrent, quota_warning, quota_error, default_model, base_url,
           supported_models, api_format, test_status, test_latency_ms, test_error, test_expires_at, last_test_at,
             oauth_client_id, oauth_client_secret_ct, oauth_tenant_id,
             created_at, updated_at, adapter_revision
    FROM provider_connections";

const SELECT_PROVIDER_ROUTING_STATE_BASE: &str = "
    SELECT id, provider_runtime_id, adapter_revision, priority, created_at, is_active, test_status
    FROM provider_connections";
const SELECT_PROVIDER_ROUTING_STATE: &str =
    "SELECT id, provider_runtime_id, adapter_revision, priority, created_at, is_active, test_status
     FROM provider_connections
     ORDER BY priority ASC, created_at DESC";

const ORDER_PROVIDER_CONNECTIONS: &str = " ORDER BY priority ASC, created_at DESC";

const INSERT_PROVIDER_CONNECTION: &str = "
    INSERT INTO provider_connections (
        id, provider_kind, provider_runtime_id, name, auth_type,
        priority, is_active,
        api_key_ct, oauth_email_ct, access_token_ct,
        refresh_token_ct, scope_ct, id_token_ct, project_id_ct, expires_at,
        max_concurrent, quota_warning, quota_error, default_model, base_url,
        supported_models, api_format, test_status, test_latency_ms, test_error, test_expires_at, last_test_at,
        oauth_client_id, oauth_client_secret_ct, oauth_tenant_id, created_at, updated_at,
        adapter_revision
    ) VALUES (
        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
        ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25,
        ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33
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
        base_url = ?20,
        supported_models = ?21,
        api_format = ?22,
        test_status = ?23,
        test_latency_ms = ?24,
        test_error = ?25,
        test_expires_at = ?26,
        last_test_at = ?27,
        oauth_client_id = ?28,
        oauth_client_secret_ct = ?29,
        oauth_tenant_id = ?30,
        updated_at = ?32,
        adapter_revision = ?33
    WHERE id = ?1 AND updated_at = ?34";

fn provider_params(
    conn: &ProviderConnection,
) -> rusqlite::ParamsFromIter<Vec<rusqlite::types::Value>> {
    let mut values = common_values(conn);
    values.push(conn.created_at.to_rfc3339().into());
    values.push(conn.updated_at.to_rfc3339().into());
    values.push(conn.adapter_revision.into());
    rusqlite::params_from_iter(values)
}

fn update_params(
    conn: &ProviderConnection,
    expected_updated_at: DateTime<Utc>,
) -> rusqlite::ParamsFromIter<Vec<rusqlite::types::Value>> {
    let mut values = common_values(conn);
    values.push(conn.created_at.to_rfc3339().into());
    values.push(conn.updated_at.to_rfc3339().into());
    values.push(conn.adapter_revision.into());
    values.push(expected_updated_at.to_rfc3339().into());
    rusqlite::params_from_iter(values)
}

fn has_duplicate_name(
    tx: &rusqlite::Transaction<'_>,
    conn: &ProviderConnection,
) -> Result<bool, RepositoryError> {
    let mut statement = tx
        .prepare("SELECT provider_kind FROM provider_connections WHERE name = ?1 AND id != ?2")
        .map_err(db_error)?;
    let providers = statement
        .query_map(params![conn.name, conn.id.to_string()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(db_error)?;
    for provider in providers {
        if rook_core::descriptor_for(&provider.map_err(db_error)?)
            .is_some_and(|descriptor| descriptor.id == conn.provider.id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn common_values(conn: &ProviderConnection) -> Vec<rusqlite::types::Value> {
    vec![
        conn.id.to_string().into(),
        conn.provider.as_str().to_string().into(),
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
        optional_string(conn.config.base_url.as_ref().map(ToString::to_string)),
        serde_json::to_string(&conn.config.supported_models)
            .expect("serializing a vector of model ids cannot fail")
            .into(),
        match conn.config.api_format {
            Some(rook_core::ApiFormat::Anthropic) => "anthropic",
            Some(rook_core::ApiFormat::OpenAI) => "openai",
            None if conn.provider.protocol == rook_core::Protocol::AnthropicMessages => "anthropic",
            None => "openai",
        }
        .to_string()
        .into(),
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
        optional_string(oauth_client_id(&conn.credentials)),
        optional_string(oauth_client_secret_ct(&conn.credentials)),
        optional_string(oauth_tenant_id(&conn.credentials)),
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
                ..
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
            oauth_client_id: row
                .get::<_, Option<String>>("oauth_client_id")?
                .unwrap_or_default(),
            oauth_client_secret: row
                .get::<_, Option<String>>("oauth_client_secret_ct")?
                .map(EncryptedBlob),
            oauth_tenant_id: row.get::<_, Option<String>>("oauth_tenant_id")?,
        },
    };

    Ok(ProviderConnection {
        id: ConnectionId::parse_str(&row.get::<_, String>("id")?)
            .map_err(|_| invalid_data("invalid connection id"))?,
        provider: rook_core::descriptor_for(&row.get::<_, String>("provider_kind")?)
            .ok_or_else(|| invalid_data("unknown provider id"))?,
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
            base_url: row.get("base_url")?,
            supported_models: serde_json::from_str(&row.get::<_, String>("supported_models")?)
                .map_err(|_| invalid_data("invalid supported_models"))?,
            api_format: match row.get::<_, String>("api_format")?.as_str() {
                "openai" => Some(rook_core::ApiFormat::OpenAI),
                "anthropic" => Some(rook_core::ApiFormat::Anthropic),
                _ => return Err(invalid_data("invalid api_format")),
            },
        },
        adapter_revision: row.get::<_, i64>("adapter_revision")?,
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
        None => {
            return Err(invalid_data(
                "missing last_test_at for non-NeverTested status",
            ))
        }
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

fn oauth_client_id(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth {
            oauth_client_id, ..
        } => Some(oauth_client_id.clone()),
        Credentials::ApiKey { .. } => None,
    }
}

fn oauth_client_secret_ct(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth {
            oauth_client_secret,
            ..
        } => oauth_client_secret
            .as_ref()
            .map(|secret| secret.as_str().to_string()),
        Credentials::ApiKey { .. } => None,
    }
}

fn oauth_tenant_id(creds: &Credentials) -> Option<String> {
    match creds {
        Credentials::OAuth {
            oauth_tenant_id, ..
        } => oauth_tenant_id.clone(),
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
    RepositoryError::DuplicateConnection(format!("{}/{}", conn.provider.as_str(), conn.name))
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

fn lock_connection(
    conn: &Mutex<Connection>,
) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
    conn.lock()
        .map_err(|_| RepositoryError::Database("sqlite mutex poisoned".to_string()))
}

fn routing_state_for_candidates(
    conn: &Connection,
    targets: &[ProviderId],
) -> Result<Vec<ProviderConnectionRoutingState>, RepositoryError> {
    const TARGET_CHUNK_SIZE: usize = 400;

    let mut states_by_id = std::collections::HashMap::new();
    for target_chunk in targets.chunks(TARGET_CHUNK_SIZE) {
        let connection_ids: Vec<_> = target_chunk
            .iter()
            .filter_map(|target| {
                ConnectionId::parse_str(target.as_str())
                    .ok()
                    .map(|connection_id| connection_id.to_string())
            })
            .collect();
        let runtime_ids: Vec<_> = target_chunk
            .iter()
            .filter(|target| ConnectionId::parse_str(target.as_str()).is_err())
            .map(|target| target.as_str().to_owned())
            .collect();
        if !connection_ids.is_empty() {
            let query = format!(
                "{SELECT_PROVIDER_ROUTING_STATE_BASE} WHERE id IN ({}) ORDER BY priority ASC, id ASC",
                sql_placeholders(connection_ids.len())
            );
            let mut statement = conn.prepare(&query).map_err(db_error)?;
            let rows = statement
                .query_map(
                    rusqlite::params_from_iter(connection_ids),
                    row_to_routing_state,
                )
                .map_err(db_error)?;
            for row in rows {
                let state = row.map_err(db_error)?;
                states_by_id.insert(state.id, state);
            }
        }
        if !runtime_ids.is_empty() {
            let query = format!(
                "SELECT id, provider_runtime_id, adapter_revision, priority, created_at, is_active, test_status
                 FROM (
                     SELECT id, provider_runtime_id, adapter_revision, priority, created_at, is_active, test_status,
                            ROW_NUMBER() OVER (
                                PARTITION BY provider_runtime_id
                                 ORDER BY priority ASC, id ASC
                            ) AS runtime_rank
                     FROM provider_connections
                     WHERE is_active = 1 AND provider_runtime_id IN ({})
                 )
                 WHERE runtime_rank = 1",
                sql_placeholders(runtime_ids.len())
            );
            let mut statement = conn.prepare(&query).map_err(db_error)?;
            let rows = statement
                .query_map(
                    rusqlite::params_from_iter(runtime_ids),
                    row_to_routing_state,
                )
                .map_err(db_error)?;
            for row in rows {
                let state = row.map_err(db_error)?;
                states_by_id.insert(state.id, state);
            }
        }
    }

    let mut states: Vec<_> = states_by_id.into_values().collect();
    states.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.0.cmp(&right.id.0))
    });
    Ok(states)
}

fn db_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Database(error.to_string())
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn row_to_routing_state(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProviderConnectionRoutingState> {
    let id = ConnectionId::parse_str(&row.get::<_, String>("id")?)
        .map_err(|_| invalid_data("invalid connection id"))?;
    let created_at = row.get::<_, String>("created_at")?;
    let status: String = row.get("test_status")?;

    Ok(ProviderConnectionRoutingState {
        id,
        provider_runtime_id: ProviderId::new(row.get::<_, String>("provider_runtime_id")?),
        adapter_revision: row.get::<_, i64>("adapter_revision")?,
        priority: row.get::<_, i64>("priority")? as u8,
        created_at: parse_datetime(&created_at)?,
        is_active: row.get("is_active")?,
        auth_expired: status == "expired",
    })
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
            provider: rook_core::descriptor_by_id("openai").expect("openai descriptor"),
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
                base_url: None,
                api_format: None,
                supported_models: Vec::new(),
            },
            test_status: TestStatus::NeverTested,
            adapter_revision: 1,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn new_provider_connections_start_with_adapter_revision_one() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let connection = connection("revision-default", 1, Utc::now());
            repo.create(&connection).await.expect("create connection");

            let revision = repo
                .lock()
                .expect("sqlite connection")
                .query_row(
                    "SELECT adapter_revision FROM provider_connections WHERE id = ?1",
                    [connection.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("adapter revision column");

            assert_eq!(revision, 1);
        });
    }

    #[test]
    fn adapter_revision_round_trips_through_connection_and_routing_state_reads() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let mut connection = connection("revision-round-trip", 1, Utc::now());
            connection.adapter_revision = 7;
            repo.create(&connection).await.expect("create connection");

            let loaded = repo
                .find(&connection.id)
                .await
                .expect("find connection")
                .expect("connection exists");
            assert_eq!(loaded.adapter_revision, 7);
            let states = repo
                .list_routing_state_for_candidates(&[ProviderId::new(connection.id.to_string())])
                .await
                .expect("candidate state");
            assert_eq!(states.len(), 1);
            assert_eq!(states[0].adapter_revision, 7);

            let expected = connection.updated_at;
            connection.adapter_revision = 8;
            connection.config.max_concurrent = 2;
            connection.updated_at += chrono::Duration::seconds(1);
            repo.update(&connection, expected)
                .await
                .expect("update adapter revision");

            let updated = repo
                .find(&connection.id)
                .await
                .expect("find updated connection")
                .expect("connection exists");
            assert_eq!(updated.adapter_revision, 8);
            let updated_states = repo
                .list_routing_state_for_candidates(&[ProviderId::new(connection.id.to_string())])
                .await
                .expect("updated candidate state");
            assert_eq!(updated_states[0].adapter_revision, 8);
        });
    }

    #[test]
    fn candidate_routing_state_query_rejects_more_than_4000_targets() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let targets: Vec<_> = (0..=4_000)
                .map(|index| ProviderId::new(format!("runtime-{index}")))
                .collect();

            let result = repo.list_routing_state_for_candidates(&targets).await;

            assert!(matches!(result, Err(RepositoryError::Database(_))));
        });
    }

    #[test]
    fn duplicate_names_reject_legacy_and_case_variant_provider_ids() {
        runtime().block_on(async {
            for stored in ["ollamacloud", "OLLAMA-CLOUD"] {
                let temp = tempfile::NamedTempFile::new().unwrap();
                let mut db = Connection::open(temp.path()).unwrap();
                db_migration::run_on_connection(&mut db).unwrap();
                let repo = SqliteProviderRepository::new(temp.path()).unwrap();
                let mut original = connection("shared", 1, Utc::now());
                original.provider = rook_core::descriptor_by_id("ollama-cloud").unwrap();
                repo.create(&original).await.unwrap();
                Connection::open(temp.path())
                    .unwrap()
                    .execute(
                        "UPDATE provider_connections SET provider_kind = ?1",
                        [stored],
                    )
                    .unwrap();
                let mut duplicate = original.clone();
                duplicate.id = ConnectionId::new();
                assert!(matches!(
                    repo.create(&duplicate).await,
                    Err(RepositoryError::DuplicateConnection(_))
                ));
                duplicate.name = "different".to_string();
                repo.create(&duplicate).await.unwrap();
                let expected = duplicate.updated_at;
                duplicate.name = "shared".to_string();
                assert!(matches!(
                    repo.update(&duplicate, expected).await,
                    Err(RepositoryError::DuplicateConnection(_))
                ));
            }
        });
    }

    #[test]
    fn pre_migration_legacy_provider_spellings_load_through_the_alias_map() {
        let temp_file = tempfile::Builder::new()
            .prefix("rook-provider-legacy-spelling-")
            .suffix(".sqlite")
            .tempfile()
            .expect("temporary database");
        let path = temp_file.path();
        {
            let mut conn = Connection::open(path).expect("legacy database");
            db_migration::run_on_connection(&mut conn).expect("migrations");
            // Rows written before the descriptor-id migration carry the serde
            // spellings of Ollama Cloud and Azure OpenAI.
            for (kind, runtime) in [
                ("ollamacloud", "runtime-legacy-ollamacloud"),
                ("azureopenai", "runtime-legacy-azureopenai"),
            ] {
                conn.execute(
                    "INSERT INTO provider_connections (
                        id, provider_kind, provider_runtime_id, name, auth_type,
                        priority, is_active, api_key_ct, max_concurrent, quota_warning,
                        quota_error, test_status, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, 'apiKey',
                        5, 1, 'enc:v1:legacy', 1, 0.7, 0.9, 'neverTested',
                        '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    params![ConnectionId::new().to_string(), kind, runtime, runtime],
                )
                .expect("legacy row");
            }
        }

        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(path).expect("repo");
            let connections = repo.list().await.expect("legacy connections load");
            let mut ids: Vec<_> = connections.iter().map(|c| c.provider.id).collect();
            ids.sort();
            assert_eq!(ids, vec!["azure-openai", "ollama-cloud"]);
            assert!(connections
                .iter()
                .all(|connection| connection.adapter_revision == 1));
        });
    }

    #[test]
    fn the_descriptor_id_migration_canonicalizes_legacy_stored_rows() {
        // Run the shipped V15 statements verbatim against rows carrying the
        // legacy serde spellings.
        let migration_sql =
            include_str!("../../db-migration/src/migrations/V15__provider_descriptor_ids.sql");
        let mut conn = Connection::open_in_memory().expect("database");
        db_migration::run_on_connection(&mut conn).expect("migrations");
        for kind in ["ollamacloud", "azureopenai"] {
            conn.execute(
                "INSERT INTO provider_connections (
                    id, provider_kind, provider_runtime_id, name, auth_type,
                    priority, is_active, api_key_ct, max_concurrent, quota_warning,
                    quota_error, test_status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'apiKey',
                    5, 1, 'enc:v1:legacy', 1, 0.7, 0.9, 'neverTested',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![
                    ConnectionId::new().to_string(),
                    kind,
                    format!("runtime-{kind}"),
                    kind
                ],
            )
            .expect("legacy row");
        }
        // A canonical row may already share a name with a legacy row; the
        // migration must resolve the collision without deleting either.
        conn.execute(
            "INSERT INTO provider_connections (
                id, provider_kind, provider_runtime_id, name, auth_type,
                priority, is_active, api_key_ct, max_concurrent, quota_warning,
                quota_error, test_status, created_at, updated_at
            ) VALUES (?1, 'ollama-cloud', 'runtime-canonical', 'ollamacloud', 'apiKey',
                5, 1, 'enc:v1:canonical', 1, 0.7, 0.9, 'neverTested',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            params![ConnectionId::new().to_string()],
        )
        .expect("canonical row");
        conn.execute_batch(migration_sql).expect("V15 applies");

        let colliding: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM provider_connections WHERE name LIKE 'ollamacloud%'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(
            colliding, 2,
            "the collision is resolved by renaming, not deleting"
        );

        let mut kinds: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT DISTINCT provider_kind FROM provider_connections")
                .expect("select");
            stmt.query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("rows")
        };
        kinds.sort();
        assert_eq!(kinds, vec!["azure-openai", "ollama-cloud"]);
    }
    #[test]
    fn connections_created_before_the_model_declaration_migration_keep_working() {
        let temp_file = tempfile::Builder::new()
            .prefix("rook-provider-legacy-")
            .suffix(".sqlite")
            .tempfile()
            .expect("temporary database");
        let path = temp_file.path();
        {
            let mut conn = Connection::open(path).expect("legacy database");
            db_migration::run_on_connection(&mut conn).expect("migrations");
            // A row written before the migration: no supported_models, no api_format.
            conn.execute(
                "INSERT INTO provider_connections (
                    id, provider_kind, provider_runtime_id, name, auth_type,
                    priority, is_active, api_key_ct, max_concurrent, quota_warning,
                    quota_error, test_status, created_at, updated_at
                ) VALUES (?1, 'openai', 'runtime-legacy', 'legacy', 'apiKey',
                    5, 1, 'enc:v1:legacy', 1, 0.7, 0.9, 'neverTested',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![ConnectionId::new().to_string()],
            )
            .expect("legacy row");
        }

        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(path).expect("repo");
            let connections = repo.list().await.expect("legacy connection loads");
            assert_eq!(connections.len(), 1);
            let legacy = &connections[0];
            assert_eq!(legacy.provider.id, "openai");
            assert!(
                legacy.config.supported_models.is_empty(),
                "a legacy connection has no declared models"
            );
            assert_eq!(
                legacy.config.api_format,
                Some(rook_core::ApiFormat::OpenAI),
                "a legacy connection keeps the OpenAI api format"
            );
        });
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

    #[test]
    fn delete_returns_not_found_for_nonexistent_id() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let result = repo.delete(&ConnectionId::new()).await;
            assert!(matches!(result, Err(RepositoryError::NotFound(_))));
        });
    }

    #[test]
    fn create_duplicate_name_reports_duplicate_connection() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let first = connection("shared-name", 1, timestamp("2026-01-01T00:00:00Z"));
            repo.create(&first).await.expect("first");

            let second = connection("shared-name", 2, timestamp("2026-01-02T00:00:00Z"));
            let result = repo.create(&second).await;
            assert!(matches!(
                result,
                Err(RepositoryError::DuplicateConnection(_))
            ));
        });
    }

    #[test]
    fn find_connection_id_by_runtime_returns_highest_priority_active_connection() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let runtime_id = ProviderId::new("openai-primary");
            let mut inactive_high =
                connection("inactive-high", 1, timestamp("2026-01-04T00:00:00Z"));
            inactive_high.provider_runtime_id = runtime_id.clone();
            inactive_high.is_active = false;
            let mut low = connection("low", 2, timestamp("2026-01-01T00:00:00Z"));
            low.provider_runtime_id = runtime_id.clone();
            let mut old_high = connection("old-high", 1, timestamp("2026-01-02T00:00:00Z"));
            old_high.provider_runtime_id = runtime_id.clone();
            let mut new_high = connection("new-high", 1, timestamp("2026-01-03T00:00:00Z"));
            new_high.provider_runtime_id = runtime_id.clone();
            let expected = new_high.id;

            repo.create(&inactive_high).await.expect("inactive");
            repo.create(&low).await.expect("low");
            repo.create(&old_high).await.expect("old");
            repo.create(&new_high).await.expect("new");

            let result = repo
                .find_connection_id_by_runtime(&runtime_id)
                .await
                .expect("find runtime");

            assert_eq!(result, Some(expected));
        });
    }

    #[test]
    fn find_connection_id_by_runtime_returns_none_for_unknown_or_toml_provider() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let result = repo
                .find_connection_id_by_runtime(&ProviderId::new("ollama-local"))
                .await
                .expect("find runtime");
            assert_eq!(result, None);
        });
    }

    #[test]
    fn list_returns_empty_when_no_connections() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let result = repo.list().await.expect("list");
            assert!(result.is_empty());
        });
    }

    #[test]
    fn list_routing_state_returns_only_connection_eligibility_fields() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let now = Utc::now();
            let active = connection("active", 1, now);
            let mut expired = connection("expired", 2, now);
            expired.test_status = TestStatus::Expired {
                last_test_at: now,
                expires_at: now.timestamp() - 1,
            };
            let mut disabled = connection("disabled", 3, now);
            disabled.is_active = false;
            repo.create(&active).await.expect("create active");
            repo.create(&expired).await.expect("create expired");
            repo.create(&disabled).await.expect("create disabled");

            let states = repo
                .list_routing_state()
                .await
                .expect("routing state projection");

            assert_eq!(states.len(), 3);
            assert_eq!(states[0].id, active.id);
            assert_eq!(states[0].provider_runtime_id, active.provider_runtime_id);
            assert!(states[0].is_active);
            assert!(!states[0].auth_expired);
            assert!(states[1].auth_expired);
            assert!(!states[2].is_active);
            assert!(!states[2].auth_expired);
        });
    }

    #[test]
    fn list_routing_state_for_candidates_returns_only_matching_targets() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let now = Utc::now();
            let exact = connection("exact", 1, now);
            let exact_id = exact.id;
            let mut runtime_match = connection("runtime-match", 2, now);
            runtime_match.provider_runtime_id = ProviderId::new("runtime-target");
            let runtime_connection_id = runtime_match.id;
            let unrelated = connection("unrelated", 3, now);
            let mut disabled = connection("disabled", 4, now);
            disabled.is_active = false;
            let disabled_id = disabled.id;
            for connection in [&exact, &runtime_match, &unrelated, &disabled] {
                repo.create(connection).await.expect("create connection");
            }

            let states = repo
                .list_routing_state_for_candidates(&[
                    ProviderId::new(exact_id.to_string()),
                    ProviderId::new("runtime-target"),
                    ProviderId::new(disabled_id.to_string()),
                ])
                .await
                .expect("targeted routing state");

            let ids: std::collections::HashSet<_> = states.iter().map(|state| state.id).collect();
            assert_eq!(ids.len(), 3);
            assert!(ids.contains(&exact_id));
            assert!(ids.contains(&runtime_connection_id));
            assert!(ids.contains(&disabled_id));
            assert!(!ids.contains(&unrelated.id));
        });
    }

    #[test]
    fn list_routing_state_for_candidates_returns_only_best_runtime_match() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let now = Utc::now();
            let mut lower_priority = connection("runtime-lower", 9, now);
            lower_priority.provider_runtime_id = ProviderId::new("shared-runtime");
            let mut best_priority = connection("runtime-best", 1, now);
            best_priority.provider_runtime_id = ProviderId::new("shared-runtime");
            let mut middle_priority = connection("runtime-middle", 5, now);
            middle_priority.provider_runtime_id = ProviderId::new("shared-runtime");
            let expected_id = best_priority.id;
            repo.create(&lower_priority)
                .await
                .expect("lower connection");
            repo.create(&best_priority).await.expect("best connection");
            repo.create(&middle_priority)
                .await
                .expect("middle connection");

            let states = repo
                .list_routing_state_for_candidates(&[ProviderId::new("shared-runtime")])
                .await
                .expect("runtime state lookup");

            assert_eq!(states.len(), 1);
            assert_eq!(states[0].id, expected_id);
        });
    }

    #[test]
    fn exact_and_runtime_matches_use_the_same_id_tie_break() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let created_at = timestamp("2026-01-01T00:00:00Z");
            let mut first = connection("runtime-tie-first", 1, created_at);
            first.id =
                ConnectionId::parse_str("00000000-0000-0000-0000-000000000001").expect("first id");
            first.provider_runtime_id = ProviderId::new("shared-runtime-tie");
            let mut second = connection("runtime-tie-second", 1, created_at);
            second.id =
                ConnectionId::parse_str("00000000-0000-0000-0000-000000000002").expect("second id");
            second.provider_runtime_id = ProviderId::new("shared-runtime-tie");
            repo.create(&first).await.expect("first connection");
            repo.create(&second).await.expect("second connection");

            let states = repo
                .list_routing_state_for_candidates(&[
                    ProviderId::new(second.id.to_string()),
                    ProviderId::new("shared-runtime-tie"),
                ])
                .await
                .expect("routing state");

            assert_eq!(
                states.iter().map(|state| state.id).collect::<Vec<_>>(),
                vec![first.id, second.id]
            );
        });
    }

    #[tokio::test]
    async fn timed_out_routing_state_reads_keep_only_one_blocking_slot() {
        let repo = Arc::new(SqliteProviderRepository::new(":memory:").expect("repo"));
        let gate = repo.routing_state_gate.clone();
        let connection = repo.conn.clone();
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let lock_task = tokio::task::spawn_blocking(move || {
            let _guard = lock_connection(&connection).expect("sqlite connection");
            let _ = locked_tx.send(());
            release_rx.recv().expect("release connection lock");
        });
        locked_rx.await.expect("blocking task acquired SQLite lock");

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                repo.list_routing_state_for_candidates(&[]),
            )
            .await
            .is_err(),
            "the test-held SQLite lock should delay the blocking query"
        );
        assert_eq!(gate.available_permits(), 0);

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                repo.list_routing_state_for_candidates(&[]),
            )
            .await
            .is_err(),
            "a second request waits for the single routing-state slot"
        );
        assert_eq!(gate.available_permits(), 0);

        release_tx.send(()).expect("release lock task");
        lock_task.await.expect("lock task exits");
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            repo.list_routing_state_for_candidates(&[]),
        )
        .await
        .expect("the detached blocking query releases its slot")
        .expect("routing state query succeeds after lock release");
    }

    #[test]
    fn find_returns_none_for_nonexistent_id() {
        runtime().block_on(async {
            let repo = SqliteProviderRepository::new(":memory:").expect("repo");
            let result = repo.find(&ConnectionId::new()).await.expect("find");
            assert!(result.is_none());
        });
    }
}
