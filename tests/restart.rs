//! Slice S2: stopping and starting the app on the same database file while a request is out.
//! The clock carries on across the restart, as the wall clock would.

mod common;

use common::*;
use routing_console::app::App;
use routing_console::clock::Clock;
use routing_console::domain::{Actor, EventKind, TaskState};

#[tokio::test(start_paused = true)]
async fn restart_before_the_due_time_notes_it_then_applies_the_result() {
    let db = TempDb::new("restart-before");
    let clock = Clock::system();
    let app = App::start_blank(db.open(), clock.clone()).unwrap();
    let t = submitted(&app, "Before", "finance", "slow_match");

    advance_ms(5_000).await;
    let app = restart(app, &db, &clock).await;
    assert_eq!(app.snapshot().unwrap().boot.count, 2);
    let noted = app.task(t.id).unwrap();
    assert_eq!((noted.state, noted.version), (TaskState::Routing, 2));
    let e = noted.events.last().unwrap();
    assert_eq!(
        (e.kind, e.actor),
        (EventKind::ServerRestarted, Actor::System)
    );
    assert_eq!(e.data["boot"], 2);
    assert_eq!(count(&noted, EventKind::ServerRestarted), 1);

    advance_ms(20_999).await; // 25.999 s
    assert_eq!(app.task(t.id).unwrap().state, TaskState::Routing);
    advance_ms(1).await; // 26 s
    let done = app.task(t.id).unwrap();
    assert_eq!((done.state, done.version), (TaskState::Offered, 3));
    assert_eq!(done.events.last().unwrap().kind, EventKind::EngineMatched);
}

#[tokio::test(start_paused = true)]
async fn restart_after_the_due_time_delivers_the_overdue_result_at_once() {
    let db = TempDb::new("restart-after");
    let clock = Clock::system();
    let app = App::start_blank(db.open(), clock.clone()).unwrap();
    let t = submitted(&app, "After", "finance", "slow_match");

    advance_ms(5_000).await;
    drop(app);
    settle().await;
    advance_ms(30_000).await; // down while the result fell due
    let app = App::start_blank(db.open(), clock.clone()).unwrap();
    settle().await;
    let done = app.task(t.id).unwrap();
    assert_eq!((done.state, done.version), (TaskState::Offered, 3));
    assert_eq!(
        kinds(&done)[2..],
        [EventKind::ServerRestarted, EventKind::EngineMatched]
    );
    assert_eq!(done.events.last().unwrap().at_ms, clock.now_ms());

    // Delivered once: a further restart has nothing left to send.
    let app = restart(app, &db, &clock).await;
    advance_ms(60_000).await;
    assert_eq!(app.task(t.id).unwrap().events.len(), done.events.len());
}

#[tokio::test(start_paused = true)]
async fn an_assignment_made_before_the_restart_survives_the_late_result() {
    let db = TempDb::new("restart-assigned");
    let clock = Clock::system();
    let app = App::start_blank(db.open(), clock.clone()).unwrap();
    let t = submitted(&app, "Assigned", "finance", "slow_match");

    advance_ms(21_000).await;
    let t = app
        .command(t.id, assign(2, 5, Some("engine_too_slow")))
        .unwrap();
    assert_eq!((t.state, t.version), (TaskState::Offered, 3));
    advance_ms(1_000).await;
    let app = restart(app, &db, &clock).await;
    let after_restart = app.task(t.id).unwrap();
    assert_eq!(
        count(&after_restart, EventKind::ServerRestarted),
        0,
        "nothing pending, nothing to note"
    );

    advance_ms(4_000).await; // 26 s
    let late = app.task(t.id).unwrap();
    assert_eq!((late.state, late.version), (TaskState::Offered, 3));
    assert_eq!(late.active_assignment.as_ref().unwrap().contributor_id, 5);
    assert_eq!(count(&late, EventKind::LateResultIgnored), 1);
    assert!(
        late.events
            .last()
            .unwrap()
            .summary
            .contains("assigning Elena Ruiz")
    );
}

#[tokio::test(start_paused = true)]
async fn never_returns_stays_routing_across_a_restart() {
    let db = TempDb::new("restart-never");
    let clock = Clock::system();
    let app = App::start_blank(db.open(), clock.clone()).unwrap();
    let t = submitted(&app, "Never", "operations", "never_returns");

    advance_ms(5_000).await;
    let app = restart(app, &db, &clock).await;
    advance_ms(10 * 60_000).await;
    let still = app.task(t.id).unwrap();
    assert_eq!((still.state, still.version), (TaskState::Routing, 2));
    assert_eq!(
        kinds(&still),
        [
            EventKind::TaskCreated,
            EventKind::RoutingStarted,
            EventKind::ServerRestarted
        ]
    );
    assert!(still.pending_request.unwrap().past_normal_window);
    drop(app);
    settle().await;
    let path = db.path.clone();
    drop(db);
    assert!(!path.exists(), "the test database is removed");
}

#[tokio::test(start_paused = true)]
async fn a_restart_with_the_seeded_board_does_not_seed_it_twice() {
    let db = TempDb::new("restart-seeded");
    let clock = Clock::system();
    let app = App::start(db.open(), clock.clone()).unwrap();
    let before = app.snapshot().unwrap();
    assert_eq!(before.tasks.len(), 6);

    let app = restart(app, &db, &clock).await;
    let after = app.snapshot().unwrap();
    assert_eq!(after.contributors.len(), 8);
    let ids = |s: &routing_console::snapshot::Snapshot| -> Vec<(i64, String)> {
        s.tasks.iter().map(|t| (t.id, t.title.clone())).collect()
    };
    assert_eq!(ids(&after), ids(&before));
    // Only the healthcare task had a request out, so only it notes the restart.
    let noted: Vec<&str> = after
        .tasks
        .iter()
        .filter(|t| count(t, EventKind::ServerRestarted) > 0)
        .map(|t| t.title.as_str())
        .collect();
    assert_eq!(noted, vec![routing_console::seed::HEALTHCARE]);
}
