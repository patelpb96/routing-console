use super::*;
use crate::model::Level;
use crate::tempdir::{fresh_root, remove_tree};

// Secret-shaped strings are built from pieces so that this source never holds one whole.
fn k(a: &str, b: &str) -> String {
    format!("{a}{b}")
}
const A20: &str = "A1b2C3d4E5f6G7h8I9j0";

fn red(line: &str) -> (String, Counts) {
    let mut c = Counts::new();
    let out = redact_line(line, &mut c).expect("comes clean");
    (out, c)
}

#[test]
fn prefix_keys_keep_their_prefix() {
    let cases = [
        (
            k("x s", &format!("k-{A20} y")),
            "x sk-[REDACTED] y",
            "sk-key",
        ),
        (
            k("sk-an", &format!("t-{A20}")),
            "sk-ant-[REDACTED]",
            "anthropic-key",
        ),
        (
            k("gh", "p_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8"),
            "ghp_[REDACTED]",
            "github-token",
        ),
        (
            k("AK", "IAABCDEFGHIJ012345"),
            "AKIA[REDACTED]",
            "aws-key-id",
        ),
        (k("xo", "xb-123456789-"), "xoxb-[REDACTED]", "slack-token"),
        (
            k("AI", "zaA1b2C3d4E5f6G7h8I9j0A1b2C3d4E5f6G7h"),
            "AIza[REDACTED]",
            "google-key",
        ),
    ];
    for (line, want, kind) in cases {
        let (got, c) = red(&line);
        assert_eq!(got, want);
        assert_eq!(c.get(kind), Some(&1), "{kind}");
    }
}

#[test]
fn bearer_password_and_private_key() {
    assert_eq!(
        red(&k("Authorization: Bearer ", A20)).0,
        "Authorization: Bearer [REDACTED]"
    );
    assert_eq!(red(&k("passw", "ord=hunter22")).0, "password=[REDACTED]");
    // Shape 2 inside a .jsonl line, where the quotes are backslash-escaped.
    let (got, c) = red(&k("{\"t\":\"{\\\"passw", "ord\\\": \\\"hunter22\\\"}\"}"));
    assert_eq!(got, "{\"t\":\"{\\\"password\\\": \\\"[REDACTED]\\\"}\"}");
    assert_eq!(c.get("password"), Some(&1));
    assert_eq!(
        red(&k("x -----BEGIN RSA ", "PRIVATE KEY----- y")).0,
        "x -----[REDACTED] RSA PRIVATE KEY----- y"
    );
}

