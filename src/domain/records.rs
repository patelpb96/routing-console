//! What `decide` reads (facts about one task, a command) and what it returns (a decision).

use serde::Serialize;
use serde_json::Value;

use super::vocab::*;

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

    pub fn has_area(&self, area: Area) -> bool {
        self.areas.contains(&area)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineResult {
    Match(i64),
    NoMatch,
}

/// What the operator did that took a request out of play, read back from the event that did it,
/// so a late result can say what happened instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Supersession {
    Rerouted,
    Cancelled,
    Assigned { contributor_name: String },
}

/// The request row an engine result names, which may no longer be the pending one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedRequest {
    pub id: i64,
    pub status: RequestStatus,
    /// A result is already stored on the row: this report is a repeat delivery.
    pub has_result: bool,
    pub superseded_by: Option<Supersession>,
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
    /// `None` for no match, or when the engine named someone the store does not know.
    EngineReturned {
        request: ReturnedRequest,
        result: EngineResult,
        contributor: Option<ContributorFacts>,
    },
    /// Written once per pending request when the app starts.
    ServerRestarted {
        boot: i64,
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
    SupersedeRequest {
        request_id: i64,
    },
    /// Stores a result on a superseded row without touching its status.
    RecordLateResult {
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

/// `state` and `attention_reason` are written only when `bump_version` is set; a decision that
/// only adds information leaves the task row alone.
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
