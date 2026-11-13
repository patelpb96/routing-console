//! Rule 10: sum the `Min` column of every ledger table (warn only).

use crate::model::{Level, Outcome};
use crate::walk::lines;
use std::fs;
use std::path::Path;

const WARN_AT: f64 = 270.0;

/// Cells of a table line: outer pipes removed, split on pipes not escaped by a backslash.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t
        .strip_suffix('|')
        .filter(|_| !t.ends_with("\\|"))
        .unwrap_or(t);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_backslash = false;
    for c in t.chars() {
        if c == '|' && !prev_backslash {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
        prev_backslash = c == '\\';
    }
    out.push(cur.trim().to_string());
    out
}

fn is_delimiter_row(cs: &[String]) -> bool {
    !cs.is_empty()
        && cs.iter().all(|c| {
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            !c.is_empty() && c.chars().all(|x| x == '-')
        })
}

/// Digits with an optional decimal part (`30`, `12.5`).
pub fn parse_number(s: &str) -> Option<f64> {
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    s.parse().ok()
}

/// (number of `Min` tables, sum, unmeasured rows)
pub fn sum_minutes(text: &str) -> (usize, f64, usize) {
    let all: Vec<&str> = lines(text).map(|(_, l)| l).collect();
    let is_row = |l: &str| l.trim_start().starts_with('|');
    let (mut tables, mut sum, mut unmeasured) = (0, 0.0, 0);
    let mut i = 0;
    while i < all.len() {
        if !(is_row(all[i]) && i + 1 < all.len() && is_row(all[i + 1])) {
            i += 1;
            continue;
        }
        let header = cells(all[i]);
        if !is_delimiter_row(&cells(all[i + 1])) {
            i += 1;
            continue;
        }
        let is_min = header.last().is_some_and(|c| c.eq_ignore_ascii_case("min"));
        let mut j = i + 2;
        while j < all.len() && is_row(all[j]) {
            if is_min {
                match cells(all[j]).last().and_then(|c| parse_number(c)) {
                    Some(n) => sum += n,
                    None => unmeasured += 1,
                }
            }
            j += 1;
        }
        if is_min {
            tables += 1;
        }
        i = j;
    }
    (tables, sum, unmeasured)
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

pub fn rule(ledger: &Path) -> Outcome {
    let Some(text) = fs::read(ledger)
        .ok()
        .filter(|_| ledger.is_file())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
    else {
        return Outcome::new(
            "time-budget",
            Level::Warn,
            format!("ledger {} is missing", ledger.display()),
        );
    };
    let (tables, sum, unmeasured) = sum_minutes(&text);
    if tables == 0 {
        return Outcome::new(
            "time-budget",
            Level::Warn,
            "ledger has no table whose last column is Min",
        );
    }
    let detail = format!(
        "{} minutes logged, {unmeasured} unmeasured row(s) (warning from 270; the brief's limit is 5 hours)",
        fmt_num(sum)
    );
    let level = if sum >= WARN_AT {
        Level::Warn
    } else {
        Level::Pass
    };
    Outcome::new("time-budget", level, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_min_tables_only() {
        let text = "| a | Min |\n|---|---:|\n| x | 100 |\n| y | n/a |\n| z | 12.5 |\n\n\
                    | a | Other |\n|---|---|\n| x | 500 |\n\n\
                    | b | MIN |\n| --- | --- |\n| x | 1h |\n| y | 1,200 |\n| z | 157.5 |\n";
        let (tables, sum, unmeasured) = sum_minutes(text);
        assert_eq!(tables, 2);
        assert_eq!(sum, 270.0);
        assert_eq!(unmeasured, 3);
    }

    #[test]
    fn number_shapes() {
        assert_eq!(parse_number("30"), Some(30.0));
        assert_eq!(parse_number("12.5"), Some(12.5));
        for bad in ["", "1.", ".5", "-3", "1h", "1,200", "n/a"] {
            assert_eq!(parse_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn boundary_is_270() {
        let t = |n: &str| format!("| a | Min |\n|---|---|\n| x | {n} |\n");
        assert!(sum_minutes(&t("269")).1 < WARN_AT);
        assert!(sum_minutes(&t("270")).1 >= WARN_AT);
    }
}
