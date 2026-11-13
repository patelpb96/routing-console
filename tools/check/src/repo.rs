//! Git helpers and rule 8, `company-name`.

use crate::model::{Level, Outcome};
use crate::walk::read_text;
use std::path::Path;
use std::process::Command;

/// Runs `git -C <dir> <args>`; `Some(stdout)` on success.
pub fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// True when `dir` exists and is itself the top level of a git work tree. A folder that is
/// merely inside some other repository (for example a case root under target/) does not count.
pub fn is_git_repo(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Some(out) = git(dir, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let top = String::from_utf8_lossy(&out).trim().to_string();
    match (std::fs::canonicalize(dir), std::fs::canonicalize(&top)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// NUL-separated output split into strings.
pub fn split_z(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

pub fn company_rule(repo: &Path, company: Option<&str>, public: bool) -> Outcome {
    let name = company.map(str::trim).filter(|s| !s.is_empty());
    let Some(name) = name else {
        return Outcome::new(
            "company-name",
            Level::Manual,
            "no company name given (--company-name or CHECK_COMPANY_NAME): check by hand",
        );
    };
    if !is_git_repo(repo) {
        return Outcome::new(
            "company-name",
            Level::Manual,
            format!("{} is not a git repository: check by hand", repo.display()),
        );
    }
    let needle = name.to_lowercase();
    let tracked = git(repo, &["ls-files", "-z"])
        .map(|b| split_z(&b))
        .unwrap_or_default();
    let mut path_hits = 0;
    let mut content_hits = Vec::new();
    for rel in &tracked {
        let path_hit = rel.to_lowercase().contains(&needle);
        if path_hit {
            path_hits += 1;
        }
        if let Some(text) = read_text(&repo.join(rel))
            && text.to_lowercase().contains(&needle)
        {
            // A path that itself names the company is not echoed.
            content_hits.push(if path_hit {
                "(a file whose path also names it)".to_string()
            } else {
                rel.clone()
            });
        }
    }
    let log = git(repo, &["log", "--all", "-z", "--format=%B"])
        .map(|b| split_z(&b))
        .unwrap_or_default();
    let msg_hits = log
        .iter()
        .filter(|m| m.to_lowercase().contains(&needle))
        .count();
    let total = path_hits + content_hits.len() + msg_hits;
    if total == 0 {
        return Outcome::new(
            "company-name",
            Level::Pass,
            format!(
                "name not found in {} tracked files, their paths or {} commit messages",
                tracked.len(),
                log.len()
            ),
        );
    }
    let level = if public { Level::Fail } else { Level::Warn };
    let mut extra: Vec<String> = content_hits
        .into_iter()
        .map(|p| format!("contents: {p}"))
        .collect();
    if path_hits > 0 {
        extra.push(format!("{path_hits} tracked path(s) contain the name"));
    }
    if msg_hits > 0 {
        extra.push(format!("{msg_hits} commit message(s) contain the name"));
    }
    let suffix = if public {
        "and the repository is public"
    } else {
        "(fails if the repository is public)"
    };
    Outcome::new(
        "company-name",
        level,
        format!("company name found {total} time(s) {suffix}"),
    )
    .with_extra(extra)
}
