use super::*;

fn task(state: TaskState, version: i64) -> Task {
    Task {
        id: 1,
        title: "Label tickets".into(),
        area: Area::Finance,
        priority: Priority::Normal,
        state,
        version,
        attention_reason: None,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

fn facts(state: TaskState) -> TaskFacts {
    TaskFacts {
        task: task(state, 3),
        pending: None,
        active: None,
    }
}

fn routing_facts() -> TaskFacts {
    TaskFacts {
        pending: Some(PendingRequest {
            id: 7,
            started_at_ms: 0,
            excluded: vec![],
        }),
        ..facts(TaskState::Routing)
    }
}

fn offered_facts() -> TaskFacts {
    TaskFacts {
        active: Some(ActiveAssignment {
            id: 4,
            contributor_id: 2,
            contributor_name: "Ada Brook".into(),
            status: AssignmentStatus::Offered,
            source: Source::Engine,
            reason_code: None,
        }),
        ..facts(TaskState::Offered)
    }
}

fn contributor(load: i64, capacity: i64) -> ContributorFacts {
    ContributorFacts {
        id: 2,
        name: "Ada Brook".into(),
        areas: vec![Area::Finance],
        capacity,
        active_load: load,
    }
}

fn op(op: OperatorOp, expected_version: i64) -> Command {
    Command::Operator {
        expected_version,
        op,
        note: None,
    }
}

fn request(status: RequestStatus, superseded_by: Option<Supersession>) -> ReturnedRequest {
    ReturnedRequest {
        id: 7,
        status,
        has_result: false,
        superseded_by,
    }
}

fn returned(status: RequestStatus, c: ContributorFacts) -> Command {
    Command::EngineReturned {
        request: request(status, None),
        result: EngineResult::Match(c.id),
        contributor: Some(c),
    }
}

fn grace() -> ContributorFacts {
    ContributorFacts {
        id: 7,
        name: "Grace Lin".into(),
        areas: vec![Area::Finance],
        capacity: 1,
        active_load: 0,
    }
}

fn route_again(code: Option<ReasonCode>, exclude_previous: bool, v: i64) -> Command {
    op(
        OperatorOp::RouteAgain {
            reason_code: code,
            exclude_previous,
        },
        v,
    )
}

fn cancel(code: Option<ReasonCode>) -> Command {
    op(OperatorOp::CancelRouting { reason_code: code }, 3)
}

fn assign(c: ContributorFacts, code: Option<ReasonCode>) -> Command {
    op(
        OperatorOp::Assign {
            contributor: c,
            reason_code: code,
        },
        3,
    )
}

const SLOW: Option<ReasonCode> = Some(ReasonCode::EngineTooSlow);

fn refused(r: Result<Decision, Refusal>) -> String {
    match r {
        Err(Refusal::NotAllowed(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn create_trims_the_title_and_records_task_created() {
    let t = create("  Survey  ", Area::Operations, Priority::High).unwrap();
    assert_eq!(t.title, "Survey");
    assert_eq!(t.event.kind, EventKind::TaskCreated);
    assert_eq!(t.event.actor, Actor::Operator);
}

#[test]
fn create_refuses_an_empty_title() {
    assert!(create("   ", Area::Finance, Priority::Normal).is_err());
}

#[test]
fn submit_moves_unassigned_to_routing_and_opens_a_request() {
    let d = decide(&facts(TaskState::Unassigned), &op(OperatorOp::Submit, 3)).unwrap();
    assert_eq!(d.state, TaskState::Routing);
    assert!(d.bump_version);
    assert_eq!(d.changes, vec![Change::OpenRequest { excluded: vec![] }]);
    assert_eq!(d.events[0].kind, EventKind::RoutingStarted);
}

#[test]
fn engine_match_with_room_offers_the_task() {
    let d = decide(
        &routing_facts(),
        &returned(RequestStatus::Pending, contributor(1, 2)),
    )
    .unwrap();
    assert_eq!(d.state, TaskState::Offered);
    assert!(d.bump_version);
    assert!(d.changes.contains(&Change::OfferAssignment {
        contributor_id: 2,
        source: Source::Engine,
        reason_code: None,
        request_id: Some(7),
    }));
    assert_eq!(d.events[0].kind, EventKind::EngineMatched);
    assert_eq!(d.events[0].actor, Actor::Engine);
}

#[test]
fn accept_moves_offered_to_assigned() {
    let cmd = Command::Reply {
        assignment_id: 4,
        reply: Reply::Accept,
    };
    let d = decide(&offered_facts(), &cmd).unwrap();
    assert_eq!(d.state, TaskState::Assigned);
    assert_eq!(
        d.changes,
        vec![Change::AcceptAssignment { assignment_id: 4 }]
    );
    assert_eq!(d.events[0].kind, EventKind::ContributorAccepted);
}

#[test]
fn a_stale_expected_version_is_refused_as_stale() {
    let r = decide(&facts(TaskState::Unassigned), &op(OperatorOp::Submit, 2));
    assert_eq!(r, Err(Refusal::Stale));
}

#[test]
fn submit_is_refused_outside_unassigned() {
    for s in [TaskState::Routing, TaskState::Assigned, TaskState::Closed] {
        let m = refused(decide(&facts(s), &op(OperatorOp::Submit, 3)));
        assert!(m.contains("submit"), "{m}");
    }
}

#[test]
fn accept_of_an_assignment_that_is_not_the_active_offer_is_refused() {
    let cmd = Command::Reply {
        assignment_id: 99,
        reply: Reply::Accept,
    };
    let m = refused(decide(&offered_facts(), &cmd));
    assert!(m.contains("99"), "{m}");
}

mod unresolved;

#[test]
fn close_is_refused_on_a_closed_task() {
    refused(decide(&facts(TaskState::Closed), &op(OperatorOp::Close, 3)));
}

#[test]
fn allowed_commands_follow_the_rows_written() {
    assert_eq!(
        allowed_commands(&task(TaskState::Unassigned, 1)),
        vec![OperatorKind::Submit]
    );
    assert_eq!(
        allowed_commands(&task(TaskState::Routing, 1)),
        vec![
            OperatorKind::RouteAgain,
            OperatorKind::CancelRouting,
            OperatorKind::Assign
        ]
    );
    for s in [
        TaskState::Offered,
        TaskState::Assigned,
        TaskState::NeedsAttention,
        TaskState::Closed,
    ] {
        assert!(allowed_commands(&task(s, 1)).is_empty());
    }
}

#[test]
fn vocabulary_strings_round_trip() {
    for k in EventKind::ALL {
        assert_eq!(EventKind::parse(k.as_str()), Some(*k));
    }
    assert_eq!(EventKind::ALL.len(), 14);
    assert_eq!(TaskState::ALL.len(), 6);
}
