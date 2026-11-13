//! Helpers shared by the integration tests. Not every test file uses every helper.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use routing_console::app::{App, CommandInput, CreateTask};
use routing_console::clock::Clock;
use routing_console::domain::EventKind;
use routing_console::snapshot::TaskView;
use routing_console::store::Store;

pub fn command(kind: &str, version: i64) -> CommandInput {
    CommandInput {
        kind: kind.into(),
        expected_version: version,
        contributor_id: None,
        reason_code: None,
        note: None,
        exclude_previous: false,
    }
}

pub fn with_reason(kind: &str, version: i64, code: &str) -> CommandInput {
    CommandInput {
        reason_code: Some(code.into()),
        ..command(kind, version)
    }
}

pub fn assign(version: i64, contributor: i64, code: Option<&str>) -> CommandInput {
    CommandInput {
        contributor_id: Some(contributor),
        reason_code: code.map(Into::into),
        ..command("assign", version)
    }
}

/// Creates a task, sets its scenario, and submits it, so the scenario applies to the first
/// request.
pub fn submitted(app: &App, title: &str, area: &str, scenario: &str) -> TaskView {
    let t = app
        .create_task(CreateTask {
            title: title.into(),
            area: area.into(),
            priority: "normal".into(),
            submit: false,
        })
        .unwrap();
    app.set_scenario(t.id, scenario).unwrap();
    app.command(t.id, command("submit", 1)).unwrap()
}

/// Lets every task that is ready run: timer tasks send, the app applies what they sent.
/// Yielding never makes the runtime idle, so the paused clock does not jump ahead.
pub async fn settle() {
    for _ in 0..6 {
        tokio::task::yield_now().await;
    }
}

pub async fn advance_ms(ms: u64) {
    settle().await;
    tokio::time::advance(Duration::from_millis(ms)).await;
    settle().await;
}

pub fn kinds(t: &TaskView) -> Vec<EventKind> {
    t.events.iter().map(|e| e.kind).collect()
}

pub fn count(t: &TaskView, kind: EventKind) -> usize {
    t.events.iter().filter(|e| e.kind == kind).count()
}

/// A database file under `target/tmp/`, unique per test, removed with its WAL files on drop.
pub struct TempDb {
    pub path: PathBuf,
}

impl TempDb {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{name}-{}-{n}.db", std::process::id()));
        let db = TempDb { path };
        db.remove();
        db
    }

    fn remove(&self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = self.path.clone().into_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn open(&self) -> Store {
        Store::open(&self.path).unwrap()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.remove();
    }
}

/// Stops the app (its timers are aborted with its simulator) and starts a new one on the same
/// file, with the same clock so time carries on.
pub async fn restart(app: Arc<App>, db: &TempDb, clock: &Clock) -> Arc<App> {
    drop(app);
    settle().await;
    App::start(db.open(), clock.clone()).unwrap()
}
