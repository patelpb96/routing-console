//! One builder per row of the transition table. `decide` picks the row; these say what it
//! writes. Each event summary is a finished sentence for the timeline.

use serde_json::json;

use super::*;

fn event(kind: EventKind, actor: Actor, summary: String, data: serde_json::Value) -> EventDraft {
    EventDraft {
        kind,
        actor,
        summary,
        data,
    }
}

fn moves_to(state: TaskState, changes: Vec<Change>, e: EventDraft) -> Decision {
    Decision {
        state,
        attention_reason: None,
        bump_version: true,
        changes,
        events: vec![e],
    }
}

/// A decision that adds records and an event but leaves the task row (state, version) as is.
fn note_only(task: &Task, changes: Vec<Change>, e: EventDraft) -> Decision {
    Decision {
        state: task.state,
        attention_reason: task.attention_reason,
        bump_version: false,
        changes,
        events: vec![e],
    }
}

fn needs_attention(reason: AttentionReason, changes: Vec<Change>, e: EventDraft) -> Decision {
    Decision {
        attention_reason: Some(reason),
        ..moves_to(TaskState::NeedsAttention, changes, e)
    }
}

pub fn submit(note: &Option<String>) -> Decision {
    moves_to(
        TaskState::Routing,
        vec![Change::OpenRequest { excluded: vec![] }],
        event(
            EventKind::RoutingStarted,
            Actor::Operator,
            "Submitted for routing; the engine is looking for a contributor.".into(),
            json!({"excluded": [], "note": note}),
        ),
    )
}

/// Routing to routing. The pending request changes, so by contract section 2 the version goes
/// up: the elapsed clock another tab was judging has restarted. The new request keeps the old
/// one's `excluded` list; `turned_down` supplies the names for the summary.
pub fn route_again(
    old: &PendingRequest,
    turned_down: &[TurnedDown],
    code: ReasonCode,
    note: &Option<String>,
) -> Decision {
    let ids = old.excluded.clone();
    let names: Vec<String> = ids
        .iter()
        .map(|id| {
            turned_down
                .iter()
                .find(|t| t.contributor_id == *id)
                .map_or_else(|| format!("contributor {id}"), |t| t.name.clone())
        })
        .collect();
    let names_ref: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut summary = format!(
        "Routed again ({}); the earlier engine request is set aside",
        code.label()
    );
    if !names.is_empty() {
        summary.push_str(&format!(", still leaving out {}", and_list(&names_ref)));
    }
    summary.push('.');
    moves_to(
        TaskState::Routing,
        vec![
            Change::SupersedeRequest { request_id: old.id },
            Change::OpenRequest {
                excluded: ids.clone(),
            },
        ],
        event(
            EventKind::OperatorRerouted,
            Actor::Operator,
            summary,
            json!({"superseded_request_id": old.id, "reason_code": code, "excluded": ids,
                   "excluded_names": names, "note": note}),
        ),
    )
}

/// Needs attention to routing. `left_out` is empty unless the operator asked to exclude the
/// contributors who declined or failed earlier.
pub fn route_again_after_attention(
    left_out: &[TurnedDown],
    exclude_previous: bool,
    code: Option<ReasonCode>,
    note: &Option<String>,
) -> Decision {
    let ids: Vec<i64> = left_out.iter().map(|t| t.contributor_id).collect();
    let names: Vec<&str> = left_out.iter().map(|t| t.name.as_str()).collect();
    let summary = match (exclude_previous, names.is_empty()) {
        (true, false) => format!("Routed again, leaving out {}.", and_list(&names)),
        (true, true) => {
            "Routed again; nobody had declined or failed, so nobody is left out.".into()
        }
        (false, _) => "Routed again; everyone with the area is eligible.".into(),
    };
    moves_to(
        TaskState::Routing,
        vec![Change::OpenRequest {
            excluded: ids.clone(),
        }],
        event(
            EventKind::OperatorRerouted,
            Actor::Operator,
            summary,
            json!({"exclude_previous": exclude_previous, "excluded": ids,
                   "excluded_names": names, "reason_code": code, "note": note}),
        ),
    )
}

