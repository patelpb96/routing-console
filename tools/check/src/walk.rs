//! Recursive walk of the submission directory and the text-file definition.

use std::fs;
use std::path::{Path, PathBuf};

/// One entry under the submission directory (symlinks are not followed).
#[derive(Clone, Debug)]
pub struct Entry {
    /// Path relative to the submission directory, `/`-separated.
    pub rel: String,
    pub path: PathBuf,
    /// A regular file (not a directory, not a symlink).
    pub is_file: bool,
    pub len: u64,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.rel.split('/')
    }
}

/// Every entry under `root`, recursively, sorted by relative path. Missing root gives nothing.
pub fn walk(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    walk_into(root, "", &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn walk_into(dir: &Path, prefix: &str, out: &mut Vec<Entry>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let path = item.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let ft = meta.file_type();
        let entry = Entry {
            rel: rel.clone(),
            path: path.clone(),
            is_file: ft.is_file(),
            len: if ft.is_file() { meta.len() } else { 0 },
        };
        out.push(entry);
        if ft.is_dir() {
            walk_into(&path, &rel, out);
        }
    }
}

/// True when no NUL byte appears in the first 8192 bytes.
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes[..bytes.len().min(8192)].contains(&0)
}

/// Reads a file as text (UTF-8, invalid bytes replaced). `None` if unreadable or not text.
pub fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if !is_text(&bytes) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Lines of a text, accepting LF and CRLF; numbered from 1.
pub fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .enumerate()
        .map(|(i, l)| (i + 1, l))
}

/// The regular text files under the submission directory, with their contents.
pub fn text_files(files: &[Entry]) -> Vec<(&Entry, String)> {
    files
        .iter()
        .filter(|e| e.is_file)
        .filter_map(|e| read_text(&e.path).map(|t| (e, t)))
        .collect()
}

pub fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_detection_only_looks_at_first_8192_bytes() {
        let mut v = vec![b'a'; 8192];
        assert!(is_text(&v));
        v.push(0);
        assert!(is_text(&v));
        v[8191] = 0;
        assert!(!is_text(&v));
    }

    #[test]
    fn crlf_lines_are_stripped() {
        let got: Vec<_> = lines("a\r\nb\n").collect();
        assert_eq!(got, vec![(1, "a"), (2, "b"), (3, "")]);
    }

    #[test]
    fn suffix_ignores_case() {
        assert!(ends_with_ci("A.JSONL", ".jsonl"));
        assert!(!ends_with_ci("a.json", ".jsonl"));
    }
}
