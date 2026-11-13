//! The routing engine boundary (contract section 3) and the seeded simulator behind it.
//!
//! The simulator stands in for an outside service: it keeps its decisions in its own tables
//! (`sim_schedule`, `sim_scenarios`) and reports back only through the channel, so the app learns
//! nothing about a request except what an engine would tell it.

use std::collections::HashMap;
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use tokio::sync::mpsc::UnboundedSender;

use crate::clock::Clock;
use crate::domain::{Area, EngineResult, Priority, vocab};
use crate::error::AppError;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub active_load: i64,
    pub capacity: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineRequest {
    pub request_id: i64,
    pub task_id: i64,
    pub area: Area,
    pub priority: Priority,
    pub eligible: Vec<Candidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineReport {
    pub request_id: i64,
    pub result: EngineResult,
}

/// Accepts a request now; reports on the channel it was built with later, or never.
pub trait RoutingEngine: Send + Sync {
    fn start(&self, request: EngineRequest) -> Result<(), AppError>;
}

vocab!(Scenario {
    Auto = "auto",
    FastMatch = "fast_match",
    SlowMatch = "slow_match",
    NoMatch = "no_match",
    NeverReturns = "never_returns",
});

pub const DEFAULT_SEED: u64 = 42;
const NEVER_RETURNS_IN_100: u64 = 20;
const AUTO_MIN_MS: u64 = 5_000;
const AUTO_MAX_MS: u64 = 20_000;

/// What the simulator will do with one request. `delay_ms` of `None` means it never reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub delay_ms: Option<u64>,
    pub result: EngineResult,
}

/// Pure: the same seed, request id, scenario and list give the same plan, whatever order
/// requests arrive in.
pub fn plan(seed: u64, request_id: i64, scenario: Scenario, eligible: &[Candidate]) -> Plan {
    let h = |salt: u64| splitmix64(seed ^ splitmix64((request_id as u64) ^ salt));
    let matched =
        || least_loaded(eligible, h(3)).map_or(EngineResult::NoMatch, EngineResult::Match);
    let (delay_ms, result) = match scenario {
        Scenario::FastMatch => (Some(6_000), matched()),
        Scenario::SlowMatch => (Some(26_000), matched()),
        Scenario::NoMatch => (Some(8_000), EngineResult::NoMatch),
        Scenario::NeverReturns => (None, EngineResult::NoMatch),
        Scenario::Auto if h(1) % 100 < NEVER_RETURNS_IN_100 => (None, EngineResult::NoMatch),
        Scenario::Auto => (
            Some(AUTO_MIN_MS + h(2) % (AUTO_MAX_MS - AUTO_MIN_MS + 1)),
            matched(),
        ),
    };
    Plan { delay_ms, result }
}

/// Lowest active load wins; a tie is broken by the hash so it does not always favour low ids.
fn least_loaded(eligible: &[Candidate], hash: u64) -> Option<i64> {
    let min = eligible.iter().map(|c| c.active_load).min()?;
    let ties: Vec<i64> = eligible
        .iter()
        .filter(|c| c.active_load == min)
        .map(|c| c.id)
        .collect();
    Some(ties[(hash % ties.len() as u64) as usize])
}

/// A fixed, well-mixed hash; std's `DefaultHasher` may change between Rust releases.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const SIM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sim_schedule (
    request_id     INTEGER PRIMARY KEY,
    task_id        INTEGER NOT NULL,
    due_at_ms      INTEGER,               -- NULL: never returns
    outcome        TEXT NOT NULL CHECK (outcome IN ('match', 'no_match', 'never')),
    contributor_id INTEGER,
    delivered      INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sim_scenarios (
    task_id  INTEGER PRIMARY KEY,
    scenario TEXT NOT NULL
);
"#;

pub struct Simulator {
    store: Store,
    clock: Clock,
    seed: u64,
    reports: UnboundedSender<EngineReport>,
}

impl Simulator {
    pub fn new(
        store: Store,
        clock: Clock,
        seed: u64,
        reports: UnboundedSender<EngineReport>,
    ) -> Result<Self, AppError> {
        store.transaction(|tx| Ok(tx.execute_batch(SIM_SCHEMA)?))?;
        Ok(Simulator {
            store,
            clock,
            seed,
            reports,
        })
    }

    pub fn seed(&self) -> u64 {
        self.seed
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

    fn mark_delivered(store: &Store, request_id: i64) -> Result<(), AppError> {
        store.transaction(|tx| {
            tx.execute(
                "UPDATE sim_schedule SET delivered = 1 WHERE request_id = ?1",
                [request_id],
            )?;
            Ok(())
        })
    }
}

impl RoutingEngine for Simulator {
    fn start(&self, request: EngineRequest) -> Result<(), AppError> {
        let scenario = self.scenario(request.task_id)?;
        let p = plan(self.seed, request.request_id, scenario, &request.eligible);
        let now = self.clock.now_ms();
        let (outcome, contributor) = match (p.delay_ms, p.result) {
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
                    p.delay_ms.map(|d| now + d as i64),
                    outcome,
                    contributor
                ],
            )?;
            Ok(())
        })?;
        let Some(delay) = p.delay_ms else {
            return Ok(());
        };
        // The deadline is fixed here, not at the task's first poll, so a test that advances
        // the paused clock before the task runs still sees the timer fire on time.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(delay);
        let store = self.store.clone();
        let reports = self.reports.clone();
        let report = EngineReport {
            request_id: request.request_id,
            result: p.result,
        };
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if let Err(e) = Simulator::mark_delivered(&store, report.request_id) {
                eprintln!(
                    "simulator: could not mark request {} delivered: {e}",
                    report.request_id
                );
            }
            // A closed channel means the app is shutting down; nothing is left to tell.
            let _ = reports.send(report);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: i64, load: i64) -> Candidate {
        Candidate {
            id,
            active_load: load,
            capacity: 2,
        }
    }

    #[test]
    fn scenarios_fix_the_delay_and_outcome() {
        let list = [c(1, 1), c(2, 0)];
        assert_eq!(
            plan(42, 1, Scenario::FastMatch, &list),
            Plan {
                delay_ms: Some(6_000),
                result: EngineResult::Match(2)
            }
        );
        assert_eq!(
            plan(42, 1, Scenario::SlowMatch, &list).delay_ms,
            Some(26_000)
        );
        assert_eq!(
            plan(42, 1, Scenario::NoMatch, &list),
            Plan {
                delay_ms: Some(8_000),
                result: EngineResult::NoMatch
            }
        );
        assert_eq!(plan(42, 1, Scenario::NeverReturns, &list).delay_ms, None);
        assert_eq!(
            plan(42, 1, Scenario::FastMatch, &[]).result,
            EngineResult::NoMatch
        );
    }

    #[test]
    fn auto_stays_inside_the_normal_window_and_about_a_fifth_never_return() {
        let list = [c(1, 0)];
        let plans: Vec<Plan> = (1..=1_000)
            .map(|id| plan(7, id, Scenario::Auto, &list))
            .collect();
        let never = plans.iter().filter(|p| p.delay_ms.is_none()).count();
        assert!(
            (150..=250).contains(&never),
            "{never} of 1000 never returned"
        );
        for d in plans.iter().filter_map(|p| p.delay_ms) {
            assert!((AUTO_MIN_MS..=AUTO_MAX_MS).contains(&d), "{d}");
        }
    }
}
