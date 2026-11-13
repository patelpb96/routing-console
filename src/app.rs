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
    /// Opens the app on a store: seeds an empty one, counts the boot, and starts consuming
    /// engine reports. Must be called inside a tokio runtime.
    pub fn start(store: Store, clock: Clock) -> Result<Arc<App>, AppError> {
        let now = clock.now_ms();
        let (seed, boot) = store.transaction(|tx| {
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
            Ok((
                seed,
                Boot {
                    count,
                    started_at_ms: now,
                },
            ))
        })?;
        let (reports, mut incoming) = mpsc::unbounded_channel();
        let sim = Arc::new(Simulator::new(store.clone(), clock.clone(), seed, reports)?);
        let (changed, _) = broadcast::channel(64);
        let app = Arc::new(App {
            store,
            clock,
            engine: sim.clone(),
            sim,
            changed,
            boot,
        });
        // Weak, so dropping the app drops the simulator's sender and ends this loop.
        let weak = Arc::downgrade(&app);
        tokio::spawn(async move {
            while let Some(report) = incoming.recv().await {
                let Some(app) = weak.upgrade() else { break };
                if let Err(e) = app.apply_engine_result(report) {
                    eprintln!("engine result for request {}: {e}", report.request_id);
                }
            }
        });
        Ok(app)
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

    pub fn apply_engine_result(&self, report: EngineReport) -> Result<TaskView, AppError> {
        let now = self.clock.now_ms();
        let (task_id, step) = self.store.transaction(|tx| {
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
            Ok((task_id, decide_and_apply(tx, &facts, &cmd, now)?))
        })?;
        self.finish(task_id, step)
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
            Ok(snapshot::task_view(tx, facts, scenario, now)?)
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
