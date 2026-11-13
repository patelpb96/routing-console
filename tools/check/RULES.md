# Submission checker: the contract

Written by the orchestrator for slice S0. The coder builds the checker from this file. The case
writers write test cases from this file and the brief. If this file and the brief disagree,
the brief wins and the disagreement is reported, not coded around.

## Command line

```
cargo run -p check -- <submission-dir> [--answer <file>] [--repo <dir>] [--ledger <file>]
                      [--company-name <text>] [--public] [--max-bytes <n>]
cargo run -p check -- --run-cases <cases-dir> [--rule <rule-id>] [--batch <n>] [--markdown]
cargo run -p check -- --repo-rules <repo-dir>
cargo run -p check -- --redact <dir>
```

Defaults: `--answer SUBMISSION.md`, `--repo .`, `--ledger docs/agent-ledger.md`,
`--max-bytes 100000000`. The company name has no default and is never written in this repository:
it is supplied on the command line or in the `CHECK_COMPANY_NAME` environment variable.

Output: exactly one line per rule, in the order below, shaped `LEVEL rule-id: detail`, where LEVEL
is `PASS`, `FAIL`, `WARN` or `MANUAL`. Extra detail lines are indented by two spaces. A closing
block headed `Not checked:` lists what the tool cannot see (contents of archives, the size of the
zip the upload page makes from a folder, the video itself, whether the repository is public unless
`--public` is given).

Exit code: 1 if any rule is `FAIL`, 2 on a usage error (including a submission directory that does
not exist), otherwise 0. `WARN` and `MANUAL` never change the exit code.

A secret that is found is never printed. Report the file, the line number and the pattern name.

## Redaction (added in slice S5)

```
cargo run -p check -- --redact <dir>
```

The first real run of the checker on copies of the session exports failed rule 4 with 48 hits.
Every one was this file's own pattern table, or the checker's source, quoted inside a transcript.
The rule cannot tell a quoted pattern from a leaked key, and it should not try. So the packed
copies are redacted instead: `--redact` rewrites the text files under `<dir>` so that rules 4 and
9 find nothing and rule 5 still passes. It shares its patterns with the rules, replaces only the
matched part (`[REDACTED]`, `[email]`, `[user]`), writes a file only after the result has been
scanned again and every JSON line still parses, and refuses any folder outside this workspace,
the workspace root itself, or anything under `.git`. The raw exports are kept elsewhere and are
never touched. It does not look for the company's name: rule 8 stays a decision for a person.

Packing order: copy the exports into `submission/`, run `--redact submission/`, then run the
checker on `submission/`.

## Definitions

- "Under the submission directory" means recursively, every regular file.
- A "text file" has no NUL byte in its first 8192 bytes. It is read as UTF-8 with invalid bytes
  replaced.
- Name comparisons for path components and file extensions ignore letter case.

## The 11 rules

### 1. `file-count` (fail)
The brief: "attach up to 10 files ... or use Attach folder to send a whole folder as one zip".
Count the entries directly inside the submission directory. A file counts as one. A folder counts
as one, however many files it holds, because it is uploaded as one zip. More than 10 is `FAIL`;
0 to 10 is `PASS`.

### 2. `file-size` (fail)
The brief: "up to 100 MB each". Every regular file under the submission directory must be at most
`--max-bytes` bytes (default 100,000,000, the stricter reading of 100 MB). Exactly the limit is
`PASS`; one byte more is `FAIL`. The detail names each offending file.

### 3. `build-output` (fail)
The brief: "delete `node_modules` and any build output". `FAIL` if any path component (a folder
name or a file name, compared whole) under the submission directory is exactly `node_modules`,
`target`, `dist` or `build`, or if any file's name ends in `.db`, `.db-wal`, `.db-shm`, `.sqlite`
or `.sqlite3`. A name that merely contains one of the words (`targets.md`, `rebuild.txt`,
`distance.csv`, `build-notes.md`) is `PASS`.

### 4. `secrets` (fail)
The brief: "Remove API keys, tokens and passwords before you upload anything". Scan every text
file under the submission directory. `FAIL` on any of these; otherwise `PASS`.

