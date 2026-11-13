//! Rules 6 and 7: the written answer's template and its session map.

use crate::model::{Level, Outcome};
use crate::walk::{Entry, ends_with_ci, lines};
use std::fs;
use std::path::Path;

const HEADINGS: [&str; 5] = [
    "Repository:",
    "Walkthrough video:",
    "Agent setup:",
    "Session map:",
    "Intentional omissions:",
];

const MAP_LINES: [&str; 3] = [
    "Initial planning or decomposition:",
    "Most important correction or change of direction:",
    "Final testing and verification:",
];

fn read_answer(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// The rest of the first line that starts (after leading spaces) with `heading`.
fn heading_value<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    lines(text).find_map(|(_, l)| l.trim_start_matches(' ').strip_prefix(heading))
}

/// Placeholders: `<`, a letter, then anything but `<`, `>` or a line break, then `>`;
/// not when the text after `<` starts with `http`.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, line) in lines(text) {
        for (i, _) in line.match_indices('<') {
            let rest = &line[i + 1..];
            if !rest.chars().next().is_some_and(char::is_alphabetic) || rest.starts_with("http") {
                continue;
            }
            if let Some(end) = rest.find(['<', '>'])
                && rest.as_bytes()[end] == b'>'
            {
                out.push(format!("<{}>", &rest[..end]));
            }
        }
    }
    out
}

fn has_word(text: &str, word: &str) -> bool {
    let low = text.to_lowercase();
    low.match_indices(word).any(|(i, _)| {
        let before = low[..i].chars().next_back();
        let after = low[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

pub fn template_rule(answer: &Path) -> Outcome {
    let Some(text) = read_answer(answer) else {
        return Outcome::new(
            "answer-template",
            Level::Fail,
            format!("answer file {} is missing", answer.display()),
        );
    };
    let mut problems = Vec::new();
    for h in HEADINGS {
        if heading_value(&text, h).is_none() {
            problems.push(format!("missing line starting with \"{h}\""));
        }
    }
    for p in placeholders(&text) {
        problems.push(format!("placeholder left: {p}"));
    }
    if let Some(v) = heading_value(&text, "Repository:") {
        let v = v.trim();
        if !(v.starts_with("https://") || has_word(v, "attached")) {
            problems.push("Repository: is neither an https:// link nor says attached".into());
        }
    }
    if let Some(v) = heading_value(&text, "Walkthrough video:")
        && !v.trim().starts_with("https://")
    {
        problems.push("Walkthrough video: does not start with https://".into());
    }
    if problems.is_empty() {
        Outcome::new(
            "answer-template",
            Level::Pass,
            "five headings present, no placeholders, links filled in",
        )
    } else {
        Outcome::new(
            "answer-template",
            Level::Fail,
            format!("{} problem(s) in the answer file", problems.len()),
        )
        .with_extra(problems)
    }
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_.-/\\".contains(c)
}

fn base_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// Base names of the `.jsonl` file names mentioned in a line. Every prefix of a run of name
/// characters that ends in `.jsonl` counts as a mention.
pub fn mentioned_jsonl(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in line.split(|c: char| !name_char(c)) {
        for (i, _) in run.char_indices() {
            let cand = &run[..i];
            if ends_with_ci(cand, ".jsonl") {
                out.push(base_name(cand).to_lowercase());
            }
        }
        if ends_with_ci(run, ".jsonl") {
            out.push(base_name(run).to_lowercase());
        }
    }
    out
}

pub fn session_map_rule(answer: &Path, files: &[Entry]) -> Outcome {
    let Some(text) = read_answer(answer) else {
        return Outcome::new(
            "session-map",
            Level::Fail,
            format!("answer file {} is missing", answer.display()),
        );
    };
    let exports: Vec<String> = files
        .iter()
        .filter(|e| e.is_file && ends_with_ci(e.name(), ".jsonl"))
        .map(|e| e.name().to_lowercase())
        .collect();
    let mut problems = Vec::new();
    for label in MAP_LINES {
        match lines(&text).find(|(_, l)| l.contains(label)) {
            None => problems.push(format!("no line containing \"{label}\"")),
            Some((n, l)) => {
                if !mentioned_jsonl(l).iter().any(|m| exports.contains(m)) {
                    problems.push(format!(
                        "line {n} (\"{label}\") names no .jsonl file that is in the submission"
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Outcome::new(
            "session-map",
            Level::Pass,
            "all three session-map lines name a submitted .jsonl file",
        )
    } else {
        Outcome::new(
            "session-map",
            Level::Fail,
            format!("{} session-map line(s) not usable", problems.len()),
        )
        .with_extra(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_detection() {
        assert_eq!(placeholders("Repository: <link>"), vec!["<link>"]);
        assert!(placeholders("see <https://example.com/x>").is_empty());
        assert!(placeholders("a <3 b > c").is_empty());
        assert!(placeholders("a < b and c > d").is_empty());
        assert_eq!(placeholders("x <a <b> y").len(), 1);
    }

    #[test]
    fn jsonl_mentions() {
        let m = mentioned_jsonl("- Initial planning: sessions\\Plan.JSONL, line 40.");
        assert!(m.contains(&"plan.jsonl".to_string()));
        assert!(mentioned_jsonl("see plan.json").is_empty());
        assert!(mentioned_jsonl("a/b.jsonl.").contains(&"b.jsonl".to_string()));
    }

    #[test]
    fn attached_is_a_word() {
        assert!(has_word("Project ATTACHED as a zip", "attached"));
        assert!(!has_word("unattached", "attached"));
    }
}
