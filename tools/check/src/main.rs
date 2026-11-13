//! Submission checker. The contract is `tools/check/RULES.md`. Standard library only.

mod answer;
mod cases;
mod harness;
mod json;
mod ledger;
mod model;
mod personal;
mod redact;
mod repo;
mod repo_rules;
mod rules_files;
mod secrets;
mod sessions;
mod tempdir;
mod walk;

use model::{Config, DEFAULT_MAX_BYTES, Level, RULE_IDS};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  check <submission-dir> [--answer <file>] [--repo <dir>] [--ledger <file>]
        [--company-name <text>] [--public] [--max-bytes <n>]
  check --run-cases <cases-dir> [--rule <rule-id>] [--batch <n>] [--markdown]
  check --repo-rules <repo-dir>
  check --redact <dir>";

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("check: {msg}\n{USAGE}");
    ExitCode::from(2)
}

/// Pulls the value that follows a flag.
fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--run-cases") => run_cases(args.into_iter().skip(1)),
        Some("--repo-rules") => repo_rules_mode(&args[1..]),
        Some("--redact") => redact_mode(&args[1..]),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => main_mode(args.into_iter()),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(msg) => usage_error(&msg),
    }
}

fn repo_rules_mode(rest: &[String]) -> Result<i32, String> {
    let [dir] = rest else {
        return Err("--repo-rules takes exactly one directory".into());
    };
    let dir = Path::new(dir);
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    Ok(repo_rules::run(dir))
}

fn redact_mode(rest: &[String]) -> Result<i32, String> {
    let [dir] = rest else {
        return Err("--redact takes exactly one directory".into());
    };
    match redact::guard(Path::new(dir), &redact::workspace_root()) {
        Ok(real) => Ok(redact::run(&real, dir)),
        Err(msg) => {
            eprintln!("check: {msg}");
            Ok(2)
        }
    }
}

fn run_cases(mut args: impl Iterator<Item = String>) -> Result<i32, String> {
    let dir = PathBuf::from(value(&mut args, "--run-cases")?);
    let (mut rule, mut batch, mut markdown) = (None, None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rule" => {
                let r = value(&mut args, "--rule")?;
                if !RULE_IDS.contains(&r.as_str()) {
                    return Err(format!("unknown rule id {r}"));
                }
                rule = Some(r);
            }
            "--batch" => {
                let b = value(&mut args, "--batch")?;
                batch = Some(
                    b.parse()
                        .map_err(|_| format!("--batch {b} is not a number"))?,
                );
            }
            "--markdown" => markdown = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let results = harness::run_dir(&dir, rule.as_deref(), batch);
    Ok(harness::print(&results, markdown, &dir))
}

fn main_mode(mut args: impl Iterator<Item = String>) -> Result<i32, String> {
    let mut submission: Option<PathBuf> = None;
    let mut cfg = Config {
        submission: PathBuf::new(),
        answer: PathBuf::from("SUBMISSION.md"),
        repo: PathBuf::from("."),
        ledger: PathBuf::from("docs/agent-ledger.md"),
        company: None,
        public: false,
        max_bytes: DEFAULT_MAX_BYTES,
    };
    let mut company_flag = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--answer" => cfg.answer = value(&mut args, &a)?.into(),
            "--repo" => cfg.repo = value(&mut args, &a)?.into(),
            "--ledger" => cfg.ledger = value(&mut args, &a)?.into(),
            "--company-name" => company_flag = Some(value(&mut args, &a)?),
            "--public" => cfg.public = true,
            "--max-bytes" => {
                let v = value(&mut args, &a)?;
                cfg.max_bytes = v
                    .parse()
                    .map_err(|_| format!("--max-bytes {v} is not a whole number"))?;
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ if submission.is_some() => return Err(format!("unexpected argument {a}")),
            _ => submission = Some(a.into()),
        }
    }
    let Some(submission) = submission else {
        return Err("no submission directory given".into());
    };
    if !submission.is_dir() {
        return Err(format!(
            "submission directory {} does not exist",
            submission.display()
        ));
    }
    cfg.submission = submission;
    cfg.company = company_flag.or_else(|| std::env::var("CHECK_COMPANY_NAME").ok());

    let outcomes = model::run_all(&cfg);
    for o in &outcomes {
        println!("{} {}: {}", o.level.label(), o.rule, o.detail);
        for x in &o.extra {
            println!("  {x}");
        }
    }
    println!("Not checked:");
    println!("  - contents of archives (zip, tar and similar files are not opened)");
    println!("  - the size of the zip the upload page makes from a folder");
    println!("  - the walkthrough video itself");
    if !cfg.public {
        println!("  - whether the repository is public (pass --public to treat it as public)");
    }
    let failed = outcomes.iter().any(|o| o.level == Level::Fail);
    Ok(if failed { 1 } else { 0 })
}
