//! Rule 5: session exports are present and every `.jsonl` line is one JSON value.

use crate::json;
use crate::model::{Level, Outcome};
use crate::walk::{Entry, ends_with_ci};
use std::fs;

const MAX_LISTED: usize = 20;

/// Problems found in one `.jsonl` file's bytes (empty when the file is fine).
pub fn check_jsonl(bytes: &[u8]) -> Vec<String> {
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return vec!["is empty or only whitespace".into()];
    }
    let mut out = Vec::new();
    for (n, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if let Err(e) = json::validate(line) {
            out.push(format!("line {}: not one JSON value ({e})", n + 1));
        }
    }
    out
}

pub fn rule(files: &[Entry]) -> Outcome {
    let mut problems = Vec::new();
    let mut count = 0;
    for e in files.iter().filter(|e| e.is_file) {
        let is_jsonl = ends_with_ci(e.name(), ".jsonl");
        let in_sessions = {
            let comps: Vec<&str> = e.components().collect();
            comps[..comps.len() - 1]
                .iter()
                .any(|c| c.eq_ignore_ascii_case("sessions"))
        };
        if in_sessions && !is_jsonl {
            problems.push(format!(
                "{}: inside a sessions folder but not a .jsonl file",
                e.rel
            ));
        }
        if !is_jsonl {
            continue;
        }
        count += 1;
        match fs::read(&e.path) {
            Ok(bytes) => {
                for p in check_jsonl(&bytes) {
                    problems.push(format!("{}: {p}", e.rel));
                }
            }
            Err(err) => problems.push(format!("{}: unreadable ({err})", e.rel)),
        }
    }
    if count == 0 {
        problems.insert(
            0,
            "no .jsonl session export under the submission directory".into(),
        );
    }
    if problems.is_empty() {
        return Outcome::new(
            "session-jsonl",
            Level::Pass,
            format!("{count} .jsonl file(s), every line parses as JSON"),
        );
    }
    let total = problems.len();
    if total > MAX_LISTED {
        problems.truncate(MAX_LISTED);
        problems.push(format!("... and {} more", total - MAX_LISTED));
    }
    Outcome::new(
        "session-jsonl",
        Level::Fail,
        format!("{total} problem(s) with session exports"),
    )
    .with_extra(problems)
}

#[cfg(test)]
mod tests {
    use super::check_jsonl;

    #[test]
    fn blank_lines_and_crlf_are_fine() {
        assert!(check_jsonl(b"{\"a\":1}\r\n\r\n  \n[2]\n").is_empty());
        assert!(check_jsonl(b"42\n\"text\"\n").is_empty());
    }

    #[test]
    fn empty_and_broken_files_fail() {
        assert_eq!(check_jsonl(b"").len(), 1);
        assert_eq!(check_jsonl(b" \r\n\n").len(), 1);
        assert_eq!(check_jsonl(b"{\"a\":1}\n{\"a\":\n").len(), 1);
        assert_eq!(check_jsonl(b"{} {}\n").len(), 1);
    }
}
