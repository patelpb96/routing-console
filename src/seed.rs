//! Seed data of contract section 7: eight invented contributors, and six tasks that reach their
//! states by replaying real commands through the app against a clock moved by hand. No task,
//! request, assignment or event row is inserted directly.

use std::sync::Arc;

use rusqlite::Connection;

use crate::app::{App, CommandInput, CreateTask};
use crate::clock::Clock;
use crate::domain::Area::{self, *};
use crate::engine::{Scenario, Simulator};
use crate::error::AppError;
use crate::snapshot::{Boot, TaskView};
use crate::store::write::insert_contributor;
use crate::store::{Store, meta_get};

/// Invented people. Nobody has `veterinary`, so a veterinary task can never be matched.
pub const CONTRIBUTORS: [(&str, &[Area], i64); 8] = [
    ("Ada Brook", &[Finance, DataAnalysis], 2),
    ("Ben Okafor", &[Healthcare], 1),
    ("Clara Voss", &[Healthcare, Operations], 2),
    ("Dev Raman", &[DataAnalysis], 2),
    ("Elena Ruiz", &[Finance], 1),
    ("Farid Haddad", &[Operations, DataAnalysis, Finance], 2),
    ("Grace Lin", &[Healthcare, DataAnalysis], 1),
    ("Hugo Marsh", &[Operations], 1),
];

/// The six task titles, as contract section 7 names them.
pub const HEALTHCARE: &str = "Grade 40 model answers on hospital discharge planning";
pub const RED_TEAM: &str = "Red-team a tax-advice assistant, 25 prompts";
pub const INTERVIEW: &str = "30-minute interview on month-end close";
pub const TICKETS: &str = "Label 300 support tickets by root cause";
pub const SURVEY: &str = "Survey on warehouse shift scheduling tools";
pub const DAIRY: &str = "Annotate dairy-herd sensor clips for lameness";

/// Meta key set once a database has had its chance to be seeded with tasks.
pub const TASKS_SEEDED: &str = "tasks_seeded";

/// How far before "now" the seeded history starts, and when the healthcare request starts.
pub const HISTORY_MS: i64 = 15 * 60_000;
pub const HEALTHCARE_STARTED_BEFORE_NOW_MS: i64 = 95_000;

/// Seeds an empty database; leaves one that already has contributors alone.
pub fn contributors_if_empty(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM contributors", [], |r| r.get(0))?;
    if count == 0 {
        for (name, areas, capacity) in CONTRIBUTORS {
            insert_contributor(conn, name, areas, capacity)?;
        }
    }
    Ok(())
}

/// True for a database that has never been seeded with tasks and holds none.
pub fn tasks_wanted(conn: &Connection) -> rusqlite::Result<bool> {
    let tasks: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
    Ok(tasks == 0 && meta_get(conn, TASKS_SEEDED)?.is_none())
}

/// Replays the seeded history, ending with the healthcare request started
/// `HEALTHCARE_STARTED_BEFORE_NOW_MS` before `now_ms`. Runs on a second app over the same
/// store: same commands, same transactions, but its clock is moved by hand and its simulator
/// sets no timers; the replay delivers each engine report itself when the clock reaches it.
pub fn tasks(store: &Store, now_ms: i64, seed: u64) -> Result<(), AppError> {
    let clock = Clock::manual(now_ms - HISTORY_MS);
    let sim = Arc::new(Simulator::unarmed(store.clone(), clock.clone(), seed)?);
    let boot = Boot {
        count: 0,
        started_at_ms: now_ms,
    };
    let r = Replay {
        app: App::assemble(store.clone(), clock.clone(), sim.clone(), boot),
        clock,
        sim,
        now_ms,
    };
    let at = |before_now_s: i64| now_ms - before_now_s * 1_000;

    // 15 minutes ago: a survey written up and left unassigned.
    r.create(SURVEY, Operations, "normal")?;

    // An interview offered by the engine and accepted.
    r.wait_until(at(840))?;
    let interview = r.submitted(INTERVIEW, Finance, "normal", Scenario::FastMatch)?;
    r.wait_until(at(780))?;
    r.reply(interview, "accept")?;

    // A red-team task offered by the engine, then declined.
    r.wait_until(at(720))?;
    let red_team = r.submitted(RED_TEAM, Finance, "high", Scenario::FastMatch)?;
    r.wait_until(at(640))?;
    r.reply(red_team, "decline")?;

    // Ticket labelling offered by the engine, still waiting for an answer.
    r.wait_until(at(540))?;
    r.submitted(TICKETS, DataAnalysis, "normal", Scenario::FastMatch)?;

    // A veterinary task nobody covers.
    r.wait_until(at(420))?;
    r.submitted(DAIRY, Veterinary, "high", Scenario::NoMatch)?;

    // The healthcare task, routing on a request that will never come back.
    r.wait_until(at(HEALTHCARE_STARTED_BEFORE_NOW_MS / 1_000))?;
    r.submitted(HEALTHCARE, Healthcare, "high", Scenario::NeverReturns)?;
    Ok(())
}

struct Replay {
    app: App,
    clock: Clock,
    sim: Arc<Simulator>,
    now_ms: i64,
}

impl Replay {
    fn create(&self, title: &str, area: Area, priority: &str) -> Result<TaskView, AppError> {
        self.app.create_task(CreateTask {
            title: title.into(),
            area: area.as_str().into(),
            priority: priority.into(),
            submit: false,
        })
    }

    /// Created, given its scenario, submitted, and left until the engine has answered (or,
    /// for `never_returns`, not).
    fn submitted(
        &self,
        title: &str,
        area: Area,
        priority: &str,
        scenario: Scenario,
    ) -> Result<i64, AppError> {
        let t = self.create(title, area, priority)?;
        self.app.set_scenario(t.id, scenario.as_str())?;
        self.app.command(
            t.id,
            CommandInput {
                kind: "submit".into(),
                expected_version: t.version,
                contributor_id: None,
                reason_code: None,
                note: None,
                exclude_previous: false,
            },
        )?;
        if scenario != Scenario::NeverReturns {
            let due = self.sim.due(self.now_ms)?;
            let latest = due.iter().map(|(_, at)| *at).max().unwrap_or(0);
            self.wait_until(latest)?;
        }
        Ok(t.id)
    }

    /// The simulated contributor answers the task's active offer.
    fn reply(&self, task_id: i64, reply: &str) -> Result<(), AppError> {
        let offer = self.app.task(task_id)?.active_assignment.ok_or_else(|| {
            AppError::Internal(format!("seed: task {task_id} has no offer to {reply}"))
        })?;
        self.app.reply(offer.id, reply)?;
        Ok(())
    }

    /// Moves the clock forward to `t`, delivering each engine report as its time comes.
    fn wait_until(&self, t: i64) -> Result<(), AppError> {
        for (report, due) in self.sim.due(t)? {
            self.move_to(due)?;
            self.app.apply_engine_result(report)?;
        }
        self.move_to(t)
    }

    fn move_to(&self, t: i64) -> Result<(), AppError> {
        let now = self.clock.now_ms();
        if t > now {
            self.clock
                .advance(t - now)
                .map_err(|e| AppError::Internal(e.into()))?;
        }
        Ok(())
    }
}
