//! Load, decide, persist, notify. Each method is one command: it loads one task inside one
//! transaction, asks `domain::decide`, writes what it returns, and after the commit tells the
//! page that something changed and hands any new request to the engine.

use std::sync::Arc;

use rusqlite::Connection;
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};

use crate::clock::Clock;
use crate::domain::{self, *};
use crate::engine::{
    Candidate, DEFAULT_SEED, EngineReport, EngineRequest, RoutingEngine, Scenario, Simulator,
};
use crate::error::AppError;
use crate::snapshot::{self, Boot, Snapshot, SnapshotInput, TaskView};
use crate::store::{Store, meta_get, meta_set, read, write};

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTask {
    pub title: String,
    pub area: String,
    pub priority: String,
    #[serde(default)]
    pub submit: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandInput {
    #[serde(rename = "type")]
    pub kind: String,
    pub expected_version: i64,
    pub contributor_id: Option<i64>,
    pub reason_code: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub exclude_previous: bool,
}

/// What a command's transaction ended in, short of an error that rolls it back.
enum Step {
    Applied(Option<EngineRequest>),
    Stale,
}

pub struct App {
    store: Store,
    clock: Clock,
    engine: Arc<dyn RoutingEngine>,
    sim: Arc<Simulator>,
    changed: broadcast::Sender<()>,
    boot: Boot,
}

impl App {
    /// Opens the app on a store: seeds an empty one (contributors and the six tasks of contract
    /// section 7), counts the boot, and starts consuming engine reports. Must be called inside
    /// a tokio runtime.
    pub fn start(store: Store, clock: Clock) -> Result<Arc<App>, AppError> {
        Self::open(store, clock, true)
    }

    /// Like `start`, but an empty database gets the contributors only, never the six seeded
    /// tasks, and is marked so a later `start` does not add them either. For tests that need
    /// a board they control.
    pub fn start_blank(store: Store, clock: Clock) -> Result<Arc<App>, AppError> {
        Self::open(store, clock, false)
    }

    fn open(store: Store, clock: Clock, seed_tasks: bool) -> Result<Arc<App>, AppError> {
        let now = clock.now_ms();
        let (seed, boot, fresh) = store.transaction(|tx| {
            crate::seed::contributors_if_empty(tx)?;
            let seed = match meta_get(tx, "seed")? {
                Some(s) => s.parse().map_err(|_| {
                    AppError::Internal(format!("stored seed {s:?} is not a number"))
                })?,
                None => {
                    meta_set(tx, "seed", &DEFAULT_SEED.to_string())?;
                    DEFAULT_SEED
                }
            };
            let count = meta_get(tx, "boot_count")?
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0)
                + 1;
            meta_set(tx, "boot_count", &count.to_string())?;
            meta_set(tx, "boot_started_at_ms", &now.to_string())?;
            // No marker: seeding never finished (or never ran). Start from scratch, and mark
            // it only once the seeding below has succeeded. A blank start marks at once.
            let fresh = !crate::seed::is_seeded(tx)?;
            if fresh && seed_tasks {
                crate::seed::clear_tasks(tx)?;
            } else if fresh {
                meta_set(tx, crate::seed::TASKS_SEEDED, "1")?;
            }
            note_restart(tx, count, now)?;
            Ok((
                seed,
                Boot {
                    count,
                    started_at_ms: now,
                },
                fresh,
            ))
        })?;
        if fresh && seed_tasks {
            crate::seed::tasks(&store, now, seed)?;
            store.transaction(|tx| Ok(meta_set(tx, crate::seed::TASKS_SEEDED, "1")?))?;
        }
        let (reports, mut incoming) = mpsc::unbounded_channel();
        let sim = Arc::new(Simulator::new(store.clone(), clock.clone(), seed, reports)?);
        let app = Arc::new(App::assemble(store, clock, sim, boot));
        // Weak, so dropping the app drops the simulator's sender and ends this loop.
        let weak = Arc::downgrade(&app);
        tokio::spawn(async move {
            while let Some(report) = incoming.recv().await {
                let Some(app) = weak.upgrade() else { break };
                if let Err(e) = app.deliver(report) {
                    eprintln!("engine result for request {}: {e}", report.request_id);
                }
            }
        });
        Ok(app)
    }

    /// An app without a report listener; `start` adds one, the seeder delivers by hand.
    pub(crate) fn assemble(store: Store, clock: Clock, sim: Arc<Simulator>, boot: Boot) -> App {
        let (changed, _) = broadcast::channel(64);
        App {
            store,
            clock,
            engine: sim.clone(),
            sim,
            changed,
            boot,
        }
    }

    /// `POST /api/sim/reset`: wipes every record (the simulator's tables included), stores
    /// the new seed when one is given, and seeds the contributors and the six tasks again.
    /// Timers of the old run are stopped, and any report one of them already sent is stale
    /// (see `RoutingEngine::is_current`), so nothing of the old run lands on the new data.
    pub fn reset(&self, seed: Option<u64>) -> Result<Snapshot, AppError> {
        self.store.transaction(|tx| {
            self.sim.wipe(tx, seed)?;
            tx.execute_batch(
                "DELETE FROM events; DELETE FROM assignments; DELETE FROM routing_requests;
                 DELETE FROM tasks; DELETE FROM contributors;",
            )?;
            crate::seed::contributors_if_empty(tx)?;
            if let Some(seed) = seed {
                meta_set(tx, "seed", &seed.to_string())?;
            }
            tx.execute(
                "DELETE FROM meta WHERE key = ?1",
                [crate::seed::TASKS_SEEDED],
            )?;
            Ok(())
        })?;
        crate::seed::tasks(&self.store, self.clock.now_ms(), self.sim.seed())?;
        self.store
            .transaction(|tx| Ok(meta_set(tx, crate::seed::TASKS_SEEDED, "1")?))?;
        // The seeder leaves nothing due, but a report it left waiting must still arrive.
        self.sim.rearm()?;
        self.notify();
        self.snapshot()
    }

    /// Fires once after every committed change.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn create_task(&self, input: CreateTask) -> Result<TaskView, AppError> {
        let area = Area::parse(&input.area)
            .ok_or_else(|| AppError::Refused(format!("Unknown area {:?}.", input.area)))?;
        let priority = Priority::parse(&input.priority)
            .ok_or_else(|| AppError::Refused(format!("Unknown priority {:?}.", input.priority)))?;
        let new = domain::create(&input.title, area, priority).map_err(refused)?;
        let now = self.clock.now_ms();
        let id = self
            .store
            .transaction(|tx| Ok(write::insert_task(tx, &new, now)?))?;
        self.notify();
        if !input.submit {
            return self.task(id);
        }
        // A second command in its own transaction, as if the operator pressed submit at once.
        self.command(
            id,
            CommandInput {
                kind: OperatorKind::Submit.as_str().into(),
                expected_version: 1,
                contributor_id: None,
                reason_code: None,
                note: None,
                exclude_previous: false,
            },
        )
    }

    pub fn command(&self, task_id: i64, input: CommandInput) -> Result<TaskView, AppError> {
        let kind = OperatorKind::parse(&input.kind)
            .ok_or_else(|| AppError::Refused(format!("Unknown command type {:?}.", input.kind)))?;
        let reason_code = input
            .reason_code
            .as_deref()
            .map(|s| {
                ReasonCode::parse(s)
                    .ok_or_else(|| AppError::Refused(format!("Unknown reason code {s:?}.")))
            })
            .transpose()?;
        let now = self.clock.now_ms();
        let step = self.store.transaction(|tx| {
            let facts = load(tx, task_id)?;
            let op = match kind {
                OperatorKind::Submit => OperatorOp::Submit,
                OperatorKind::RouteAgain => OperatorOp::RouteAgain {
                    reason_code,
                    exclude_previous: input.exclude_previous,
                },
                OperatorKind::CancelRouting => OperatorOp::CancelRouting { reason_code },
                OperatorKind::Assign => OperatorOp::Assign {
                    contributor: assignee(tx, input.contributor_id)?,
                    reason_code,
                },
                OperatorKind::Close => OperatorOp::Close,
            };
            let cmd = Command::Operator {
                expected_version: input.expected_version,
                op,
                note: input.note.clone(),
            };
            decide_and_apply(tx, &facts, &cmd, now)
        })?;
        self.finish(task_id, step)
    }

    /// The simulated contributor's answer to an offer.
    pub fn reply(&self, assignment_id: i64, reply: &str) -> Result<TaskView, AppError> {
        let reply = Reply::parse(reply)
            .ok_or_else(|| AppError::Refused(format!("Unknown reply {reply:?}.")))?;
        let now = self.clock.now_ms();
        let (task_id, step) = self.store.transaction(|tx| {
            let task_id = read::task_of_assignment(tx, assignment_id)?.ok_or_else(|| {
                AppError::Refused(format!(
                    "Assignment {assignment_id} is not an active offer."
                ))
            })?;
            let facts = load(tx, task_id)?;
            let cmd = Command::Reply {
                assignment_id,
                reply,
            };
            Ok((task_id, decide_and_apply(tx, &facts, &cmd, now)?))
        })?;
        self.finish(task_id, step)
    }

    /// Applies one engine report and acknowledges it, so the engine stops re-sending it. A
    /// refused report (a repeat after a restart, say) is acknowledged too: sending it again
    /// would be refused again. Only an internal error leaves it for the next start. A report
    /// from before a reset is dropped unread and unacknowledged, as `NotFound`.
    pub fn apply_engine_result(&self, report: EngineReport) -> Result<TaskView, AppError> {
        let outcome = self.engine_result(report);
        match &outcome {
            Err(AppError::Internal(_)) | Ok(None) => {}
            _ => self.engine.acknowledge(&report)?,
        }
        outcome?.ok_or_else(|| {
            AppError::NotFound(format!(
                "Report for request {} belongs to data wiped by a reset.",
                report.request_id
            ))
        })
    }

    /// The report listener's entry point.
    fn deliver(&self, report: EngineReport) -> Result<(), AppError> {
        match self.apply_engine_result(report) {
            Ok(_) | Err(AppError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn engine_result(&self, report: EngineReport) -> Result<Option<TaskView>, AppError> {
        let now = self.clock.now_ms();
        let applied = self.store.transaction(|tx| {
            // Under the store lock, which a reset holds while it wipes.
            if !self.engine.is_current(&report) {
                return Ok(None);
            }
            let (task_id, request) = read::request(tx, report.request_id)?.ok_or_else(|| {
                AppError::Internal(format!("unknown request {}", report.request_id))
            })?;
            let facts = load(tx, task_id)?;
            let contributor = match report.result {
                EngineResult::Match(id) => read::contributor(tx, id)?,
                EngineResult::NoMatch => None,
            };
            let cmd = Command::EngineReturned {
                request,
                result: report.result,
                contributor,
            };
            Ok(Some((task_id, decide_and_apply(tx, &facts, &cmd, now)?)))
        })?;
        match applied {
            None => Ok(None),
            Some((task_id, step)) => self.finish(task_id, step).map(Some),
        }
    }

    pub fn set_scenario(&self, task_id: i64, scenario: &str) -> Result<TaskView, AppError> {
        let scenario = Scenario::parse(scenario)
            .ok_or_else(|| AppError::Refused(format!("Unknown scenario {scenario:?}.")))?;
        self.store.transaction(|tx| load(tx, task_id).map(drop))?;
        self.sim.set_scenario(task_id, scenario)?;
        self.notify();
        self.task(task_id)
    }

    pub fn task(&self, task_id: i64) -> Result<TaskView, AppError> {
        let scenario = self.sim.scenario(task_id)?;
        let now = self.clock.now_ms();
        self.store.transaction(|tx| {
            let facts = load(tx, task_id)?;
            let contributors = read::contributors(tx)?;
            Ok(snapshot::task_view(
                tx,
                facts,
                scenario,
                &contributors,
                now,
            )?)
        })
    }

    pub fn snapshot(&self) -> Result<Snapshot, AppError> {
        let input = SnapshotInput {
            now_ms: self.clock.now_ms(),
            boot: self.boot.clone(),
            seed: self.sim.seed(),
            scenarios: self.sim.scenarios()?,
        };
        self.store.transaction(|tx| Ok(snapshot::build(tx, input)?))
    }

    /// `GET /api/overrides.jsonl`, read in one transaction.
    pub fn overrides_jsonl(&self) -> Result<String, AppError> {
        self.store
            .transaction(|tx| Ok(crate::overrides::jsonl(tx)?))
    }

    fn finish(&self, task_id: i64, step: Step) -> Result<TaskView, AppError> {
        match step {
            Step::Stale => Err(AppError::Stale(Box::new(self.task(task_id)?))),
            Step::Applied(start) => {
                self.notify();
                if let Some(request) = start {
                    self.engine.start(request)?;
                }
                self.task(task_id)
            }
        }
    }

    fn notify(&self) {
        // An error only means no page is listening right now.
        let _ = self.changed.send(());
    }
}

fn refused(r: Refusal) -> AppError {
    match r {
        Refusal::NotAllowed(m) => AppError::Refused(m),
        Refusal::Stale => AppError::Refused("The task changed; reload and try again.".into()),
    }
}

fn load(conn: &Connection, task_id: i64) -> Result<TaskFacts, AppError> {
    read::load_facts(conn, task_id)?
        .ok_or_else(|| AppError::NotFound(format!("No task with id {task_id}.")))
}

/// Contract: on start, one `server_restarted` event on every task still waiting on the engine.
/// The request itself stays open; nothing else is rebuilt.
fn note_restart(conn: &Connection, boot: i64, now: i64) -> Result<(), AppError> {
    for facts in read::all_facts(conn)? {
        if facts.pending.is_none() {
            continue;
        }
        let decision =
            domain::decide(&facts, &Command::ServerRestarted { boot }).map_err(refused)?;
        write::apply(conn, &facts, &decision, now)?;
    }
    Ok(())
}

fn assignee(conn: &Connection, id: Option<i64>) -> Result<ContributorFacts, AppError> {
    let id = id.ok_or_else(|| AppError::Refused("assign needs a contributor_id.".into()))?;
    read::contributor(conn, id)?
        .ok_or_else(|| AppError::Refused(format!("No contributor with id {id}.")))
}

/// The heart of every command. Runs inside the caller's transaction; a refusal returns an
/// error so nothing is written, a stale version is reported without writing anything.
fn decide_and_apply(
    conn: &Connection,
    facts: &TaskFacts,
    cmd: &Command,
    now: i64,
) -> Result<Step, AppError> {
    let decision = match domain::decide(facts, cmd) {
        Ok(d) => d,
        Err(Refusal::Stale) => return Ok(Step::Stale),
        Err(r) => return Err(refused(r)),
    };
    let applied = write::apply(conn, facts, &decision, now)?;
    let Some(request_id) = applied.opened_request else {
        return Ok(Step::Applied(None));
    };
    let excluded = decision
        .changes
        .iter()
        .find_map(|c| match c {
            Change::OpenRequest { excluded } => Some(excluded.as_slice()),
            _ => None,
        })
        .unwrap_or_default();
    let eligible = read::eligible(conn, facts.task.area, excluded)?
        .into_iter()
        .map(|c| Candidate {
            id: c.id,
            active_load: c.active_load,
            capacity: c.capacity,
        })
        .collect();
    Ok(Step::Applied(Some(EngineRequest {
        request_id,
        task_id: facts.task.id,
        area: facts.task.area,
        priority: facts.task.priority,
        eligible,
    })))
}
