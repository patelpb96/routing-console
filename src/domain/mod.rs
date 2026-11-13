//! The state machine of contract section 2. Pure: no IO and no clock. The caller loads what is
//! stored about one task, asks `decide`, and writes the returned changes and events in one
//! transaction.

mod records;
mod rows;
#[cfg(test)]
mod tests;
mod vocab;

pub use records::*;
pub(crate) use vocab::vocab;
pub use vocab::*;

use serde_json::json;

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

/// The transition table. Anything without an arm falls through to the refusal at the bottom,
/// so adding a row is one new arm here (and its kind in `allowed_commands` for operator rows).
pub fn decide(facts: &TaskFacts, cmd: &Command) -> Result<Decision, Refusal> {
    use Command::{EngineReturned, Operator, ServerRestarted};
    use OperatorOp as Op;
    use TaskState::*;
    let task = &facts.task;
    if let Operator {
        expected_version, ..
    } = cmd
        && *expected_version != task.version
    {
        return Err(Refusal::Stale);
    }
    match (task.state, cmd) {
        (
            Unassigned,
            Operator {
                op: Op::Submit,
                note,
                ..
            },
        ) => Ok(rows::submit(note)),
        (
            Routing,
            Operator {
                op:
                    Op::RouteAgain {
                        reason_code,
                        exclude_previous,
                    },
                note,
                ..
            },
        ) => {
            if *exclude_previous {
                return Err(refuse(
                    "exclude_previous is only for a task that needs attention.",
                ));
            }
            let code = required(*reason_code, OperatorKind::RouteAgain)?;
            Ok(rows::route_again(pending(facts)?, code, note))
        }
        (
            Routing,
            Operator {
                op: Op::CancelRouting { reason_code },
                note,
                ..
            },
        ) => {
            let code = required(*reason_code, OperatorKind::CancelRouting)?;
            Ok(rows::cancel_routing(pending(facts)?, code, note))
        }
        (
            Routing,
            Operator {
                op:
                    Op::Assign {
                        contributor,
                        reason_code,
                    },
                note,
                ..
            },
        ) => {
            let code = required(*reason_code, OperatorKind::Assign)?;
            assignable(facts, contributor)?;
            Ok(rows::operator_assign(
                pending(facts)?,
                contributor,
                code,
                note,
            ))
        }
        (
            Routing,
            EngineReturned {
                request,
                result,
                contributor,
            },
        ) if is_the_pending(facts, request) => match (result, contributor) {
            (EngineResult::Match(_), Some(c)) if c.has_area(task.area) && c.has_room() => {
                Ok(rows::engine_matched(request.id, c))
            }
            (EngineResult::Match(_), Some(c)) => {
                Ok(rows::selection_unavailable(task, request.id, c))
            }
            (EngineResult::Match(id), None) => Err(unknown_contributor(*id)),
            (EngineResult::NoMatch, _) => Ok(rows::no_match(request.id)),
        },
        (
            _,
            EngineReturned {
                request,
                result,
                contributor,
            },
        ) if request.status == RequestStatus::Superseded && !request.has_result => {
            match (result, contributor) {
                (EngineResult::Match(id), None) => Err(unknown_contributor(*id)),
                _ => Ok(rows::late_result(
                    task,
                    request,
                    *result,
                    contributor.as_ref(),
                )),
            }
        }
        (Routing, ServerRestarted { boot }) => {
            Ok(rows::server_restarted(task, pending(facts)?, *boot))
        }
        (
            NeedsAttention,
            Operator {
                op:
                    Op::RouteAgain {
                        reason_code,
                        exclude_previous,
                    },
                note,
                ..
            },
        ) => {
            let left_out: &[TurnedDown] = if *exclude_previous {
                &facts.turned_down
            } else {
                &[]
            };
            Ok(rows::route_again_after_attention(
                left_out,
                *exclude_previous,
                *reason_code,
                note,
            ))
        }
        (
            NeedsAttention,
            Operator {
                op:
                    Op::Assign {
                        contributor,
                        reason_code,
                    },
                note,
                ..
            },
        ) => {
            let code = required(*reason_code, OperatorKind::Assign)?;
            assignable(facts, contributor)?;
            Ok(rows::assign_by_hand(contributor, code, note))
        }
        (
            Offered | Assigned,
            Operator {
                op:
                    Op::Assign {
                        contributor,
                        reason_code,
                    },
                note,
                ..
            },
        ) => {
            let code = required(*reason_code, OperatorKind::Assign)?;
            let Some(current) = &facts.active else {
                return Err(refuse("The task is held but has no active assignment."));
            };
            assignable(facts, contributor)?;
            Ok(rows::reassign(current, contributor, code, note))
        }
        (
            NeedsAttention,
            Operator {
                op: Op::Close,
                note,
                ..
            },
        ) => Ok(rows::close(note)),
        (
            Offered,
            Command::Reply {
                assignment_id,
                reply,
            },
        ) => match &facts.active {
            Some(a) if a.id == *assignment_id && a.status == AssignmentStatus::Offered => {
                Ok(match reply {
                    Reply::Accept => rows::accept(a),
                    Reply::Decline => rows::decline(a),
                    Reply::Fail => rows::fail(a),
                })
            }
            _ => Err(not_active_offer(*assignment_id)),
        },
        (_, Command::Reply { assignment_id, .. })
            if facts.active.as_ref().map(|a| a.id) != Some(*assignment_id) =>
        {
            Err(not_active_offer(*assignment_id))
        }
        (state, cmd) => Err(refuse(&not_in_table(state, cmd))),
    }
}