| Pattern name | Matches |
|---|---|
| `anthropic-key` | `sk-ant-` then 20 or more of `A-Z a-z 0-9 _ -` |
| `sk-key` | `sk-` then 20 or more of `A-Z a-z 0-9`, optionally with `proj-` straight after `sk-` |
| `github-token` | `ghp_`, `gho_`, `ghs_`, `ghu_` or `ghr_` then 36 or more of `A-Z a-z 0-9`; or `github_pat_` then 22 or more of `A-Z a-z 0-9 _` |
| `aws-key-id` | `AKIA` then 16 or more of `A-Z 0-9` |
| `slack-token` | `xoxb-`, `xoxp-`, `xoxa-` or `xoxs-` then 10 or more of `A-Z a-z 0-9 -` |
| `google-key` | `AIza` then 35 or more of `A-Z a-z 0-9 _ -` |
| `private-key` | a line containing `-----BEGIN ` and, later on the same line, `PRIVATE KEY-----` |
| `bearer` | `bearer` in any letter case, one space, then 20 or more of `A-Z a-z 0-9 . _ ~ + / = -` |
| `password` | see below |

Left boundary: for every prefix pattern (all rows except `private-key` and `password`), the
character just before the match must not be a letter, a digit, `_` or `-`. So `task-routing` and
`disk-ABCDEFGHIJKLMNOPQRSTUVWXYZ` do not match `sk-key`.

`password`, in any letter case, matches in two shapes:

1. `password`, optional spaces, `=`, optional spaces, an optional quote (`"` or `'`, itself
   optionally preceded by a backslash), then a value.
2. `"password"` (the quotes optionally backslash-escaped), optional spaces, `:`, optional spaces, a
   `"` (optionally backslash-escaped), then a value.

A value is 4 or more characters that are not whitespace, a quote or a backslash. It does not count
when its first character is one of `< { $ * % [ (`, or when it starts with `redacted` or `xxxx` in
any letter case. So a placeholder, a masked value, an empty value or prose such as "the password
is required" is `PASS`.

### 5. `session-jsonl` (fail)
The brief: "include their native exports". `FAIL` when any of these holds, otherwise `PASS`:

- there is no file ending in `.jsonl` under the submission directory;
- a `.jsonl` file is empty or holds only whitespace;
- a line of a `.jsonl` file that is not blank fails to parse as one complete JSON value (RFC 8259;
  nothing may follow the value on the line except whitespace);
- a file inside a folder named `sessions` does not end in `.jsonl`.

Blank and whitespace-only lines are skipped. Both LF and CRLF line endings are accepted.

### 6. `answer-template` (fail)
The brief's template has five headings. Read the answer file (`--answer`). `FAIL` when any of
these holds, otherwise `PASS`:

- the answer file is missing;
- any of these five is missing as the start of a line (leading spaces allowed, letter case exact):
  `Repository:`, `Walkthrough video:`, `Agent setup:`, `Session map:`, `Intentional omissions:`;
- a placeholder is left: `<`, then a letter, then any characters except `<`, `>` and a line break,
  then `>`, unless the text straight after `<` is `http`;
- the text after `Repository:` or after `Walkthrough video:` on the same line, trimmed, does not
  start with `https://`.

