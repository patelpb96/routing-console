//! Shared types: the configuration the rules read and the outcome each rule returns.

use std::path::PathBuf;

/// The 11 rule ids, in output order.
pub const RULE_IDS: [&str; 11] = [
    "file-count",
    "file-size",
    "build-output",
    "secrets",
    "session-jsonl",
    "answer-template",
    "session-map",
    "company-name",
    "personal-info",
    "time-budget",
    "video-length",
];

pub const DEFAULT_MAX_BYTES: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Pass,
    Fail,
    Warn,
    Manual,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Pass => "PASS",
            Level::Fail => "FAIL",
            Level::Warn => "WARN",
            Level::Manual => "MANUAL",
        }
    }

    pub fn lower(self) -> &'static str {
        match self {
            Level::Pass => "pass",
            Level::Fail => "fail",
            Level::Warn => "warn",
            Level::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pass" => Some(Level::Pass),
            "fail" => Some(Level::Fail),
            "warn" => Some(Level::Warn),
            "manual" => Some(Level::Manual),
            _ => None,
        }
    }
}

/// One rule's result: a level, a one-line detail and optional extra lines.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub rule: &'static str,
    pub level: Level,
    pub detail: String,
    pub extra: Vec<String>,
}

impl Outcome {
    pub fn new(rule: &'static str, level: Level, detail: impl Into<String>) -> Outcome {
        Outcome {
            rule,
            level,
            detail: detail.into(),
            extra: Vec::new(),
        }
    }

    pub fn with_extra(mut self, extra: Vec<String>) -> Outcome {
        self.extra = extra;
        self
    }
}

/// Everything the rules need. Built from the command line or by the case harness.
#[derive(Clone, Debug)]
pub struct Config {
    pub submission: PathBuf,
    pub answer: PathBuf,
    pub repo: PathBuf,
    pub ledger: PathBuf,
    pub company: Option<String>,
    pub public: bool,
    pub max_bytes: u64,
}

/// Runs all 11 rules in order.
pub fn run_all(cfg: &Config) -> Vec<Outcome> {
    let files = crate::walk::walk(&cfg.submission);
    vec![
        crate::rules_files::file_count(&cfg.submission),
        crate::rules_files::file_size(&files, cfg.max_bytes),
        crate::rules_files::build_output(&files),
        crate::secrets::rule(&files),
        crate::sessions::rule(&files),
        crate::answer::template_rule(&cfg.answer),
        crate::answer::session_map_rule(&cfg.answer, &files),
        crate::repo::company_rule(&cfg.repo, cfg.company.as_deref(), cfg.public),
        crate::personal::rule(&files),
        crate::ledger::rule(&cfg.ledger),
        Outcome::new(
            "video-length",
            Level::Manual,
            "[ ] the walkthrough video is 10 minutes or shorter (check it by hand)",
        ),
    ]
}
