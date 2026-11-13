# Routing console

A small operator console for assigning expert tasks to contributors. A routing engine proposes
assignments in the background; one request in five never returns, and nothing tells the operator
which ones. The console is built around that fact: it never claims to know whether a pending
request is slow or dead. It shows how long the request has been outstanding against the engine's
normal window and leaves the decision to the operator.

This is a take-home exercise. The brief itself is not included in this repository.

## Run it

```
cargo run
```

Then open http://localhost:3000. That one command builds the server, creates
`routing-console.db` in the current folder, seeds it, and serves the page. There is no Node step.
Needs a recent stable Rust (built with 1.99) and a C compiler for the bundled SQLite.

- `PORT=3111 cargo run` changes the port. `ROUTING_DB=some/path.db` changes the database file.
- Delete `routing-console.db*` or press **Reset** in the simulation panel to start again.
- `cargo test` runs the tests. `cargo check-all` runs the whole gate (see "How it is checked").

## What to look at first

The page opens on seeded data that already shows the awkward cases:

| Seed task | State | Shows |
|---|---|---|
| Grade 40 model answers on hospital discharge planning (high) | Routing, far past the normal window | Unresolved routing: no verdict, only elapsed time |
| Red-team a tax-advice assistant (high) | Needs attention: declined | Recovery choices |
| Annotate dairy-herd sensor clips (high) | Needs attention: no match | Nobody on the panel has the expertise |
| Label 300 support tickets | Offered | Waiting on the contributor |
| 30-minute interview on month-end close | Assigned | Routing complete |
| Survey on warehouse shift scheduling tools | Unassigned | Ready to submit |

The four use cases from the brief, and how to see each:

1. **Routine assignment.** Create a task, leave the engine scenario on `fast match`, submit. After
   about six seconds the engine offers it to a contributor. In the simulation panel, have the
   contributor accept.
2. **Unresolved routing.** The seeded high-priority healthcare task has been routing for longer
   than the normal 5 to 20 second window. It sits at the top of the attention queue. The app does
   not call it failed; it shows the elapsed time on a scale and offers the operator's options.
3. **Human intervention.** Create a task with scenario `slow match` (returns at 26 seconds) and
   submit it. After 20 seconds it is flagged as past the normal window. Assign someone by hand
   with a reason code. At 26 seconds the engine's answer arrives: the history shows it was
   recorded and not applied.
4. **Recovery.** Have a contributor decline an offer (simulation panel). The task moves to needs
   attention with the reason kept, and the operator chooses: route again without that person,
   assign by hand, or close.

Restart: stop the server while a request is pending and start it again. The task is still
routing, its history gains a "server restarted" line, and the simulated engine delivers what came
due while the app was down.

## Important decisions

**Unknown stays unknown.** A pending request has no timeout and never becomes "failed" on its
own. "Past the normal window" is worked out from the clock each time the page is drawn and is
never stored, so it needs no timer and is still right after a restart. A bounded, opt-in retry
policy is what I would add next (see "What would come next"); it is left out so the operator's
judgment stays visible.

**A late answer is recorded, not applied.** When the operator steps in, the outstanding request is
marked superseded and stays on record. If it returns later, the result is written to that request
and to the task's history, and nothing else changes. The check ("is this request still the
pending one?") and the write are one SQLite transaction.

**Six stored states; requests and assignments are their own records.** `unassigned`, `routing`,
`offered`, `assigned`, `needs_attention`, `closed`. A task can have several routing attempts and
several offers, and its whole story stays readable. "At most one active assignee" is a partial
unique index in SQLite, so a code bug cannot break it.

**One writer.** One SQLite connection behind one lock; one transaction per command; every
transition writes its history event in the same transaction. There is no concurrent mutation to
reason about. Several tasks routing at once are just several timers delivering commands to the
same queue.

**A restart cannot lose a request.** A new request is committed first and handed to the engine
after. If the process dies in between, the next start hands every still-pending request to the
engine again; the engine ignores a request id it already holds, so nothing is scheduled twice.
The resent request names the contributors eligible at the restart, not at the original submit;
the app checks the engine's pick again when the answer arrives, so that is safe. With a real
engine the request id would be sent as an idempotency key, so a resend could not be processed
twice.

**Each operator action carries the task version it saw.** A stale one is refused with the current
state (two tabs, or an engine result landing first). Notes that only add information, such as a
late result, do not change the version, so they cannot make the operator's next action stale.

**The state machine is a pure function.** `src/domain` takes what is stored about one task and a
command and returns events and record changes, or a refusal. No IO, no clock. The page asks the
server which commands are allowed, so the transition table exists once.

