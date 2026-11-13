//! `check --redact <dir>`: rewrites the text files under `<dir>` in place so that rules 4
//! (`secrets`) and 9 (`personal-info`) find nothing, using those rules' own scanners. Each file
//! is written to a temporary file beside it and renamed over the original only when a rescan
//! is clean and, for `.jsonl`, every line still parses. Rule 8 (the company's name) is left to
//! the person.

use crate::personal;
use crate::secrets;
use crate::sessions::check_jsonl;
use crate::walk::{Entry, ends_with_ci, is_text, walk};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_PASSES: usize = 5;
const REDACTED: &str = "[REDACTED]";

/// Replacement counts by kind (a secret pattern name, `email` or `home-path`).
pub type Counts = BTreeMap<&'static str, usize>;

/// The workspace root, fixed at compile time as for the case harness.
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

/// The script's guard: the target must exist, sit inside the workspace root without being it,
/// and have no `.git` component. Returns the resolved directory.
pub fn guard(dir: &Path, root: &Path) -> Result<PathBuf, String> {
    let refuse = |why: String| {
        Err(format!(
            "--redact refused by this script's guard: {why}. It only rewrites packed copies \
             inside the workspace (such as submission/ or a folder under target/)."
        ))
    };
    if dir
        .components()
        .any(|c| c == Component::Normal(".git".as_ref()))
    {
        return refuse(format!("{} has a .git component", dir.display()));
    }
    let Ok(real) = dir.canonicalize() else {
        return refuse(format!("{} does not exist", dir.display()));
    };
    if !real.is_dir() {
        return refuse(format!("{} is not a directory", dir.display()));
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("workspace root {}: {e}", root.display()))?;
    if real == root {
        return refuse(format!("{} is the workspace root itself", dir.display()));
    }
    if !real.starts_with(&root) {
        return refuse(format!("{} is outside the workspace root", dir.display()));
    }
    if real
        .components()
        .any(|c| c == Component::Normal(".git".as_ref()))
    {
        return refuse(format!("{} resolves into a .git folder", dir.display()));
    }
    Ok(real)
}

/// One replacement: byte range, what goes in, and its kind.
struct Edit {
    start: usize,
    end: usize,
    with: &'static str,
    kind: &'static str,
}

/// If `start` falls just after a backslash that opens a JSON escape (an odd run of
/// backslashes), move it past that escape so the escape stays whole.
fn after_escape(line: &str, start: usize) -> usize {
    let b = line.as_bytes();
    let run = b[..start].iter().rev().take_while(|c| **c == b'\\').count();
    if run % 2 == 0 || start >= b.len() {
        return start;
    }
    let skip = if b[start] == b'u' { 5 } else { 1 };
    let mut s = (start + skip).min(b.len());
    while !line.is_char_boundary(s) {
        s += 1;
    }
    s
}

fn edits(line: &str) -> Vec<Edit> {
    let mut out: Vec<Edit> = secrets::hits(line)
        .into_iter()
        .map(|h| Edit {
            start: h.start,
            end: h.end,
            with: REDACTED,
            kind: h.name,
        })
        .collect();
    for (s, e) in personal::emails(line) {
        out.push(Edit {
            start: after_escape(line, s),
            end: e,
            with: "[email]",
            kind: "email",
        });
    }
    for (_, name, e) in personal::home_path_names(line) {
        out.push(Edit {
            start: name,
            end: e,
            with: "[user]",
            kind: "home-path",
        });
    }
    out.retain(|e| e.start < e.end);
    out.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    out
}

fn dirty(line: &str) -> bool {
    !secrets::scan_line(line).is_empty() || !personal::scan_line(line).is_empty()
}

/// Redacts one line, repeating until no pattern matches. `Err` if it still matches after
/// `MAX_PASSES` passes.
pub fn redact_line(line: &str, counts: &mut Counts) -> Result<String, ()> {
    let mut cur = line.to_string();
    for _ in 0..MAX_PASSES {
        let edits = edits(&cur);
        if edits.is_empty() {
            break;
        }
        let mut next = String::with_capacity(cur.len());
        let mut at = 0;
        for e in edits {
            if e.start < at {
                continue; // overlaps an earlier edit; the next pass sees what is left
            }
            next.push_str(&cur[at..e.start]);
            next.push_str(e.with);
            at = e.end;
            *counts.entry(e.kind).or_default() += 1;
        }
        next.push_str(&cur[at..]);
        cur = next;
    }
    if dirty(&cur) { Err(()) } else { Ok(cur) }
}

