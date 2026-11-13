//! SQLite. One connection behind one mutex, one transaction per command. Calls are synchronous
//! under the lock: each is sub-millisecond, and staying off `spawn_blocking` keeps paused-clock
//! tests deterministic.

pub mod read;
pub mod write;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::AppError;

/// The whole schema of contract section 1 except the simulator's own tables (see `engine`).
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS contributors (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    areas    TEXT NOT NULL,            -- JSON array of area strings
    capacity INTEGER NOT NULL CHECK (capacity IN (1, 2))
);
CREATE TABLE IF NOT EXISTS tasks (
    id               INTEGER PRIMARY KEY,
    title            TEXT NOT NULL,
    area             TEXT NOT NULL,
    priority         TEXT NOT NULL CHECK (priority IN ('high', 'normal')),
    state            TEXT NOT NULL CHECK (state IN ('unassigned', 'routing', 'offered',
                         'assigned', 'needs_attention', 'closed')),
    version          INTEGER NOT NULL DEFAULT 1,
    attention_reason TEXT CHECK (attention_reason IN ('declined', 'offer_failed', 'no_match',
                         'selection_unavailable')),
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS routing_requests (
    id                    INTEGER PRIMARY KEY,
    task_id               INTEGER NOT NULL REFERENCES tasks(id),
    status                TEXT NOT NULL CHECK (status IN ('pending', 'resolved', 'superseded')),
    started_at_ms         INTEGER NOT NULL,
    ended_at_ms           INTEGER,
    excluded              TEXT NOT NULL DEFAULT '[]',   -- JSON array of contributor ids
    result_kind           TEXT CHECK (result_kind IN ('match', 'no_match')),
    result_contributor_id INTEGER REFERENCES contributors(id),
    returned_at_ms        INTEGER
);
-- Invariant 2, held by the database so a code bug cannot open a second pending request.
CREATE UNIQUE INDEX IF NOT EXISTS one_pending_request_per_task
    ON routing_requests(task_id) WHERE status = 'pending';
CREATE TABLE IF NOT EXISTS assignments (
    id             INTEGER PRIMARY KEY,
    task_id        INTEGER NOT NULL REFERENCES tasks(id),
    contributor_id INTEGER NOT NULL REFERENCES contributors(id),
    status         TEXT NOT NULL CHECK (status IN ('offered', 'accepted', 'declined', 'failed',
                       'withdrawn')),
    source         TEXT NOT NULL CHECK (source IN ('engine', 'operator')),
    reason_code    TEXT CHECK (reason_code IN ('engine_too_slow', 'better_fit_known',
                       'contributor_unavailable', 'priority', 'other')),
    request_id     INTEGER REFERENCES routing_requests(id),
    created_at_ms  INTEGER NOT NULL,
    ended_at_ms    INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS one_active_assignment_per_task
    ON assignments(task_id) WHERE status IN ('offered', 'accepted');
CREATE TABLE IF NOT EXISTS events (
    id      INTEGER PRIMARY KEY,
    task_id INTEGER NOT NULL REFERENCES tasks(id),
    at_ms   INTEGER NOT NULL,
    kind    TEXT NOT NULL,
    actor   TEXT NOT NULL CHECK (actor IN ('operator', 'engine', 'contributor', 'system')),
    summary TEXT NOT NULL,
    data    TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS events_by_task ON events(task_id, id);
CREATE TRIGGER IF NOT EXISTS events_are_append_only BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append only');
END;
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, AppError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, AppError> {
        // Programs built here may not write to the system temp folder.
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic under the lock rolled its transaction back when it unwound, so the
        // connection is still usable.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Runs `f` in one immediate transaction: committed when `f` returns `Ok`, rolled back
    /// otherwise.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

pub fn meta_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
