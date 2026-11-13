//! The routing engine boundary (contract section 3) and the seeded simulator behind it.
//!
//! The simulator stands in for an outside service: it keeps its decisions in its own tables
//! (`sim_schedule`, `sim_scenarios`) and reports back only through the channel, so the app learns
//! nothing about a request except what an engine would tell it.

mod sim;

pub use sim::Simulator;

use crate::domain::{Area, EngineResult, Priority, vocab};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub active_load: i64,
    pub capacity: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineRequest {
    pub request_id: i64,
    pub task_id: i64,
    pub area: Area,
    pub priority: Priority,
    pub eligible: Vec<Candidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineReport {
    pub request_id: i64,
    pub result: EngineResult,
}

/// Accepts a request now; reports on the channel it was built with later, or never.
///
/// Delivery is at least once: a report the app has not acknowledged is sent again after a
/// restart, so a crash between the engine sending and the app committing loses nothing. The app
/// recognises a repeat because the request row already holds a result.
pub trait RoutingEngine: Send + Sync {
    fn start(&self, request: EngineRequest) -> Result<(), AppError>;
    /// The app has committed (or deliberately refused) the report for this request.
    fn acknowledge(&self, request_id: i64) -> Result<(), AppError>;
}

vocab!(Scenario {
    Auto = "auto",
    FastMatch = "fast_match",
    SlowMatch = "slow_match",
    NoMatch = "no_match",
    NeverReturns = "never_returns",
});

pub const DEFAULT_SEED: u64 = 42;
const NEVER_RETURNS_IN_100: u64 = 20;
const AUTO_MIN_MS: u64 = 5_000;
const AUTO_MAX_MS: u64 = 20_000;

/// What the simulator will do with one request. `delay_ms` of `None` means it never reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub delay_ms: Option<u64>,
    pub result: EngineResult,
}

/// Pure: the same seed, request id, scenario and list give the same plan, whatever order
/// requests arrive in.
pub fn plan(seed: u64, request_id: i64, scenario: Scenario, eligible: &[Candidate]) -> Plan {
    let h = |salt: u64| splitmix64(seed ^ splitmix64((request_id as u64) ^ salt));
    let matched =
        || least_loaded(eligible, h(3)).map_or(EngineResult::NoMatch, EngineResult::Match);
    let (delay_ms, result) = match scenario {
        Scenario::FastMatch => (Some(6_000), matched()),
        Scenario::SlowMatch => (Some(26_000), matched()),
        Scenario::NoMatch => (Some(8_000), EngineResult::NoMatch),
        Scenario::NeverReturns => (None, EngineResult::NoMatch),
        Scenario::Auto if h(1) % 100 < NEVER_RETURNS_IN_100 => (None, EngineResult::NoMatch),
        Scenario::Auto => (
            Some(AUTO_MIN_MS + h(2) % (AUTO_MAX_MS - AUTO_MIN_MS + 1)),
            matched(),
        ),
    };
    Plan { delay_ms, result }
}

/// Lowest active load wins; a tie is broken by the hash so it does not always favour low ids.
fn least_loaded(eligible: &[Candidate], hash: u64) -> Option<i64> {
    let min = eligible.iter().map(|c| c.active_load).min()?;
    let ties: Vec<i64> = eligible
        .iter()
        .filter(|c| c.active_load == min)
        .map(|c| c.id)
        .collect();
    Some(ties[(hash % ties.len() as u64) as usize])
}

/// A fixed, well-mixed hash; std's `DefaultHasher` may change between Rust releases.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: i64, load: i64) -> Candidate {
        Candidate {
            id,
            active_load: load,
            capacity: 2,
        }
    }

    #[test]
    fn scenarios_fix_the_delay_and_outcome() {
        let list = [c(1, 1), c(2, 0)];
        assert_eq!(
            plan(42, 1, Scenario::FastMatch, &list),
            Plan {
                delay_ms: Some(6_000),
                result: EngineResult::Match(2)
            }
        );
        assert_eq!(
            plan(42, 1, Scenario::SlowMatch, &list).delay_ms,
            Some(26_000)
        );
        assert_eq!(
            plan(42, 1, Scenario::NoMatch, &list),
            Plan {
                delay_ms: Some(8_000),
                result: EngineResult::NoMatch
            }
        );
        assert_eq!(plan(42, 1, Scenario::NeverReturns, &list).delay_ms, None);
        assert_eq!(
            plan(42, 1, Scenario::FastMatch, &[]).result,
            EngineResult::NoMatch
        );
    }

    #[test]
    fn auto_stays_inside_the_normal_window_and_about_a_fifth_never_return() {
        let list = [c(1, 0)];
        let plans: Vec<Plan> = (1..=1_000)
            .map(|id| plan(7, id, Scenario::Auto, &list))
            .collect();
        let never = plans.iter().filter(|p| p.delay_ms.is_none()).count();
        assert!(
            (150..=250).contains(&never),
            "{never} of 1000 never returned"
        );
        for d in plans.iter().filter_map(|p| p.delay_ms) {
            assert!((AUTO_MIN_MS..=AUTO_MAX_MS).contains(&d), "{d}");
        }
    }
}
