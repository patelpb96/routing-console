//! The seeded simulator. It stands in for an outside service: it keeps its decisions in its own
//! tables (`sim_schedule`, `sim_scenarios`) and reports only through the channel, so the app
//! learns nothing about a request except what an engine would tell it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::AbortHandle;

use super::{EngineReport, EngineRequest, RoutingEngine, Scenario, plan};
use crate::clock::Clock;
use crate::domain::EngineResult;
use crate::error::AppError;
use crate::store::Store;

const SIM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sim_schedule (
    request_id     INTEGER PRIMARY KEY,
    task_id        INTEGER NOT NULL,
    due_at_ms      INTEGER,               -- NULL: never returns
    outcome        TEXT NOT NULL CHECK (outcome IN ('match', 'no_match', 'never')),
    contributor_id INTEGER,
    delivered      INTEGER NOT NULL DEFAULT 0   -- set when the app acknowledges
);
CREATE TABLE IF NOT EXISTS sim_scenarios (
    task_id  INTEGER PRIMARY KEY,
    scenario TEXT NOT NULL
);
"#;

pub struct Simulator {
    store: Store,
    clock: Clock,
    seed: AtomicU64,
    /// Bumped by `wipe`; reports carry the value they were armed under.
    epoch: AtomicU64,
    /// False for the seeder's simulator: it writes the schedule but sets no timers, and the
    /// seeder delivers what falls due as it moves its own clock.
    armed: bool,
    reports: UnboundedSender<EngineReport>,
    timers: Mutex<Vec<AbortHandle>>,
}

impl Simulator {
    /// Opens the simulator's tables and re-arms every report not yet acknowledged, the way an
    /// outside service would have kept running while the app was down. An overdue report is
    /// sent at once; a `never` row stays silent.
    pub fn new(
        store: Store,
        clock: Clock,
        seed: u64,
        reports: UnboundedSender<EngineReport>,
    ) -> Result<Self, AppError> {
        let sim = Self::build(store, clock, seed, reports, true)?;
        sim.rearm()?;
        Ok(sim)
    }

    /// A simulator that plans and records like the real one but never sets a timer. The
    /// seeder drives it with `due` on a clock it moves by hand.
    pub fn unarmed(store: Store, clock: Clock, seed: u64) -> Result<Self, AppError> {
        let (reports, _) = unbounded_channel();
        Self::build(store, clock, seed, reports, false)
    }

    fn build(
        store: Store,
        clock: Clock,
        seed: u64,
        reports: UnboundedSender<EngineReport>,
        armed: bool,
    ) -> Result<Self, AppError> {
        store.transaction(|tx| Ok(tx.execute_batch(SIM_SCHEMA)?))?;
        Ok(Simulator {
            store,
            clock,
            seed: AtomicU64::new(seed),
            epoch: AtomicU64::new(0),
            armed,
            reports,
            timers: Mutex::new(Vec::new()),
        })
    }

    /// Arms a timer for every report not yet acknowledged.
    pub fn rearm(&self) -> Result<(), AppError> {
        for (report, due) in self.waiting(None)? {
            self.arm(report, due);
        }
        Ok(())
    }

    /// Reports not yet acknowledged that fall due by `until_ms` (all of them for `None`),
    /// earliest first.
    pub fn due(&self, until_ms: i64) -> Result<Vec<(EngineReport, i64)>, AppError> {
        self.waiting(Some(until_ms))
    }

