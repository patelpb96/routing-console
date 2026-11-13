//! Queries that turn rows back into domain records.

use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::domain::*;

/// A stored event as the snapshot shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub at_ms: i64,
    pub kind: EventKind,
    pub actor: Actor,
    pub summary: String,
    pub data: Value,
}

/// Turns a stored string back into a vocabulary word; an unknown one is a corrupt row.
fn word<T>(row: &Row<'_>, idx: usize, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    parse(&s).ok_or_else(|| bad(idx, format!("unknown value {s:?}")))
}

fn opt_word<T>(
    row: &Row<'_>,
    idx: usize,
    parse: fn(&str) -> Option<T>,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(idx)? {
        None => Ok(None),
        Some(s) => parse(&s)
            .map(Some)
            .ok_or_else(|| bad(idx, format!("unknown value {s:?}"))),
    }
}

fn json<T: serde::de::DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| bad(idx, e.to_string()))
}

fn bad(idx: usize, msg: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, msg.into())
}

const TASK_COLUMNS: &str = "id, title, area, priority, state, version, attention_reason, \
                            created_at_ms, updated_at_ms";

fn task_row(r: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        area: word(r, 2, Area::parse)?,
        priority: word(r, 3, Priority::parse)?,
        state: word(r, 4, TaskState::parse)?,
        version: r.get(5)?,
        attention_reason: opt_word(r, 6, AttentionReason::parse)?,
        created_at_ms: r.get(7)?,
        updated_at_ms: r.get(8)?,
    })
}

pub fn load_facts(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<TaskFacts>> {
    let task = conn
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
            [task_id],
            task_row,
        )
        .optional()?;
    task.map(|t| complete_facts(conn, t)).transpose()
}

pub fn all_facts(conn: &Connection) -> rusqlite::Result<Vec<TaskFacts>> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY id"))?;
    let tasks = stmt
        .query_map([], task_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    tasks.into_iter().map(|t| complete_facts(conn, t)).collect()
}

fn complete_facts(conn: &Connection, task: Task) -> rusqlite::Result<TaskFacts> {
    let pending = conn
        .query_row(
            "SELECT id, started_at_ms, excluded FROM routing_requests
             WHERE task_id = ?1 AND status = 'pending'",
            [task.id],
            |r| {
                Ok(PendingRequest {
                    id: r.get(0)?,
                    started_at_ms: r.get(1)?,
                    excluded: json(r, 2)?,
                })
            },
        )
        .optional()?;
    let active = conn
        .query_row(
            "SELECT a.id, a.contributor_id, c.name, a.status, a.source, a.reason_code
             FROM assignments a JOIN contributors c ON c.id = a.contributor_id
             WHERE a.task_id = ?1 AND a.status IN ('offered', 'accepted')",
            [task.id],
            |r| {
                Ok(ActiveAssignment {
                    id: r.get(0)?,
                    contributor_id: r.get(1)?,
                    contributor_name: r.get(2)?,
                    status: word(r, 3, AssignmentStatus::parse)?,
                    source: word(r, 4, Source::parse)?,
                    reason_code: opt_word(r, 5, ReasonCode::parse)?,
                })
            },
        )
        .optional()?;
    Ok(TaskFacts {
        task,
        pending,
        active,
    })
}

const CONTRIBUTOR_QUERY: &str = "SELECT c.id, c.name, c.areas, c.capacity,
        (SELECT COUNT(*) FROM assignments a
         WHERE a.contributor_id = c.id AND a.status IN ('offered', 'accepted'))
     FROM contributors c";

fn contributor_row(r: &Row<'_>) -> rusqlite::Result<ContributorFacts> {
    let names: Vec<String> = json(r, 2)?;
    let areas = names
        .iter()
        .map(|s| Area::parse(s).ok_or_else(|| bad(2, format!("unknown area {s:?}"))))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ContributorFacts {
        id: r.get(0)?,
        name: r.get(1)?,
        areas,
        capacity: r.get(3)?,
        active_load: r.get(4)?,
    })
}

pub fn contributors(conn: &Connection) -> rusqlite::Result<Vec<ContributorFacts>> {
    let mut stmt = conn.prepare(&format!("{CONTRIBUTOR_QUERY} ORDER BY c.id"))?;
    stmt.query_map([], contributor_row)?.collect()
}

pub fn contributor(conn: &Connection, id: i64) -> rusqlite::Result<Option<ContributorFacts>> {
    conn.query_row(
        &format!("{CONTRIBUTOR_QUERY} WHERE c.id = ?1"),
        [id],
        contributor_row,
    )
    .optional()
}

/// Contract section 3: has the task's area and is not excluded. Full contributors stay in,
/// because the engine may still pick one (the `selection_unavailable` row).
pub fn eligible(
    conn: &Connection,
    area: Area,
    excluded: &[i64],
) -> rusqlite::Result<Vec<ContributorFacts>> {
    Ok(contributors(conn)?
        .into_iter()
        .filter(|c| c.areas.contains(&area) && !excluded.contains(&c.id))
        .collect())
}

pub fn task_of_assignment(conn: &Connection, assignment_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT task_id FROM assignments WHERE id = ?1",
        [assignment_id],
        |r| r.get(0),
    )
    .optional()
}

pub fn request(
    conn: &Connection,
    request_id: i64,
) -> rusqlite::Result<Option<(i64, ReturnedRequest)>> {
    let row = conn
        .query_row(
            "SELECT task_id, id, status, returned_at_ms IS NOT NULL FROM routing_requests
             WHERE id = ?1",
            [request_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    word(r, 2, RequestStatus::parse)?,
                    r.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((task_id, id, status, has_result)) = row else {
        return Ok(None);
    };
    let superseded_by = match status {
        RequestStatus::Superseded => superseded_by(conn, id)?,
        _ => None,
    };
    Ok(Some((
        task_id,
        ReturnedRequest {
            id,
            status,
            has_result,
            superseded_by,
        },
    )))
}

/// The operator event that set a request aside, found by the id it recorded.
fn superseded_by(conn: &Connection, request_id: i64) -> rusqlite::Result<Option<Supersession>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT kind, json_extract(data, '$.contributor_name') FROM events
             WHERE json_extract(data, '$.superseded_request_id') = ?1
             ORDER BY id LIMIT 1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(kind, name)| match EventKind::parse(&kind)? {
        EventKind::OperatorRerouted => Some(Supersession::Rerouted),
        EventKind::OperatorCancelledRouting => Some(Supersession::Cancelled),
        EventKind::OperatorAssigned => Some(Supersession::Assigned {
            contributor_name: name.unwrap_or_else(|| "another contributor".into()),
        }),
        _ => None,
    }))
}

pub fn events(conn: &Connection, task_id: i64) -> rusqlite::Result<Vec<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, at_ms, kind, actor, summary, data FROM events
         WHERE task_id = ?1 ORDER BY id",
    )?;
    stmt.query_map(params![task_id], |r| {
        Ok(EventRow {
            id: r.get(0)?,
            at_ms: r.get(1)?,
            kind: word(r, 2, EventKind::parse)?,
            actor: word(r, 3, Actor::parse)?,
            summary: r.get(4)?,
            data: json(r, 5)?,
        })
    })?
    .collect()
}