#[test]
fn emails_and_home_paths() {
    assert_eq!(red("mail jo.doe@example.org now").0, "mail [email] now");
    let keep = "NoReply@example.com and no-reply@example.com";
    assert_eq!(red(keep), (keep.to_string(), Counts::new()));
    // A Windows path inside JSON: doubled backslashes stay exactly as they were.
    assert_eq!(
        red(r#"{"cwd":"C:\\Users\\sam.k\\proj"}"#).0,
        r#"{"cwd":"C:\\Users\\[user]\\proj"}"#
    );
    assert_eq!(
        red("e:/USERS/sam and /home/kim/x").0,
        "e:/USERS/[user] and /home/[user]/x"
    );
    // An address right after a JSON escape keeps the escape whole.
    assert_eq!(red(r#""a\nalice@example.org""#).0, r#""a\n[email]""#);
    assert_eq!(red(r#""\u003calice@example.org""#).0, r#""\u003c[email]""#);
}

#[test]
fn several_hits_on_one_line_then_idempotent() {
    let line = format!(
        "{} jo@example.org C:\\Users\\sam {} {}",
        k("passw", "ord=hunter22"),
        k("s", &format!("k-{A20}")),
        k("-----BEGIN ", "PRIVATE KEY-----")
    );
    let (once, c) = red(&line);
    assert_eq!(
        once,
        "password=[REDACTED] [email] C:\\Users\\[user] sk-[REDACTED] -----[REDACTED] PRIVATE KEY-----"
    );
    assert_eq!(c.values().sum::<usize>(), 5);
    assert_eq!(red(&once), (once.clone(), Counts::new()));
    let mut c = Counts::new();
    let text = format!("{line}\r\nplain\n");
    let out = redact_text(&text, &mut c).unwrap();
    assert_eq!(out, format!("{once}\r\nplain\n"));
}

fn jsonl_with_every_kind() -> String {
    [
        format!("{{\"t\":\"key {}\"}}", k("s", &format!("k-{A20}"))),
        format!("{{\"t\":\"{}\"}}", k("sk-an", &format!("t-{A20}"))),
        format!(
            "{{\"t\":\"{}\"}}",
            k("gh", "o_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8")
        ),
        format!("{{\"t\":\"{}\"}}", k("AK", "IAABCDEFGHIJ012345")),
        format!("{{\"t\":\"{}\"}}", k("xo", "xp-123456789-")),
        format!(
            "{{\"t\":\"{}\"}}",
            k("AI", "zaA1b2C3d4E5f6G7h8I9j0A1b2C3d4E5f6G7h")
        ),
        format!("{{\"t\":\"Bearer {A20}\"}}"),
        format!(
            "{{\"t\":\"{}\"}}",
            k("{\\\"passw", "ord\\\": \\\"hunter22\\\"}")
        ),
        format!("{{\"t\":\"{}\"}}", k("passw", "ord = 'hunter22'")),
        format!("{{\"t\":\"{}\"}}", k("-----BEGIN EC ", "PRIVATE KEY-----")),
        r#"{"t":"write to jo.doe@example.org\nalice@example.org, not noreply@example.com"}"#.into(),
        r#"{"cwd":"C:\\Users\\sam\\proj","h":"/home/kim/x","m":"/Users/lee/y"}"#.into(),
        String::new(),
    ]
    .join("\n")
}

#[test]
fn end_to_end_rules_pass_after_redaction() {
    let root = fresh_root().unwrap();
    let file = root.join("session.jsonl");
    fs::write(&file, jsonl_with_every_kind()).unwrap();
    fs::write(root.join("blob.bin"), b"x\0y").unwrap();
    let files = walk(&root);
    assert_eq!(crate::secrets::rule(&files).level, Level::Fail);
    assert_eq!(crate::personal::rule(&files).level, Level::Warn);

    assert_eq!(run(&root, "test"), 0);
    let files = walk(&root);
    assert_eq!(crate::secrets::rule(&files).level, Level::Pass);
    assert_eq!(crate::sessions::rule(&files).level, Level::Pass);
    assert_eq!(crate::personal::rule(&files).level, Level::Pass);
    let after = fs::read(&file).unwrap();
    assert!(check_jsonl(&after).is_empty());
    assert!(String::from_utf8_lossy(&after).contains("noreply@example.com"));
    assert!(!root.join("session.jsonl.redact-tmp").exists());

    // A second run changes nothing.
    assert_eq!(run(&root, "test"), 0);
    assert_eq!(fs::read(&file).unwrap(), after);
    remove_tree(&root).unwrap();
}

#[test]
fn a_file_that_would_break_is_left_untouched() {
    let root = fresh_root().unwrap();
    let file = root.join("broken.jsonl");
    let text = "{\"t\": \"jo@example.org\"\n";
    fs::write(&file, text).unwrap();
    assert_eq!(run(&root, "test"), 1);
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
    remove_tree(&root).unwrap();
}

#[test]
fn the_guard_refuses_dangerous_targets() {
    let ws = workspace_root();
    let outside = ws.parent().unwrap();
    assert!(
        guard(outside, &ws)
            .unwrap_err()
            .contains("outside the workspace")
    );
    assert!(
        guard(&ws, &ws)
            .unwrap_err()
            .contains("workspace root itself")
    );
    let root = fresh_root().unwrap();
    let git = root.join(".git");
    fs::create_dir_all(&git).unwrap();
    let e = guard(&git, &ws).unwrap_err();
    assert!(
        e.contains(".git component") && e.contains("this script's guard"),
        "{e}"
    );
    assert!(
        guard(&root.join("missing"), &ws)
            .unwrap_err()
            .contains("does not exist")
    );
    assert!(guard(&root, &ws).is_ok());
    remove_tree(&root).unwrap();
}
