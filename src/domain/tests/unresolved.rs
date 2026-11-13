//! Rows added for unresolved routing: overrides, late results, restarts.

use super::*;

#[test]
fn a_match_for_a_full_contributor_needs_attention_as_selection_unavailable() {
    let d = decide(
        &routing_facts(),
        &returned(RequestStatus::Pending, contributor(2, 2)),
    )
    .unwrap();
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(
        d.attention_reason,
        Some(AttentionReason::SelectionUnavailable)
    );
    assert!(d.bump_version);
    assert_eq!(
        d.changes,
        vec![Change::ResolveRequest {
            request_id: 7,
            result: EngineResult::Match(2)
        }]
    );
    assert_eq!(d.events[0].kind, EventKind::EngineSelectionUnavailable);
    assert!(
        d.events[0].summary.contains("no room"),
        "{}",
        d.events[0].summary
    );
}

#[test]
fn no_match_needs_attention_as_no_match() {
    let cmd = Command::EngineReturned {
        request: request(RequestStatus::Pending, None),
        result: EngineResult::NoMatch,
        contributor: None,
    };
    let d = decide(&routing_facts(), &cmd).unwrap();
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(d.attention_reason, Some(AttentionReason::NoMatch));
    assert_eq!(d.events[0].kind, EventKind::EngineNoMatch);
}

#[test]
fn a_match_naming_an_unknown_contributor_is_refused() {
    let cmd = Command::EngineReturned {
        request: request(RequestStatus::Pending, None),
        result: EngineResult::Match(99),
        contributor: None,
    };
    assert!(refused(decide(&routing_facts(), &cmd)).contains("99"));
}

#[test]
fn route_again_supersedes_opens_and_bumps_the_version() {
    // Contract section 2: the pending request changed, so the version goes up.
    let d = decide(&routing_facts(), &route_again(SLOW, false, 3)).unwrap();
    assert_eq!(d.state, TaskState::Routing);
    assert!(d.bump_version);
    assert_eq!(
        d.changes,
        vec![
            Change::SupersedeRequest { request_id: 7 },
            Change::OpenRequest { excluded: vec![] }
        ]
    );
    assert_eq!(d.events[0].kind, EventKind::OperatorRerouted);
    assert_eq!(d.events[0].data["superseded_request_id"], 7);
    assert_eq!(d.events[0].data["reason_code"], "engine_too_slow");
}

#[test]
fn route_again_refuses_a_missing_code_and_exclude_previous() {
    assert!(refused(decide(&routing_facts(), &route_again(None, false, 3))).contains("reason"));
    refused(decide(&routing_facts(), &route_again(SLOW, true, 3)));
    assert_eq!(
        decide(&routing_facts(), &route_again(SLOW, false, 2)),
        Err(Refusal::Stale)
    );
}

#[test]
fn cancel_routing_supersedes_and_returns_to_unassigned() {
    let d = decide(&routing_facts(), &cancel(SLOW)).unwrap();
    assert_eq!(d.state, TaskState::Unassigned);
    assert!(d.bump_version);
    assert_eq!(d.changes, vec![Change::SupersedeRequest { request_id: 7 }]);
    assert_eq!(d.events[0].kind, EventKind::OperatorCancelledRouting);
    refused(decide(&routing_facts(), &cancel(None)));
}

#[test]
fn assign_supersedes_and_offers_with_source_operator() {
    let d = decide(&routing_facts(), &assign(grace(), SLOW)).unwrap();
    assert_eq!(d.state, TaskState::Offered);
    assert!(d.bump_version);
    assert_eq!(
        d.changes,
        vec![
            Change::SupersedeRequest { request_id: 7 },
            Change::OfferAssignment {
                contributor_id: 7,
                source: Source::Operator,
                reason_code: SLOW,
                request_id: None,
            }
        ]
    );
    assert_eq!(d.events[0].kind, EventKind::OperatorAssigned);
    assert_eq!(d.events[0].data["contributor_name"], "Grace Lin");
}

