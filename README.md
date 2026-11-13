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
never stored, so it needs no timer and is still right after a restart. A bounded retry policy is
the first thing I would add next; it is left out so the operator's judgment stays visible.

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
src/http.rs     JSON routes, server-sent events, embedded static files
web/            index.html, app.js, style.css (plain JavaScript, no build step)
tests/          use cases on tokio's paused clock, restart, random command runs
tools/check/    submission checker for the brief's file rules (standard library only)
tools/check-all the one gate command
docs/           brief, plan, contract, agent ledger, test-case log
```
