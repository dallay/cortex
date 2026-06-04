// usage_retention — startup and periodic retention sweep for usage_history
//
// Runs before the server accepts traffic and then on a configured interval.
// Failures warn but do not crash the server.

use std::sync::Arc;
use std::time::Duration;

use tokio::time;
use tracing::warn;

/// Run a single retention sweep, deleting usage records older than `retention_days`.
pub async fn run_startup_usage_retention_sweep(
    usage_repo: &SqliteUsageRepository,
    retention_days: u32,
) -> anyhow::Result<u64> {
    let deleted = usage_repo
        .delete_older_than(retention_days)
        .await
        .map_err(|e| anyhow::anyhow!("startup retention sweep failed: {e}"))?;
    tracing::info!(
        deleted_rows = deleted,
        retention_days = retention_days,
        "startup retention sweep complete"
    );
    Ok(deleted)
}

/// Spawn a background task that runs a retention sweep every `sweep_interval_hours`.
///
/// The task logs failures with `tracing::warn!` but does not crash.
/// The task runs until the runtime shuts down.
pub fn spawn_periodic_usage_retention_sweep(
    usage_repo: Arc<SqliteUsageRepository>,
    retention_days: u32,
    sweep_interval_hours: u32,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval_secs = sweep_interval_hours.max(1) as u64 * 3600;
        let mut ticker = time::interval(Duration::from_secs(interval_secs));

        // Run immediately on startup (skip the initial tick delay)
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match usage_repo.delete_older_than(retention_days).await {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!(deleted_rows = deleted, "periodic retention sweep complete");
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        retention_days = retention_days,
                        "usage retention sweep failed"
                    );
                }
            }
        }
    })
}

// Re-export SqliteUsageRepository for use in this module
pub use audit_sqlite::SqliteUsageRepository;
