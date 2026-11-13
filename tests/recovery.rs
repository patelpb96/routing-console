//! Slice S3: what the operator can do once an offer is declined or fails, or the engine finds
//! nobody; reassigning; closing. Paused clock, driven through the library.

mod common;

use common::*;
use routing_console::app::App;
use routing_console::clock::Clock;
use routing_console::domain::{Area, AttentionReason, EventKind, OperatorKind, Source, TaskState};
use routing_console::error::AppError;
use routing_console::store::{Store, write};

const FINANCE: [i64; 3] = [1, 5, 6]; // Ada Brook, Elena Ruiz, Farid Haddad
const FAST: u64 = 6_000;

#[tokio::test(start_paused = true)]
async fn decline_then_route_again_leaving_out_the_decliners_until_someone_accepts() {
    let (app, store) = blank();
    let t = submitted(&app, "Ledger review", "finance", "fast_match");
    advance_ms(FAST).await;
    let mut t = app.task(t.id).unwrap();
    let mut decliners = Vec::new();

    // Two finance contributors decline in turn; each time the next request leaves out
    // everyone who declined so far.
    for _ in 0..2 {
        assert_eq!(t.state, TaskState::Offered);
        let offer = t.active_assignment.clone().unwrap();
        assert!(!decliners.contains(&offer.contributor_id));
        decliners.push(offer.contributor_id);

        let d = reply(&app, &t, "decline");
        assert_eq!(d.state, TaskState::NeedsAttention);
        assert_eq!(d.attention_reason, Some(AttentionReason::Declined));
        assert_eq!(d.version, t.version + 1);
        assert_eq!(assignment_row(&store, offer.id), ("declined".into(), true));
        assert_eq!(
            d.allowed_commands,
            vec![
                OperatorKind::RouteAgain,
                OperatorKind::Assign,
                OperatorKind::Close
            ]
        );

        let r = app.command(t.id, exclude_previous(d.version)).unwrap();
        assert_eq!((r.state, r.version), (TaskState::Routing, d.version + 1));
        let rerouted = r.events.last().unwrap();
        assert_eq!(rerouted.kind, EventKind::OperatorRerouted);
        assert_eq!(rerouted.data["excluded"], serde_json::json!(decliners));
        advance_ms(FAST).await;
        t = app.task(t.id).unwrap();
    }

    // The third finance contributor gets it and accepts.
    let last = t.active_assignment.clone().unwrap();
    let third = *FINANCE.iter().find(|c| !decliners.contains(c)).unwrap();
    assert_eq!(last.contributor_id, third);
    let done = reply(&app, &t, "accept");
    assert_eq!(done.state, TaskState::Assigned);
    // No decliner was offered the task again.
    let offers = offered_to(&store, t.id);
    assert_eq!(offers, vec![decliners[0], decliners[1], third]);
}

#[tokio::test(start_paused = true)]
async fn a_failed_offer_then_assign_by_hand_even_to_the_same_contributor() {
    let (app, store) = blank();
    let t = submitted(&app, "Chart audit", "healthcare", "fast_match");
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    let offer = t.active_assignment.clone().unwrap();

    let f = reply(&app, &t, "fail");
    assert_eq!(
        (f.state, f.attention_reason, f.version),
        (
            TaskState::NeedsAttention,
            Some(AttentionReason::OfferFailed),
            4
        )
    );
    assert_eq!(assignment_row(&store, offer.id), ("failed".into(), true));
    let failed = f.events.last().unwrap();
    assert_eq!(failed.kind, EventKind::OfferFailed);
    assert_eq!(
        failed.summary,
        format!(
            "The offer to {} could not be delivered. The task needs a decision: route again \
             without them, assign by hand, or close.",
            name(&app, offer.contributor_id)
        )
    );
    assert!(f.assignable_contributor_ids.contains(&offer.contributor_id));

    // Assigning by hand needs a reason, and may pick the contributor whose offer failed.
    refused(app.command(t.id, assign(4, offer.contributor_id, None)));
    let a = app
        .command(
            t.id,
            assign(4, offer.contributor_id, Some("contributor_unavailable")),
        )
        .unwrap();
    assert_eq!((a.state, a.version), (TaskState::Offered, 5));
    let by_hand = a.active_assignment.clone().unwrap();
    assert_eq!(by_hand.source, Source::Operator);
    assert_ne!(by_hand.id, offer.id);
    assert_eq!(
        a.events.last().unwrap().summary,
        format!(
            "Assigned by hand to {} (contributor unavailable).",
            name(&app, offer.contributor_id)
        )
    );
    let done = reply(&app, &a, "accept");
    assert_eq!((done.state, done.version), (TaskState::Assigned, 6));
}

