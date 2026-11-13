//! Slice S6: the overrides export, one JSON line per operator command with a reason code,
//! joined to what the engine said about the request it set aside. Paused clock.

mod common;

use common::*;
use routing_console::app::App;
use routing_console::clock::Clock;
use routing_console::store::Store;
use serde_json::Value;

const FIELDS: [&str; 19] = [
    "event_id",
    "at_ms",
    "task_id",
    "task_title",
    "area",
    "priority",
    "command",
    "reason_code",
    "note",
    "chosen_contributor_id",
    "chosen_contributor_name",
    "replaced_contributor_id",
    "superseded_request_id",
    "request_elapsed_ms",
    "engine_result",
    "engine_contributor_id",
    "engine_contributor_name",
    "engine_returned_at_ms",
    "agreed",
];

/// Parses the export line by line, checking each line stands alone, has every field, and
/// that the lines come in event order.
fn export(app: &App) -> Vec<Value> {
    let body = app.overrides_jsonl().unwrap();
    assert!(body.is_empty() || body.ends_with('\n'));
    let lines: Vec<Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is JSON on its own"))
        .collect();
    for l in &lines {
        let obj = l.as_object().unwrap();
        assert_eq!(obj.len(), FIELDS.len(), "{l}");
        for f in FIELDS {
            assert!(obj.contains_key(f), "missing {f} in {l}");
        }
    }
    let ids: Vec<i64> = lines
        .iter()
        .map(|l| l["event_id"].as_i64().unwrap())
        .collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    lines
}

#[tokio::test(start_paused = true)]
async fn the_late_return_story_turns_none_yet_into_match() {
    let (app, store) = blank();
    let a = submitted(&app, "Late A", "healthcare", "slow_match");
    let b = submitted(&app, "Late B", "healthcare", "slow_match");
    let (req_a, req_b) = (
        a.pending_request.clone().unwrap().id,
        b.pending_request.clone().unwrap().id,
    );
    let (pick_a, pick_b) = (engine_pick(&store, req_a), engine_pick(&store, req_b));
    // A goes to someone the engine did not pick for either; B to the engine's own pick.
    let other = *[2, 3, 7]
        .iter()
        .find(|&&c| c != pick_a && c != pick_b)
        .unwrap();
    let note = "Waited \"long enough\"\nsecond line";

    advance_ms(21_000).await;
    let mut cmd = assign(2, other, Some("engine_too_slow"));
    cmd.note = Some(note.into());
    app.command(a.id, cmd).unwrap();
    app.command(b.id, assign(2, pick_b, Some("engine_too_slow")))
        .unwrap();

    let before = export(&app);
    assert_eq!(before.len(), 2);
    let l = &before[0];
    assert_eq!(l["task_id"], a.id);
    assert_eq!(l["task_title"], "Late A");
    assert_eq!(
        (l["area"].as_str(), l["priority"].as_str()),
        (Some("healthcare"), Some("normal"))
    );
    assert_eq!(l["command"], "assign");
    assert_eq!(l["reason_code"], "engine_too_slow");
    assert_eq!(l["note"], note);
    assert_eq!(l["chosen_contributor_id"], other);
    assert_eq!(l["chosen_contributor_name"], name(&app, other));
    assert_eq!(l["replaced_contributor_id"], Value::Null);
    assert_eq!(l["superseded_request_id"], req_a);
    assert_eq!(l["request_elapsed_ms"], 21_000);
    assert_eq!(l["engine_result"], "none_yet");
    assert_eq!(l["engine_contributor_id"], Value::Null);
    assert_eq!(l["engine_returned_at_ms"], Value::Null);
    assert_eq!(l["agreed"], Value::Null);
    assert_eq!(before[1]["engine_result"], "none_yet");

    advance_ms(5_000).await; // 26 s: both late results arrive
    let after = export(&app);
    assert_eq!(after.len(), 2);
    let l = &after[0];
    assert_eq!(l["event_id"], before[0]["event_id"]);
    assert_eq!(l["engine_result"], "match");
    assert_eq!(l["engine_contributor_id"], pick_a);
    assert_eq!(l["engine_contributor_name"], name(&app, pick_a));
    assert_eq!(
        l["engine_returned_at_ms"],
        l["at_ms"].as_i64().unwrap() + 5_000
    );
    assert_eq!(l["agreed"], other == pick_a);
    assert_eq!(l["agreed"], false);
    let l = &after[1];
    assert_eq!(
        (l["engine_contributor_id"].as_i64(), l["agreed"].as_bool()),
        (Some(pick_b), Some(true))
    );
}