#[test]
fn assign_refuses_missing_code_wrong_area_and_no_room() {
    assert!(refused(decide(&routing_facts(), &assign(grace(), None))).contains("reason"));
    let vet = ContributorFacts {
        areas: vec![Area::Veterinary],
        ..grace()
    };
    assert!(refused(decide(&routing_facts(), &assign(vet, SLOW))).contains("does not cover"));
    let full = ContributorFacts {
        active_load: 1,
        ..grace()
    };
    assert!(refused(decide(&routing_facts(), &assign(full, SLOW))).contains("no room"));
}

#[test]
fn a_late_result_is_recorded_and_told_but_changes_nothing_else() {
    let mut f = offered_facts();
    f.task.state = TaskState::Offered;
    let cmd = Command::EngineReturned {
        request: request(
            RequestStatus::Superseded,
            Some(Supersession::Assigned {
                contributor_name: "Omar Haddad".into(),
            }),
        ),
        result: EngineResult::Match(7),
        contributor: Some(grace()),
    };
    let d = decide(&f, &cmd).unwrap();
    assert_eq!(d.state, TaskState::Offered);
    assert!(!d.bump_version);
    assert_eq!(
        d.changes,
        vec![Change::RecordLateResult {
            request_id: 7,
            result: EngineResult::Match(7)
        }]
    );
    assert_eq!(d.events.len(), 1);
    assert_eq!(d.events[0].kind, EventKind::LateResultIgnored);
    assert_eq!(
        d.events[0].summary,
        "The engine returned Grace Lin for a request you had already replaced by assigning \
         Omar Haddad. Recorded, not applied."
    );
}

#[test]
fn a_late_result_keeps_the_attention_reason_of_any_state() {
    let mut f = facts(TaskState::NeedsAttention);
    f.task.attention_reason = Some(AttentionReason::NoMatch);
    let cmd = Command::EngineReturned {
        request: request(RequestStatus::Superseded, Some(Supersession::Rerouted)),
        result: EngineResult::NoMatch,
        contributor: None,
    };
    let d = decide(&f, &cmd).unwrap();
    assert_eq!(d.state, TaskState::NeedsAttention);
    assert_eq!(d.attention_reason, Some(AttentionReason::NoMatch));
    assert!(d.events[0].summary.contains("routing again"));
}

#[test]
fn a_second_result_for_the_same_request_is_refused() {
    let mut late = request(RequestStatus::Superseded, Some(Supersession::Cancelled));
    late.has_result = true;
    let cmd = Command::EngineReturned {
        request: late,
        result: EngineResult::NoMatch,
        contributor: None,
    };
    refused(decide(&facts(TaskState::Unassigned), &cmd));
    let resolved = Command::EngineReturned {
        request: request(RequestStatus::Resolved, None),
        result: EngineResult::NoMatch,
        contributor: None,
    };
    refused(decide(&offered_facts(), &resolved));
}

#[test]
fn server_restarted_is_a_system_note_without_a_version_change() {
    let d = decide(&routing_facts(), &Command::ServerRestarted { boot: 2 }).unwrap();
    assert_eq!(d.state, TaskState::Routing);
    assert!(!d.bump_version);
    assert!(d.changes.is_empty());
    assert_eq!(d.events[0].kind, EventKind::ServerRestarted);
    assert_eq!(d.events[0].actor, Actor::System);
    refused(decide(
        &facts(TaskState::Unassigned),
        &Command::ServerRestarted { boot: 2 },
    ));
}

#[test]
fn close_from_routing_is_still_refused() {
    refused(decide(&routing_facts(), &op(OperatorOp::Close, 3)));
}

#[test]
fn assignable_contributors_uses_the_assign_check_and_only_where_assign_is_allowed() {
    let vet = ContributorFacts {
        id: 8,
        areas: vec![Area::Veterinary],
        ..grace()
    };
    let full = ContributorFacts {
        id: 9,
        active_load: 1,
        ..grace()
    };
    let all = [contributor(0, 2), grace(), vet, full];
    assert_eq!(assignable_contributors(&routing_facts(), &all), vec![2, 7]);
    // Reassigning from offered is a row now; the holder (2) is left out.
    assert_eq!(assignable_contributors(&offered_facts(), &all), vec![7]);
    assert!(assignable_contributors(&facts(TaskState::Closed), &all).is_empty());
    assert!(assignable_contributors(&facts(TaskState::Unassigned), &all).is_empty());
}
