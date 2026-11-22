use std::path::{Path, PathBuf};

use audit_sqlite::SqliteUsageRepository;
use chrono::{DateTime, Utc};
use rook_core::{ProviderId, QuotaObservation, QuotaObservationRecorderPort};
use rusqlite::{params, Connection};
use shared_kernel::{ConnectionId, CortexResult};

const CONNECTION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
}

/// One row of `provider_quota_observations`, mirroring the stored schema.
struct Fixture {
    provider_kind: String,
    provider_runtime_id: String,
    connection_id: Option<String>,
    metric: String,
    source: String,
    scope: Option<String>,
    window: Option<String>,
    limit: Option<f64>,
    remaining: Option<f64>,
    reset_at: Option<String>,
    reset_after_milliseconds: Option<i64>,
    observed_at: String,
}

fn base_fixture() -> Fixture {
    Fixture {
        provider_kind: "openai".to_string(),
        provider_runtime_id: "openai-primary".to_string(),
        connection_id: Some(CONNECTION_ID.to_string()),
        metric: "requests".to_string(),
        source: "headers".to_string(),
        scope: Some("account".to_string()),
        window: Some("daily".to_string()),
        limit: Some(100.0),
        remaining: Some(42.5),
        reset_at: Some("2026-06-01T12:00:00+00:00".to_string()),
        reset_after_milliseconds: Some(60_000),
        observed_at: "2026-06-01T11:00:00+00:00".to_string(),
    }
}

fn migrated_database() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("temporary test directory");
    let db_path = directory.path().join("audit.db");
    let db_path_str = db_path.to_string_lossy().into_owned();
    db_migration::run_migrations(&db_path_str).expect("migrations");
    (directory, db_path)
}

/// Open the migrated database for fixture seeding.
///
/// Malformed fixtures must be seedable even when a column carries a CHECK
/// constraint, because the repository still has to reject them on read.
fn open_fixture_connection(db_path: &Path) -> Connection {
    let conn = Connection::open(db_path).expect("open migrated database");
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;")
        .expect("relax check constraints for fixture seeding");
    conn
}

fn insert(conn: &Connection, fixture: &Fixture) {
    conn.execute(
        "INSERT INTO provider_quota_observations (
            provider_kind, provider_runtime_id, connection_id, metric, source,
            scope, window, quota_limit, remaining, reset_at,
            reset_after_milliseconds, observed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            fixture.provider_kind,
            fixture.provider_runtime_id,
            fixture.connection_id,
            fixture.metric,
            fixture.source,
            fixture.scope,
            fixture.window,
            fixture.limit,
            fixture.remaining,
            fixture.reset_at,
            fixture.reset_after_milliseconds,
            fixture.observed_at,
        ],
    )
    .expect("insert quota observation");
}

/// Read the seeded observations through the repository boundary.
async fn list_observations(db_path: &Path) -> CortexResult<Vec<QuotaObservation>> {
    let repo = SqliteUsageRepository::new(db_path).expect("usage repository");
    let rows = repo.list_observations(None).await;
    repo.shutdown().await.expect("shutdown quota writer");
    rows
}

/// Assert that the repository rejected a malformed row.
///
/// The repository reports the failure as a provider error whose message
/// carries the rejected column index and name, so the contract is verified
/// without exposing the private row mapper.
fn expect_mapping_error(
    result: CortexResult<Vec<QuotaObservation>>,
    expected_index: usize,
    expected_name: &str,
) {
    let error = result.expect_err("expected a quota row mapping error");
    let message = error.to_string();
    assert!(
        message.contains(&format!("index: {expected_index}")),
        "unexpected mapping error: {message}"
    );
    assert!(
        message.contains(&format!("name: {expected_name}")),
        "unexpected mapping error: {message}"
    );
}