    fn waiting(&self, until_ms: Option<i64>) -> Result<Vec<(EngineReport, i64)>, AppError> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        self.store.transaction(|tx| {
            let mut stmt = tx.prepare(
                "SELECT request_id, due_at_ms, outcome, contributor_id FROM sim_schedule
                 WHERE delivered = 0 AND due_at_ms IS NOT NULL
                   AND (?1 IS NULL OR due_at_ms <= ?1)
                 ORDER BY due_at_ms, request_id",
            )?;
            let rows = stmt.query_map([until_ms], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (request_id, due, outcome, who) = row?;
                let result = match (outcome.as_str(), who) {
                    ("match", Some(c)) => EngineResult::Match(c),
                    ("no_match", None) => EngineResult::NoMatch,
                    _ => {
                        return Err(AppError::Internal(format!(
                            "sim_schedule row {request_id} is inconsistent: {outcome} / {who:?}"
                        )));
                    }
                };
                let report = EngineReport {
                    request_id,
                    result,
                    epoch,
                };
                out.push((report, due));
            }
            Ok(out)
        })
    }

    /// The simulation reset's part, run inside the reset's own transaction: every timer of
    /// the old run is stopped and every report it already sent is made stale (the epoch
    /// moves on, and the app checks it under the same lock), then the simulator's tables are
    /// emptied.
    pub fn wipe(&self, conn: &Connection, seed: Option<u64>) -> Result<(), AppError> {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let mut timers = self.timers.lock().unwrap_or_else(|p| p.into_inner());
        for t in timers.drain(..) {
            t.abort();
        }
        conn.execute_batch("DELETE FROM sim_schedule; DELETE FROM sim_scenarios;")?;
        if let Some(seed) = seed {
            self.seed.store(seed, Ordering::SeqCst);
        }
        Ok(())
    }

    pub fn seed(&self) -> u64 {
        self.seed.load(Ordering::SeqCst)
    }

    pub fn set_scenario(&self, task_id: i64, scenario: Scenario) -> Result<(), AppError> {
        self.store.transaction(|tx| {
            tx.execute(
                "INSERT INTO sim_scenarios (task_id, scenario) VALUES (?1, ?2)
                 ON CONFLICT(task_id) DO UPDATE SET scenario = excluded.scenario",
                params![task_id, scenario.as_str()],
            )?;
            Ok(())
        })
    }

    /// Tasks without a row run on `auto`.
    pub fn scenarios(&self) -> Result<HashMap<i64, Scenario>, AppError> {
        self.store.transaction(|tx| {
            let mut stmt = tx.prepare("SELECT task_id, scenario FROM sim_scenarios")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = HashMap::new();
            for row in rows {
                let (task, s) = row?;
                let s = Scenario::parse(&s)
                    .ok_or_else(|| AppError::Internal(format!("unknown scenario {s:?}")))?;
                out.insert(task, s);
            }
            Ok(out)
        })
    }

    pub fn scenario(&self, task_id: i64) -> Result<Scenario, AppError> {
        let stored: Option<String> = self.store.transaction(|tx| {
            Ok(tx
                .query_row(
                    "SELECT scenario FROM sim_scenarios WHERE task_id = ?1",
                    [task_id],
                    |r| r.get(0),
                )
                .optional()?)
        })?;
        Ok(stored
            .as_deref()
            .and_then(Scenario::parse)
            .unwrap_or(Scenario::Auto))
    }

    /// Sends `report` at `due_at_ms` on the app's clock, or at once when that has passed.
    fn arm(&self, report: EngineReport, due_at_ms: i64) {
        if !self.armed {
            return;
        }
        let wait = (due_at_ms - self.clock.now_ms()).max(0) as u64;
        // The deadline is fixed here, not at the task's first poll, so a test that advances
        // the paused clock before the task runs still sees the timer fire on time.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        let reports = self.reports.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            // A closed channel means the app has stopped; the row stays unacknowledged and is
            // sent again on the next start.
            let _ = reports.send(report);
        });
        let mut timers = self.timers.lock().unwrap_or_else(|p| p.into_inner());
        timers.retain(|t| !t.is_finished());
        timers.push(handle.abort_handle());
    }
}

/// Stopping the app stops its engine: no timer of an old run may report into a new one.
impl Drop for Simulator {
    fn drop(&mut self) {
        let timers = self.timers.get_mut().unwrap_or_else(|p| p.into_inner());
        for t in timers.drain(..) {
            t.abort();
        }
    }
}

impl RoutingEngine for Simulator {
    fn start(&self, request: EngineRequest) -> Result<(), AppError> {
        let scenario = self.scenario(request.task_id)?;
        let p = plan(self.seed(), request.request_id, scenario, &request.eligible);
        let due = p.delay_ms.map(|d| self.clock.now_ms() + d as i64);
        let (outcome, contributor) = match (due, p.result) {
            (None, _) => ("never", None),
            (Some(_), EngineResult::Match(c)) => ("match", Some(c)),
            (Some(_), EngineResult::NoMatch) => ("no_match", None),
        };
        self.store.transaction(|tx| {
            tx.execute(
                "INSERT INTO sim_schedule (request_id, task_id, due_at_ms, outcome, contributor_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    request.request_id,
                    request.task_id,
                    due,
                    outcome,
                    contributor
                ],
            )?;
            Ok(())
        })?;
        if let Some(due) = due {
            let report = EngineReport {
                request_id: request.request_id,
                result: p.result,
                epoch: self.epoch.load(Ordering::SeqCst),
            };
            self.arm(report, due);
        }
        Ok(())
    }

    fn acknowledge(&self, report: &EngineReport) -> Result<(), AppError> {
        self.store.transaction(|tx| {
            // Checked under the store lock, which a reset also holds while it wipes: a stale
            // acknowledgement must not mark a new request with the same id as delivered.
            if self.is_current(report) {
                tx.execute(
                    "UPDATE sim_schedule SET delivered = 1 WHERE request_id = ?1",
                    [report.request_id],
                )?;
            }
            Ok(())
        })
    }

    fn is_current(&self, report: &EngineReport) -> bool {
        report.epoch == self.epoch.load(Ordering::SeqCst)
    }
}