#[tokio::test(start_paused = true)]
async fn veterinary_no_match_then_close_and_closed_refuses_everything() {
    let (app, store) = blank();
    let t = submitted(&app, "Hoof clips", "veterinary", "auto");
    let request = t.pending_request.clone().unwrap().id;
    // Nobody covers veterinary: unless this request is one that never returns, the engine
    // answers no match within the normal window.
    advance_ms(20_000).await;
    let t = app.task(t.id).unwrap();
    let t = if t.state == TaskState::Routing {
        assert_eq!(request_row(&store, request).0, "pending");
        app.set_scenario(t.id, "no_match").unwrap();
        app.command(
            t.id,
            with_reason("route_again", t.version, "engine_too_slow"),
        )
        .unwrap();
        advance_ms(8_000).await;
        app.task(t.id).unwrap()
    } else {
        t
    };
    assert_eq!(t.state, TaskState::NeedsAttention);
    assert_eq!(t.attention_reason, Some(AttentionReason::NoMatch));
    assert!(t.assignable_contributor_ids.is_empty(), "nobody covers it");

    let c = app.command(t.id, command("close", t.version)).unwrap();
    assert_eq!((c.state, c.version), (TaskState::Closed, t.version + 1));
    assert_eq!(c.attention_reason, None);
    assert_eq!(c.events.last().unwrap().summary, "Closed by the operator.");
    assert!(c.allowed_commands.is_empty());
    assert!(c.assignable_contributor_ids.is_empty());

    let v = c.version;
    for input in [
        command("submit", v),
        command("route_again", v),
        exclude_previous(v),
        with_reason("route_again", v, "priority"),
        with_reason("cancel_routing", v, "priority"),
        assign(v, 2, Some("priority")),
        command("close", v),
    ] {
        let m = refused(app.command(c.id, input));
        assert!(m.contains("closed"), "{m}");
    }
    let end = app.task(c.id).unwrap();
    assert_eq!((end.state, end.version), (TaskState::Closed, v));
    assert_eq!(end.events.len(), c.events.len());
}

#[tokio::test(start_paused = true)]
async fn reassign_from_offered_and_from_assigned_withdraws_the_old_assignment() {
    let (app, store) = blank();
    let t = submitted(&app, "Ticket triage", "data_analysis", "fast_match");
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    let first = t.active_assignment.clone().unwrap();
    let pick = |not: &[i64]| *[1, 4, 6, 7].iter().find(|c| !not.contains(c)).unwrap();

    // From offered.
    let second = pick(&[first.contributor_id]);
    let r = app
        .command(t.id, assign(t.version, second, Some("better_fit_known")))
        .unwrap();
    assert_eq!((r.state, r.version), (TaskState::Offered, t.version + 1));
    assert_eq!(assignment_row(&store, first.id), ("withdrawn".into(), true));
    assert_eq!(load(&app, first.contributor_id), 0);
    assert_eq!(load(&app, second), 1);
    let e = r.events.last().unwrap();
    assert_eq!(e.kind, EventKind::OperatorAssigned);
    assert_eq!(
        e.summary,
        format!(
            "Reassigned from {} to {} (better fit known).",
            name(&app, first.contributor_id),
            name(&app, second)
        )
    );
    assert_eq!(e.data["withdrawn_assignment_id"], first.id);

    // From assigned.
    let accepted = reply(&app, &r, "accept");
    assert_eq!(accepted.state, TaskState::Assigned);
    let held = accepted.active_assignment.clone().unwrap();
    let third = pick(&[first.contributor_id, second]);
    let r2 = app
        .command(t.id, assign(accepted.version, third, Some("priority")))
        .unwrap();
    assert_eq!(
        (r2.state, r2.version),
        (TaskState::Offered, accepted.version + 1)
    );
    assert_eq!(assignment_row(&store, held.id), ("withdrawn".into(), true));
    assert_eq!(load(&app, second), 0);
    assert_eq!(load(&app, third), 1);
    assert_eq!(r2.active_assignment.unwrap().contributor_id, third);
    // The withdrawn offer cannot be answered any more.
    refused(app.reply(first.id, "accept"));
}

#[tokio::test(start_paused = true)]
async fn reassign_refusals_leave_the_task_alone() {
    let (app, _) = blank();
    // Elena Ruiz (5, capacity 1) is made full on another task.
    let filler = submitted(&app, "Filler", "finance", "never_returns");
    app.command(filler.id, assign(2, 5, Some("priority")))
        .unwrap();
    let t = submitted(&app, "Accruals", "finance", "fast_match");
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    let holder = t.active_assignment.clone().unwrap().contributor_id;
    let other = *FINANCE.iter().find(|&&c| c != holder && c != 5).unwrap();

    let same = refused(app.command(t.id, assign(t.version, holder, Some("priority"))));
    assert!(same.contains("already holds"), "{same}");
    let no_code = refused(app.command(t.id, assign(t.version, other, None)));
    assert!(no_code.contains("reason code"), "{no_code}");
    let full = refused(app.command(t.id, assign(t.version, 5, Some("priority"))));
    assert!(full.contains("no room"), "{full}");
    let wrong = refused(app.command(t.id, assign(t.version, 2, Some("priority"))));
    assert!(wrong.contains("does not cover finance"), "{wrong}");
    match app.command(t.id, assign(t.version - 1, other, Some("priority"))) {
        Err(AppError::Stale(task)) => assert_eq!(task.version, t.version),
        r => panic!("expected stale, got {r:?}"),
    }
    let after = app.task(t.id).unwrap();
    assert_eq!((after.state, after.version), (t.state, t.version));
    assert_eq!(after.events.len(), t.events.len());
    let holder_of = |v: &routing_console::snapshot::TaskView| {
        v.active_assignment
            .as_ref()
            .map(|a| (a.id, a.contributor_id))
    };
    assert_eq!(holder_of(&after), holder_of(&t));
}

