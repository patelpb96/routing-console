//! The repository gate (`cargo check-all`). Standard library only.
//!
//! 1. `cargo fmt --all`          applied, never blocks
//! 2. `cargo clippy --workspace --all-targets`   warn only
//! 3. `cargo test --workspace`   blocking (also covers the build)
//! 4. `cargo run -q -p check -- --repo-rules .`  warn only

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;

struct Step {
    label: &'static str,
    blocking: bool,
    args: &'static [&'static str],
}

const STEPS: [Step; 4] = [
    Step {
        label: "fmt",
        blocking: false,
        args: &["fmt", "--all"],
    },
    Step {
        label: "clippy",
        blocking: false,
        args: &["clippy", "--workspace", "--all-targets"],
    },
    Step {
        label: "tests",
        blocking: true,
        args: &["test", "--workspace"],
    },
    Step {
        label: "repo-rules",
        blocking: false,
        args: &["run", "-q", "-p", "check", "--", "--repo-rules", "."],
    },
];

/// Exit status (None if the command did not start) and the captured output, streamed live.
fn run(args: &[&str], tmp: &Path) -> (Option<i32>, String) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let child = Command::new(cargo)
        .args(args)
        .env("TMP", tmp)
        .env("TEMP", tmp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (None, format!("could not start cargo: {e}")),
    };
    let err = child
        .stderr
        .take()
        .map(|s| thread::spawn(move || tee(s, false)));
    let mut out = child
        .stdout
        .take()
        .map(|s| tee(s, true))
        .unwrap_or_default();
    if let Some(h) = err {
        out.push_str(&h.join().unwrap_or_default());
    }
    let code = child.wait().ok().and_then(|s| s.code());
    (code, out)
}

/// Copies a stream to our stdout or stderr line by line and returns what it saw.
fn tee(stream: impl Read, to_stdout: bool) -> String {
    let mut seen = String::new();
    for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line).into_owned();
        if to_stdout {
            let _ = writeln!(std::io::stdout(), "{line}");
        } else {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
        seen.push_str(&line);
        seen.push('\n');
    }
    seen
}

fn summarise(step: &Step, code: Option<i32>, out: &str) -> String {
    let ok = code == Some(0);
    match step.label {
        "fmt" if code.is_none() || out.contains("not installed") => {
            "skipped (rustfmt unavailable)".into()
        }
        "fmt" if ok => "applied".into(),
        "clippy" => {
            let n = out
                .lines()
                .filter(|l| l.starts_with("warning:") && !l.contains("generated"))
                .count();
            let errors = out.lines().filter(|l| l.starts_with("error")).count();
            match (ok, n) {
                (true, 0) => "clean".into(),
                (true, n) => format!("{n} warnings"),
                (false, _) => format!("exit {code:?}, {errors} error lines, {n} warnings"),
            }
        }
        "tests" if ok => "passed".into(),
        "tests" => "FAILED".into(),
        "repo-rules" => {
            let w = out.lines().filter(|l| l.starts_with("WARN ")).count();
            match (ok, w) {
                (true, 0) => "all PASS".into(),
                (true, w) => format!("{w} WARN"),
                (false, _) => format!("did not run cleanly (exit {code:?})"),
            }
        }
        _ => format!("exit {code:?}"),
    }
}

/// `<workspace root>/target/tmp`, created if missing.
fn workspace_tmp() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
    let tmp = root.join("target").join("tmp");
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        eprintln!("check-all: cannot create {}: {e}", tmp.display());
    }
    tmp
}

fn main() -> ExitCode {
    // On the development machine, project-built executables (and the linker they start) are
    // denied writes to the system temp folder, so every step gets TMP/TEMP inside target/.
    let tmp = workspace_tmp();
    let mut lines = Vec::new();
    let mut blocked = false;
    for (i, step) in STEPS.iter().enumerate() {
        println!(
            "\n== check-all {}/4: cargo {} ==",
            i + 1,
            step.args.join(" ")
        );
        let (code, out) = run(step.args, &tmp);
        let summary = summarise(step, code, &out);
        if step.blocking && code != Some(0) {
            blocked = true;
        }
        if !step.blocking && code != Some(0) {
            println!(
                "check-all: {} exited {code:?}; warn only, continuing",
                step.label
            );
        }
        let kind = if step.blocking {
            "BLOCKING"
        } else {
            "WARN-ONLY"
        };
        lines.push(format!("{kind} {}: {summary}", step.label));
    }
    println!("\n== check-all summary ==");
    for l in &lines {
        println!("{l}");
    }
    if blocked {
        println!("check-all: FAILED (a blocking step failed)");
        ExitCode::FAILURE
    } else {
        println!("check-all: OK");
        ExitCode::SUCCESS
    }
}
