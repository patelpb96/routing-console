//! Random command runs (contract section 4). Each seed drives the app through a sequence of
//! random steps on the paused clock; after every step the nine invariants are checked by reading
//! the tables directly, and the snapshot must agree with the tables.
//!
//! `INVARIANT_SEEDS` and `INVARIANT_STEPS` raise the default 150 seeds of 80 steps.

#[path = "../common/mod.rs"]
mod common;
mod rules;
mod tables;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use routing_console::app::{App, CommandInput, CreateTask};
use routing_console::clock::Clock;
use routing_console::domain::{Area, OperatorKind, ReasonCode};
use routing_console::engine::Scenario;
use routing_console::error::AppError;
use routing_console::store::Store;
use rules::{agrees, check};
use tables::{Db, dump};

/// xorshift64*: small, deterministic, no new dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
    fn chance(&mut self, in_100: u64) -> bool {
        self.below(100) < in_100
    }
}

#[derive(Debug, Clone)]
enum Step {
    Create {
        area: Area,
        submit: bool,
    },
    Command {
        task: i64,
        input: CommandInput,
    },
    Reply {
        assignment: i64,
        reply: &'static str,
    },
    Scenario {
        task: i64,
        scenario: Scenario,
    },
    Advance {
        ms: u64,
    },
    Restart,
}

// ---- The run ----------------------------------------------------------------------------

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn random_step(rng: &mut Rng, db: &Db, restarts: bool) -> Step {
    let task = |rng: &mut Rng| {
        let n = db.tasks.len() as u64;
        if n == 0 || rng.chance(5) {
            (n + 1) as i64
        } else {
            (rng.below(n) + 1) as i64
        }
    };
    let roll = rng.below(100);
    match roll {
        0..10 => Step::Create {
            area: rng.pick(Area::ALL),
            submit: rng.chance(50),
        },
        10..45 => {
            let task = task(rng);
            let version = db.task(task).map_or(1, |t| t.version);
            let version = match rng.below(10) {
                0 => version - 1,
                1 => version + 1,
                _ => version,
            };
            let kind = rng.pick(OperatorKind::ALL).as_str().to_string();
            let contributor_id = match rng.below(10) {
                0 => None,
                1 => Some(99),
                _ => Some(db.contributors[rng.below(db.contributors.len() as u64) as usize].id),
            };
            let reason_code =
                (!rng.chance(20)).then(|| rng.pick(ReasonCode::ALL).as_str().to_string());
            Step::Command {
                task,
                input: CommandInput {
                    kind,
                    expected_version: version,
                    contributor_id,
                    reason_code,
                    note: rng.chance(10).then(|| "random note".to_string()),
                    exclude_previous: rng.chance(15),
                },
            }
        }
        45..60 => Step::Reply {
            assignment: (rng.below(db.assignments.len() as u64 + 2) + 1) as i64,
            reply: rng.pick(&["accept", "accept", "decline", "fail"]),
        },
        60..70 => Step::Scenario {
            task: task(rng),
            scenario: rng.pick(Scenario::ALL),
        },
        70..95 => Step::Advance {
            ms: rng.below(30_001),
        },
        _ if restarts => Step::Restart,
        _ => Step::Advance { ms: 0 },
    }
}

fn is_refusal<T>(r: &Result<T, AppError>) -> Result<bool, String> {
    match r {
        Ok(_) => Ok(false),
        Err(AppError::Refused(_) | AppError::Stale(_) | AppError::NotFound(_)) => Ok(true),
        Err(e) => Err(format!("internal error: {e}")),
    }
}

async fn run(seed: u64, steps: u64, seen: &mut BTreeMap<String, u64>) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    // A third of the seeds run on a file and restart on it; the rest stay in memory.
    let file = seed
        .is_multiple_of(3)
        .then(|| TempDb::new(&format!("invariants-{seed}")));
    let clock = Clock::system();
    let mut store = match &file {
        Some(db) => db.open(),
        None => Store::open_in_memory().unwrap(),
    };
    let mut app: Arc<App> = App::start(store.clone(), clock.clone()).unwrap();
    let mut log: Vec<Step> = Vec::new();
    let mut before = dump(&store, clock.now_ms());
    for n in 0..steps {
        let step = random_step(&mut rng, &before, file.is_some());
        log.push(step.clone());
        let fail = |n: u64, log: &[Step], why: String| {
            let steps: Vec<String> = log
                .iter()
                .enumerate()
                .map(|(i, s)| format!("  {i}: {s:?}"))
                .collect();
            format!(
                "seed {seed}, step {n}: {why}\nsteps so far:\n{}",
                steps.join("\n")
            )
        };
        let refused = match step {
            Step::Create { area, submit } => is_refusal(&app.create_task(CreateTask {
                title: format!("Task {n}"),
                area: area.as_str().into(),
                priority: "normal".into(),
                submit,
            })),
            Step::Command { task, input } => is_refusal(&app.command(task, input)),
            Step::Reply { assignment, reply } => is_refusal(&app.reply(assignment, reply)),
            Step::Scenario { task, scenario } => {
                is_refusal(&app.set_scenario(task, scenario.as_str()))
            }
            Step::Advance { ms } => {
                tokio::time::advance(std::time::Duration::from_millis(ms)).await;
                Ok(false)
            }
            Step::Restart => {
                let db = file.as_ref().unwrap();
                drop(app);
                drop(store);
                settle().await;
                store = db.open();
                app = App::start(store.clone(), clock.clone()).unwrap();
                Ok(false)
            }
        }
        .map_err(|e| fail(n, &log, e))?;
        if refused {
            let right_after = dump(&store, clock.now_ms());
            if right_after != before {
                return Err(fail(n, &log, "a refusal changed the tables".into()));
            }
        }
        settle().await;
        let after = dump(&store, clock.now_ms());
        check(&before, &after).map_err(|e| fail(n, &log, e))?;
        agrees(&app, &after).map_err(|e| fail(n, &log, e))?;
        before = after;
    }
    for e in &before.events {
        *seen.entry(e.kind.clone()).or_default() += 1;
    }
    drop(app);
    drop(store);
    settle().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn random_command_runs_keep_every_invariant() {
    let seeds = env("INVARIANT_SEEDS", 150);
    let steps = env("INVARIANT_STEPS", 80);
    let started = std::time::Instant::now();
    let mut seen: BTreeMap<String, u64> = BTreeMap::new();
    for seed in 1..=seeds {
        if let Err(report) = run(seed, steps, &mut seen).await {
            panic!("{report}");
        }
    }
    eprintln!(
        "invariants: {seeds} seeds x {steps} steps in {:.2?}; events written: {seen:?}",
        started.elapsed()
    );
    // The runs must reach the rows they are meant to test, or passing proves little.
    for kind in [
        "engine_matched",
        "engine_selection_unavailable",
        "engine_no_match",
        "late_result_ignored",
        "server_restarted",
        "operator_assigned",
        "operator_rerouted",
        "operator_cancelled_routing",
        "contributor_accepted",
    ] {
        assert!(
            seen.get(kind).copied().unwrap_or(0) > 0,
            "no {kind} in any run"
        );
    }
}