#[tokio::test(start_paused = true)]
async fn route_again_without_exclude_previous_may_offer_the_decliner_again() {
    // One finance contributor only, so the engine has nobody else to pick.
    let store = Store::open_in_memory().unwrap();
    store
        .transaction(|tx| {
            write::insert_contributor(tx, "Solo Finance", &[Area::Finance], 1)?;
            Ok(())
        })
        .unwrap();
    let app = App::start_blank(store, Clock::system()).unwrap();
    let t = submitted(&app, "Close the books", "finance", "fast_match");
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    let d = reply(&app, &t, "decline");

    let again = app
        .command(t.id, command("route_again", d.version))
        .unwrap();
    assert_eq!(
        again.events.last().unwrap().summary,
        "Routed again; everyone with the area is eligible."
    );
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    assert_eq!(t.state, TaskState::Offered);
    assert_eq!(t.active_assignment.clone().unwrap().contributor_id, 1);

    // Leaving the decliner out leaves nobody: the engine answers no match.
    let d = reply(&app, &t, "decline");
    let r = app.command(t.id, exclude_previous(d.version)).unwrap();
    assert_eq!(
        r.events.last().unwrap().summary,
        "Routed again, leaving out Solo Finance."
    );
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    assert_eq!(t.state, TaskState::NeedsAttention);
    assert_eq!(t.attention_reason, Some(AttentionReason::NoMatch));
}

/// The stored `excluded` list of a request row, as ids.
fn excluded_of(store: &Store, request_id: i64) -> Vec<i64> {
    let text: String = store
        .transaction(|tx| {
            Ok(tx.query_row(
                "SELECT excluded FROM routing_requests WHERE id = ?1",
                [request_id],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test(start_paused = true)]
async fn route_again_while_routing_keeps_leaving_out_the_decliner() {
    // Grace is the only healthcare contributor, so whether the engine may pick her decides
    // the outcome: offered to her again, or no match.
    let store = Store::open_in_memory().unwrap();
    let grace = store
        .transaction(|tx| {
            Ok(write::insert_contributor(
                tx,
                "Grace Lin",
                &[Area::Healthcare],
                2,
            )?)
        })
        .unwrap();
    let app = App::start_blank(store.clone(), Clock::system()).unwrap();
    let t = submitted(&app, "Ward rota", "healthcare", "fast_match");
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    assert_eq!(t.active_assignment.clone().unwrap().contributor_id, grace);

    // Grace declines; route again leaving her out. The engine is slow.
    let d = reply(&app, &t, "decline");
    app.set_scenario(t.id, "slow_match").unwrap();
    let r = app.command(t.id, exclude_previous(d.version)).unwrap();
    let first = r.pending_request.clone().unwrap().id;
    assert_eq!(excluded_of(&store, first), vec![grace]);
    advance_ms(21_000).await;
    let r = app.task(t.id).unwrap();
    assert_eq!(r.state, TaskState::Routing);

    // Too slow: route again. The new request still leaves Grace out.
    app.set_scenario(t.id, "fast_match").unwrap();
    let again = app
        .command(
            t.id,
            with_reason("route_again", r.version, "engine_too_slow"),
        )
        .unwrap();
    let second = again.pending_request.clone().unwrap().id;
    assert_ne!(second, first);
    assert_eq!(request_row(&store, first).0, "superseded");
    assert_eq!(excluded_of(&store, second), vec![grace]);
    let rerouted = again.events.last().unwrap();
    assert_eq!(rerouted.data["excluded"], serde_json::json!([grace]));
    assert!(
        rerouted.summary.contains("leaving out Grace Lin"),
        "{}",
        rerouted.summary
    );

    // The engine's candidates leave her out too: with nobody else, it answers no match.
    advance_ms(FAST).await;
    let t = app.task(t.id).unwrap();
    assert_eq!(request_row(&store, second).1.as_deref(), Some("no_match"));
    assert_eq!(t.state, TaskState::NeedsAttention);
    assert_eq!(t.attention_reason, Some(AttentionReason::NoMatch));
    assert_eq!(offered_to(&store, t.id), vec![grace]);
}