/// Operator commands the page may offer for a task, read from the same rows as `decide`.
pub fn allowed_commands(task: &Task) -> Vec<OperatorKind> {
    use OperatorKind::*;
    match task.state {
        TaskState::Unassigned => vec![Submit],
        TaskState::Routing => vec![RouteAgain, CancelRouting, Assign],
        TaskState::NeedsAttention => vec![RouteAgain, Assign, Close],
        TaskState::Offered | TaskState::Assigned => vec![Assign],
        TaskState::Closed => Vec::new(),
    }
}

fn is_the_pending(facts: &TaskFacts, request: &ReturnedRequest) -> bool {
    request.status == RequestStatus::Pending
        && facts.pending.as_ref().map(|p| p.id) == Some(request.id)
}

fn pending(facts: &TaskFacts) -> Result<&PendingRequest, Refusal> {
    // Invariant 3 says a routing task always has one; refusing keeps a broken row from
    // turning into a panic.
    facts
        .pending
        .as_ref()
        .ok_or_else(|| refuse("The task is routing but has no pending request."))
}

/// Overrides are labelled disagreements with the engine, so the label is never defaulted.
fn required(code: Option<ReasonCode>, kind: OperatorKind) -> Result<ReasonCode, Refusal> {
    code.ok_or_else(|| refuse(&format!("{kind} needs a reason code.")))
}

/// Who the operator could assign right now: the same check `assign` runs, applied to every
/// contributor, and nobody when `assign` is not a command the task allows. Advisory for the
/// page; the command is still checked inside its own transaction.
pub fn assignable_contributors(facts: &TaskFacts, contributors: &[ContributorFacts]) -> Vec<i64> {
    if !allowed_commands(&facts.task).contains(&OperatorKind::Assign) {
        return Vec::new();
    }
    contributors
        .iter()
        .filter(|c| assignable(facts, c).is_ok())
        .map(|c| c.id)
        .collect()
}

/// Expertise and capacity are hard limits that no operator override can lift, and the
/// contributor who already holds the task cannot be assigned it again.
fn assignable(facts: &TaskFacts, c: &ContributorFacts) -> Result<(), Refusal> {
    let task = &facts.task;
    if facts.active.as_ref().map(|a| a.contributor_id) == Some(c.id) {
        return Err(refuse(&format!("{} already holds this task.", c.name)));
    }
    if !c.has_area(task.area) {
        return Err(refuse(&format!("{} does not cover {}.", c.name, task.area)));
    }
    if !c.has_room() {
        return Err(refuse(&format!(
            "{} has no room ({} of {} active).",
            c.name, c.active_load, c.capacity
        )));
    }
    Ok(())
}

fn refuse(message: &str) -> Refusal {
    Refusal::NotAllowed(message.to_string())
}

fn unknown_contributor(id: i64) -> Refusal {
    refuse(&format!(
        "The engine named contributor {id}, who is not on record."
    ))
}

fn not_active_offer(assignment_id: i64) -> Refusal {
    refuse(&format!(
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
            "An engine result for request {} ({}) cannot be applied while the task is {state}.",
            request.id, request.status
        ),
        Command::ServerRestarted { .. } => {
            format!("A restart note needs a pending request; the task is {state}.")
        }
    }
}