pub fn cancel_routing(old: &PendingRequest, code: ReasonCode, note: &Option<String>) -> Decision {
    moves_to(
        TaskState::Unassigned,
        vec![Change::SupersedeRequest { request_id: old.id }],
        event(
            EventKind::OperatorCancelledRouting,
            Actor::Operator,
            format!(
                "Routing cancelled ({}); the task is back to unassigned.",
                code.label()
            ),
            json!({"superseded_request_id": old.id, "reason_code": code, "note": note}),
        ),
    )
}

pub fn operator_assign(
    old: &PendingRequest,
    c: &ContributorFacts,
    code: ReasonCode,
    note: &Option<String>,
) -> Decision {
    moves_to(
        TaskState::Offered,
        vec![
            Change::SupersedeRequest { request_id: old.id },
            Change::OfferAssignment {
                contributor_id: c.id,
                source: Source::Operator,
                reason_code: Some(code),
                request_id: None,
            },
        ],
        event(
            EventKind::OperatorAssigned,
            Actor::Operator,
            format!(
                "Assigned by hand to {} ({}); the pending engine request is set aside.",
                c.name,
                code.label()
            ),
            json!({"superseded_request_id": old.id, "contributor_id": c.id,
                   "contributor_name": c.name, "reason_code": code, "note": note}),
        ),
    )
}

/// Needs attention to offered: there is no pending request to set aside.
pub fn assign_by_hand(c: &ContributorFacts, code: ReasonCode, note: &Option<String>) -> Decision {
    moves_to(
        TaskState::Offered,
        vec![Change::OfferAssignment {
            contributor_id: c.id,
            source: Source::Operator,
            reason_code: Some(code),
            request_id: None,
        }],
        event(
            EventKind::OperatorAssigned,
            Actor::Operator,
            format!("Assigned by hand to {} ({}).", c.name, why(code)),
            json!({"contributor_id": c.id, "contributor_name": c.name, "reason_code": code,
                   "note": note}),
        ),
    )
}

/// Offered or assigned to offered: the current assignment is withdrawn first, so the
/// one-active-assignment index never sees two.
pub fn reassign(
    old: &ActiveAssignment,
    c: &ContributorFacts,
    code: ReasonCode,
    note: &Option<String>,
) -> Decision {
    moves_to(
        TaskState::Offered,
        vec![
            Change::EndAssignment {
                assignment_id: old.id,
                status: AssignmentStatus::Withdrawn,
            },
            Change::OfferAssignment {
                contributor_id: c.id,
                source: Source::Operator,
                reason_code: Some(code),
                request_id: None,
            },
        ],
        event(
            EventKind::OperatorAssigned,
            Actor::Operator,
            format!(
                "Reassigned from {} to {} ({}).",
                old.contributor_name,
                c.name,
                why(code)
            ),
            json!({"withdrawn_assignment_id": old.id,
                   "previous_contributor_id": old.contributor_id,
                   "previous_contributor_name": old.contributor_name,
                   "previous_status": old.status,
                   "contributor_id": c.id, "contributor_name": c.name,
                   "reason_code": code, "note": note}),
        ),
    )
}

pub fn close(note: &Option<String>) -> Decision {
    moves_to(
        TaskState::Closed,
        vec![],
        event(
            EventKind::TaskClosed,
            Actor::Operator,
            "Closed by the operator.".into(),
            json!({"note": note}),
        ),
    )
}