**Overrides carry a reason code.** Engine too slow, better fit known, contributor unavailable,
priority, other. Each override is a labelled disagreement with the engine, which is the data you
would want when grading a real matcher later.

**The simulator is deterministic.** Each request's outcome comes from a hash of the seed and the
request id, not from the order things happen in. Per-task scenarios (auto, fast match, slow match,
no match, never returns) make every path repeatable. The simulator keeps its own schedule table
and reloads it on start, the way an outside service would have kept running.

**Overrides can be exported with the engine's eventual answer.** "Download overrides (JSON
lines)" gives one line per operator command that carried a reason code, joined to what the engine
later said for the request it set aside: `match`, `no_match` or `none_yet`, and for hand
assignments whether the two agreed. Stepping in on a silent request is not counted as a
disagreement; it is recorded as no answer yet. This was the plan's stretch item.

**The browser holds no state.** One render function draws everything from one server snapshot;
server-sent events tell the page to fetch it again. A refresh changes nothing.

The rules above are written as a testable contract in [docs/contract.md](docs/contract.md).

## Assumptions

- One operator, no sign-in. Contributors do not have a screen; their replies come from a
  simulation panel that is kept visually apart from operator actions.
- Expertise and capacity are hard limits. The operator can override the engine's choice among
  eligible people, but cannot assign someone without the area or with no room.
- Routing ends when a contributor accepts, as the brief says. An accepted task can still be
  reassigned by the operator.
- "Fails before work begins" is modelled as an offer that could not be delivered.
- Time is the server's clock. The page shows elapsed time from the server's timestamps.

## Layout

```
src/domain/     pure state machine: (stored facts, command) to events or a refusal
src/store/      SQLite: schema, reads, writes; one transaction per command
src/engine.rs   RoutingEngine trait and the seeded simulator with its own schedule table
src/app.rs      load, decide, persist, notify
src/snapshot.rs the one JSON document the page draws from
src/overrides.rs the override export, one JSON object per line
src/http.rs     JSON routes, server-sent events, embedded static files
web/            index.html, app.js, style.css (plain JavaScript, no build step)
tests/          use cases on tokio's paused clock, restart, random command runs
tools/check/    submission checker for the brief's file rules (standard library only)
tools/check-all the one gate command
docs/           brief, plan, contract, agent ledger, test-case log
```

## How it is checked

`cargo check-all` is the one gate. It applies formatting, runs clippy (warnings only), runs every
test (blocking), and runs three repository rules (warnings only: allowed file types, no file over
1 MB, no source file over 400 lines).

| Tests | What they prove |
|---|---|
| `src/domain/tests/` | Every row of the transition table and its refusals, with no database. |
| `tests/happy_path.rs`, `tests/unresolved.rs`, `tests/recovery.rs` | Each scenario on tokio's paused clock: 26 seconds of waiting runs in milliseconds and gives the same answer every time. |
| `tests/use_cases.rs` | The brief's four use cases as four named tests, plus the seed states and reset. |
| `tests/shutdown.rs` | On real time: stopping the server ends an open event stream and the server exits within seconds. |
| `tests/overrides.rs` | The override export: the late-return story before and after the engine answers, and every command that does or does not produce a line. |
| `tests/restart.rs` | A real database file, the app stopped and started again: pending requests survive, overdue results are delivered, a late result after a restart is still not applied. |
| `tests/invariants/` | Random command runs. A seeded generator fires operator commands (right and stale versions), contributor replies, scenario changes, clock jumps and restarts; after every step nine invariants from `docs/contract.md` are checked against the tables with SQL, and the snapshot is compared with the tables. Default 150 seeds of 80 steps; `INVARIANT_SEEDS` and `INVARIANT_STEPS` raise it. |
| `tools/check/cases/` | 55 data-file cases for the submission checker, five per file rule. |

The submission checker is separate from the product: `cargo run -p check -- submission/` prints
one line per rule taken from the brief (file count, file size, build output, secrets, session
exports, answer template, session map, company name, personal information, time budget, video
length) and says what it could not check. `cargo run -p check -- --redact submission/` first
rewrites the packed copies of the session exports (never the originals) so that no key-shaped
string, email address or home-folder name is left in them. Its contract is `tools/check/RULES.md`.

## How it was built

With coding agents, directed and checked by an orchestrating session; the human decisions and
corrections are in [docs/agent-ledger.md](docs/agent-ledger.md), one row per task card.

- The plan ([docs/plan.html](docs/plan.html)) was written, corrected and approved before any code.
- One task card per slice: scope, files the coder may touch, the contract, acceptance checks,
  what is out of scope. One coding agent writes at a time (two only when their folders do not
  overlap). Coders have no web access; small agents fetch pages and write test cases as data.
