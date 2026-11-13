//! Slice S2: requests that never return, results that come back too late, operator overrides
//! and their refusals. Paused clock, driven through the library.

mod common;

use common::*;
use routing_console::app::App;
use routing_console::clock::Clock;
use routing_console::domain::{AttentionReason, EventKind, Source, TaskState};
use routing_console::error::AppError;
use routing_console::store::{Store, write};

const HEALTHCARE: [i64; 3] = [2, 3, 7]; // Ben Okafor, Clara Voss, Grace Lin

fn start() -> (std::sync::Arc<App>, Store) {
    let store = Store::open_in_memory().unwrap();
    let app = App::start(store.clone(), Clock::system()).unwrap();
    (app, store)
}

/// What the simulator decided for a request, read from its own table.
fn engine_pick(store: &Store, request_id: i64) -> i64 {
    store
        .transaction(|tx| {
            Ok(tx.query_row(
                "SELECT contributor_id FROM sim_schedule WHERE request_id = ?1",
                [request_id],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

/// (status, result_kind, result_contributor_id) of a request row.
fn request_row(store: &Store, id: i64) -> (String, Option<String>, Option<i64>) {
    store
        .transaction(|tx| {
            Ok(tx.query_row(
                "SELECT status, result_kind, result_contributor_id FROM routing_requests
                 WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap()
}

fn name(app: &App, id: i64) -> String {
    let snap = app.snapshot().unwrap();
    snap.contributors
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .name
        .clone()
}

fn refused(r: Result<routing_console::snapshot::TaskView, AppError>) -> String {
    match r {
        Err(AppError::Refused(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn never_returns_stays_routing_with_nothing_added() {
    let (app, _) = start();
    let t = submitted(&app, "Never", "finance", "never_returns");
    assert_eq!((t.state, t.version), (TaskState::Routing, 2));
    let events = t.events.len();

    advance_ms(20_000).await;
    let p = app.task(t.id).unwrap().pending_request.unwrap();
    assert_eq!(p.elapsed_ms, 20_000);
    assert!(!p.past_normal_window, "exactly 20000 ms is still inside");
    advance_ms(1).await;
    let p = app.task(t.id).unwrap().pending_request.unwrap();
    assert_eq!(p.elapsed_ms, 20_001);
    assert!(p.past_normal_window);

    advance_ms(10 * 60_000 - 20_001).await;
    let now = app.task(t.id).unwrap();
    assert_eq!((now.state, now.version), (TaskState::Routing, 2));
    assert_eq!(now.events.len(), events, "{:?}", kinds(&now));
    let p = now.pending_request.unwrap();
    assert_eq!(p.elapsed_ms, 600_000);
    assert!(p.past_normal_window);
}

#[tokio::test(start_paused = true)]
async fn a_late_result_after_assign_by_hand_is_recorded_not_applied() {
    let (app, store) = start();
    let t = submitted(&app, "Late", "healthcare", "slow_match");
    let request = t.pending_request.clone().unwrap().id;
    let picked = engine_pick(&store, request);
    let other = *HEALTHCARE.iter().find(|&&c| c != picked).unwrap();

    advance_ms(21_000).await;
    let t = app
        .command(t.id, assign(2, other, Some("engine_too_slow")))
        .unwrap();
    assert_eq!((t.state, t.version), (TaskState::Offered, 3));
    let offer = t.active_assignment.clone().unwrap();
    assert_eq!(offer.source, Source::Operator);
    assert_eq!(offer.contributor_id, other);
    assert_eq!(request_row(&store, request).0, "superseded");

    advance_ms(5_000).await; // 26 s: the engine answers the replaced request
    let after = app.task(t.id).unwrap();
    assert_eq!(count(&after, EventKind::LateResultIgnored), 1);
    assert_eq!(after.events.len(), t.events.len() + 1);
    assert_eq!((after.state, after.version), (TaskState::Offered, 3));
    assert_eq!(after.active_assignment.as_ref().unwrap().id, offer.id);
    let late = after.events.last().unwrap();
    assert_eq!(
        late.summary,
        format!(
            "The engine returned {} for a request you had already replaced by assigning {}. \
             Recorded, not applied.",
            name(&app, picked),
            name(&app, other)
        )
    );
    assert_eq!(
        request_row(&store, request),
        ("superseded".into(), Some("match".into()), Some(picked))
    );

    // Nothing comes again, and the version seen before the late result is still current.
    advance_ms(60_000).await;
    assert_eq!(app.task(t.id).unwrap().events.len(), after.events.len());
    match app.command(t.id, assign(3, picked, Some("better_fit_known"))) {
        Err(AppError::Refused(m)) => assert!(m.contains("offered"), "{m}"), // S3 adds this row
        other => panic!("version 3 must not be stale: {other:?}"),
    }
    let done = app.reply(offer.id, "accept").unwrap();
    assert_eq!((done.state, done.version), (TaskState::Assigned, 4));
}

#[tokio::test(start_paused = true)]
async fn route_again_ignores_the_old_result_and_applies_the_new_one() {
    let (app, store) = start();
    let t = submitted(&app, "Again", "healthcare", "slow_match");
    let old = t.pending_request.clone().unwrap().id;

    advance_ms(21_000).await;
    app.set_scenario(t.id, "fast_match").unwrap();
    let t = app
        .command(t.id, with_reason("route_again", 2, "engine_too_slow"))
        .unwrap();
    assert_eq!((t.state, t.version), (TaskState::Routing, 2));
    let new = t.pending_request.clone().unwrap();
    assert_ne!(new.id, old);
    assert_eq!(new.elapsed_ms, 0);

    advance_ms(5_000).await; // 26 s: the old one answers
    let mid = app.task(t.id).unwrap();
    assert_eq!((mid.state, mid.version), (TaskState::Routing, 2));
    assert_eq!(count(&mid, EventKind::LateResultIgnored), 1);
    assert!(
        mid.events
            .last()
            .unwrap()
            .summary
            .contains("replaced by routing again")
    );
    assert_eq!(request_row(&store, old).1.as_deref(), Some("match"));

    advance_ms(1_000).await; // 27 s: the new one answers
    let end = app.task(t.id).unwrap();
    assert_eq!((end.state, end.version), (TaskState::Offered, 3));
    assert_eq!(end.events.last().unwrap().kind, EventKind::EngineMatched);
    assert_eq!(end.events.last().unwrap().data["request_id"], new.id);
    assert_eq!(count(&end, EventKind::LateResultIgnored), 1);
}

#[tokio::test(start_paused = true)]
async fn cancel_routing_then_the_late_result_then_submit_again() {
    let (app, _) = start();
    let t = submitted(&app, "Cancel", "finance", "slow_match");

    advance_ms(10_000).await;
    let t = app
        .command(t.id, with_reason("cancel_routing", 2, "other"))
        .unwrap();
    assert_eq!((t.state, t.version), (TaskState::Unassigned, 3));
    assert!(t.pending_request.is_none());

    advance_ms(16_000).await; // 26 s
    let late = app.task(t.id).unwrap();
    assert_eq!((late.state, late.version), (TaskState::Unassigned, 3));
    assert_eq!(count(&late, EventKind::LateResultIgnored), 1);
    assert!(
        late.events
            .last()
            .unwrap()
            .summary
            .contains("you had already cancelled")
    );

    // Submitted again with the version read before the late result arrived.
    app.set_scenario(t.id, "fast_match").unwrap();
    let again = app.command(t.id, command("submit", 3)).unwrap();
    assert_eq!((again.state, again.version), (TaskState::Routing, 4));
    advance_ms(6_000).await;
    assert_eq!(app.task(t.id).unwrap().state, TaskState::Offered);
}

#[tokio::test(start_paused = true)]
async fn two_tasks_one_contributor_with_room_for_one() {
    let store = Store::open_in_memory().unwrap();
    store
        .transaction(|tx| {
            write::insert_contributor(
                tx,
                "Iris Vale",
                &[routing_console::domain::Area::Veterinary],
                1,
            )?;
            Ok(())
        })
        .unwrap();
    let app = App::start(store, Clock::system()).unwrap();
    let a = submitted(&app, "First", "veterinary", "fast_match");
    let b = submitted(&app, "Second", "veterinary", "fast_match");

    advance_ms(6_000).await;
    let mut both = [app.task(a.id).unwrap(), app.task(b.id).unwrap()];
    both.sort_by_key(|t| t.state != TaskState::Offered);
    let [offered, full] = both;
    assert_eq!(offered.state, TaskState::Offered);
    assert_eq!(full.state, TaskState::NeedsAttention);
    assert_eq!(
        full.attention_reason,
        Some(AttentionReason::SelectionUnavailable)
    );
    let e = full.events.last().unwrap();
    assert_eq!(e.kind, EventKind::EngineSelectionUnavailable);
    assert!(e.summary.contains("Iris Vale") && e.summary.contains("no room"));
}

#[tokio::test(start_paused = true)]
async fn refusals_leave_the_task_as_it_was() {
    let (app, _) = start();
    let t = submitted(&app, "Refuse", "finance", "never_returns");
    // Elena Ruiz (5) covers finance with room for one: fill her with another task.
    let other = submitted(&app, "Filler", "finance", "never_returns");
    app.command(other.id, assign(2, 5, Some("priority")))
        .unwrap();
    let before = app.task(t.id).unwrap();

    for kind in ["route_again", "cancel_routing"] {
        assert!(refused(app.command(t.id, command(kind, 2))).contains("reason code"));
    }
    assert!(refused(app.command(t.id, assign(2, 1, None))).contains("reason code"));
    let m = refused(app.command(t.id, assign(2, 2, Some("other")))); // Ben: healthcare only
    assert!(m.contains("does not cover finance"), "{m}");
    let m = refused(app.command(t.id, assign(2, 5, Some("other"))));
    assert!(m.contains("no room"), "{m}");
    refused(app.command(t.id, assign(2, 99, Some("other"))));
    refused(app.command(t.id, with_reason("route_again", 2, "made_up")));
    match app.command(t.id, with_reason("route_again", 1, "other")) {
        Err(AppError::Stale(current)) => {
            assert_eq!((current.state, current.version), (TaskState::Routing, 2));
            assert_eq!(current.id, t.id);
        }
        other => panic!("expected stale, got {other:?}"),
    }

    let after = app.task(t.id).unwrap();
    assert_eq!((after.state, after.version), (before.state, before.version));
    assert_eq!(after.events.len(), before.events.len());
    assert_eq!(
        after.pending_request.unwrap().id,
        before.pending_request.unwrap().id
    );
}

#[tokio::test(start_paused = true)]
async fn assignable_ids_follow_the_assign_rule() {
    let (app, _) = start();
    // Grace Lin (7) is the one healthcare contributor with room for one: fill her.
    let filler = submitted(&app, "Filler", "healthcare", "never_returns");
    app.command(filler.id, assign(2, 7, Some("priority")))
        .unwrap();
    let t = submitted(&app, "Pick", "healthcare", "never_returns");

    let snap = app.snapshot().unwrap();
    let view = snap.tasks.iter().find(|v| v.id == t.id).unwrap();
    let expected: Vec<i64> = snap
        .contributors
        .iter()
        .filter(|c| c.areas.contains(&view.area) && c.active_load < c.capacity)
        .map(|c| c.id)
        .collect();
    assert_eq!(view.assignable_contributor_ids, expected);
    assert_eq!(view.assignable_contributor_ids, vec![2, 3]);
    assert!(!view.assignable_contributor_ids.contains(&7), "full");
    assert!(
        !view.assignable_contributor_ids.contains(&1),
        "no healthcare"
    );

    // Offered: assign is not among its commands in this slice, so the list is empty.
    let offered = snap.tasks.iter().find(|v| v.id == filler.id).unwrap();
    assert!(
        !offered
            .allowed_commands
            .iter()
            .any(|k| k.as_str() == "assign")
    );
    assert!(offered.assignable_contributor_ids.is_empty());
}
