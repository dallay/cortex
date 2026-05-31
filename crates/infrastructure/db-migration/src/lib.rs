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
pub fn run_on_connection(conn: &mut rusqlite::Connection) -> anyhow::Result<usize> {
    let runner = build_runner();

    let report = runner.run(conn)?;
    let count = report.applied_migrations().len();
    if count > 0 {
        tracing::info!(count, "migrations applied at startup");
    } else {
        tracing::debug!("database schema is current");
    }
    Ok(count)
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