- A coder's "all green" is not accepted as given. The orchestrator reruns the gate in a plain
  shell, reads the diff against the card, and drives a real browser. Twice that found something
  the report had not: a pass that depended on a private helper script, and the word "null" on the
  page.
- A failed gate is sorted before anything is fixed: it blocks only if the build, a test, an
  invariant, a use case or one of the brief's file rules fails. Everything else is logged as
  deferred and listed below.
- Pass or fail questions a script can answer are answered by a script.

## Known gaps

- Four files are over the 400-line guide (`web/app.js` 447, `src/app.rs` 482,
  `src/domain/rows.rs` 453, `tests/use_cases.rs` 427). Left as they are rather than split late.
- Reset (simulation panel) is not atomic with seeding: a command sent in the instant between the
  wipe and the reseed could land on an empty board.
- A value picked in the assign form is kept as a draft and can reappear in the reassign form
  after the task changes state.
- If handing a new request to the engine fails while the server keeps running, the reply is an
  error although the command committed and the task is already routing. The request then looks
  like one that never answers until the next start, which hands it over again (see "Important
  decisions"). Nothing resends it sooner.
- Redaction of the packed session copies covers key-shaped strings, email addresses and
  home-folder paths. The machine's user name can still appear in other forms (a dashed project
  folder name, a `DOMAIN
ame` account), so "personal-info: PASS" does not mean the name is gone.
- No automated tests for the HTTP layer itself (status codes, the event stream's content); it was
  exercised by hand and through the browser. Every snapshot carries every task's full history,
  which is fine for a seed-sized board and would need paging beyond it. The server's clock reads
  wall time once at start and then counts monotonic time, so it can fall behind after the machine
  sleeps.
- The page was verified in one browser engine (Chromium), in light and dark and at 360 px width.
  No screen-reader pass.
- The submission checker knows four build-folder names (`node_modules`, `target`, `dist`,
  `build`), scans for secrets by pattern, and can only look for the company's name when it is
  given on the command line. It is a safety net, not a guarantee.
- Development machine only: one Visual Studio Build Tools install there is incomplete, so
  `.cargo/config.toml` links the C runtime statically and builds SQLite without compiler
  intrinsics. Both settings are harmless on a complete toolchain. On that machine the first build
  after `cargo clean` must be `cargo check-all`.

## What would come next: bounded automatic retry

Written down as a plan, not built. Today nothing happens to a silent request until the operator
acts. The next step is to let the app route again by itself, within limits.

- **A timeout is a trigger, not a verdict.** The app would still never call a pending request
  failed. What makes acting on a timeout safe already exists: routing again supersedes the old
  request, and a result that arrives late is recorded and never applied.
- **Policy.** Route again automatically at about 30 seconds, at most three attempts in all, then
  hand the task to the operator as needs attention. Opt-in and off by default, so the
  operator-driven behaviour the brief describes stays the default.
- **Why about 30 seconds and not a minute.** The engine's stated window is 5 to 20 seconds. Past
  roughly 30 seconds, waiting longer buys almost nothing, and a minute would add 30 seconds of
  delay to one task in five. With real data the threshold should come from the measured 99th
  percentile of return times.
- **What it buys.** One request in five never returns, and in the simulator each request's fate
  is independent of the last (a hash of the seed and the request id). So the share of tasks that
  need a person for this reason alone falls from 20% today to 4% with one retry (0.2 x 0.2) and
  0.8% with two (0.2 x 0.2 x 0.2). Tasks with no eligible contributor still need a person either
  way.
- **Two guards a real system needs.** If requests wait in a queue, start the clock when the
  engine picks a request up, not when it is submitted; otherwise a busy queue makes healthy
  requests look lost and the retries add load exactly when it hurts. And tell one lost request
  from a disruption by the share of pending requests past the window: about 20% is normal, far
  above that means stop retrying and tell the operator.
- **One trade-off already made on purpose.** A superseded request that later returns a usable
  match is still discarded. Taking the first valid answer would be faster, but superseding is
  simpler to explain and stays correct with "route again without the last person".

## Left out on purpose

- **Work after acceptance.** The brief says routing is complete once a contributor accepts, so
  there is no in-progress or done tracking.
- **Automatic retries and timeouts.** A pending request is unknown, and the app does not turn
  unknown into failed. The retry policy I would add next is described above.
- **Contributor screens and sign-in.** Contributor replies come from the simulation panel. There
  is one operator and no accounts.
- **A real matching algorithm.** The simulator picks the least loaded eligible contributor. The
  brief says matching quality is not the point.
- **Deployment, several servers, an external queue.** One process and one SQLite file.
- **Editing contributors or their expertise.** The panel is seed data.
