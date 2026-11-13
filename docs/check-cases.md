# File-rule test cases

Every case written for the 11 file rules in `tools/check/RULES.md`, with what happened when it
ran. Cases are data files under `tools/check/cases/<rule>/`. Haiku agents wrote them, one agent per
rule, in parallel. The orchestrator read every case against the brief before it ran. One
Opus 5.5 coder is the only writer of the checker.

A rule closes on a clean batch of five. A miss means the coder edits the checker and the rule gets
five new cases. Limit of three batches per rule.

## Review before running, batch 1 (55 cases)

No case was dropped or changed: each expected outcome follows from the brief. The writers'
reports raised points the contract had left open. Those were settled in the contract, not in the
cases:

| Rule | Raised by the case writer | What was done |
|---|---|---|
| file-count | The brief offers "attach files" and "Attach folder" as alternatives; the rule counts both together | Kept. Counting every top-level entry is the stricter reading, so it cannot pass something the upload page would refuse. |
| build-output | The brief says "any build output"; the rule knows four folder names | Kept as the plan lists them. Logged as a known limit of the checker. |
| secrets | Is the 20-character minimum counted after `proj-`? Does `password` need a left boundary? | Clarified: counted after `proj-`; no left boundary. |
| session-jsonl | Does "inside a folder named sessions" mean any depth? Is a bare scalar a valid line? | Clarified: any depth; yes. |
| answer-template | The exception for `<http...>` said "is http", which read literally excludes `<https://...>`. The brief lets a private project be attached instead of linked, but the rule demanded `https://` | Clarified to "starts with http". **Rule changed in the brief's favour**: `Repository:` also passes when it says the project is attached. |
| session-map | Are base names compared ignoring letter case? | Clarified: yes. |
| company-name | Are paths with spaces allowed in case files? | Clarified: yes. Also: the harness ignores the environment variable while running cases. |
| personal-info | Letter case of `noreply` and `Users`; what counts as a name | Clarified. |
| time-budget | The brief's limit is 300 minutes; the rule warns from 270 | Kept: 270 is the plan's early warning and only warns. Sum runs across all `Min` tables. |
| video-length | The harness compares the level only, so the detail text is not tested | Accepted. |

## Results

### Batch 1, run 2026-11-12 after correction card S0-C1 (plain shell, `cargo run -q -p check -- --run-cases tools/check/cases --markdown`)

55 of 55 matched. Every rule closed on its first batch. The cases stay as regression tests: `cargo test` runs them all.

Before trusting a clean sheet, the harness was shown to fail: two real cases were copied to a scratch folder with the expected outcome flipped, and both printed `MISS` with exit code 1.

