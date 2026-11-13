//! What the tables say, read with plain SQL rather than through the app.

use routing_console::store::Store;
use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: i64,
    pub area: String,
    pub state: String,
    pub version: i64,
    pub attention: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestRow {
    pub id: i64,
    pub task_id: i64,
    pub status: String,
    pub result_kind: Option<String>,
    pub result_contributor_id: Option<i64>,
    pub returned_at_ms: Option<i64>,
    pub excluded: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentRow {
    pub id: i64,
    pub task_id: i64,
    pub contributor_id: i64,
    pub status: String,
    pub request_id: Option<i64>,
    pub ended_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub task_id: i64,
    pub at_ms: i64,
    pub kind: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContributorRow {
    pub id: i64,
    pub areas: Vec<String>,
    pub capacity: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Db {
    pub tasks: Vec<TaskRow>,
    pub requests: Vec<RequestRow>,
    pub assignments: Vec<AssignmentRow>,
    pub events: Vec<EventRow>,
    pub contributors: Vec<ContributorRow>,
    /// Engine reports that fell due and were never acknowledged.
    pub undelivered_due: Vec<i64>,
}

fn rows<T>(
    c: &Connection,
    sql: &str,
    f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    let mut stmt = c.prepare(sql).unwrap();
    stmt.query_map([], f)
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub fn dump(store: &Store, now_ms: i64) -> Db {
    store
        .transaction(|c| {
            Ok(Db {
                tasks: rows(
                    c,
                    "SELECT id, area, state, version, attention_reason FROM tasks ORDER BY id",
                    |r| {
                        Ok(TaskRow {
                            id: r.get(0)?,
                            area: r.get(1)?,
                            state: r.get(2)?,
                            version: r.get(3)?,
                            attention: r.get(4)?,
                        })
                    },
                ),
                requests: rows(
                    c,
                    "SELECT id, task_id, status, result_kind, result_contributor_id, returned_at_ms,
                         excluded
                     FROM routing_requests ORDER BY id",
                    |r| {
                        let excluded: String = r.get(6)?;
                        Ok(RequestRow {
                            id: r.get(0)?,
                            task_id: r.get(1)?,
                            status: r.get(2)?,
                            result_kind: r.get(3)?,
                            result_contributor_id: r.get(4)?,
                            returned_at_ms: r.get(5)?,
                            excluded: serde_json::from_str(&excluded).unwrap(),
                        })
                    },
                ),
                assignments: rows(
                    c,
                    "SELECT id, task_id, contributor_id, status, request_id, ended_at_ms
                     FROM assignments ORDER BY id",
                    |r| {
                        Ok(AssignmentRow {
                            id: r.get(0)?,
                            task_id: r.get(1)?,
                            contributor_id: r.get(2)?,
                            status: r.get(3)?,
                            request_id: r.get(4)?,
                            ended_at_ms: r.get(5)?,
                        })
                    },
                ),
                events: rows(
                    c,
                    "SELECT id, task_id, at_ms, kind, data FROM events ORDER BY id",
                    |r| {
                        Ok(EventRow {
                            id: r.get(0)?,
                            task_id: r.get(1)?,
                            at_ms: r.get(2)?,
                            kind: r.get(3)?,
                            data: r.get(4)?,
                        })
                    },
                ),
                contributors: rows(
                    c,
                    "SELECT id, areas, capacity FROM contributors ORDER BY id",
                    |r| {
                        let areas: String = r.get(1)?;
                        Ok(ContributorRow {
                            id: r.get(0)?,
                            areas: serde_json::from_str(&areas).unwrap(),
                            capacity: r.get(2)?,
                        })
                    },
                ),
                undelivered_due: rows(
                    c,
                    &format!(
                        "SELECT request_id FROM sim_schedule
                         WHERE delivered = 0 AND due_at_ms IS NOT NULL AND due_at_ms <= {now_ms}
                         ORDER BY request_id"
                    ),
                    |r| r.get(0),
                ),
            })
        })
        .unwrap()
}

impl Db {
    pub fn active(&self, task: i64) -> Vec<&AssignmentRow> {
        self.assignments
            .iter()
            .filter(|a| a.task_id == task && (a.status == "offered" || a.status == "accepted"))
            .collect()
    }
    pub fn pending(&self, task: i64) -> Vec<&RequestRow> {
        self.requests
            .iter()
            .filter(|r| r.task_id == task && r.status == "pending")
            .collect()
    }
    pub fn load(&self, contributor: i64) -> i64 {
        self.assignments
            .iter()
            .filter(|a| {
                a.contributor_id == contributor && (a.status == "offered" || a.status == "accepted")
            })
            .count() as i64
    }
    pub fn task(&self, id: i64) -> Option<&TaskRow> {
        self.tasks.iter().find(|t| t.id == id)
    }
    pub fn new_events<'a>(&'a self, before: &Db) -> &'a [EventRow] {
        &self.events[before.events.len().min(self.events.len())..]
    }
}

pub fn event_request_id(e: &EventRow) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(&e.data).unwrap()["request_id"].as_i64()
}
