//! Rules 1 to 3: file count, file size and build output.

use crate::model::{Level, Outcome};
use crate::walk::{Entry, ends_with_ci};
use std::fs;
use std::path::Path;

const MAX_ENTRIES: usize = 10;
const BUILD_NAMES: [&str; 4] = ["node_modules", "target", "dist", "build"];
const DB_SUFFIXES: [&str; 5] = [".db", ".db-wal", ".db-shm", ".sqlite", ".sqlite3"];

/// Rule 1: entries directly inside the submission directory; a folder counts as one.
pub fn file_count(submission: &Path) -> Outcome {
    let n = fs::read_dir(submission).map(|rd| rd.count()).unwrap_or(0);
    let detail = format!("{n} top-level entries (limit {MAX_ENTRIES}; a folder counts as one zip)");
    let level = if n > MAX_ENTRIES {
        Level::Fail
    } else {
        Level::Pass
    };
    Outcome::new("file-count", level, detail)
}

/// Rule 2: every regular file at most `max_bytes`.
pub fn file_size(files: &[Entry], max_bytes: u64) -> Outcome {
    let over: Vec<String> = files
        .iter()
        .filter(|e| e.is_file && e.len > max_bytes)
        .map(|e| format!("{} ({} bytes)", e.rel, e.len))
        .collect();
    if over.is_empty() {
        let n = files.iter().filter(|e| e.is_file).count();
        return Outcome::new(
            "file-size",
            Level::Pass,
            format!("{n} files, none over {max_bytes} bytes"),
        );
    }
    Outcome::new(
        "file-size",
        Level::Fail,
        format!("{} file(s) over {max_bytes} bytes", over.len()),
    )
    .with_extra(over)
}

/// Rule 3: forbidden path components and database files.
pub fn build_output(files: &[Entry]) -> Outcome {
    let mut hits = Vec::new();
    for e in files {
        if let Some(c) = e
            .components()
            .find(|c| BUILD_NAMES.iter().any(|b| c.eq_ignore_ascii_case(b)))
        {
            // Report each offending folder once, not every file inside it.
            let upto = e.rel.split('/').take_while(|x| *x != c).count() + 1;
            let shown: Vec<&str> = e.rel.split('/').take(upto).collect();
            let shown = shown.join("/");
            if !hits.contains(&shown) {
                hits.push(shown);
            }
            continue;
        }
        if e.is_file && DB_SUFFIXES.iter().any(|s| ends_with_ci(e.name(), s)) {
            hits.push(e.rel.clone());
        }
    }
    if hits.is_empty() {
        Outcome::new(
            "build-output",
            Level::Pass,
            "no node_modules, target, dist, build or database files",
        )
    } else {
        Outcome::new(
            "build-output",
            Level::Fail,
            format!("{} build-output or database path(s) found", hits.len()),
        )
        .with_extra(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(rel: &str, len: u64) -> Entry {
        Entry {
            rel: rel.into(),
            path: PathBuf::from(rel),
            is_file: true,
            len,
        }
    }

    #[test]
    fn default_limit_is_exact() {
        let at = [file("a.bin", crate::model::DEFAULT_MAX_BYTES)];
        assert_eq!(
            file_size(&at, crate::model::DEFAULT_MAX_BYTES).level,
            Level::Pass
        );
        let over = [file("a.bin", crate::model::DEFAULT_MAX_BYTES + 1)];
        assert_eq!(
            file_size(&over, crate::model::DEFAULT_MAX_BYTES).level,
            Level::Fail
        );
    }

    #[test]
    fn build_names_are_whole_components() {
        let ok = [
            file("targets.md", 1),
            file("rebuild.txt", 1),
            file("distance.csv", 1),
            file("x.dbx", 1),
        ];
        assert_eq!(build_output(&ok).level, Level::Pass);
        assert_eq!(build_output(&[file("app/Target/x", 1)]).level, Level::Fail);
        assert_eq!(build_output(&[file("data.SQLite3", 1)]).level, Level::Fail);
    }
}
