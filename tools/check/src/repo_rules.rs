//! `--repo-rules`: three warn-only rules of the orchestrator's own (not from the brief).

use crate::repo::{git, split_z};
use crate::walk::ends_with_ci;
use std::fs;
use std::path::Path;

const ALLOWED_EXT: [&str; 13] = [
    "rs", "toml", "lock", "md", "html", "css", "js", "json", "jsonl", "case", "txt", "sql", "svg",
];
const KNOWN_NAMES: [&str; 6] = [
    "LICENSE",
    "COPYING",
    "Makefile",
    "Dockerfile",
    ".gitignore",
    ".gitattributes",
];
const MAX_FILE_BYTES: u64 = 1_048_576;
const MAX_LINES: usize = 400;
const LINE_LIMITED: [&str; 4] = [".rs", ".js", ".css", ".html"];

/// Extension allowed, or an extension-less dotfile, or a known extension-less name.
pub fn ext_allowed(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if KNOWN_NAMES.iter().any(|k| k.eq_ignore_ascii_case(name)) {
        return true;
    }
    if let Some(stem) = name.strip_prefix('.')
        && !stem.is_empty()
        && !stem.contains('.')
    {
        return true; // extension-less dotfile such as .editorconfig
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            ALLOWED_EXT.iter().any(|a| a.eq_ignore_ascii_case(ext))
        }
        _ => false,
    }
}

pub fn count_lines(bytes: &[u8]) -> usize {
    let n = bytes.iter().filter(|b| **b == b'\n').count();
    if bytes.last().is_some_and(|b| *b != b'\n') {
        n + 1
    } else {
        n
    }
}

fn report(id: &str, ok_detail: &str, offenders: &[String]) {
    if offenders.is_empty() {
        println!("PASS {id}: {ok_detail}");
    } else {
        println!("WARN {id}: {} file(s)", offenders.len());
        for o in offenders {
            println!("  {o}");
        }
    }
}

/// Prints the three lines. Returns the exit code (always 0 once the directory exists).
pub fn run(dir: &Path) -> i32 {
    let Some(out) = git(
        dir,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) else {
        for id in ["repo-ext", "repo-size", "repo-lines"] {
            println!("WARN {id}: {} is not a git repository", dir.display());
        }
        return 0;
    };
    let mut files = split_z(&out);
    files.sort();
    files.dedup();
    let mut bad_ext = Vec::new();
    let mut too_big = Vec::new();
    let mut too_long = Vec::new();
    for rel in &files {
        if !ext_allowed(rel) {
            bad_ext.push(rel.clone());
        }
        let path = dir.join(rel);
        let Ok(meta) = fs::metadata(&path) else {
            continue; // tracked but deleted from the working tree
        };
        if meta.len() > MAX_FILE_BYTES {
            too_big.push(format!("{rel} ({} bytes)", meta.len()));
        }
        let limited = LINE_LIMITED.iter().any(|s| ends_with_ci(rel, s));
        let in_docs = rel.len() >= 5 && rel[..5].eq_ignore_ascii_case("docs/");
        if limited
            && !in_docs
            && let Ok(bytes) = fs::read(&path)
        {
            let n = count_lines(&bytes);
            if n > MAX_LINES {
                too_long.push(format!("{rel} ({n} lines)"));
            }
        }
    }
    report(
        "repo-ext",
        &format!("{} files, all extensions allowed", files.len()),
        &bad_ext,
    );
    report(
        "repo-size",
        &format!("no file over {MAX_FILE_BYTES} bytes"),
        &too_big,
    );
    report(
        "repo-lines",
        &format!("no .rs/.js/.css/.html file outside docs/ over {MAX_LINES} lines"),
        &too_long,
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        for ok in [
            "src/main.rs",
            "Cargo.lock",
            ".gitignore",
            ".cargo/config.toml",
            "tools/check/cases/x/b1-1.case",
            "LICENSE",
            "A.MD",
        ] {
            assert!(ext_allowed(ok), "{ok}");
        }
        for bad in ["a.exe", "app.db", "README", "x.tar.gz", ".env.local"] {
            assert!(!ext_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn line_count() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }
}
