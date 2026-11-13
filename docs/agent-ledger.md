# Agent ledger

One row per task card or decision. The session map in the written answer is read off this file.
Times are wall-clock minutes of the author's attention. Where a time was not measured it says so; do
not fill gaps with guesses.

## Standing rules (approved on the plan page, 2026-11-12)

- Fetching: Haiku. Coding, all slices S0 to S4: one Opus 5.5 subagent at medium effort. Planning,
  diff review and gates: the orchestrator session.
- Coders get no web tools. Haiku agents write data files only (research notes, test cases), never
  code.
- A failed gate gets a second pass before any fix. It blocks only if the build breaks, a test or
  invariant fails, one of the four use cases is wrong, or a file rule from the brief fails.
  Everything else is logged below as deferred and work moves on.
- File rules are tested in batches of five cases per rule. Haiku writes cases in parallel, one
  agent per rule. A rule closes on a clean batch. Limit of three batches per rule.
- Front end: plain JavaScript served by the Rust binary. No Node build step.

## Rows

| # | Date | Who | Model, effort | Card or decision | Result | Sent back or changed | Min |
|---|---|---|---|---|---|---|---|
| 1 | 11-12 | Orchestrator | Fable 5.2 | Read the brief, load the two engineering skills, check the toolchain | Rust 1.99.1 with MSVC tools and Node 24 found on Windows | none | not measured |
| 2 | 11-12 | Research desk | Haiku | Fetch the company's public pages and current crate versions | Report returned in about 70 s; summaries only, no verbatim quotes; no tech stack found | none | 1 |
| 3 | 11-12 | Orchestrator | Fable 5.2 | Borrow the shape of an earlier multi-agent project of the author's | Desk roles, fetch-once research, scripts as publish rules. That project's own pipeline not used | none | not measured |
| 4 | 11-12 | Orchestrator | Fable 5.2 | Publish plan v1 with three workflow diagrams | Published. Model ladder drawn as Haiku plus Fable High and Max | see rows 5 to 8 | not measured |
| 5 | 11-12 | Author | n/a | Correction: routine coder is Opus 5.5 Medium, not Fable High | Applied in plan v2 | Orchestrator had replaced the Opus coder from the first message on its own reading of a later note | n/a |
| 6 | 11-12 | Author | n/a | Correction: second pass on every failed gate, no nitpick fixes | Applied in plan v3. Gates split into blocking and deferred | Plan v1 had every gate failure start a fix | n/a |
| 7 | 11-12 | Author | n/a | Decision: slice S2 on Opus 5.5 as well | Applied in plan v4. All coding on one model and effort | Plan had S2 on Fable Max | n/a |
| 8 | 11-12 | Author | n/a | Correction: each file rule tested in batches of five Haiku-written cases, single Opus writer | Applied in plan v5. S0 box 0:20 to 0:35, total 4 h 45 | Plan v1 had only two fixtures | n/a |
| 9 | 11-12 | Author | n/a | Approved all three decisions; asked for the project folder | Folder created with brief, plan, research notes and this ledger. No product code yet | none | n/a |
| 10 | 11-12 | Orchestrator | Fable 5.2 | Build session starts 16:18. Write the checker contract `tools/check/RULES.md` (11 rules, case format, join marker so no case holds an unbroken secret-shaped string) and the product contract `docs/contract.md` | Both written before any code | none | n/a |
| 11 | 11-12 | Case writers, 11 in parallel | Haiku | Batch 1: five cases per file rule as data files, from RULES.md and the brief | 55 cases. Orchestrator read all 55 against the brief before running: none dropped or changed | Writers' reports exposed 10 open points in the contract. Settled in RULES.md "Clarifications". One rule changed in the brief's favour: `Repository:` may say the project is attached (private repo) | n/a |
| 12 | 11-12 | Coder | Opus 5.5, medium | Card S0: workspace, `tools/check` (std only), case harness, `cargo check-all` | Coder reported 55 of 55 and all gates green, but only through its own helper script. Orchestrator's plain-shell run failed: `LNK1104 vcruntime.lib` | Orchestrator one-line fix: static C runtime for the Windows target in `.cargo/config.toml`. Then tests failed: project-built programs are denied writes to the system temp folder on this machine. Second pass: blocks (tests fail). Correction card S0-C1 | n/a |
| 13 | 11-12 | Coder | Opus 5.5, medium | Card S0-C1: scratch folders under `target/`, rule 8 must not mistake the outer repository for a case's | Orchestrator's plain-shell run: `cargo check-all` OK, 29 tests, 55 of 55 cases, all 11 rules closed on batch 1. Harness shown to report `MISS` on two flipped cases | none. Slice S0 closed 16:36, box 0:35 | 18 |
| 14 | 11-12 | Coder (new agent) | Opus 5.5, medium | Card S1 tracer: full schema, pure `domain`, store, engine trait and seeded simulator, HTTP routes with the final snapshot shape, bare page, happy-path test on paused time | Coder reported all checks green but only with three environment variables: the same Build Tools install also lacks `immintrin.h` and `oldnames.lib`. It did not edit `.cargo/config.toml` (off its card) and proposed a fix | Orchestrator applied two of the three proposed settings (SQLite without intrinsics, no `oldnames.lib`) and rejected the third (forcing the temp folder for every build), because it could break a reviewer's Windows build. Diff read against the card: `domain` is pure and table-shaped; no change asked | n/a |
| 15 | 11-12 | Orchestrator | Fable 5.2 | Gate and browser run for S1 | Plain shell: `cargo check-all` OK (29 + 17 + 2 tests). Browser on `cargo run`: create, scenario `fast_match`, submit, offered after about 6 s, accept, assigned at version 4 with four events, no console errors | none. Slice S1 closed 16:55, box 0:45 | 19 |

## Deferred items

- Checker rule 3 knows four build-folder names (`node_modules`, `target`, `dist`, `build`); the brief says "any build output". Other names (`out`, `bin`, `.next`, `coverage`) are not detected. Not blocking: this project produces only `target/`.
- Machine setup, not project code: one Visual Studio Build Tools install lacks `vcruntime.lib`, and project-built programs cannot write to the system temp folder. It also lacks `immintrin.h` and `oldnames.lib`. Worked around in `.cargo/config.toml` and by keeping scratch files under `target/`. On this machine the first build after `cargo clean` must be `cargo check-all` (it points the C compiler's temp folder inside the project); after that a plain `cargo run` works. The author may want to look at the cause.

Edited by Claude Code (Fable 5.2), 2026-11-12
