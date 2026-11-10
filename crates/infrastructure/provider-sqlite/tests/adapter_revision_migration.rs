use provider_sqlite::SqliteProviderRepository;
use rook_core::{ConnectionId, ProviderRepositoryPort};
use shared_kernel::ProviderId;

#[tokio::test]
async fn v15_connection_remains_readable_with_default_adapter_revision_after_upgrade() {
    let database = tempfile::NamedTempFile::new().expect("temporary database");
    let path = database.path();
    let mut conn = rusqlite::Connection::open(path).expect("open database");

    let mut v15_runner = db_migration::migrations::runner();
    v15_runner.set_migration_table_name("_migrations");
    let v15_runner = v15_runner.set_target(refinery::Target::Version(15));
    let report = v15_runner.run(&mut conn).expect("apply V0 through V15");
    assert_eq!(
        report
            .applied_migrations()
            .last()
            .map(|migration| migration.version()),
        Some(15)
    );

    let connection_id = ConnectionId::new();
    conn.execute(
        "INSERT INTO provider_connections (
            id, provider_kind, provider_runtime_id, name, auth_type,
            priority, is_active, api_key_ct, max_concurrent, quota_warning,
            quota_error, test_status, created_at, updated_at
        ) VALUES (?1, 'openai', 'legacy-runtime', 'legacy connection', 'apiKey',
            1, 1, 'encrypted-key', 1, 0.7, 0.9, 'neverTested',
            '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        rusqlite::params![connection_id.to_string()],
    )
    .expect("insert a V15 connection without adapter_revision");
    drop(conn);

    db_migration::run_migrations(path.to_str().expect("database path"))
        .expect("normal migration runner applies V16");
    let repository = SqliteProviderRepository::new(path).expect("open repository");
    let loaded = repository
        .find(&connection_id)
        .await
        .expect("read upgraded connection")
        .expect("legacy connection remains present");

    assert_eq!(
        loaded.provider_runtime_id,
        ProviderId::new("legacy-runtime")
    );
    assert_eq!(loaded.adapter_revision, 1);
}
