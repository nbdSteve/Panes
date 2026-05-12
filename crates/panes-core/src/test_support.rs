//! Test-only helpers for constructing panes-core state.
//!
//! Gated behind the `test-utils` feature. Downstream crates enable it
//! via `[dev-dependencies] panes-core = { workspace = true, features =
//! ["test-utils"] }` so production builds never pull this module in.
//!
//! The helpers here intentionally skip the usual `db::initialize`
//! bootstrap path (WAL mode, stale-thread recovery). They're for
//! fast-isolated test setup, not for driving the real app.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::mpsc;

use crate::db::DbHandle;
use crate::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::ThreadEvent;

/// Build an in-memory sqlite DB with all migrations applied. Returns a
/// ready-to-use `DbHandle`.
pub fn in_memory_db() -> DbHandle {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    crate::db::run_migrations(&conn).expect("run migrations");
    DbHandle::new(conn)
}

/// Build a `SessionManager` wired to a fresh in-memory DB and a
/// per-test shadow-blob tempdir. Returns the manager alongside a
/// receiver for forwarded events (some tests don't need it — just
/// drop the receiver).
///
/// The tempdir is leaked intentionally: its lifetime needs to match
/// the manager, and the OS reaps it when the test process exits.
pub async fn test_session_manager() -> (
    SessionManager,
    DbHandle,
    mpsc::UnboundedReceiver<ThreadEvent>,
) {
    let db = in_memory_db();
    let (tx, rx) = mpsc::unbounded_channel();
    let cost_tracker = Arc::new(CostTracker::new());
    let blob_root: PathBuf = tempfile::tempdir()
        .expect("create shadow blob tempdir")
        .keep();
    let mgr = SessionManager::new(cost_tracker, tx, db.clone(), blob_root).await;
    (mgr, db, rx)
}
