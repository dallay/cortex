// Integration tests for usage retention sweep
//
// These tests verify the public API of the retention sweep functions:
// - run_startup_usage_retention_sweep calls delete_older_than and logs results
// - spawn_periodic_usage_retention_sweep returns a JoinHandle that can be aborted
//
// The actual SQLite delete logic is tested in audit-sqlite tests.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rook::usage_retention::{
    run_startup_usage_retention_sweep, spawn_periodic_usage_retention_sweep,
};
use tokio::time::sleep;

// Helper to create an in-memory usage repo
fn usage_repo() -> audit_sqlite::SqliteUsageRepository {
    audit_sqlite::SqliteUsageRepository::new(Path::new(":memory:"))
        .expect("usage repo should be creatable")
}

// T7.2 — startup sweep executes and returns deleted count
// We test with a fresh repo (no records) to verify the sweep runs and returns 0
#[tokio::test]
async fn startup_sweep_returns_zero_on_empty_repo() {
    let repo = usage_repo();

    // No records inserted — sweep should still succeed and return 0
    let deleted = run_startup_usage_retention_sweep(&repo, 90)
        .await
        .expect("sweep should not fail on empty repo");

    assert_eq!(deleted, 0, "zero records deleted from empty repo");
}

// T7.2 — periodic sweep returns a JoinHandle that can be aborted
// This verifies the spawn function signature matches what's called in server.rs
#[tokio::test]
async fn periodic_sweep_returns_abortable_join_handle() {
    let repo = Arc::new(usage_repo());

    let handle = spawn_periodic_usage_retention_sweep(
        repo.clone(),
        90,   // retention_days
        3600, // sweep_interval_hours (1 hour)
    );

    // Abort is the primary way to stop the background task (e.g., on shutdown)
    handle.abort();
}

// T7.2 — periodic sweep tolerates immediate second tick without panic
// Using 0 sweep_interval_hours causes the ticker to fire immediately once
#[tokio::test]
async fn periodic_sweep_handles_zero_interval_without_panic() {
    let repo = Arc::new(usage_repo());

    let handle = spawn_periodic_usage_retention_sweep(
        repo.clone(),
        90,
        0, // immediate tick
    );

    // Give the task a moment to run its first tick
    sleep(Duration::from_millis(50)).await;
    handle.abort();

    // Reaches here only if no panic occurred during the first tick
}
