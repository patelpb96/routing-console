//! `GET /api/overrides.jsonl`: every operator command that carried a reason code, joined to
//! what the engine said about the request it set aside. Read from the events and the request,
//! task and contributor rows as they are now, so a late result shows up once it has arrived.
//!
//! `engine_result` keeps "not answered yet" (`none_yet`) apart from an answer that differs from
//! the operator's (`match` with `agreed: false`, or `no_match`).

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct OverrideLine {
    pub event_id: i64,
    pub at_ms: i64,
    pub task_id: i64,
    pub task_title: String,
    pub area: String,
    pub priority: String,
    pub command: &'static str,
    pub reason_code: String,
    pub note: Option<String>,
    pub chosen_contributor_id: Option<i64>,
    pub chosen_contributor_name: Option<String>,
    pub replaced_contributor_id: Option<i64>,
    pub superseded_request_id: Option<i64>,
    pub request_elapsed_ms: Option<i64>,
    pub engine_result: Option<&'static str>,
    pub engine_contributor_id: Option<i64>,
    pub engine_contributor_name: Option<String>,
    pub engine_returned_at_ms: Option<i64>,
    pub agreed: Option<bool>,
}

/// The export body: one serde-encoded line per override, ordered by event id, each ending in
/// a newline. No overrides gives an empty string.
pub fn jsonl(conn: &Connection) -> rusqlite::Result<String> {
    let mut out = String::new();
    for line in lines(conn)? {
        out.push_str(&serde_json::to_string(&line).expect("plain data serializes"));
        out.push('\n');
    }
    Ok(out)
}

pub fn lines(conn: &Connection) -> rusqlite::Result<Vec<OverrideLine>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.at_ms, e.task_id, e.kind, e.data, t.title, t.area, t.priority
         FROM events e JOIN tasks t ON t.id = e.task_id
         WHERE e.kind IN ('operator_assigned', 'operator_rerouted', 'operator_cancelled_routing')
         ORDER BY e.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for (event_id, at_ms, task_id, kind, data, task_title, area, priority) in rows {
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        // Routing again from needs attention may carry no reason code: not an override.
        let Some(reason_code) = data["reason_code"].as_str().map(str::to_owned) else {
            continue;
        };
        let command = match kind.as_str() {
            "operator_assigned" => "assign",
            "operator_rerouted" => "route_again",
            _ => "cancel_routing",
        };
        let chosen_contributor_id = data["contributor_id"].as_i64();
        let superseded_request_id = data["superseded_request_id"].as_i64();
        let mut line = OverrideLine {
            event_id,
            at_ms,
            task_id,
            task_title,
            area,
            priority,
            command,
            reason_code,
            note: data["note"].as_str().map(str::to_owned),
            chosen_contributor_id,
            chosen_contributor_name: match chosen_contributor_id {
                Some(id) => name(conn, id)?,
                None => None,
            },
            replaced_contributor_id: data["previous_contributor_id"].as_i64(),
            superseded_request_id,
            request_elapsed_ms: None,
            engine_result: None,
            engine_contributor_id: None,
            engine_contributor_name: None,
            engine_returned_at_ms: None,
            agreed: None,
        };
        if let Some(request_id) = superseded_request_id {
            let (started, kind, contributor, returned): (
                i64,
                Option<String>,
                Option<i64>,
                Option<i64>,
            ) = conn.query_row(
                "SELECT started_at_ms, result_kind, result_contributor_id, returned_at_ms
                     FROM routing_requests WHERE id = ?1",
                [request_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            line.request_elapsed_ms = Some(at_ms - started);
            line.engine_result = Some(match kind.as_deref() {
                None => "none_yet",
                Some("match") => "match",
                Some(_) => "no_match",
            });
            line.engine_returned_at_ms = returned;
            if kind.as_deref() == Some("match") {
                line.engine_contributor_id = contributor;
                line.engine_contributor_name = match contributor {
                    Some(id) => name(conn, id)?,
                    None => None,
                };
                if command == "assign" {
                    line.agreed = Some(contributor == chosen_contributor_id);
                }
            }
        }
        out.push(line);
    }
    Ok(out)
}

fn name(conn: &Connection, id: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT name FROM contributors WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()
}
