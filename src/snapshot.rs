//! The snapshot JSON of contract section 5. Every field is always present; absent records are
//! `null` and time-dependent fields are worked out here, at read time, and never stored.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::domain::*;
use crate::engine::Scenario;
use crate::store::read::{EventRow, events};

pub const NORMAL_WINDOW_MIN_MS: i64 = 5_000;
pub const NORMAL_WINDOW_MAX_MS: i64 = 20_000;

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub now_ms: i64,
    pub boot: Boot,
    pub sim: SimInfo,
    pub normal_window_ms: Window,
    pub areas: Vec<Area>,
    pub reason_codes: Vec<ReasonCodeView>,
    pub contributors: Vec<ContributorFacts>,
    pub tasks: Vec<TaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Boot {
    pub count: i64,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimInfo {
    pub seed: u64,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Window {
    pub min: i64,
    pub max: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReasonCodeView {
    pub code: ReasonCode,
    pub label: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub id: i64,
    pub title: String,
    pub area: Area,
    pub priority: Priority,
    pub state: TaskState,
    pub version: i64,
    pub attention_reason: Option<AttentionReason>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub pending_request: Option<PendingView>,
    pub active_assignment: Option<AssignmentView>,
    pub scenario: Scenario,
    pub allowed_commands: Vec<OperatorKind>,
    pub events: Vec<EventView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingView {
    pub id: i64,
    pub started_at_ms: i64,
    pub elapsed_ms: i64,
    pub past_normal_window: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssignmentView {
    pub id: i64,
    pub contributor_id: i64,
    pub status: AssignmentStatus,
    pub source: Source,
    pub reason_code: Option<ReasonCode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventView {
    pub id: i64,
    pub at_ms: i64,
    pub kind: EventKind,
    pub actor: Actor,
    pub summary: String,
    pub data: Value,
}

impl From<EventRow> for EventView {
    fn from(e: EventRow) -> Self {
        EventView {
            id: e.id,
            at_ms: e.at_ms,
            kind: e.kind,
            actor: e.actor,
            summary: e.summary,
            data: e.data,
        }
    }
}

pub fn task_view(
    conn: &rusqlite::Connection,
    facts: TaskFacts,
    scenario: Scenario,
    now_ms: i64,
) -> rusqlite::Result<TaskView> {
    let TaskFacts {
        task,
        pending,
        active,
    } = facts;
    let events = events(conn, task.id)?
        .into_iter()
        .map(EventView::from)
        .collect();
    Ok(TaskView {
        allowed_commands: allowed_commands(&task),
        pending_request: pending.map(|p| {
            let elapsed_ms = now_ms - p.started_at_ms;
            PendingView {
                id: p.id,
                started_at_ms: p.started_at_ms,
                elapsed_ms,
                past_normal_window: elapsed_ms > NORMAL_WINDOW_MAX_MS,
            }
        }),
        active_assignment: active.map(|a| AssignmentView {
            id: a.id,
            contributor_id: a.contributor_id,
            status: a.status,
            source: a.source,
            reason_code: a.reason_code,
        }),
        id: task.id,
        title: task.title,
        area: task.area,
        priority: task.priority,
        state: task.state,
        version: task.version,
        attention_reason: task.attention_reason,
        created_at_ms: task.created_at_ms,
        updated_at_ms: task.updated_at_ms,
        scenario,
        events,
    })
}

pub struct SnapshotInput {
    pub now_ms: i64,
    pub boot: Boot,
    pub seed: u64,
    pub scenarios: HashMap<i64, Scenario>,
}

pub fn build(conn: &rusqlite::Connection, input: SnapshotInput) -> rusqlite::Result<Snapshot> {
    let tasks = crate::store::read::all_facts(conn)?
        .into_iter()
        .map(|f| {
            let s = input
                .scenarios
                .get(&f.task.id)
                .copied()
                .unwrap_or(Scenario::Auto);
            task_view(conn, f, s, input.now_ms)
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Snapshot {
        now_ms: input.now_ms,
        boot: input.boot,
        sim: SimInfo {
            seed: input.seed,
            scenarios: Scenario::ALL.to_vec(),
        },
        normal_window_ms: Window {
            min: NORMAL_WINDOW_MIN_MS,
            max: NORMAL_WINDOW_MAX_MS,
        },
        areas: Area::ALL.to_vec(),
        reason_codes: ReasonCode::ALL
            .iter()
            .map(|&code| ReasonCodeView {
                code,
                label: code.label(),
            })
            .collect(),
        contributors: crate::store::read::contributors(conn)?,
        tasks,
    })
}
