//! The words of the contract (sections 1, 2 and 5) as types. Each is stored and sent as the same
//! snake_case string, so the database, the JSON and the page all read the same spelling.

/// One enum per vocabulary list, with its string form generated from a single table.
macro_rules! vocab {
    ($(#[$m:meta])* $name:ident { $($var:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($var),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$var => $s),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$var),)+ _ => None }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
pub(crate) use vocab;

vocab!(Area {
    Finance = "finance",
    Healthcare = "healthcare",
    DataAnalysis = "data_analysis",
    Operations = "operations",
    Veterinary = "veterinary",
});

vocab!(Priority {
    High = "high",
    Normal = "normal",
});

vocab!(TaskState {
    Unassigned = "unassigned",
    Routing = "routing",
    Offered = "offered",
    Assigned = "assigned",
    NeedsAttention = "needs_attention",
    Closed = "closed",
});

vocab!(AttentionReason {
    Declined = "declined",
    OfferFailed = "offer_failed",
    NoMatch = "no_match",
    SelectionUnavailable = "selection_unavailable",
});

vocab!(ReasonCode {
    EngineTooSlow = "engine_too_slow",
    BetterFitKnown = "better_fit_known",
    ContributorUnavailable = "contributor_unavailable",
    Priority = "priority",
    Other = "other",
});

impl ReasonCode {
    pub fn label(self) -> &'static str {
        match self {
            ReasonCode::EngineTooSlow => "Engine too slow",
            ReasonCode::BetterFitKnown => "Better fit known",
            ReasonCode::ContributorUnavailable => "Contributor unavailable",
            ReasonCode::Priority => "Priority",
            ReasonCode::Other => "Other",
        }
    }
}

vocab!(RequestStatus {
    Pending = "pending",
    Resolved = "resolved",
    Superseded = "superseded",
});

vocab!(ResultKind {
    Match = "match",
    NoMatch = "no_match",
});

vocab!(AssignmentStatus {
    Offered = "offered",
    Accepted = "accepted",
    Declined = "declined",
    Failed = "failed",
    Withdrawn = "withdrawn",
});

vocab!(Source {
    Engine = "engine",
    Operator = "operator",
});

vocab!(Actor {
    Operator = "operator",
    Engine = "engine",
    Contributor = "contributor",
    System = "system",
});

vocab!(EventKind {
    TaskCreated = "task_created",
    RoutingStarted = "routing_started",
    EngineMatched = "engine_matched",
    EngineNoMatch = "engine_no_match",
    EngineSelectionUnavailable = "engine_selection_unavailable",
    OperatorAssigned = "operator_assigned",
    OperatorRerouted = "operator_rerouted",
    OperatorCancelledRouting = "operator_cancelled_routing",
    LateResultIgnored = "late_result_ignored",
    ContributorAccepted = "contributor_accepted",
    ContributorDeclined = "contributor_declined",
    OfferFailed = "offer_failed",
    TaskClosed = "task_closed",
    ServerRestarted = "server_restarted",
});

vocab!(
    /// The operator commands of `POST /api/tasks/{id}/commands`.
    OperatorKind {
        Submit = "submit",
        RouteAgain = "route_again",
        CancelRouting = "cancel_routing",
        Assign = "assign",
        Close = "close",
    }
);

vocab!(Reply {
    Accept = "accept",
    Decline = "decline",
    Fail = "fail",
});