#[tokio::test(start_paused = true)]
async fn route_again_and_cancel_while_pending_name_the_request_set_aside() {
    let (app, _) = blank();
    let r = submitted(&app, "Reroute", "finance", "never_returns");
    let c = submitted(&app, "Cancel", "finance", "never_returns");
    let (req_r, req_c) = (r.pending_request.unwrap().id, c.pending_request.unwrap().id);
    advance_ms(10_000).await;
    app.command(r.id, with_reason("route_again", 2, "priority"))
        .unwrap();
    app.command(c.id, with_reason("cancel_routing", 2, "other"))
        .unwrap();

    let lines = export(&app);
    assert_eq!(lines.len(), 2);
    for (l, cmd, req, code) in [
        (&lines[0], "route_again", req_r, "priority"),
        (&lines[1], "cancel_routing", req_c, "other"),
    ] {
        assert_eq!(l["command"], cmd);
        assert_eq!(l["reason_code"], code);
        assert_eq!(l["superseded_request_id"], req);
        assert_eq!(l["request_elapsed_ms"], 10_000);
        assert_eq!(l["engine_result"], "none_yet");
        assert_eq!(l["chosen_contributor_id"], Value::Null);
        assert_eq!(l["chosen_contributor_name"], Value::Null);
        assert_eq!(l["agreed"], Value::Null);
        assert_eq!(l["note"], Value::Null);
    }
}

#[tokio::test(start_paused = true)]
async fn assign_from_attention_then_reassign_from_assigned() {
    let (app, _) = blank();
    let t = submitted(&app, "Recover", "finance", "fast_match");
    advance_ms(6_000).await;
    let t = app.task(t.id).unwrap();
    let decliner = t.active_assignment.clone().unwrap().contributor_id;
    let d = reply(&app, &t, "decline");

    let hand = *[1, 5, 6].iter().find(|&&c| c != decliner).unwrap();
    let a = app
        .command(
            t.id,
            assign(d.version, hand, Some("contributor_unavailable")),
        )
        .unwrap();
    let held = reply(&app, &a, "accept");
    let third = *[1, 5, 6]
        .iter()
        .find(|&&c| c != decliner && c != hand)
        .unwrap();
    app.command(t.id, assign(held.version, third, Some("better_fit_known")))
        .unwrap();

    let lines = export(&app);
    assert_eq!(lines.len(), 2);
    let l = &lines[0];
    assert_eq!(l["command"], "assign");
    assert_eq!(l["chosen_contributor_id"], hand);
    assert_eq!(l["replaced_contributor_id"], Value::Null);
    for f in [
        "superseded_request_id",
        "request_elapsed_ms",
        "engine_result",
        "agreed",
    ] {
        assert_eq!(l[f], Value::Null, "{f}");
    }
    let l = &lines[1];
    assert_eq!(l["reason_code"], "better_fit_known");
    assert_eq!(l["chosen_contributor_id"], third);
    assert_eq!(l["replaced_contributor_id"], hand);
    assert_eq!(l["engine_result"], Value::Null);
}

#[tokio::test(start_paused = true)]
async fn route_again_from_attention_without_a_reason_is_not_an_override() {
    let (app, _) = blank();
    let t = submitted(&app, "No reason", "finance", "fast_match");
    advance_ms(6_000).await;
    let t = app.task(t.id).unwrap();
    let d = reply(&app, &t, "decline");
    let r = app.command(t.id, exclude_previous(d.version)).unwrap();
    assert!(export(&app).is_empty());

    // The same command with a reason code is one.
    advance_ms(6_000).await;
    let t = app.task(r.id).unwrap();
    let d = reply(&app, &t, "decline");
    app.command(t.id, with_reason("route_again", d.version, "other"))
        .unwrap();
    let lines = export(&app);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["command"], "route_again");
    assert_eq!(lines[0]["superseded_request_id"], Value::Null);
    assert_eq!(lines[0]["engine_result"], Value::Null);
}

#[tokio::test(start_paused = true)]
async fn the_seeded_board_alone_exports_nothing() {
    let app = App::start(Store::open_in_memory().unwrap(), Clock::system()).unwrap();
    assert_eq!(app.snapshot().unwrap().tasks.len(), 6);
    assert_eq!(app.overrides_jsonl().unwrap(), "");
}
