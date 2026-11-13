//! Parsing of `.case` files (format in RULES.md, "Case files").

use crate::model::{Level, RULE_IDS};

pub const JOIN_MARKER: &str = "{+}";

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Dir(String),
    File {
        path: String,
        lines: Vec<String>,
        untracked: bool,
    },
    Empty(String),
    Bytes(String, u64),
}

impl Item {
    pub fn path(&self) -> &str {
        match self {
            Item::Dir(p) | Item::Empty(p) | Item::Bytes(p, _) => p,
            Item::File { path, .. } => path,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Case {
    pub rule: String,
    pub why: String,
    pub expect: Option<Level>,
    pub max_bytes: Option<u64>,
    pub company: Option<String>,
    pub public: bool,
    pub items: Vec<Item>,
    pub commits: Vec<String>,
    /// Short directive summary for `--markdown`, never holding file contents.
    pub setup: Vec<String>,
}

impl Case {
    /// True when the harness must create a git repository in `<root>/repo`.
    pub fn needs_repo(&self) -> bool {
        !self.commits.is_empty()
            || self
                .items
                .iter()
                .any(|i| i.path() == "repo" || i.path().starts_with("repo/"))
    }
}

fn check_path(p: &str) -> Result<String, String> {
    let p = p.trim();
    if p.is_empty() {
        return Err("empty path".into());
    }
    if p.starts_with('/') || p.contains('\\') || p.contains(':') {
        return Err(format!("path must be relative and use /: {p}"));
    }
    if p.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(format!("bad path component in {p}"));
    }
    Ok(p.to_string())
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Parses one case file's text.
pub fn parse(text: &str) -> Result<Case, String> {
    let mut c = Case::default();
    let mut open_file: Option<usize> = None; // index into items of the file taking content
    // Directive summaries in order; Err(index) stands for a file item, summarised at the end.
    let mut setup: Vec<Result<String, usize>> = Vec::new();
    let mut expect_seen = false;
    for (n, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let at = |m: String| format!("line {}: {m}", n + 1);
        if line == "|" || line.starts_with("| ") {
            let Some(idx) = open_file else {
                return Err(at("content line outside a file: or untracked: block".into()));
            };
            let content = if line == "|" { "" } else { &line[2..] };
            if let Item::File { lines, .. } = &mut c.items[idx] {
                lines.push(content.replace(JOIN_MARKER, ""));
            }
            continue;
        }
        open_file = None;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('|') {
            return Err(at("content line must be \"| text\" or \"|\"".into()));
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(at(format!("not a directive: {line}")));
        };
        let value = value.trim();
        match key.trim() {
            "rule" => {
                if !c.rule.is_empty() {
                    return Err(at("rule: given twice".into()));
                }
                if !RULE_IDS.contains(&value) {
                    return Err(at(format!("unknown rule id {value}")));
                }
                c.rule = value.to_string();
            }
            "why" => c.why = value.to_string(),
            "expect" => {
                if expect_seen {
                    return Err(at("expect: given twice".into()));
                }
                expect_seen = true;
                c.expect = Some(Level::parse(value).ok_or_else(|| {
                    at(format!(
                        "expect must be pass, fail, warn or manual, not {value}"
                    ))
                })?);
            }
            "max-bytes" => {
                let v: u64 = value
                    .parse()
                    .map_err(|_| at(format!("max-bytes is not a number: {value}")))?;
                c.max_bytes = Some(v);
                setup.push(Ok(format!("max-bytes {v}")));
            }
            "company" => {
                c.company = Some(value.replace(JOIN_MARKER, ""));
                setup.push(Ok("company set".into()));
            }
            "public" => {
                if !value.eq_ignore_ascii_case("yes") {
                    return Err(at(format!("public: must be yes, not {value}")));
                }
                c.public = true;
                setup.push(Ok("public yes".into()));
            }
            "dir" => {
                let p = check_path(value).map_err(at)?;
                setup.push(Ok(format!("dir {p}")));
                c.items.push(Item::Dir(p));
            }
            k @ ("file" | "untracked") => {
                let p = check_path(value).map_err(at)?;
                c.items.push(Item::File {
                    path: p,
                    lines: Vec::new(),
                    untracked: k == "untracked",
                });
                open_file = Some(c.items.len() - 1);
                setup.push(Err(c.items.len() - 1));
            }
            "empty" => {
                let p = check_path(value).map_err(at)?;
                setup.push(Ok(format!("empty {p}")));
                c.items.push(Item::Empty(p));
            }
            "bytes" => {
                let Some((p, count)) = value.rsplit_once(' ') else {
                    return Err(at("bytes: needs a path and a count".into()));
                };
                let count: u64 = count
                    .parse()
                    .map_err(|_| at(format!("bytes count is not a number: {count}")))?;
                let p = check_path(p).map_err(at)?;
                setup.push(Ok(format!("bytes {p} {count}")));
                c.items.push(Item::Bytes(p, count));
            }
            "commit" => {
                c.commits.push(value.replace(JOIN_MARKER, ""));
                setup.push(Ok("commit".into()));
            }
            other => return Err(at(format!("unknown directive {other}:"))),
        }
    }
    if c.rule.is_empty() {
        return Err("missing rule:".into());
    }
    if c.expect.is_none() {
        return Err("missing expect:".into());
    }
    if c.why.is_empty() {
        return Err("missing why:".into());
    }
    c.setup = setup
        .into_iter()
        .map(|s| match s {
            Ok(text) => text,
            Err(idx) => match &c.items[idx] {
                Item::File {
                    path,
                    lines,
                    untracked,
                } => {
                    let kind = if *untracked { "untracked" } else { "file" };
                    format!("{kind} {path} ({})", plural(lines.len(), "line"))
                }
                other => other.path().to_string(),
            },
        })
        .collect();
    Ok(c)
}

/// `b<batch>-<n>` from a case file stem.
pub fn batch_and_number(stem: &str) -> Option<(u32, u32)> {
    let (b, n) = stem.strip_prefix('b')?.split_once('-')?;
    Some((b.parse().ok()?, n.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_case() {
        let text = "# comment\nrule: file-size\nwhy: limit\nexpect: FAIL\nmax-bytes: 10\n\
                    file: submission/a b.jsonl\n| {\"k\":\"s{+}k\"}\n|\nbytes: submission/big file.bin 11\n\
                    commit: first\nuntracked: repo/x.txt\n| y\n";
        let c = parse(text).unwrap();
        assert_eq!(c.expect, Some(Level::Fail));
        assert_eq!(
            c.items[0],
            Item::File {
                path: "submission/a b.jsonl".into(),
                lines: vec!["{\"k\":\"sk\"}".into(), "".into()],
                untracked: false
            }
        );
        assert_eq!(
            c.items[1],
            Item::Bytes("submission/big file.bin".into(), 11)
        );
        assert!(c.needs_repo());
        assert_eq!(
            c.setup.join("; "),
            "max-bytes 10; file submission/a b.jsonl (2 lines); bytes submission/big file.bin 11; commit; untracked repo/x.txt (1 line)"
        );
    }

    #[test]
    fn rejects_bad_cases() {
        assert!(parse("rule: file-count\nwhy: x\n").is_err());
        assert!(parse("rule: nope\nwhy: x\nexpect: pass\n").is_err());
        assert!(parse("rule: file-count\nwhy: x\nexpect: pass\n| stray\n").is_err());
        assert!(parse("rule: file-count\nwhy: x\nexpect: pass\nfile: ../x\n").is_err());
        assert!(parse("rule: file-count\nwhy: x\nexpect: maybe\n").is_err());
    }

    #[test]
    fn batch_names() {
        assert_eq!(batch_and_number("b1-3"), Some((1, 3)));
        assert_eq!(batch_and_number("x1-3"), None);
    }
}
