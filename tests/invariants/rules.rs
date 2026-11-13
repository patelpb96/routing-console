//! The nine invariants of contract section 4, and the snapshot-agrees check.

use std::collections::HashMap;

use routing_console::app::App;
use routing_console::domain::OperatorKind;

use crate::tables::*;

pub fn check(before: &Db, after: &Db) -> Result<(), String> {
    for t in &after.tasks {
        let active = after.active(t.id);
        let pending = after.pending(t.id);
        // 1 and 2
        if active.len() > 1 {
            return Err(format!(
                "1: task {} has {} active assignments",
                t.id,
                active.len()
            ));
        }
        if pending.len() > 1 {
            return Err(format!(
                "2: task {} has {} pending requests",
                t.id,
                pending.len()
            ));
        }
        // 3
        let status = active.first().map(|a| a.status.as_str());
        let ok = match t.state.as_str() {
            "routing" => pending.len() == 1 && status.is_none(),
            "offered" => pending.is_empty() && status == Some("offered"),
            "assigned" => pending.is_empty() && status == Some("accepted"),
            "unassigned" | "needs_attention" | "closed" => pending.is_empty() && status.is_none(),
            s => return Err(format!("3: task {} has unknown state {s}", t.id)),
        };
        if !ok {
            return Err(format!(
                "3: task {} is {} with {} pending and active {status:?}",
                t.id,
                t.state,
                pending.len()
            ));
        }
        if (t.state == "needs_attention") != t.attention.is_some() {
            return Err(format!(
                "3: task {} is {} with reason {:?}",
                t.id, t.state, t.attention
            ));
        }
        // 5
        if let Some(a) = active.first() {
            let c = after
                .contributors
                .iter()
                .find(|c| c.id == a.contributor_id)
                .unwrap();
            if !c.areas.contains(&t.area) {
                return Err(format!(
                    "5: task {} held by {} without {}",
                    t.id, c.id, t.area
                ));
            }
        }
    }
    // 4
    for c in &after.contributors {
        if after.load(c.id) > c.capacity {
            return Err(format!(
                "4: contributor {} has load {} over {}",
                c.id,
                after.load(c.id),
                c.capacity
            ));
        }
    }
    // 6 and 9, task by task
    for b in &before.tasks {
        let a = after
            .task(b.id)
            .ok_or(format!("task {} disappeared", b.id))?;
        if a.version < b.version {
            return Err(format!(
                "6: task {} version went down {} -> {}",
                b.id, b.version, a.version
            ));
        }
        let holder = |db: &Db| db.active(b.id).first().map(|x| (x.id, x.status.clone()));
        let pending = |db: &Db| db.pending(b.id).first().map(|r| r.id);
        let changed = a.state != b.state
            || holder(before) != holder(after)
            || pending(before) != pending(after);
        if changed != (a.version != b.version) {
            return Err(format!(
                "6: task {} {} -> {} (holder {:?} -> {:?}, pending {:?} -> {:?}) but version \
                 {} -> {}",
                b.id,
                b.state,
                a.state,
                holder(before),
                holder(after),
                pending(before),
                pending(after),
                b.version,
                a.version
            ));
        }
        if b.state == "closed" && a.state != "closed" {
            return Err(format!("9: task {} left closed", b.id));
        }
    }
    // 7: a result for a superseded request touches its own row and adds one event, nothing else.
    let new = after.new_events(before);
    for r in &before.requests {
        let now = after.requests.iter().find(|x| x.id == r.id).unwrap();
        if r.status != "pending" && r != now {
            let late_record = r.status == "superseded"
                && r.returned_at_ms.is_none()
                && now.status == "superseded"
                && now.returned_at_ms.is_some();
            if !late_record {
                return Err(format!("7: ended request changed: {r:?} -> {now:?}"));
            }
            let told = new
                .iter()
                .filter(|e| e.kind == "late_result_ignored" && event_request_id(e) == Some(r.id))
                .count();
            if told != 1 {
                return Err(format!(
                    "7: request {} got a late result and {told} events",
                    r.id
                ));
            }
        }
    }
    for e in new.iter().filter(|e| e.kind == "late_result_ignored") {
        let r = event_request_id(e);
        let ok = before
            .requests
            .iter()
            .any(|x| Some(x.id) == r && x.status == "superseded" && x.returned_at_ms.is_none());
        if !ok {
            return Err(format!(
                "7: late_result_ignored for request {r:?} not awaiting one"
            ));
        }
    }
    for b in &before.tasks {
        let mine: Vec<&EventRow> = new.iter().filter(|e| e.task_id == b.id).collect();
        let notes_only = !mine.is_empty()
            && mine
                .iter()
                .all(|e| e.kind == "late_result_ignored" || e.kind == "server_restarted");
        if notes_only {
            let same = before.task(b.id) == after.task(b.id)
                && before.active(b.id) == after.active(b.id)
                && before.pending(b.id) == after.pending(b.id);
            if !same {
                return Err(format!("7: task {} changed by a note-only step", b.id));
            }
        }
    }
    // The engine is never handed, and so never offers, a contributor its request left out;
    // and an ended assignment has an end time.
    for a in &after.assignments {
        if let Some(r) = a
            .request_id
            .and_then(|id| after.requests.iter().find(|r| r.id == id))
            && r.excluded.contains(&a.contributor_id)
        {
            return Err(format!(
                "exclude: assignment {} went to {}, left out of request {}",
                a.id, a.contributor_id, r.id
            ));
        }
        let ended = !matches!(a.status.as_str(), "offered" | "accepted");
        if ended != a.ended_at_ms.is_some() {
            return Err(format!(
                "assignment {} is {} with end {:?}",
                a.id, a.status, a.ended_at_ms
            ));
        }
    }
    // 8
    if after.events.len() < before.events.len()
        || after.events[..before.events.len()] != before.events[..]
    {
        return Err("8: an earlier event was changed or removed".into());
    }
    let mut last: HashMap<i64, i64> = HashMap::new();
    for e in &after.events {
        match last.get(&e.task_id) {
            None if e.kind != "task_created" => {
                return Err(format!("8: task {} starts with {}", e.task_id, e.kind));
            }
            Some(&t) if e.at_ms < t => {
                return Err(format!("8: event {} at {} before {t}", e.id, e.at_ms));
            }
            _ => {}
        }
        last.insert(e.task_id, e.at_ms);
    }
    // Every report that fell due was applied and acknowledged.
    if !after.undelivered_due.is_empty() {
        return Err(format!(
            "engine reports due but not applied: {:?}",
            after.undelivered_due
        ));
    }
    Ok(())
}

