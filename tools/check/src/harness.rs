//! `--run-cases`: build each case in a fresh temporary root, run the rules, compare.

use crate::cases::{Case, Item, batch_and_number, parse};
use crate::model::{Config, DEFAULT_MAX_BYTES, Level, Outcome, RULE_IDS, run_all};
use crate::tempdir::{fresh_root, remove_tree};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The result of one case file.
pub struct CaseResult {
    pub rule: String,
    pub batch: Option<u32>,
    pub number: u32,
    pub name: String,
    pub setup: String,
    pub expected: Option<Level>,
    /// `Ok(outcome)` when the case ran, `Err(reason)` for a BAD-CASE.
    pub actual: Result<Outcome, String>,
}

impl CaseResult {
    pub fn matched(&self) -> bool {
        matches!((&self.actual, self.expected), (Ok(o), Some(e)) if o.level == e)
    }
}

/// Every `*.case` file under `dir`, recursively. A missing directory gives none.
fn case_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            case_files(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("case"))
        {
            out.push(p);
        }
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=case",
            "-c",
            "user.email=case@invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git did not start: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Creates the case's files (and repository) under `root`.
fn build(case: &Case, root: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| format!("setup failed: {e}");
    fs::create_dir_all(root.join("submission")).map_err(io)?;
    for item in &case.items {
        let path = root.join(item.path());
        if let Item::Dir(_) = item {
            fs::create_dir_all(&path).map_err(io)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        match item {
            Item::File { lines, .. } => {
                let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
                fs::write(&path, text).map_err(io)?;
            }
            Item::Empty(_) => fs::write(&path, b"").map_err(io)?,
            Item::Bytes(_, n) => fs::File::create(&path)
                .and_then(|f| f.set_len(*n))
                .map_err(io)?,
            Item::Dir(_) => {}
        }
    }
    if !case.needs_repo() {
        return Ok(());
    }
    let repo = root.join("repo");
    fs::create_dir_all(&repo).map_err(io)?;
    git(&repo, &["init", "-q"])?;
    let tracked: Vec<&str> = case
        .items
        .iter()
        .filter_map(|i| match i {
            Item::File {
                path,
                untracked: false,
                ..
            } => path.strip_prefix("repo/"),
            _ => None,
        })
        .collect();
    if !tracked.is_empty() {
        let mut args = vec!["add", "--"];
        args.extend(tracked);
        git(&repo, &args)?;
    }
    let mut messages = case.commits.iter().map(String::as_str);
    let first = messages.next().unwrap_or("init");
    let commit = |m: &str| {
        git(
            &repo,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "--allow-empty-message",
                "-m",
                m,
            ],
        )
    };
    commit(first)?;
    for m in messages {
        commit(m)?;
    }
    Ok(())
}

/// Runs one parsed case in its own temporary root and returns its rule's outcome.
pub fn run_case(case: &Case) -> Result<Outcome, String> {
    let root = fresh_root().map_err(|e| format!("cannot create temp root: {e}"))?;
    let result = build(case, &root).map(|()| {
        // Only the case's directives configure the run: CHECK_COMPANY_NAME is not read here.
        let cfg = Config {
            submission: root.join("submission"),
            answer: root.join("SUBMISSION.md"),
            repo: root.join("repo"),
            ledger: root.join("ledger.md"),
            company: case.company.clone(),
            public: case.public,
            max_bytes: case.max_bytes.unwrap_or(DEFAULT_MAX_BYTES),
        };
        run_all(&cfg).into_iter().find(|o| o.rule == case.rule)
    });
    let cleanup = remove_tree(&root);
    let outcome = result?.ok_or_else(|| format!("rule {} produced no outcome", case.rule))?;
    cleanup.map_err(|e| format!("temp root {} not removed: {e}", root.display()))?;
    Ok(outcome)
}