#[test]
fn list_observations_reads_every_column() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        insert(&conn, &base_fixture());
        drop(conn);

        let rows = list_observations(&db_path)
            .await
            .expect("list quota observations");
        assert_eq!(rows.len(), 1);
        let observation = &rows[0];
        assert_eq!(observation.provider_id, "openai");
        assert_eq!(observation.provider, ProviderId::new("openai-primary"));
        assert_eq!(
            observation.connection_id,
            Some(ConnectionId::parse_str(CONNECTION_ID).expect("valid connection id"))
        );
        assert_eq!(observation.metric, "requests");
        assert_eq!(observation.source, "headers");
        assert_eq!(observation.scope.as_deref(), Some("account"));
        assert_eq!(observation.window.as_deref(), Some("daily"));
        assert_eq!(observation.limit, Some(100.0));
        assert_eq!(observation.remaining, Some(42.5));
        assert_eq!(
            observation.reset_at,
            Some(
                DateTime::parse_from_rfc3339("2026-06-01T12:00:00+00:00")
                    .expect("valid reset timestamp")
                    .with_timezone(&Utc)
            )
        );
        assert_eq!(observation.reset_after_milliseconds, Some(60_000));
        assert_eq!(
            observation.observed_at,
            DateTime::parse_from_rfc3339("2026-06-01T11:00:00+00:00")
                .expect("valid observed timestamp")
                .with_timezone(&Utc)
        );
    });
}

#[test]
fn list_observations_maps_absent_optional_columns_to_none() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.connection_id = None;
        fixture.scope = None;
        fixture.window = None;
        fixture.limit = None;
        fixture.remaining = None;
        fixture.reset_at = None;
        fixture.reset_after_milliseconds = None;
        insert(&conn, &fixture);
        drop(conn);

        let rows = list_observations(&db_path)
            .await
            .expect("list quota observations");
        assert_eq!(rows.len(), 1);
        let observation = &rows[0];
        assert!(observation.connection_id.is_none());
        assert!(observation.scope.is_none());
        assert!(observation.window.is_none());
        assert!(observation.limit.is_none());
        assert!(observation.remaining.is_none());
        assert!(observation.reset_at.is_none());
        assert!(observation.reset_after_milliseconds.is_none());
        assert_eq!(observation.metric, "requests");
        assert_eq!(observation.provider_id, "openai");
    });
}

#[test]
fn list_observations_resolves_legacy_provider_spellings() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.provider_kind = "azureopenai".to_string();
        insert(&conn, &fixture);
        drop(conn);

        let rows = list_observations(&db_path)
            .await
            .expect("list quota observations");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "azure-openai");
    });
}

#[test]
fn list_observations_rejects_an_unknown_provider() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.provider_kind = "not-a-provider".to_string();
        insert(&conn, &fixture);
        drop(conn);

        expect_mapping_error(list_observations(&db_path).await, 0, "provider_kind");
    });
}

#[test]
fn list_observations_rejects_a_malformed_connection_id() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.connection_id = Some("not-a-uuid".to_string());
        insert(&conn, &fixture);
        drop(conn);

        expect_mapping_error(list_observations(&db_path).await, 2, "connection_id");
    });
}

#[test]
fn list_observations_rejects_a_malformed_reset_at() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.reset_at = Some("not-a-timestamp".to_string());
        insert(&conn, &fixture);
        drop(conn);

        expect_mapping_error(list_observations(&db_path).await, 9, "reset_at");
    });
}

#[test]
fn list_observations_rejects_a_malformed_observed_at() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.observed_at = "not-a-timestamp".to_string();
        insert(&conn, &fixture);
        drop(conn);

        expect_mapping_error(list_observations(&db_path).await, 11, "observed_at");
    });
}

#[test]
fn list_observations_rejects_a_negative_reset_duration() {
    runtime().block_on(async {
        let (_directory, db_path) = migrated_database();
        let conn = open_fixture_connection(&db_path);
        let mut fixture = base_fixture();
        fixture.reset_after_milliseconds = Some(-1);
        insert(&conn, &fixture);
        drop(conn);

        expect_mapping_error(
            list_observations(&db_path).await,
            10,
            "reset_after_milliseconds",
        );
    });
}