| rule | batch | case | setup | expected | actual | result | action |
|---|---|---|---|---|---|---|---|
| file-count | 1 | b1-1 | file submission/f01.txt (1 line); file submission/f02.txt (1 line); file submission/f03.txt (1 line); file submission/f04.txt (1 line); file submission/f05.txt (1 line); file submission/f06.txt (1 line); file submission/f07.txt (1 line); file submission/f08.txt (1 line); file submission/f09.txt (1 line); file submission/f10.txt (1 line) | pass | pass | MATCH | kept as regression test |
| file-count | 1 | b1-2 | file submission/f01.txt (1 line); file submission/f02.txt (1 line); file submission/f03.txt (1 line); file submission/f04.txt (1 line); file submission/f05.txt (1 line); file submission/f06.txt (1 line); file submission/f07.txt (1 line); file submission/f08.txt (1 line); file submission/f09.txt (1 line); file submission/f10.txt (1 line); file submission/f11.txt (1 line) | fail | fail | MATCH | kept as regression test |
| file-count | 1 | b1-3 | dir submission/bundle; file submission/bundle/f01.txt (1 line); file submission/bundle/f02.txt (1 line); file submission/bundle/f03.txt (1 line); file submission/bundle/f04.txt (1 line); file submission/bundle/f05.txt (1 line); file submission/bundle/f06.txt (1 line); file submission/bundle/f07.txt (1 line); file submission/bundle/f08.txt (1 line); file submission/bundle/f09.txt (1 line); file submission/bundle/f10.txt (1 line); file submission/bundle/f11.txt (1 line); file submission/bundle/f12.txt (1 line) | pass | pass | MATCH | kept as regression test |
| file-count | 1 | b1-4 | file submission/a01.txt (1 line); file submission/a02.txt (1 line); file submission/a03.txt (1 line); file submission/a04.txt (1 line); file submission/a05.txt (1 line); file submission/a06.txt (1 line); file submission/a07.txt (1 line); dir submission/folder-a; file submission/folder-a/x1.txt (1 line); file submission/folder-a/x2.txt (1 line); dir submission/folder-b; file submission/folder-b/x1.txt (1 line); file submission/folder-b/x2.txt (1 line); dir submission/folder-c; file submission/folder-c/x1.txt (1 line); file submission/folder-c/x2.txt (1 line); dir submission/folder-d; file submission/folder-d/x1.txt (1 line); file submission/folder-d/x2.txt (1 line) | fail | fail | MATCH | kept as regression test |
| file-count | 1 | b1-5 |  | pass | pass | MATCH | kept as regression test |
| file-size | 1 | b1-1 | max-bytes 1000; bytes submission/exact-limit.bin 1000 | pass | pass | MATCH | kept as regression test |
| file-size | 1 | b1-2 | max-bytes 1000; bytes submission/one-over.bin 1001 | fail | fail | MATCH | kept as regression test |
| file-size | 1 | b1-3 | max-bytes 1000; dir submission/exports; dir submission/exports/week1; bytes submission/exports/week1/nested.bin 1001 | fail | fail | MATCH | kept as regression test |
| file-size | 1 | b1-4 | max-bytes 1000; bytes submission/part-1.bin 600; bytes submission/part-2.bin 600; bytes submission/part-3.bin 600 | pass | pass | MATCH | kept as regression test |
| file-size | 1 | b1-5 | max-bytes 100; file submission/notes.txt (1 line) | fail | fail | MATCH | kept as regression test |
| build-output | 1 | b1-1 | file submission/apps/web/packages/api/node_modules/left-pad/index.txt (1 line) | fail | fail | MATCH | kept as regression test |
| build-output | 1 | b1-2 | file submission/notes/targets.md (1 line); file submission/notes/rebuild.txt (1 line); file submission/notes/distance.csv (1 line); file submission/notes/node_modules-readme.txt (1 line) | pass | pass | MATCH | kept as regression test |
| build-output | 1 | b1-3 | file submission/src/main.txt (1 line) | pass | pass | MATCH | kept as regression test |
| build-output | 1 | b1-4 | bytes submission/db/state.db-wal 32 | fail | fail | MATCH | kept as regression test |
| build-output | 1 | b1-5 | file submission/Target/app.txt (1 line) | fail | fail | MATCH | kept as regression test |
| secrets | 1 | b1-1 | file submission/notes.txt (1 line) | fail | fail | MATCH | kept as regression test |
| secrets | 1 | b1-2 | file submission/notes.txt (1 line) | pass | pass | MATCH | kept as regression test |
| secrets | 1 | b1-3 | file submission/notes.txt (1 line) | pass | pass | MATCH | kept as regression test |
| secrets | 1 | b1-4 | file submission/settings-sample.txt (1 line) | fail | fail | MATCH | kept as regression test |
| secrets | 1 | b1-5 | file submission/setup-notes.txt (2 lines) | pass | pass | MATCH | kept as regression test |
| session-jsonl | 1 | b1-1 | file submission/session-planning.jsonl (4 lines) | pass | pass | MATCH | kept as regression test |
| session-jsonl | 1 | b1-2 | file submission/session-planning.jsonl (2 lines) | fail | fail | MATCH | kept as regression test |
| session-jsonl | 1 | b1-3 | file submission/session-empty.jsonl (2 lines) | fail | fail | MATCH | kept as regression test |
| session-jsonl | 1 | b1-4 | file submission/notes.txt (1 line); file submission/export.json (1 line) | fail | fail | MATCH | kept as regression test |
| session-jsonl | 1 | b1-5 | dir submission/sessions/archive; file submission/sessions/planning.jsonl (1 line); file submission/sessions/archive/Final-Check.JSONL (1 line) | pass | pass | MATCH | kept as regression test |
| answer-template | 1 | b1-1 | file SUBMISSION.md (13 lines) | pass | pass | MATCH | kept as regression test |
| answer-template | 1 | b1-2 | file SUBMISSION.md (13 lines) | pass | pass | MATCH | kept as regression test |
| answer-template | 1 | b1-3 | file SUBMISSION.md (12 lines) | fail | fail | MATCH | kept as regression test |
| answer-template | 1 | b1-4 | file SUBMISSION.md (13 lines) | fail | fail | MATCH | kept as regression test |
| answer-template | 1 | b1-5 | file SUBMISSION.md (13 lines) | fail | fail | MATCH | kept as regression test |
| session-map | 1 | b1-1 | file submission/planning.jsonl (1 line); file submission/correction.jsonl (1 line); file submission/testing.jsonl (1 line); file SUBMISSION.md (4 lines) | pass | pass | MATCH | kept as regression test |
| session-map | 1 | b1-2 | file submission/planning.jsonl (1 line); file submission/correction.jsonl (1 line); file submission/final-test.jsonl (1 line); file SUBMISSION.md (4 lines) | fail | fail | MATCH | kept as regression test |
| session-map | 1 | b1-3 | file submission/planning.jsonl (1 line); file submission/correction.jsonl (1 line); file submission/testing.jsonl (1 line); file SUBMISSION.md (4 lines) | fail | fail | MATCH | kept as regression test |
| session-map | 1 | b1-4 | file submission/planning.jsonl (1 line); file submission/export/run-3/build.jsonl (1 line); file submission/testing.jsonl (1 line); file SUBMISSION.md (4 lines) | pass | pass | MATCH | kept as regression test |
| session-map | 1 | b1-5 | file submission/planning.jsonl (1 line); file submission/correction.jsonl (1 line); file submission/final.jsonl (1 line); file SUBMISSION.md (4 lines) | fail | fail | MATCH | kept as regression test |
| company-name | 1 | b1-1 | company set; public yes; file repo/README.md (2 lines); commit | fail | fail | MATCH | kept as regression test |
| company-name | 1 | b1-2 | company set; file repo/notes.txt (1 line); commit | warn | warn | MATCH | kept as regression test |
| company-name | 1 | b1-3 | company set; public yes; file repo/app.txt (1 line); untracked repo/scratch-notes.txt (1 line); commit | pass | pass | MATCH | kept as regression test |
| company-name | 1 | b1-4 | public yes; file repo/README.md (2 lines); commit | manual | manual | MATCH | kept as regression test |
| company-name | 1 | b1-5 | company set; public yes; file repo/Quillmere Labs/plan.txt (1 line); commit | fail | fail | MATCH | kept as regression test |
| personal-info | 1 | b1-1 | file submission/contact.md (1 line) | warn | warn | MATCH | kept as regression test |
| personal-info | 1 | b1-2 | file submission/notices.md (2 lines) | pass | pass | MATCH | kept as regression test |
| personal-info | 1 | b1-3 | file submission/session-meta.json (1 line) | warn | warn | MATCH | kept as regression test |
| personal-info | 1 | b1-4 | file submission/run-log.txt (1 line) | warn | warn | MATCH | kept as regression test |
| personal-info | 1 | b1-5 | file submission/prose.md (4 lines) | pass | pass | MATCH | kept as regression test |
| time-budget | 1 | b1-1 | file ledger.md (5 lines) | pass | pass | MATCH | kept as regression test |
| time-budget | 1 | b1-2 | file ledger.md (5 lines) | warn | warn | MATCH | kept as regression test |
| time-budget | 1 | b1-3 | file ledger.md (7 lines) | pass | pass | MATCH | kept as regression test |
| time-budget | 1 | b1-4 |  | warn | warn | MATCH | kept as regression test |
| time-budget | 1 | b1-5 | file ledger.md (5 lines) | warn | warn | MATCH | kept as regression test |
| video-length | 1 | b1-1 |  | manual | manual | MATCH | kept as regression test |
| video-length | 1 | b1-2 | file SUBMISSION.md (2 lines) | manual | manual | MATCH | kept as regression test |
| video-length | 1 | b1-3 | file SUBMISSION.md (2 lines) | manual | manual | MATCH | kept as regression test |
| video-length | 1 | b1-4 | file submission/planning.jsonl (1 line); file submission/testing.jsonl (1 line) | manual | manual | MATCH | kept as regression test |
| video-length | 1 | b1-5 | file submission/node_modules/example-lib/index.js (1 line) | manual | manual | MATCH | kept as regression test |
