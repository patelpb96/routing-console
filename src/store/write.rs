//! Writes a decision's record changes and events. Called inside `Store::transaction`.

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::domain::*;

/// Ids SQLite handed out while applying a decision.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub opened_request: Option<i64>,
    pub offered_assignment: Option<i64>,
}

pub fn insert_task(conn: &Connection, t: &NewTask, now: i64) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO tasks (title, area, priority, state, version, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, 'unassigned', 1, ?4, ?4)",
        params![t.title, t.area.as_str(), t.priority.as_str(), now],
    )?;
    let id = conn.last_insert_rowid();
    insert_event(conn, id, &t.event, &t.event.data, now)?;
    Ok(id)
}

pub fn insert_contributor(
    conn: &Connection,
    name: &str,
    areas: &[Area],
    capacity: i64,
) -> rusqlite::Result<i64> {
    let areas: Vec<&str> = areas.iter().map(|a| a.as_str()).collect();
    conn.execute(
        "INSERT INTO contributors (name, areas, capacity) VALUES (?1, ?2, ?3)",
        params![name, json!(areas).to_string(), capacity],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn apply(
    conn: &Connection,
    facts: &TaskFacts,
    d: &Decision,
    now: i64,
) -> rusqlite::Result<Applied> {
    let task_id = facts.task.id;
    let mut applied = Applied::default();
    for change in &d.changes {
        match change {
            Change::OpenRequest { excluded } => {
                conn.execute(
                    "INSERT INTO routing_requests (task_id, status, started_at_ms, excluded)
                     VALUES (?1, 'pending', ?2, ?3)",
                    params![task_id, now, json!(excluded).to_string()],
                )?;
                applied.opened_request = Some(conn.last_insert_rowid());
            }
            Change::ResolveRequest { request_id, result } => {
                let (kind, who) = result_columns(*result);
                conn.execute(
                    "UPDATE routing_requests SET status = 'resolved', ended_at_ms = ?2,
                         result_kind = ?3, result_contributor_id = ?4, returned_at_ms = ?2
                     WHERE id = ?1",
                    params![request_id, now, kind, who],
                )?;
            }
            Change::SupersedeRequest { request_id } => {
                conn.execute(
                    "UPDATE routing_requests SET status = 'superseded', ended_at_ms = ?2
                     WHERE id = ?1",
                    params![request_id, now],
                )?;
            }
            Change::RecordLateResult { request_id, result } => {
                let (kind, who) = result_columns(*result);
                conn.execute(
                    "UPDATE routing_requests SET result_kind = ?2, result_contributor_id = ?3,
                         returned_at_ms = ?4
                     WHERE id = ?1",
                    params![request_id, kind, who, now],
                )?;
            }
            Change::OfferAssignment {
                contributor_id,
                source,
                reason_code,
                request_id,
            } => {
                conn.execute(
                    "INSERT INTO assignments (task_id, contributor_id, status, source,
                         reason_code, request_id, created_at_ms)
                     VALUES (?1, ?2, 'offered', ?3, ?4, ?5, ?6)",
                    params![
                        task_id,
                        contributor_id,
                        source.as_str(),
                        reason_code.map(|r| r.as_str()),
                        request_id,
                        now
                    ],
                )?;
                applied.offered_assignment = Some(conn.last_insert_rowid());
            }
            Change::AcceptAssignment { assignment_id } => {
                conn.execute(
                    "UPDATE assignments SET status = 'accepted' WHERE id = ?1",
                    [assignment_id],
                )?;
            }
            Change::EndAssignment {
                assignment_id,
                status,
            } => {
                conn.execute(
                    "UPDATE assignments SET status = ?2, ended_at_ms = ?3 WHERE id = ?1",
                    params![assignment_id, status.as_str(), now],
                )?;
            }
        }
    }
    if d.bump_version {
        conn.execute(
            "UPDATE tasks SET state = ?2, attention_reason = ?3, version = version + 1,
                 updated_at_ms = ?4
             WHERE id = ?1",
            params![
                task_id,
                d.state.as_str(),
                d.attention_reason.map(|r| r.as_str()),
                now
            ],
        )?;
    }
    for e in &d.events {
        insert_event(conn, task_id, e, &with_new_ids(&e.data, applied), now)?;
    }
    Ok(applied)
}

fn result_columns(result: EngineResult) -> (&'static str, Option<i64>) {
    match result {
        EngineResult::Match(c) => (ResultKind::Match.as_str(), Some(c)),
        EngineResult::NoMatch => (ResultKind::NoMatch.as_str(), None),
    }
}

/// `decide` cannot know ids SQLite assigns, so the store adds them to the event's data.
fn with_new_ids(data: &Value, applied: Applied) -> Value {
    let mut data = data.clone();
    if let Value::Object(map) = &mut data {
        if let Some(id) = applied.opened_request {
            map.insert("request_id".into(), json!(id));
        }
        if let Some(id) = applied.offered_assignment {
            map.insert("assignment_id".into(), json!(id));
        }
    }
    data
}

fn insert_event(
    conn: &Connection,
    task_id: i64,
    e: &EventDraft,
    data: &Value,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO events (task_id, at_ms, kind, actor, summary, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            task_id,
            now,
            e.kind.as_str(),
            e.actor.as_str(),
            e.summary,
            data.to_string()
        ],
    )?;
    Ok(())
}
