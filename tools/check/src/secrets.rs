//! Rule 4: secret patterns. A found secret is never printed: only file, line and pattern name.

use crate::model::{Level, Outcome};
use crate::walk::{Entry, lines, text_files};

type CharSet = fn(u8) -> bool;

fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}
fn alnum_us_dash(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}
fn alnum_us(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn alnum_dash(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}
fn upper_digit(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}
fn bearer_chars(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b".-_~+/=".contains(&b)
}

/// (pattern name, case-sensitive prefix, character set, minimum run length)
const PREFIXES: &[(&str, &str, CharSet, usize)] = &[
    ("anthropic-key", "sk-ant-", alnum_us_dash, 20),
    ("sk-key", "sk-proj-", alnum, 20),
    ("sk-key", "sk-", alnum, 20),
    ("github-token", "ghp_", alnum, 36),
    ("github-token", "gho_", alnum, 36),
    ("github-token", "ghs_", alnum, 36),
    ("github-token", "ghu_", alnum, 36),
    ("github-token", "ghr_", alnum, 36),
    ("github-token", "github_pat_", alnum_us, 22),
    ("aws-key-id", "AKIA", upper_digit, 16),
    ("slack-token", "xoxb-", alnum_dash, 10),
    ("slack-token", "xoxp-", alnum_dash, 10),
    ("slack-token", "xoxa-", alnum_dash, 10),
    ("slack-token", "xoxs-", alnum_dash, 10),
    ("google-key", "AIza", alnum_us_dash, 35),
];

fn run_len(bytes: &[u8], from: usize, set: CharSet) -> usize {
    bytes[from.min(bytes.len())..]
        .iter()
        .take_while(|b| set(**b))
        .count()
}

/// The character just before byte `i` is not a letter, digit, `_` or `-`.
fn left_ok(line: &str, i: usize) -> bool {
    match line[..i].chars().next_back() {
        None => true,
        Some(c) => !(c.is_alphanumeric() || c == '_' || c == '-'),
    }
}

/// Names of every pattern that matches in one line (each name at most once).
pub fn scan_line(line: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    let mut add = |n: &'static str| {
        if !found.contains(&n) {
            found.push(n);
        }
    };
    let bytes = line.as_bytes();
    for (name, prefix, set, min) in PREFIXES {
        for (i, _) in line.match_indices(prefix) {
            if left_ok(line, i) && run_len(bytes, i + prefix.len(), *set) >= *min {
                add(name);
            }
        }
    }
    let lower = line.to_ascii_lowercase();
    for (i, _) in lower.match_indices("bearer ") {
        if left_ok(line, i) && run_len(bytes, i + 7, bearer_chars) >= 20 {
            add("bearer");
        }
    }
    if let Some(b) = line.find("-----BEGIN ")
        && line[b..].contains("PRIVATE KEY-----")
    {
        add("private-key");
    }
    if password_match(&lower) {
        add("password");
    }
    found
}

fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    i
}

/// At `i`: an optional quote, itself optionally preceded by a backslash. Returns the new index.
fn optional_quote(b: &[u8], i: usize) -> usize {
    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
        return i + 1;
    }
    if i + 1 < b.len() && b[i] == b'\\' && (b[i + 1] == b'"' || b[i + 1] == b'\'') {
        return i + 2;
    }
    i
}

/// At `i`: a required `"`, optionally preceded by a backslash.
fn required_dquote(b: &[u8], i: usize) -> Option<usize> {
    if i < b.len() && b[i] == b'"' {
        return Some(i + 1);
    }
    if i + 1 < b.len() && b[i] == b'\\' && b[i + 1] == b'"' {
        return Some(i + 2);
    }
    None
}

/// A value: 4+ chars that are not whitespace, quote or backslash, not a placeholder or mask.
pub fn is_value(s: &str) -> bool {
    let v: String = s
        .chars()
        .take_while(|c| !(c.is_whitespace() || *c == '"' || *c == '\'' || *c == '\\'))
        .collect();
    if v.chars().count() < 4 {
        return false;
    }
    let first = v.chars().next().unwrap_or(' ');
    if "<{$*%[(".contains(first) {
        return false;
    }
    let low = v.to_lowercase();
    !(low.starts_with("redacted") || low.starts_with("xxxx"))
}

