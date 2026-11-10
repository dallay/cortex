//! Shared SQLite connection configuration for repository adapters.
//!
//! Every Rook SQLite repository opens the database file through this module so
//! that concurrent connections behave consistently.

use std::thread::sleep;
use std::time::Duration;

/// Busy timeout applied to every connection, in milliseconds.
pub const BUSY_TIMEOUT_MS: u32 = 5_000;

/// Pause between attempts to switch the journal mode to WAL.
const WAL_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Maximum number of attempts to switch the journal mode to WAL.
///
/// The budget matches the busy timeout: 250 attempts of 20 milliseconds wait
/// for up to 5 seconds before the mode switch fails.
const WAL_MAX_ATTEMPTS: usize = 250;

/// Configure a SQLite connection for concurrent use.
///
/// The busy timeout is applied first, so ordinary statements wait for a lock
/// instead of failing immediately.
///
/// The WAL switch runs afterwards. Switching the journal mode needs an
/// exclusive lock that the busy handler does not cover, so SQLite answers
/// `SQLITE_BUSY` while another connection performs the same switch. The
/// helper retries the switch until it succeeds. Once the database file runs in
/// WAL mode, the switch is a no-op that needs no lock.
pub fn configure_connection(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )?;
    enable_wal(conn)
}

/// Switch the journal mode to WAL, retrying while the mode switch is locked.
fn enable_wal(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    for attempt in 1..=WAL_MAX_ATTEMPTS {
        if runs_in_wal_mode(conn) {
            return Ok(());
        }
        match conn.execute_batch("PRAGMA journal_mode = WAL;") {
            Ok(()) => return Ok(()),
            Err(error) => {
                if !is_busy(&error) || attempt == WAL_MAX_ATTEMPTS {
                    return Err(error);
                }
                sleep(WAL_RETRY_DELAY);
            }
        }
    }
    Ok(())
}

/// Return true when the database file already runs in WAL mode.
///
/// The journal mode is stored in the database file, so a database that already
/// runs in WAL mode needs no mode switch and therefore no exclusive lock.
fn runs_in_wal_mode(conn: &rusqlite::Connection) -> bool {
    conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map(|mode| mode.eq_ignore_ascii_case("wal"))
        .unwrap_or(false)
}

/// Return true when the error reports a locked or busy database.
fn is_busy(error: &rusqlite::Error) -> bool {
    match error {
        rusqlite::Error::SqliteFailure(inner, _) => matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_connection_enables_wal_and_busy_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("configured.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        configure_connection(&conn).unwrap();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, BUSY_TIMEOUT_MS as i64);
    }

    #[test]
    fn concurrent_connections_can_all_enable_wal() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("concurrent.db");

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let db_path = db_path.clone();
                std::thread::spawn(move || {
                    let conn = rusqlite::Connection::open(&db_path).unwrap();
                    configure_connection(&conn).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("worker thread");
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }
}