/// Loads, filters and runs every case under `dir`, sorted by rule order, batch, number.
pub fn run_dir(dir: &Path, rule: Option<&str>, batch: Option<u32>) -> Vec<CaseResult> {
    let mut files = Vec::new();
    case_files(dir, &mut files);
    let mut results = Vec::new();
    for path in files {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel_parent = path.parent().and_then(|p| p.strip_prefix(dir).ok());
        let folder = rel_parent
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let bn = batch_and_number(&stem);
        if rule.is_some_and(|r| r != folder) || batch.is_some_and(|b| bn.map(|x| x.0) != Some(b)) {
            continue;
        }
        let mut r = CaseResult {
            rule: folder.clone(),
            batch: bn.map(|x| x.0),
            number: bn.map(|x| x.1).unwrap_or(0),
            name: stem,
            setup: String::new(),
            expected: None,
            actual: Err(String::new()),
        };
        r.actual = (|| {
            if !RULE_IDS.contains(&folder.as_str()) {
                return Err(format!("folder {folder} is not a rule id"));
            }
            if bn.is_none() {
                return Err("file name is not b<batch>-<n>.case".into());
            }
            let text = fs::read(&path).map_err(|e| format!("unreadable: {e}"))?;
            let case = parse(&String::from_utf8_lossy(&text))?;
            if case.rule != folder {
                return Err(format!(
                    "rule: {} does not match folder {folder}",
                    case.rule
                ));
            }
            r.setup = case.setup.join("; ");
            r.expected = case.expect;
            run_case(&case)
        })();
        results.push(r);
    }
    let order = |r: &str| RULE_IDS.iter().position(|x| *x == r).unwrap_or(usize::MAX);
    results.sort_by(|a, b| {
        (order(&a.rule), &a.rule, a.batch, a.number, &a.name).cmp(&(
            order(&b.rule),
            &b.rule,
            b.batch,
            b.number,
            &b.name,
        ))
    });
    results
}

fn exp(r: &CaseResult) -> &'static str {
    r.expected.map(Level::lower).unwrap_or("?")
}

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Prints the results; returns the exit code (1 on any miss or bad case).
pub fn print(results: &[CaseResult], markdown: bool, dir: &Path) -> i32 {
    if results.is_empty() {
        println!("no cases found under {}", dir.display());
        return 0;
    }
    if markdown {
        println!("| rule | batch | case | setup | expected | actual | result |");
        println!("|---|---|---|---|---|---|---|");
    }
    for r in results {
        let batch = r.batch.map(|b| b.to_string()).unwrap_or("?".into());
        let (actual, result) = match &r.actual {
            Ok(o) => (o.level.lower(), if r.matched() { "MATCH" } else { "MISS" }),
            Err(_) => ("-", "BAD-CASE"),
        };
        if markdown {
            println!(
                "| {} | {batch} | {} | {} | {} | {actual} | {result} |",
                esc(&r.rule),
                esc(&r.name),
                esc(&r.setup),
                exp(r)
            );
            continue;
        }
        match &r.actual {
            Ok(o) => {
                println!(
                    "{result} {} {} expected={} actual={actual} -- {}",
                    r.rule,
                    r.name,
                    exp(r),
                    o.detail
                );
                for x in &o.extra {
                    println!("    {x}");
                }
            }
            Err(reason) => println!("BAD-CASE {} {}: {reason}", r.rule, r.name),
        }
    }
    if !markdown {
        let mut groups: Vec<(&str, Option<u32>, usize, usize)> = Vec::new();
        for r in results {
            match groups.last_mut() {
                Some(g) if g.0 == r.rule && g.1 == r.batch => {
                    g.2 += r.matched() as usize;
                    g.3 += 1;
                }
                _ => groups.push((&r.rule, r.batch, r.matched() as usize, 1)),
            }
        }
        for (rule, batch, ok, n) in groups {
            let b = batch.map(|b| b.to_string()).unwrap_or("?".into());
            println!("{rule} batch {b}: {ok}/{n}");
        }
    }
    if results.iter().all(CaseResult::matched) {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case under tools/check/cases must match. Passes when the folder is missing or empty.
    #[test]
    fn all_cases_match() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases");
        let misses: Vec<String> = run_dir(&dir, None, None)
            .iter()
            .filter(|r| !r.matched())
            .map(|r| match &r.actual {
                Ok(o) => format!(
                    "MISS {} {}: expected={} actual={}",
                    r.rule,
                    r.name,
                    exp(r),
                    o.level.lower()
                ),
                Err(e) => format!("BAD-CASE {} {}: {e}", r.rule, r.name),
            })
            .collect();
        assert!(misses.is_empty(), "{}", misses.join("\n"));
    }

    #[test]
    fn temp_root_with_git_repo_is_removed() {
        let case = parse(
            "rule: company-name\nwhy: t\nexpect: warn\ncompany: Acme{+}Corp\n\
             file: repo/a.txt\n| acmecorp\ncommit: one\ncommit: two\n",
        )
        .unwrap();
        let out = run_case(&case).unwrap();
        assert_eq!(out.level, Level::Warn);
        // run_case returns Err when removal fails, so reaching here means the tree is gone.
    }

    /// The case root sits inside this repository's target/ folder; a case without repo/
    /// must still see "not a git repository" rather than the outer repository.
    #[test]
    fn case_without_repo_is_manual_inside_outer_repo() {
        let case = parse(
            "rule: company-name
why: t
expect: manual
company: Acme
",
        )
        .unwrap();
        assert!(!case.needs_repo());
        assert_eq!(run_case(&case).unwrap().level, Level::Manual);
    }
}