pub fn engine_matched(request_id: i64, c: &ContributorFacts) -> Decision {
    moves_to(
        TaskState::Offered,
        vec![
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
        event(
            EventKind::EngineMatched,
            Actor::Engine,
            format!(
                "The engine matched {}; the task is offered to them.",
                c.name
            ),
            json!({"request_id": request_id, "contributor_id": c.id}),
        ),
    )
}

pub fn selection_unavailable(task: &Task, request_id: i64, c: &ContributorFacts) -> Decision {
    let why = if c.has_area(task.area) {
        format!("has no room ({} of {} active)", c.active_load, c.capacity)
    } else {
        format!("does not cover {}", task.area)
    };
    needs_attention(
        AttentionReason::SelectionUnavailable,
        vec![Change::ResolveRequest {
            request_id,
            result: EngineResult::Match(c.id),
        }],
        event(
            EventKind::EngineSelectionUnavailable,
            Actor::Engine,
            format!("The engine picked {}, who {why}. Needs attention.", c.name),
            json!({"request_id": request_id, "contributor_id": c.id}),
        ),
    )
}

pub fn no_match(request_id: i64) -> Decision {
    needs_attention(
        AttentionReason::NoMatch,
        vec![Change::ResolveRequest {
            request_id,
            result: EngineResult::NoMatch,
        }],
        event(
            EventKind::EngineNoMatch,
            Actor::Engine,
            "The engine found no eligible contributor. Needs attention.".into(),
            json!({"request_id": request_id}),
        ),
    )
}

/// A result for a request the operator already replaced: stored on its row, told in one
/// event, and nothing else, so a late answer can never undo what the operator did.
pub fn late_result(
    task: &Task,
    request: &ReturnedRequest,
    result: EngineResult,
    contributor: Option<&ContributorFacts>,
) -> Decision {
    let suggested = match (result, contributor) {
        (EngineResult::Match(_), Some(c)) => c.name.clone(),
        (EngineResult::Match(id), None) => format!("contributor {id}"),
        (EngineResult::NoMatch, _) => "no match".into(),
    };
    let instead = match &request.superseded_by {
        Some(Supersession::Assigned { contributor_name }) => {
            format!("you had already replaced by assigning {contributor_name}")
        }
        Some(Supersession::Rerouted) => "you had already replaced by routing again".into(),
        Some(Supersession::Cancelled) => "you had already cancelled".into(),
        None => "that had already been set aside".into(),
    };
    let contributor_id = match result {
        EngineResult::Match(id) => Some(id),
        EngineResult::NoMatch => None,
    };
    note_only(
        task,
        vec![Change::RecordLateResult {
            request_id: request.id,
            result,
        }],
        event(
            EventKind::LateResultIgnored,
            Actor::Engine,
            format!(
                "The engine returned {suggested} for a request {instead}. Recorded, not applied."
            ),
            json!({"request_id": request.id, "contributor_id": contributor_id}),
        ),
    )
}

pub fn server_restarted(task: &Task, pending: &PendingRequest, boot: i64) -> Decision {
    note_only(
        task,
        vec![],
        event(
            EventKind::ServerRestarted,
            Actor::System,
            format!(
                "The server restarted while request {} was pending. It stays open; the engine \
                 may still answer.",
                pending.id
            ),
            json!({"request_id": pending.id, "boot": boot}),
        ),
    )
}

pub fn accept(a: &ActiveAssignment) -> Decision {
    moves_to(
        TaskState::Assigned,
        vec![Change::AcceptAssignment {
            assignment_id: a.id,
        }],
        event(
            EventKind::ContributorAccepted,
            Actor::Contributor,
            format!(
                "{} accepted the offer. Routing is complete.",
                a.contributor_name
            ),
            json!({"assignment_id": a.id, "contributor_id": a.contributor_id}),
        ),
    )
}

const DECISION_NEEDED: &str =
    "The task needs a decision: route again without them, assign by hand, or close.";

pub fn decline(a: &ActiveAssignment) -> Decision {
    needs_attention(
        AttentionReason::Declined,
        vec![Change::EndAssignment {
            assignment_id: a.id,
            status: AssignmentStatus::Declined,
        }],
        event(
            EventKind::ContributorDeclined,
            Actor::Contributor,
            format!(
                "{} declined the offer. {DECISION_NEEDED}",
                a.contributor_name
            ),
            json!({"assignment_id": a.id, "contributor_id": a.contributor_id,
                   "contributor_name": a.contributor_name}),
        ),
    )
}

pub fn fail(a: &ActiveAssignment) -> Decision {
    needs_attention(
        AttentionReason::OfferFailed,
        vec![Change::EndAssignment {
            assignment_id: a.id,
            status: AssignmentStatus::Failed,
        }],
        event(
            EventKind::OfferFailed,
            Actor::System,
            format!(
                "The offer to {} could not be delivered. {DECISION_NEEDED}",
                a.contributor_name
            ),
            json!({"assignment_id": a.id, "contributor_id": a.contributor_id,
                   "contributor_name": a.contributor_name}),
        ),
    )
}

/// A reason code as it reads inside a sentence: "(better fit known)".
fn why(code: ReasonCode) -> String {
    code.label().to_lowercase()
}

/// "A", "A and B", "A, B and C".
fn and_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