/// The snapshot shows what the tables hold.
pub fn agrees(app: &App, db: &Db) -> Result<(), String> {
    let snap = app.snapshot().map_err(|e| e.to_string())?;
    if snap.tasks.len() != db.tasks.len() {
        return Err("snapshot: task count differs".into());
    }
    for (v, t) in snap.tasks.iter().zip(&db.tasks) {
        let pending = db.pending(t.id).first().map(|r| r.id);
        let active = db.active(t.id).first().map(|a| a.id);
        if v.id != t.id
            || v.state.as_str() != t.state
            || v.version != t.version
            || v.pending_request.as_ref().map(|p| p.id) != pending
            || v.active_assignment.as_ref().map(|a| a.id) != active
        {
            return Err(format!(
                "snapshot disagrees on task {}: {v:?} vs {t:?}",
                t.id
            ));
        }
        let can_assign = v.allowed_commands.contains(&OperatorKind::Assign);
        let holder = db.active(t.id).first().map(|a| a.contributor_id);
        let expected: Vec<i64> = db
            .contributors
            .iter()
            .filter(|c| {
                can_assign
                    && c.areas.contains(&t.area)
                    && db.load(c.id) < c.capacity
                    && Some(c.id) != holder
            })
            .map(|c| c.id)
            .collect();
        if v.assignable_contributor_ids != expected {
            return Err(format!(
                "snapshot: task {} assignable {:?}, tables say {expected:?}",
                t.id, v.assignable_contributor_ids
            ));
        }
    }
    Ok(())
}
