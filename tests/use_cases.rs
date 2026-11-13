//! The four use cases of the brief, each told as a story on a paused clock, then the seeded
//! board and the simulation reset (slice S3).

mod common;

use common::*;
use routing_console::app::{App, CreateTask};
use routing_console::clock::Clock;
use routing_console::domain::{AttentionReason, EngineResult, EventKind, Source, TaskState};
use routing_console::engine::{EngineReport, Scenario};
use routing_console::error::AppError;
use routing_console::seed;
use routing_console::snapshot::{Snapshot, TaskView};
use routing_console::store::Store;

use EventKind::*;

fn high(app: &App, title: &str, area: &str, scenario: &str) -> TaskView {
    let t = app
        .create_task(CreateTask {
            title: title.into(),
            area: area.into(),
            priority: "high".into(),
            submit: false,
        })
        .unwrap();
    app.set_scenario(t.id, scenario).unwrap();
    app.command(t.id, command("submit", 1)).unwrap()
}

fn by_title<'a>(snap: &'a Snapshot, title: &str) -> &'a TaskView {
    snap.tasks.iter().find(|t| t.title == title).unwrap()
}

fn seeded() -> std::sync::Arc<App> {
    App::start(Store::open_in_memory().unwrap(), Clock::system()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn routine_assignment() {
    let (app, _) = blank();
    // A task comes in and is sent to the engine at once.
    let t = submitted(&app, "Review 50 loan summaries", "finance", "fast_match");
    assert_eq!((t.state, t.version), (TaskState::Routing, 2));

    // The engine finds a finance contributor with room and the task is offered to them.
    advance_ms(6_000).await;
    let offered = app.task(t.id).unwrap();
    assert_eq!((offered.state, offered.version), (TaskState::Offered, 3));
    let offer = offered.active_assignment.clone().unwrap();
    assert_eq!(offer.source, Source::Engine);
    assert!([1, 5, 6].contains(&offer.contributor_id));
    assert_eq!(load(&app, offer.contributor_id), 1);

    // They accept, and the work is theirs.
    let done = reply(&app, &offered, "accept");
    assert_eq!((done.state, done.version), (TaskState::Assigned, 4));
    assert_eq!(load(&app, offer.contributor_id), 1);
    assert_eq!(
        kinds(&done),
        vec![
            TaskCreated,
            RoutingStarted,
            EngineMatched,
            ContributorAccepted
        ]
    );
    assert!(
        done.events
            .last()
            .unwrap()
            .summary
            .contains(&name(&app, offer.contributor_id))
    );
}

#[tokio::test(start_paused = true)]
async fn unresolved_routing_of_a_high_priority_task() {
    let (app, _) = blank();
    // Two high-priority tasks: one engine answer is merely slow, the other never comes.
    let slow = high(&app, "Slow answer", "healthcare", "slow_match");
    let never = high(&app, "No answer", "healthcare", "never_returns");

    // Past the normal window, the operator sees the same thing for both: still routing,
    // flagged as overdue, and nothing in the history that tells them apart.
    advance_ms(21_000).await;
    let (s, n) = (app.task(slow.id).unwrap(), app.task(never.id).unwrap());
    for t in [&s, &n] {
        assert_eq!((t.state, t.version), (TaskState::Routing, 2));
        let p = t.pending_request.clone().unwrap();
        assert!(p.past_normal_window);
        assert_eq!(p.elapsed_ms, 21_000);
        assert_eq!(kinds(t), vec![TaskCreated, RoutingStarted]);
    }

    // The slow one answers at 26 s.
    advance_ms(5_000).await;
    assert_eq!(app.task(slow.id).unwrap().state, TaskState::Offered);

    // The other waits for as long as nobody acts: an hour later it is still routing and
    // nothing has been added.
    advance_ms(60 * 60_000).await;
    let n = app.task(never.id).unwrap();
    assert_eq!((n.state, n.version), (TaskState::Routing, 2));
    assert_eq!(kinds(&n), vec![TaskCreated, RoutingStarted]);
    assert!(n.pending_request.unwrap().elapsed_ms > 60 * 60_000);
    // What the operator can do about it.
    assert!(
        n.allowed_commands
            .iter()
            .any(|k| k.as_str() == "route_again")
    );
    assert!(n.allowed_commands.iter().any(|k| k.as_str() == "assign"));
}

#[tokio::test(start_paused = true)]
async fn human_intervention_while_pending_with_the_late_return() {
    let (app, store) = blank();
    let t = high(&app, "Urgent triage notes", "healthcare", "slow_match");
    let request = t.pending_request.clone().unwrap().id;
    let picked = engine_pick(&store, request);
    let chosen = *[2, 3, 7].iter().find(|&&c| c != picked).unwrap();

    // The operator does not wait: at 10 s they assign by hand.
    advance_ms(10_000).await;
    let by_hand = app
        .command(t.id, assign(2, chosen, Some("priority")))
        .unwrap();
    assert_eq!((by_hand.state, by_hand.version), (TaskState::Offered, 3));
    assert_eq!(request_row(&store, request).0, "superseded");

    // At 26 s the engine's answer arrives anyway: recorded, not applied.
    advance_ms(16_000).await;
    let late = app.task(t.id).unwrap();
    assert_eq!((late.state, late.version), (TaskState::Offered, 3));
    assert_eq!(
        late.active_assignment.as_ref().unwrap().contributor_id,
        chosen
    );
    assert_eq!(load(&app, picked), 0, "the engine's pick got nothing");
    let e = late.events.last().unwrap();
    assert_eq!(e.kind, LateResultIgnored);
    assert!(e.summary.contains(&name(&app, picked)), "{}", e.summary);

    let done = reply(&app, &late, "accept");
    assert_eq!(done.state, TaskState::Assigned);
    assert_eq!(
        kinds(&done),
        vec![
            TaskCreated,
            RoutingStarted,
            OperatorAssigned,
            LateResultIgnored,
            ContributorAccepted
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn recovery_after_a_decline() {
    let (app, store) = blank();
    let t = submitted(&app, "Quarter-end variance notes", "finance", "fast_match");
    advance_ms(6_000).await;
    let t = app.task(t.id).unwrap();
    let first = t.active_assignment.clone().unwrap();

    // The contributor declines. The task says why it stopped and what can be done.
    let d = reply(&app, &t, "decline");
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(d.attention_reason, Some(AttentionReason::Declined));
    let why = d.events.last().unwrap();
    assert_eq!(why.kind, ContributorDeclined);
    assert!(why.summary.starts_with(&name(&app, first.contributor_id)));
    assert!(
        why.summary
            .contains("route again without them, assign by hand, or close")
    );
    assert_eq!(load(&app, first.contributor_id), 0);

    // The operator routes again without them; someone else is offered it.
    let r = app.command(t.id, exclude_previous(d.version)).unwrap();
    assert_eq!(r.state, TaskState::Routing);
    advance_ms(6_000).await;
    let second = app.task(t.id).unwrap();
    assert_eq!(second.state, TaskState::Offered);
    let second_offer = second.active_assignment.clone().unwrap();
    assert_ne!(second_offer.contributor_id, first.contributor_id);

    // That offer cannot be delivered, so the operator picks someone by hand.
    let f = reply(&app, &second, "fail");
    assert_eq!(f.attention_reason, Some(AttentionReason::OfferFailed));
    let hand = *f
        .assignable_contributor_ids
        .iter()
        .find(|&&c| c != first.contributor_id && c != second_offer.contributor_id)
        .unwrap();
    let a = app
        .command(
            t.id,
            assign(f.version, hand, Some("contributor_unavailable")),
        )
        .unwrap();
    let done = reply(&app, &a, "accept");
    assert_eq!(done.state, TaskState::Assigned);
    assert_eq!(
        kinds(&done),
        vec![
            TaskCreated,
            RoutingStarted,
            EngineMatched,
            ContributorDeclined,
            OperatorRerouted,
            EngineMatched,
            OfferFailed,
            OperatorAssigned,
            ContributorAccepted
        ]
    );
    assert_eq!(
        offered_to(&store, t.id),
        vec![first.contributor_id, second_offer.contributor_id, hand]
    );
    assert_eq!(assignment_row(&store, first.id).0, "declined");
    assert_eq!(assignment_row(&store, second_offer.id).0, "failed");
}

#[tokio::test(start_paused = true)]
async fn seed_reaches_the_documented_states() {
    let app = seeded();
    let snap = app.snapshot().unwrap();
    assert_eq!(snap.tasks.len(), 6);
    assert_eq!(snap.contributors.len(), 8);
    let now = snap.now_ms;

    let expect = [
        (seed::HEALTHCARE, TaskState::Routing, None),
        (
            seed::RED_TEAM,
            TaskState::NeedsAttention,
            Some(AttentionReason::Declined),
        ),
        (seed::INTERVIEW, TaskState::Assigned, None),
        (seed::TICKETS, TaskState::Offered, None),
        (seed::SURVEY, TaskState::Unassigned, None),
        (
            seed::DAIRY,
            TaskState::NeedsAttention,
            Some(AttentionReason::NoMatch),
        ),
    ];
    for (title, state, reason) in expect {
        let t = by_title(&snap, title);
        assert_eq!((t.state, t.attention_reason), (state, reason), "{title}");
        assert_eq!(t.events[0].kind, TaskCreated, "{title}");
        assert!(
            t.events.windows(2).all(|w| w[0].at_ms <= w[1].at_ms),
            "{title}"
        );
        for e in &t.events {
            assert!(
                e.at_ms >= now - seed::HISTORY_MS && e.at_ms <= now,
                "{title}"
            );
            assert_ne!(e.kind, ServerRestarted, "{title}");
        }
    }

    let hc = by_title(&snap, seed::HEALTHCARE);
    assert_eq!(hc.scenario, Scenario::NeverReturns);
    let p = hc.pending_request.clone().unwrap();
    assert!(p.past_normal_window);
    assert_eq!(p.elapsed_ms, seed::HEALTHCARE_STARTED_BEFORE_NOW_MS);
    assert!(!hc.assignable_contributor_ids.is_empty());

    let red = by_title(&snap, seed::RED_TEAM);
    assert_eq!(
        kinds(red),
        vec![
            TaskCreated,
            RoutingStarted,
            EngineMatched,
            ContributorDeclined
        ]
    );
    assert!(!red.assignable_contributor_ids.is_empty());
    let interview = by_title(&snap, seed::INTERVIEW);
    assert_eq!(kinds(interview).last(), Some(&ContributorAccepted));
    assert_eq!(
        kinds(by_title(&snap, seed::DAIRY)).last(),
        Some(&EngineNoMatch)
    );
    assert_eq!(kinds(by_title(&snap, seed::SURVEY)), vec![TaskCreated]);
    assert!(by_title(&snap, seed::TICKETS).pending_request.is_none());

    // The seeded healthcare request never answers.
    advance_ms(10 * 60_000).await;
    assert_eq!(
        app.task(hc.id).unwrap().events.len(),
        hc.events.len(),
        "never_returns stays silent"
    );
}

/// What a snapshot holds, minus anything that depends on when it was taken.
fn board(s: &Snapshot) -> Vec<(i64, String, TaskState, Option<AttentionReason>, usize)> {
    s.tasks
        .iter()
        .map(|t| {
            (
                t.id,
                t.title.clone(),
                t.state,
                t.attention_reason,
                t.events.len(),
            )
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn reset_replaces_the_data() {
    let app = seeded();
    let fresh = app.snapshot().unwrap();
    // Change the board: a new task, the vet task closed, the offered task declined.
    submitted(&app, "Extra", "operations", "never_returns");
    let dairy = by_title(&fresh, seed::DAIRY);
    app.command(dairy.id, command("close", dairy.version))
        .unwrap();
    reply(&app, by_title(&fresh, seed::TICKETS), "decline");
    assert_eq!(app.snapshot().unwrap().tasks.len(), 7);

    let mut changed = app.subscribe();
    advance_ms(60_000).await;
    let after = app.reset(None).unwrap();
    assert!(changed.try_recv().is_ok(), "reset tells the page");
    assert_eq!(after.tasks.len(), 6);
    assert_eq!(
        after.sim.seed, fresh.sim.seed,
        "no seed given keeps the seed"
    );
    assert_eq!(board(&after), board(&fresh));
    assert_eq!(
        after.tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    let loads = |s: &Snapshot| {
        s.contributors
            .iter()
            .map(|c| (c.id, c.active_load))
            .collect::<Vec<_>>()
    };
    assert_eq!(loads(&after), loads(&fresh));
    // Times are relative to the reset, not to the first start.
    assert_eq!(
        by_title(&after, seed::HEALTHCARE)
            .pending_request
            .clone()
            .unwrap()
            .elapsed_ms,
        seed::HEALTHCARE_STARTED_BEFORE_NOW_MS
    );
}

#[tokio::test(start_paused = true)]
async fn a_stale_timer_cannot_deliver_into_the_new_data() {
    let app = seeded();
    let old = submitted(&app, "Before the reset", "healthcare", "slow_match");
    let old_request = old.pending_request.unwrap().id;

    advance_ms(5_000).await;
    app.reset(None).unwrap();
    // After the reset request ids start again, so a new request takes the same id.
    let new = submitted(&app, "After the reset", "healthcare", "never_returns");
    assert_eq!(new.pending_request.clone().unwrap().id, old_request);

    // The old timer was due at 26 s; nothing lands on the new request.
    advance_ms(30_000).await;
    let t = app.task(new.id).unwrap();
    assert_eq!((t.state, t.version), (TaskState::Routing, 2));
    assert_eq!(kinds(&t), vec![TaskCreated, RoutingStarted]);

    // A report the old run had already sent is refused the same way.
    let stale = EngineReport {
        request_id: old_request,
        result: EngineResult::Match(2),
        epoch: 0,
    };
    match app.apply_engine_result(stale) {
        Err(AppError::NotFound(m)) => assert!(m.contains("reset"), "{m}"),
        r => panic!("expected the stale report to be dropped, got {r:?}"),
    }
    let t = app.task(new.id).unwrap();
    assert_eq!((t.state, t.version), (TaskState::Routing, 2));
    assert_eq!(load(&app, 2), 0);
}

/// Resets with `seed`, routes one data-analysis task on `auto`, and says what became of it.
async fn auto_outcome(app: &App, seed: u64) -> (TaskState, Option<i64>, i64) {
    let snap = app.reset(Some(seed)).unwrap();
    assert_eq!(snap.sim.seed, seed);
    let t = submitted(app, "Auto", "data_analysis", "auto");
    let started = t.pending_request.unwrap().started_at_ms;
    advance_ms(21_000).await;
    let t = app.task(t.id).unwrap();
    let answered = t.events.last().unwrap().at_ms - started;
    (
        t.state,
        t.active_assignment.map(|a| a.contributor_id),
        answered,
    )
}

#[tokio::test(start_paused = true)]
async fn a_different_seed_changes_an_auto_outcome() {
    let app = seeded();
    let mut outcomes = Vec::new();
    for s in 1..=10 {
        outcomes.push(auto_outcome(&app, s).await);
    }
    let first = outcomes[0];
    assert!(
        outcomes.iter().any(|o| *o != first),
        "ten seeds, one outcome: {outcomes:?}"
    );
    // The same seed gives the same outcome again.
    assert_eq!(auto_outcome(&app, 1).await, first);
}