### 7. `session-map` (fail)
The brief asks for a session map with three entries. In the answer file, find the three lines that
contain `Initial planning or decomposition:`, `Most important correction or change of direction:`
and `Final testing and verification:`. Each line must contain at least one file name ending in
`.jsonl` (characters `A-Z a-z 0-9 _ . - / \`) whose base name equals the base name of a `.jsonl`
file under the submission directory. A missing answer file, a missing line, or a line naming no
existing session file is `FAIL`.

### 8. `company-name` (fail if public)
The brief: "Please don't publish a public repository that names [the company] or this exercise".

- No company name supplied, or `--repo` is not a git repository: `MANUAL`.
- Otherwise search, ignoring letter case, for the name as a plain substring in: the contents of
  every tracked text file, every tracked file's path, and every commit message.
- No hit: `PASS`. A hit with `--public`: `FAIL`. A hit without `--public`: `WARN`.

### 9. `personal-info` (warn)
The brief: "You may redact ... personal information, filesystem paths". Scan every text file under
the submission directory for:

- email addresses (`local@domain.tld`), except those whose local part is `noreply` or `no-reply`;
- home-folder paths: `C:\Users\<name>` (any drive letter, with `\`, `\\` or `/` as the
  separator), `/Users/<name>` and `/home/<name>`.

Any hit is `WARN`, with a count per file and up to five distinct examples. No hit is `PASS`.

### 10. `time-budget` (warn)
The brief: "Spend no more than 5 hours". Read the ledger file (`--ledger`). In every markdown
table whose header row's last cell is `Min` (letter case ignored), add up the last cell of each
row where it is a number. Cells such as `n/a` or `not measured` add nothing and are counted as
unmeasured. A sum of 270 or more is `WARN`; below 270 is `PASS`. A missing ledger, or one with no
such table, is `WARN`. The detail gives the sum and the number of unmeasured rows.

### 11. `video-length` (manual)
The brief: "Loom link, 10 minutes max". It cannot be read from disk. Always `MANUAL`, with one
line for the person to tick. It never changes the exit code.

## Case files

One case per file: `tools/check/cases/<rule-id>/b<batch>-<n>.case`, for example
`tools/check/cases/file-count/b1-3.case`. Plain text, one directive per line. A case tests one
rule: only that rule's line is compared with `expect`.

```
rule: <rule-id>
why: <one line saying why this outcome is right, citing the brief or this file>
expect: <pass | fail | warn | manual>
max-bytes: <n>                 optional, sets --max-bytes for this case
company: <text>                optional, sets --company-name for this case
public: yes                    optional, sets --public for this case
dir: <path>                    creates an empty folder
file: <path>                   creates a text file; its content is the lines that follow
| <one content line>           content line: "| " then the text; "|" alone is an empty line
empty: <path>                  creates a zero-byte file
bytes: <path> <n>              creates a file of exactly n zero bytes
commit: <message>              a commit message in the case's repository
untracked: <path>              like file:, but the file is left out of the commit
```

Paths are relative to the case's own temporary root and use `/`. The harness passes
`<root>/submission` as the submission directory (always created), `<root>/SUBMISSION.md` as the
answer file, `<root>/repo` as the repository and `<root>/ledger.md` as the ledger. So a session
export is `file: submission/planning.jsonl` and the answer is `file: SUBMISSION.md`.

Repository: if the case has any `repo/` path or any `commit:` line, the harness runs `git init`
in `<root>/repo`, commits every `file:` under `repo/` with the first `commit:` message (or `init`
if there is none), and makes one empty commit for each further `commit:` line. Without them
`<root>/repo` does not exist.

Join marker: the three characters `{+}` are removed from content lines, `company:` and `commit:`
values before use. **Every string shaped like a secret must be written split by the marker** (for
example a key prefix, then `{+}`, then the rest), so that no case file, and no agent transcript
that quotes one, contains a string the secret scan would flag. Use made-up values only: never a
real key, a real person's email address, or the real company's name.

Content lines end with LF and the file ends with one LF. Lines starting with `#` outside a
content block are comments.

## Clarifications (added after the batch 1 case writers reported ambiguities)

These settle questions the case writers raised. None changes a rule above.

- Rule 1: hidden entries (names starting with a dot) count like any other entry.
- Rule 3: an empty folder with a forbidden name still fails. The database extensions apply to
  files only, not to folder names. The brief says "any build output"; the four names are the ones
  the plan lists, and other build folders (`out`, `bin`, `.next`, `coverage`) are not detected.
  That gap is listed in the README as a known limit of the checker.
- Rule 4: for `sk-key` with `proj-`, the 20 characters are counted after `proj-`. The `password`
  pattern has no left boundary, so `mypassword=hunter{+}22` style text matches.
- Rule 5: "inside a folder named `sessions`" means at any depth: any file with a path component
  equal to `sessions`. A bare scalar such as `42` or `"text"` is a valid JSON value for a line.
- Rule 6, placeholder exception: "is `http`" means the text after `<` starts with `http`, so a
  markdown autolink such as `<https://example.com/x>` is not a placeholder.
- Rule 6, repository line (**a change in the brief's favour, raised by a case writer**): the brief
  says "If your repository is private, attach the project itself instead". So the text after
  `Repository:` passes when it starts with `https://` **or** contains the word `attached` in any
  letter case. The walkthrough line still must start with `https://`.
- Rule 7: base names are compared ignoring letter case.
- Rule 9: the `noreply` / `no-reply` test and the `Users` / `home` path words ignore letter case.
  `<name>` is one or more of `A-Z a-z 0-9 . _ -`. A `/home/<name>` inside a URL is still listed:
  this rule only warns, so it errs towards listing.
- Rule 10: the sum runs across every `Min` table in the file. A number is digits with an optional
  decimal part (`30`, `12.5`); anything else (`1h`, `1,200`) counts as unmeasured. The brief's limit
  is 300 minutes; warning from 270 is the plan's early warning, not the brief's rule.
- Case files: paths may contain spaces. For `bytes:` the byte count is the last space-separated
  token and the path is everything before it. While running cases the harness ignores the
  `CHECK_COMPANY_NAME` environment variable; only a `company:` directive supplies the name.