/// The two `password` shapes. `lower` is the line in ASCII lower case. No left boundary.
fn password_match(lower: &str) -> bool {
    let b = lower.as_bytes();
    for (i, _) in lower.match_indices("password") {
        let after = i + "password".len();
        // Shape 1: password = 'value'
        let j = skip_spaces(b, after);
        if j < b.len() && b[j] == b'=' {
            let k = optional_quote(b, skip_spaces(b, j + 1));
            if is_value(&lower[k..]) {
                return true;
            }
        }
        // Shape 2: "password" : "value" (quotes optionally backslash-escaped)
        if i >= 1
            && b[i - 1] == b'"'
            && let Some(j) = required_dquote(b, after)
        {
            let j = skip_spaces(b, j);
            if j < b.len() && b[j] == b':' {
                let k = skip_spaces(b, j + 1);
                if let Some(k) = required_dquote(b, k)
                    && is_value(&lower[k..])
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Rule 4 over every text file under the submission directory.
pub fn rule(files: &[Entry]) -> Outcome {
    let mut hits = Vec::new();
    for (e, text) in text_files(files) {
        for (n, line) in lines(&text) {
            for name in scan_line(line) {
                hits.push(format!("{}:{} {}", e.rel, n, name));
            }
        }
    }
    if hits.is_empty() {
        Outcome::new(
            "secrets",
            Level::Pass,
            "no secret patterns found in text files",
        )
    } else {
        Outcome::new(
            "secrets",
            Level::Fail,
            format!("{} possible secret(s) found (values not shown)", hits.len()),
        )
        .with_extra(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Secret-shaped strings are built from pieces so that this source never holds one whole.
    fn k(a: &str, b: &str) -> String {
        format!("{a}{b}")
    }
    const A20: &str = "A1b2C3d4E5f6G7h8I9j0";

    #[test]
    fn sk_key_and_left_boundary() {
        assert_eq!(scan_line(&k("sk", &format!("-{A20}"))), vec!["sk-key"]);
        assert_eq!(
            scan_line(&k("x sk", &format!("-proj-{A20}"))),
            vec!["sk-key"]
        );
        assert!(scan_line(&k("sk", "-A1b2C3d4E5f6G7h8I9j")).is_empty());
        assert!(scan_line(&k("di", &format!("sk-{A20}"))).is_empty());
        assert!(scan_line(&k("task", "-routing")).is_empty());
        assert!(scan_line(&k("_sk", &format!("-{A20}"))).is_empty());
        assert_eq!(scan_line(&k("(sk", &format!("-{A20}"))), vec!["sk-key"]);
        assert_eq!(
            scan_line(&k("sk-an", &format!("t-{A20}"))),
            vec!["anthropic-key"]
        );
    }

    #[test]
    fn other_prefixes() {
        let gh = k("gh", "p_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8");
        assert_eq!(scan_line(&gh), vec!["github-token"]);
        assert!(scan_line(&gh[..gh.len() - 3]).is_empty());
        assert_eq!(
            scan_line(&k("AK", "IAABCDEFGHIJ012345")),
            vec!["aws-key-id"]
        );
        assert!(scan_line(&k("AK", "IAABCDEFGHIJ01234")).is_empty());
        assert_eq!(scan_line(&k("xo", "xb-123456789-")), vec!["slack-token"]);
        assert_eq!(scan_line(&k("Authorization: BEARER ", A20)), vec!["bearer"]);
        assert!(scan_line(&k("bearer  ", A20)).is_empty());
        assert_eq!(
            scan_line(&k("-----BEGIN RSA ", "PRIVATE KEY-----")),
            vec!["private-key"]
        );
    }

    #[test]
    fn password_shapes() {
        assert!(password_match(&k("passw", "ord=hunter22")));
        assert!(password_match(&k("mypassw", "ord = 'hunter22'")));
        assert!(password_match(&k("passw", "ord=\\\"hunter22\\\"")));
        assert!(password_match(&k("{\"passw", "ord\": \"hunter22\"}")));
        assert!(password_match(&k(
            "{\\\"passw",
            "ord\\\":\\\"hunter22\\\"}"
        )));
        assert!(!password_match(&k("passw", "ord=abc")));
        assert!(!password_match(&k("passw", "ord=<your password>")));
        assert!(!password_match(&k("passw", "ord=\"\"")));
        assert!(!password_match(&k("passw", "ord=REDACTED-1")));
        assert!(!password_match(&k("passw", "ord=xxxxxxxx")));
        assert!(!password_match(&k("passw", "ord=${DB_PASS}")));
        assert!(!password_match("the password is required"));
        assert!(!password_match(&k("\"passw", "ord\": 12345")));
    }
}