/// Redacts a whole text, keeping LF / CRLF endings. `Err` lists the line numbers that would
/// not come clean.
pub fn redact_text(text: &str, counts: &mut Counts) -> Result<String, Vec<usize>> {
    let mut out = String::with_capacity(text.len());
    let mut bad = Vec::new();
    for (n, raw) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let (line, cr) = match raw.strip_suffix('\r') {
            Some(l) => (l, "\r"),
            None => (raw, ""),
        };
        match redact_line(line, counts) {
            Ok(l) => out.push_str(&l),
            Err(()) => {
                bad.push(n + 1);
                out.push_str(line);
            }
        }
        out.push_str(cr);
    }
    if bad.is_empty() { Ok(out) } else { Err(bad) }
}

enum FileResult {
    Unchanged,
    Changed(Counts),
    Skipped(&'static str),
    Failed(String),
}

fn redact_file(e: &Entry) -> FileResult {
    let Ok(bytes) = fs::read(&e.path) else {
        return FileResult::Failed("unreadable".into());
    };
    if !is_text(&bytes) {
        return FileResult::Skipped("not text");
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return FileResult::Failed("not valid UTF-8; left untouched".into());
    };
    let mut counts = Counts::new();
    let new = match redact_text(&text, &mut counts) {
        Ok(t) => t,
        Err(lines) => {
            return FileResult::Failed(format!(
                "line(s) {lines:?} still match after {MAX_PASSES} passes; file left untouched"
            ));
        }
    };
    if new == text {
        return FileResult::Unchanged;
    }
    if new.split('\n').any(dirty) {
        return FileResult::Failed("rescan found a hit; file left untouched".into());
    }
    if ends_with_ci(e.name(), ".jsonl") {
        let problems = check_jsonl(new.as_bytes());
        if !problems.is_empty() {
            return FileResult::Failed(format!(
                "redacted copy would not parse ({}); file left untouched",
                problems[0]
            ));
        }
    }
    let mut tmp = e.path.clone().into_os_string();
    tmp.push(".redact-tmp");
    let tmp = PathBuf::from(tmp);
    if let Err(err) = fs::write(&tmp, &new).and_then(|_| fs::rename(&tmp, &e.path)) {
        let _ = fs::remove_file(&tmp);
        return FileResult::Failed(format!("could not write: {err}"));
    }
    FileResult::Changed(counts)
}

/// Runs the mode over `dir` (already through `guard`) and prints the report. Exit code.
pub fn run(dir: &Path, shown: &str) -> i32 {
    let (mut total, mut skipped, mut failed) = (0, Vec::new(), false);
    for e in walk(dir).iter().filter(|e| e.is_file) {
        if e.components().any(|c| c == ".git") {
            skipped.push(format!("{} (inside .git)", e.rel));
            continue;
        }
        match redact_file(e) {
            FileResult::Unchanged => println!("{}: no change", e.rel),
            FileResult::Changed(c) => {
                let n: usize = c.values().sum();
                total += n;
                let kinds: Vec<String> = c.iter().map(|(k, v)| format!("{k} {v}")).collect();
                println!("{}: {n} replaced ({})", e.rel, kinds.join(", "));
            }
            FileResult::Skipped(why) => skipped.push(format!("{} ({why})", e.rel)),
            FileResult::Failed(why) => {
                failed = true;
                println!("{}: NOT REDACTED: {why}", e.rel);
            }
        }
    }
    if total == 0 && !failed {
        println!("total: 0 replacements; nothing changed");
    } else {
        println!("total: {total} replacement(s)");
    }
    if skipped.is_empty() {
        println!("skipped: none");
    } else {
        println!("skipped: {}", skipped.join(", "));
    }
    println!("Now run the checker again: cargo run -q -p check -- {shown}");
    i32::from(failed)
}

#[cfg(test)]
mod tests;
