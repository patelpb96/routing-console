//! Rule 9: email addresses and home-folder paths in text files (warn only).

use crate::model::{Level, Outcome};
use crate::walk::{Entry, lines, text_files};

const MAX_EXAMPLES: usize = 5;

fn local_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"._%+-".contains(&b)
}
fn domain_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
}
fn name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'
}

/// `local@domain.tld` spans in a line (byte ranges), excluding noreply / no-reply.
pub fn emails(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    for (at, _) in line.match_indices('@') {
        let mut s = at;
        while s > 0 && local_char(b[s - 1]) {
            s -= 1;
        }
        let mut e = at + 1;
        while e < b.len() && domain_char(b[e]) {
            e += 1;
        }
        while e > at + 1 && (b[e - 1] == b'.' || b[e - 1] == b'-') {
            e -= 1;
        }
        if s == at || e <= at + 1 {
            continue;
        }
        let domain = &line[at + 1..e];
        let Some((head, tld)) = domain.rsplit_once('.') else {
            continue;
        };
        if head.is_empty() || tld.len() < 2 || !tld.bytes().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let local = line[s..at].to_ascii_lowercase();
        if local == "noreply" || local == "no-reply" {
            continue;
        }
        out.push((s, e));
    }
    out
}

/// Length of a separator (`\\`, `\` or `/`) starting at `i`, or 0.
fn sep_at(b: &[u8], i: usize) -> usize {
    if b[i..].starts_with(b"\\\\") {
        2
    } else if i < b.len() && (b[i] == b'\\' || b[i] == b'/') {
        1
    } else {
        0
    }
}

/// Length of a separator ending just before `i`, or 0.
fn sep_before(b: &[u8], i: usize) -> usize {
    if i >= 2 && &b[i - 2..i] == b"\\\\" {
        2
    } else if i >= 1 && (b[i - 1] == b'\\' || b[i - 1] == b'/') {
        1
    } else {
        0
    }
}

fn name_end(b: &[u8], from: usize) -> Option<usize> {
    let n = b[from.min(b.len())..]
        .iter()
        .take_while(|c| name_char(**c))
        .count();
    (n > 0).then_some(from + n)
}

/// Home-folder path spans: `X:\Users\name` (any separator), `/Users/name`, `/home/name`.
fn home_paths(line: &str) -> Vec<(usize, usize)> {
    home_path_names(line)
        .into_iter()
        .map(|(s, _, e)| (s, e))
        .collect()
}

/// Home-folder paths as (start of the path, start of the name, end): `--redact` replaces only
/// the name, so the drive, the `Users` / `home` part and the separators stay as they were.
pub fn home_path_names(line: &str) -> Vec<(usize, usize, usize)> {
    let lower = line.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut out = Vec::new();
    for (i, _) in lower.match_indices("users") {
        let after = i + 5;
        let s = sep_at(b, after);
        let Some(end) = (s > 0).then(|| name_end(b, after + s)).flatten() else {
            continue;
        };
        let sb = sep_before(b, i);
        if sb > 0 && i >= sb + 2 && b[i - sb - 1] == b':' && b[i - sb - 2].is_ascii_alphabetic() {
            out.push((i - sb - 2, after + s, end));
        } else if i >= 1 && b[i - 1] == b'/' && b[after] == b'/' {
            out.push((i - 1, after + s, end));
        }
    }
    for (i, _) in lower.match_indices("/home/") {
        if let Some(end) = name_end(b, i + 6) {
            out.push((i, i + 6, end));
        }
    }
    out
}

/// Every hit in a line as text, overlapping spans merged (earliest, then longest, wins).
pub fn scan_line(line: &str) -> Vec<String> {
    let mut spans = emails(line);
    spans.extend(home_paths(line));
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut out = Vec::new();
    let mut last_end = 0;
    for (s, e) in spans {
        if s < last_end {
            continue;
        }
        out.push(line[s..e].to_string());
        last_end = e;
    }
    out
}

pub fn rule(files: &[Entry]) -> Outcome {
    let mut per_file = Vec::new();
    let mut examples: Vec<String> = Vec::new();
    let mut total = 0;
    for (e, text) in text_files(files) {
        let mut n = 0;
        for (_, line) in lines(&text) {
            for hit in scan_line(line) {
                n += 1;
                if examples.len() < MAX_EXAMPLES && !examples.contains(&hit) {
                    examples.push(hit);
                }
            }
        }
        if n > 0 {
            per_file.push(format!("{}: {n}", e.rel));
            total += n;
        }
    }
    if total == 0 {
        return Outcome::new(
            "personal-info",
            Level::Pass,
            "no email addresses or home-folder paths found",
        );
    }
    let mut extra = per_file;
    extra.push(format!("examples: {}", examples.join(", ")));
    Outcome::new(
        "personal-info",
        Level::Warn,
        format!("{total} email address(es) or home-folder path(s) found; consider redacting"),
    )
    .with_extra(extra)
}

#[cfg(test)]
mod tests {
    use super::scan_line;

    #[test]
    fn emails_found_and_noreply_skipped() {
        assert_eq!(
            scan_line("mail jo.doe@example.org now"),
            vec!["jo.doe@example.org"]
        );
        assert!(scan_line("NoReply@example.com and no-reply@example.com").is_empty());
        assert!(scan_line("user@localhost and @handle").is_empty());
        assert_eq!(scan_line("<a@b.co>."), vec!["a@b.co"]);
    }

    #[test]
    fn home_paths_in_all_shapes() {
        assert_eq!(scan_line(r"C:\Users\sam\x"), vec![r"C:\Users\sam"]);
        assert_eq!(scan_line(r"d:\\users\\sam.k"), vec![r"d:\\users\\sam.k"]);
        assert_eq!(scan_line("e:/USERS/sam"), vec!["e:/USERS/sam"]);
        assert_eq!(
            scan_line("/Users/sam/x and /home/kim"),
            vec!["/Users/sam", "/home/kim"]
        );
        assert!(scan_line("/users/ and /home/ and users/sam").is_empty());
    }
}
