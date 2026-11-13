//! The state machine of contract section 2. Pure: no IO and no clock. The caller loads what is
//! stored about one task, asks `decide`, and writes the returned changes and events in one
//! transaction.

#[cfg(test)]
mod tests;
mod vocab;

pub(crate) use vocab::vocab;
pub use vocab::*;

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub area: Area,
    pub priority: Priority,
    pub state: TaskState,
    pub version: i64,
    pub attention_reason: Option<AttentionReason>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequest {
    pub id: i64,
    pub started_at_ms: i64,
    pub excluded: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAssignment {
    pub id: i64,
    pub contributor_id: i64,
    pub contributor_name: String,
    pub status: AssignmentStatus,
    pub source: Source,
    pub reason_code: Option<ReasonCode>,
}

/// Everything `decide` may look at for one task.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFacts {
    pub task: Task,
    pub pending: Option<PendingRequest>,
    pub active: Option<ActiveAssignment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContributorFacts {
    pub id: i64,
    pub name: String,
    pub areas: Vec<Area>,
    pub capacity: i64,
    pub active_load: i64,
}

impl ContributorFacts {
    pub fn has_room(&self) -> bool {
        self.active_load < self.capacity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineResult {
    Match(i64),
    NoMatch,
}

/// The request row an engine result names, which may no longer be the pending one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnedRequest {
    pub id: i64,
    pub status: RequestStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperatorOp {
    Submit,
    RouteAgain {
        reason_code: Option<ReasonCode>,
        exclude_previous: bool,
    },
    CancelRouting {
        reason_code: Option<ReasonCode>,
    },
    Assign {
        contributor: ContributorFacts,
        reason_code: Option<ReasonCode>,
    },
    Close,
}

impl OperatorOp {
    pub fn kind(&self) -> OperatorKind {
        match self {
            OperatorOp::Submit => OperatorKind::Submit,
            OperatorOp::RouteAgain { .. } => OperatorKind::RouteAgain,
            OperatorOp::CancelRouting { .. } => OperatorKind::CancelRouting,
            OperatorOp::Assign { .. } => OperatorKind::Assign,
            OperatorOp::Close => OperatorKind::Close,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Operator {
        expected_version: i64,
        op: OperatorOp,
        note: Option<String>,
    },
    Reply {
        assignment_id: i64,
        reply: Reply,
    },
    /// `contributor` is the matched contributor as stored now, so `decide` can check room.
    EngineReturned {
        request: ReturnedRequest,
        result: EngineResult,
        contributor: Option<ContributorFacts>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    OpenRequest {
        excluded: Vec<i64>,
    },
    ResolveRequest {
        request_id: i64,
        result: EngineResult,
    },
    OfferAssignment {
        contributor_id: i64,
        source: Source,
        reason_code: Option<ReasonCode>,
        request_id: Option<i64>,
    },
    AcceptAssignment {
        assignment_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventDraft {
    pub kind: EventKind,
    pub actor: Actor,
    pub summary: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub state: TaskState,
    pub attention_reason: Option<AttentionReason>,
    pub bump_version: bool,
    pub changes: Vec<Change>,
    pub events: Vec<EventDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Stale,
    NotAllowed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTask {
    pub title: String,
    pub area: Area,
    pub priority: Priority,
    pub event: EventDraft,
}

pub const MAX_TITLE_CHARS: usize = 200;

/// Creating is not a transition (there is no stored task yet), so it sits beside `decide`.
pub fn create(title: &str, area: Area, priority: Priority) -> Result<NewTask, Refusal> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Refusal::NotAllowed("A task needs a title.".into()));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(Refusal::NotAllowed(format!(
            "A title has at most {MAX_TITLE_CHARS} characters."
        )));
    }
    Ok(NewTask {
        title: title.to_string(),
        area,
        priority,
        event: EventDraft {
            kind: EventKind::TaskCreated,
            actor: Actor::Operator,
            summary: format!("Task created: {area}, {priority} priority."),
            data: json!({"title": title, "area": area, "priority": priority}),
        },
    })
}

/// The transition table. Rows not written yet fall through to the refusal at the bottom, so
/// adding a row is one new arm here (and its kind in `allowed_commands` for operator rows).
pub fn decide(facts: &TaskFacts, cmd: &Command) -> Result<Decision, Refusal> {
    use TaskState::*;
    if let Command::Operator {
        expected_version, ..
    } = cmd
        && *expected_version != facts.task.version
    {
        return Err(Refusal::Stale);
    }
    match (facts.task.state, cmd) {
        (
            Unassigned,
            Command::Operator {
                op: OperatorOp::Submit,
                note,
                ..
            },
        ) => Ok(submit(note)),
        (
            Routing,
            Command::EngineReturned {
                request,
                result: EngineResult::Match(id),
                contributor: Some(c),
            },
        ) if is_the_pending(facts, request) && c.id == *id && c.has_room() => {
            Ok(engine_matched(request.id, c))
        }
        (
            Offered,
            Command::Reply {
                assignment_id,
                reply: Reply::Accept,
            },
        ) => match &facts.active {
            Some(a) if a.id == *assignment_id && a.status == AssignmentStatus::Offered => {
                Ok(accept(a))
            }
            _ => Err(not_active_offer(*assignment_id)),
        },
        (_, Command::Reply { assignment_id, .. })
            if facts.active.as_ref().map(|a| a.id) != Some(*assignment_id) =>
        {
            Err(not_active_offer(*assignment_id))
        }
        (state, cmd) => Err(Refusal::NotAllowed(not_in_table(state, cmd))),
    }
}

/// Operator commands the page may offer for a task, read from the same rows as `decide`.
pub fn allowed_commands(task: &Task) -> Vec<OperatorKind> {
    match task.state {
        TaskState::Unassigned => vec![OperatorKind::Submit],
        _ => Vec::new(),
    }
}

fn is_the_pending(facts: &TaskFacts, request: &ReturnedRequest) -> bool {
    request.status == RequestStatus::Pending
        && facts.pending.as_ref().map(|p| p.id) == Some(request.id)
}

fn submit(note: &Option<String>) -> Decision {
    Decision {
        state: TaskState::Routing,
        attention_reason: None,
        bump_version: true,
        changes: vec![Change::OpenRequest {
            excluded: Vec::new(),
        }],
        events: vec![EventDraft {
            kind: EventKind::RoutingStarted,
            actor: Actor::Operator,
            summary: "Submitted for routing; the engine is looking for a contributor.".into(),
            data: json!({"excluded": [], "note": note}),
        }],
    }
}

fn engine_matched(request_id: i64, c: &ContributorFacts) -> Decision {
    Decision {
        state: TaskState::Offered,
        attention_reason: None,
        bump_version: true,
        changes: vec![
            Change::ResolveRequest {
                request_id,
                result: EngineResult::Match(c.id),
            },
            Change::OfferAssignment {
                contributor_id: c.id,
                source: Source::Engine,
                reason_code: None,
                request_id: Some(request_id),
            },
        ],
        events: vec![EventDraft {
            kind: EventKind::EngineMatched,
            actor: Actor::Engine,
            summary: format!(
                "The engine matched {}; the task is offered to them.",
                c.name
            ),
            data: json!({"request_id": request_id, "contributor_id": c.id}),
        }],
    }
}

fn accept(a: &ActiveAssignment) -> Decision {
    Decision {
        state: TaskState::Assigned,
        attention_reason: None,
        bump_version: true,
        changes: vec![Change::AcceptAssignment {
            assignment_id: a.id,
        }],
        events: vec![EventDraft {
            kind: EventKind::ContributorAccepted,
            actor: Actor::Contributor,
            summary: format!(
                "{} accepted the offer. Routing is complete.",
                a.contributor_name
            ),
            data: json!({"assignment_id": a.id, "contributor_id": a.contributor_id}),
        }],
    }
}

fn not_active_offer(assignment_id: i64) -> Refusal {
    Refusal::NotAllowed(format!(
        "Assignment {assignment_id} is not the task's active offer."
    ))
}

fn not_in_table(state: TaskState, cmd: &Command) -> String {
    match cmd {
        Command::Operator { op, .. } => {
            format!(
                "Command {} is not allowed while the task is {state}.",
                op.kind()
            )
        }
        Command::Reply { reply, .. } => {
            format!("Reply {reply} is not allowed while the task is {state}.")
        }
        Command::EngineReturned { request, .. } => format!(
            "An engine result for request {} cannot be applied while the task is {state}.",
            request.id
        ),
    }
}
