//! Slice S1: one task from create to assigned, on real SQLite and the paused tokio clock,
//! driven through the library rather than HTTP.

use std::collections::BTreeSet;
use std::time::Duration;

use routing_console::app::{App, CommandInput, CreateTask};
use routing_console::clock::Clock;
use routing_console::domain::{EngineResult, EventKind, Source, TaskState};
use routing_console::engine::{Candidate, Scenario, plan};
use routing_console::error::AppError;
use routing_console::store::Store;
use serde_json::Value;

fn submit(expected_version: i64) -> CommandInput {
    CommandInput {
        kind: "submit".into(),
        expected_version,
        contributor_id: None,
        reason_code: None,
        note: None,
        exclude_previous: false,
    }
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("expected an object, got {v}"))
        .keys()
        .cloned()
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn snapshot_task(app: &App, id: i64) -> (Value, Value) {
    let snap = serde_json::to_value(app.snapshot().unwrap()).unwrap();
    let task = snap["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .cloned()
        .unwrap();
    (snap, task)
}

const TASK_FIELDS: &[&str] = &[
    "id",
    "title",
    "area",
    "priority",
    "state",
    "version",
    "attention_reason",
    "created_at_ms",
    "updated_at_ms",
    "pending_request",
    "active_assignment",
    "scenario",
    "allowed_commands",
    "assignable_contributor_ids",
    "events",
];

#[tokio::test(start_paused = true)]
async fn one_task_goes_from_create_to_assigned() {
    let app = App::start(Store::open_in_memory().unwrap(), Clock::system()).unwrap();

    let task = app
        .create_task(CreateTask {
            title: "30-minute interview on month-end close".into(),
            area: "finance".into(),
            priority: "normal".into(),
            submit: false,
        })
        .unwrap();
    assert_eq!(task.state, TaskState::Unassigned);
    assert_eq!(task.version, 1);
    app.set_scenario(task.id, "fast_match").unwrap();

    let task = app.command(task.id, submit(1)).unwrap();
    assert_eq!(task.state, TaskState::Routing);
    assert_eq!(task.version, 2);
    let pending = task.pending_request.clone().expect("a pending request");
    assert!(task.active_assignment.is_none());

    // While routing: every contract field is present, the pending request is filled in.
    let (snap, json) = snapshot_task(&app, task.id);
    assert_eq!(
        keys(&snap),
        set(&[
            "now_ms",
            "boot",
            "sim",
            "normal_window_ms",
            "areas",
            "reason_codes",
            "contributors",
            "tasks"
        ])
    );
    assert_eq!(keys(&json), set(TASK_FIELDS));
    assert_eq!(
        keys(&json["pending_request"]),
        set(&["id", "started_at_ms", "elapsed_ms", "past_normal_window"])
    );
    assert!(json["active_assignment"].is_null());
    assert!(json["attention_reason"].is_null());
    assert_eq!(json["scenario"], "fast_match");
    assert_eq!(
        keys(&json["events"][0]),
        set(&["id", "at_ms", "kind", "actor", "summary", "data"])
    );
    assert_eq!(snap["contributors"].as_array().unwrap().len(), 8);

    // Not yet at 5 s.
    tokio::time::advance(Duration::from_secs(5)).await;
    let task = app.task(task.id).unwrap();
    assert_eq!(task.state, TaskState::Routing);
    assert_eq!(task.pending_request.unwrap().elapsed_ms, 5_000);

    // Past 6 s the simulator reports and the app applies the match.
    let mut changed = app.subscribe();
    tokio::time::advance(Duration::from_secs(2)).await;
    changed.recv().await.unwrap();
    let task = app.task(task.id).unwrap();
    assert_eq!(task.state, TaskState::Offered);
    assert_eq!(task.version, 3);
    assert!(task.pending_request.is_none());
    let offer = task.active_assignment.clone().expect("an offer");
    assert_eq!(offer.source, Source::Engine);
    let snap = app.snapshot().unwrap();
    let who = snap
        .contributors
        .iter()
        .find(|c| c.id == offer.contributor_id)
        .unwrap();
    assert!(who.areas.contains(&task.area), "{} lacks finance", who.name);
    assert_eq!(task.events[2].data["request_id"], pending.id);

    let task = app.reply(offer.id, "accept").unwrap();
    assert_eq!(task.state, TaskState::Assigned);
    assert_eq!(task.version, 4);
    let kinds: Vec<EventKind> = task.events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::TaskCreated,
            EventKind::RoutingStarted,
            EventKind::EngineMatched,
            EventKind::ContributorAccepted
        ]
    );

    let (_, json) = snapshot_task(&app, task.id);
    assert_eq!(keys(&json), set(TASK_FIELDS));
    assert_eq!(
        keys(&json["active_assignment"]),
        set(&["id", "contributor_id", "status", "source", "reason_code"])
    );
    assert_eq!(json["active_assignment"]["status"], "accepted");
    assert!(json["pending_request"].is_null());

    // A command against an old version is refused as stale and carries the current task.
    match app.command(task.id, submit(1)) {
        Err(AppError::Stale(current)) => {
            assert_eq!(current.version, 4);
            assert_eq!(current.state, TaskState::Assigned);
        }
        other => panic!("expected stale, got {other:?}"),
    }
    assert_eq!(app.task(task.id).unwrap().version, 4);
}

#[test]
fn the_simulator_decides_the_same_way_every_time() {
    let list = [
        Candidate {
            id: 1,
            active_load: 0,
            capacity: 2,
        },
        Candidate {
            id: 2,
            active_load: 0,
            capacity: 1,
        },
        Candidate {
            id: 3,
            active_load: 1,
            capacity: 2,
        },
    ];
    for seed in [42, 7] {
        for request_id in 1..=50 {
            for scenario in Scenario::ALL {
                let first = plan(seed, request_id, *scenario, &list);
                let again = plan(seed, request_id, *scenario, &list);
                assert_eq!(
                    first, again,
                    "seed {seed}, request {request_id}, {scenario}"
                );
                if let EngineResult::Match(id) = first.result {
                    assert!(id == 1 || id == 2, "picked a more loaded contributor {id}");
                }
            }
        }
    }
    let by_seed = |seed| -> Vec<_> {
        (1..=50)
            .map(|r| plan(seed, r, Scenario::Auto, &list))
            .collect()
    };
    assert_ne!(
        by_seed(42),
        by_seed(7),
        "the seed should change the decisions"
    );
}
