//! Database migration runner powered by Refinery.

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("src/migrations");
}

pub use embedded::migrations;

/// Build the migration runner with the configured migration table name.
fn build_runner() -> refinery::Runner {
    let mut r = embedded::migrations::runner();
    r.set_migration_table_name("_migrations");
    r
}

/// Run all pending migrations on the given database path.
pub fn run_migrations(db_path: &str) -> anyhow::Result<usize> {
    let mut conn = rusqlite::Connection::open(db_path)?;
    run_on_connection(&mut conn)
}

/// Run all pending migrations on an existing open connection.
///
/// This function is idempotent — if migrations were already applied (e.g., by
/// another connection sharing the same in-memory database via `cache=shared`),
/// this returns 0 without error.
pub fn run_on_connection(conn: &mut rusqlite::Connection) -> anyhow::Result<usize> {
    let runner = build_runner();

    // A new database has no migration table yet. Refinery reports that expected
    // condition as an error; other errors must not be masked as an empty schema.
    let applied = match runner.get_applied_migrations(conn) {
        Ok(applied) => applied,
        Err(error) if is_missing_migration_table(&error) => Vec::new(),
        Err(error) => return Err(error.into()),
    };

    // If migrations are already fully applied, skip.
    let all_migrations_runner = embedded::migrations::runner();
    let all_migrations = all_migrations_runner.get_migrations();
    if !applied.is_empty() && applied.len() >= all_migrations.len() {
        tracing::debug!("database schema is current");
        return Ok(0);
    }

    // Run pending migrations. If another connection already created tables in a
    // shared in-memory DB, treat "already exists" errors as non-fatal.
    let report = match runner.run(conn) {
        Ok(report) => report,
        Err(e) => {
            // If the error mentions "already exists", another connection won the race.
            // Re-check applied migrations to confirm the schema is current.
            let err_msg = e.to_string();
            if err_msg.contains("already exists") {
                let applied_after = runner.get_applied_migrations(conn)?;
                if !applied_after.is_empty() {
                    tracing::debug!(
                        count = applied_after.len(),
                        "migrations already applied by another connection"
                    );
                    return Ok(0);
                }
            }
            return Err(e.into());
        }
    };

    let count = report.applied_migrations().len();
    if count > 0 {
        tracing::info!(count, "migrations applied at startup");
    } else {
        tracing::debug!("database schema is current");
    }
    Ok(count)
}

fn is_missing_migration_table(error: &refinery::Error) -> bool {
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        if let Some(sqlite_error) = cause.downcast_ref::<rusqlite::Error>() {
            return matches!(
                sqlite_error,
                rusqlite::Error::SqliteFailure(_, Some(message))
                    if message == "no such table: _migrations"
            );
        }
        source = cause.source();
    }
    false
}

/// Migration runner for CLI use cases.
pub struct MigrationRunner {
    db_path: String,
}

impl MigrationRunner {
    pub fn new(db_path: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    pub fn status(&self) -> anyhow::Result<MigrationStatus> {
        let mut conn = rusqlite::Connection::open(&self.db_path)?;

        let runner = build_runner();

        let applied = runner.get_applied_migrations(&mut conn)?;

        Ok(MigrationStatus {
            current_version: applied.last().map(|m| m.version()),
            applied: applied
                .iter()
                .map(|m| AppliedMigration {
                    version: m.version(),
                    name: m.name().to_string(),
                })
                .collect(),
        })
    }

    pub fn run(&self) -> anyhow::Result<usize> {
        let mut conn = rusqlite::Connection::open(&self.db_path)?;

        let runner = build_runner();

        let report = runner.run(&mut conn)?;
        Ok(report.applied_migrations().len())
    }
}

#[derive(Debug, Clone)]
pub struct MigrationStatus {
    pub current_version: Option<i32>,
    pub applied: Vec<AppliedMigration>,
}

#[derive(Debug, Clone)]
pub struct AppliedMigration {
    pub version: i32,
    pub name: String,
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn fresh_database_runs_embedded_migrations() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        assert!(run_on_connection(&mut conn).is_ok());
        let applied = build_runner().get_applied_migrations(&mut conn).unwrap();
        assert_eq!(
            applied.len(),
            embedded::migrations::runner().get_migrations().len()
        );
    }

    #[test]
    fn database_with_applied_migrations_is_idempotent() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let applied = run_on_connection(&mut conn).unwrap();
        assert!(applied > 0);
        assert_eq!(run_on_connection(&mut conn).unwrap(), 0);
    }

    #[test]
    fn migration_runner_reports_status_after_applying_migrations() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("rook.sqlite");
        let runner = MigrationRunner::new(db_path.to_string_lossy().into_owned());

        let applied_count = runner.run().unwrap();
        let status = runner.status().unwrap();

        assert_eq!(status.applied.len(), applied_count);
        assert!(status.current_version.is_some());
        assert_eq!(
            status.current_version,
            status.applied.last().map(|m| m.version)
        );
    }

    #[test]
    fn run_migrations_applies_and_then_skips_current_schema() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("rook.sqlite");
        let db_path = db_path.to_string_lossy().into_owned();

        let first_run = run_migrations(&db_path).unwrap();
        let second_run = run_migrations(&db_path).unwrap();

        assert!(first_run > 0);
        assert_eq!(second_run, 0);
    }
}
