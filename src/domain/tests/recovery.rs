//! Rows of slice S3: decline, fail, the needs-attention decisions and reassigning.

use super::*;

fn attention(reason: AttentionReason) -> TaskFacts {
    let mut f = facts(TaskState::NeedsAttention);
    f.task.attention_reason = Some(reason);
    f
}

fn declined_by_grace() -> TaskFacts {
    TaskFacts {
        turned_down: vec![
            TurnedDown {
                contributor_id: 7,
                name: "Grace Lin".into(),
            },
            TurnedDown {
                contributor_id: 5,
                name: "Elena Ruiz".into(),
            },
        ],
        ..attention(AttentionReason::Declined)
    }
}

fn reply(r: Reply) -> Command {
    Command::Reply {
        assignment_id: 4,
        reply: r,
    }
}

fn assigned_facts() -> TaskFacts {
    let mut f = offered_facts();
    f.task.state = TaskState::Assigned;
    f.active.as_mut().unwrap().status = AssignmentStatus::Accepted;
    f
}

const FIT: Option<ReasonCode> = Some(ReasonCode::BetterFitKnown);

#[test]
fn decline_ends_the_offer_and_needs_attention_as_declined() {
    let d = decide(&offered_facts(), &reply(Reply::Decline)).unwrap();
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(d.attention_reason, Some(AttentionReason::Declined));
    assert!(d.bump_version);
    assert_eq!(
        d.changes,
        vec![Change::EndAssignment {
            assignment_id: 4,
            status: AssignmentStatus::Declined
        }]
    );
    let e = &d.events[0];
    assert_eq!(e.kind, EventKind::ContributorDeclined);
    assert_eq!(e.actor, Actor::Contributor);
    assert_eq!(
        e.summary,
        "Ada Brook declined the offer. The task needs a decision: route again without them, \
         assign by hand, or close."
    );
    assert_eq!(e.data["contributor_id"], 2);
}

#[test]
fn fail_ends_the_offer_and_needs_attention_as_offer_failed() {
    let d = decide(&offered_facts(), &reply(Reply::Fail)).unwrap();
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(d.attention_reason, Some(AttentionReason::OfferFailed));
    assert_eq!(
        d.changes,
        vec![Change::EndAssignment {
            assignment_id: 4,
            status: AssignmentStatus::Failed
        }]
    );
    assert_eq!(d.events[0].kind, EventKind::OfferFailed);
    assert!(d.events[0].summary.contains("Ada Brook"));
}

#[test]
fn replies_are_refused_once_the_task_is_assigned_or_for_another_assignment() {
    refused(decide(&assigned_facts(), &reply(Reply::Decline)));
    let other = Command::Reply {
        assignment_id: 9,
        reply: Reply::Fail,
    };
    refused(decide(&offered_facts(), &other));
}

#[test]
fn route_again_from_attention_with_exclude_previous_leaves_out_every_decliner() {
    let d = decide(&declined_by_grace(), &route_again(None, true, 3)).unwrap();
    assert_eq!(d.state, TaskState::Routing);
    assert_eq!(d.attention_reason, None);
    assert!(d.bump_version);
    assert_eq!(
        d.changes,
        vec![Change::OpenRequest {
            excluded: vec![7, 5]
        }]
    );
    assert_eq!(d.events[0].kind, EventKind::OperatorRerouted);
    assert_eq!(
        d.events[0].summary,
        "Routed again, leaving out Grace Lin and Elena Ruiz."
    );
    assert_eq!(d.events[0].data["excluded"], serde_json::json!([7, 5]));
}

#[test]
fn route_again_from_attention_without_exclude_previous_leaves_nobody_out() {
    let d = decide(&declined_by_grace(), &route_again(None, false, 3)).unwrap();
    assert_eq!(d.changes, vec![Change::OpenRequest { excluded: vec![] }]);
    let d = decide(
        &attention(AttentionReason::NoMatch),
        &route_again(None, true, 3),
    )
    .unwrap();
    assert_eq!(d.changes, vec![Change::OpenRequest { excluded: vec![] }]);
}

#[test]
fn assign_from_attention_needs_a_code_and_the_limits_but_may_pick_a_decliner() {
    let f = declined_by_grace();
    assert!(refused(decide(&f, &assign(grace(), None))).contains("reason"));
    let d = decide(&f, &assign(grace(), FIT)).unwrap();
    assert_eq!(d.state, TaskState::Offered);
    assert_eq!(
        d.changes,
        vec![Change::OfferAssignment {
            contributor_id: 7,
            source: Source::Operator,
            reason_code: FIT,
            request_id: None,
        }]
    );
    assert_eq!(
        d.events[0].summary,
        "Assigned by hand to Grace Lin (better fit known)."
    );
    let full = ContributorFacts {
        active_load: 1,
        ..grace()
    };
    assert!(refused(decide(&f, &assign(full, FIT))).contains("no room"));
}

#[test]
fn reassign_withdraws_the_current_assignment_first() {
    for f in [offered_facts(), assigned_facts()] {
        let d = decide(&f, &assign(grace(), FIT)).unwrap();
        assert_eq!(d.state, TaskState::Offered);
        assert!(d.bump_version);
        assert_eq!(
            d.changes[0],
            Change::EndAssignment {
                assignment_id: 4,
                status: AssignmentStatus::Withdrawn
            }
        );
        assert!(matches!(
            d.changes[1],
            Change::OfferAssignment {
                contributor_id: 7,
                ..
            }
        ));
        assert_eq!(d.events[0].kind, EventKind::OperatorAssigned);
        assert_eq!(
            d.events[0].summary,
            "Reassigned from Ada Brook to Grace Lin (better fit known)."
        );
    }
}

#[test]
fn reassign_refuses_the_holder_and_a_missing_code() {
    let holder = contributor(1, 2);
    assert!(refused(decide(&offered_facts(), &assign(holder, FIT))).contains("already holds"));
    assert!(refused(decide(&assigned_facts(), &assign(grace(), None))).contains("reason"));
}

#[test]
fn close_from_attention_is_final() {
    let d = decide(
        &attention(AttentionReason::NoMatch),
        &op(OperatorOp::Close, 3),
    )
    .unwrap();
    assert_eq!(d.state, TaskState::Closed);
    assert_eq!(d.attention_reason, None);
    assert_eq!(d.events[0].kind, EventKind::TaskClosed);
    assert_eq!(d.events[0].summary, "Closed by the operator.");
    let closed = facts(TaskState::Closed);
    refused(decide(&closed, &route_again(None, true, 3)));
    refused(decide(&closed, &assign(grace(), FIT)));
    refused(decide(&closed, &op(OperatorOp::Submit, 3)));
    refused(decide(&closed, &reply(Reply::Accept)));
}
