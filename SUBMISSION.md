Repository: <link>
Walkthrough video: <Loom link, 10 minutes max>

Agent setup:
Claude Code desktop app. One planning session wrote the plan as a page I could inspect, and I
corrected it before approving it (ledger rows 5 to 8). One build session then acted as orchestrator: it
wrote a task card per slice (scope, files allowed, contract, acceptance checks, out of scope),
handed each card to a single coding subagent (Opus 5.5, medium effort), reran every gate itself in
a plain shell, read the diff against the card, and drove a real browser for verification. Small
Haiku subagents fetched pages and wrote test cases as data files only; coders had no web access.
Pass or fail questions were answered by scripts: `cargo check-all` (format, lint, tests, repo
rules) and a submission checker with 55 data-file cases. A failed gate was sorted before any fix:
it blocked only for a broken build, a failing test or invariant, a wrong use case or a failed file
rule from the brief; everything else was logged as deferred. Every card, result and correction is
one row in `docs/agent-ledger.md`. Two short side sessions reviewed the front end and kept two
other tools' sessions moving through a shared board; their suggestions reached the build only
through me.

Session map:
- Initial planning or decomposition: claude-planning-bf75140f.jsonl, from the first message to the approved plan v5 (ledger rows 1 to 9); the slices, the contract and the task cards are at the start of claude-build-3706823d.jsonl (ledger row 10).
- Most important correction or change of direction: claude-planning-bf75140f.jsonl, my corrections to plan v1 (ledger rows 5 to 8): a second pass on every failed gate instead of fixing every finding, and five-case batches per file rule with a single writer of the checker. In the build, claude-build-3706823d.jsonl at ledger row 12: the coder's all-green report did not reproduce in a plain shell and was sent back.
- Final testing and verification: claude-build-3706823d.jsonl, ledger row 21 onward: the gate output, the browser run of the four use cases with a server restart and the late-return path, both themes and phone width, and the submission checker's output.

Intentional omissions:
- Work after acceptance: the brief says routing is complete once a contributor accepts, so there is no in-progress or done tracking.
- Automatic retries and timeouts: a pending request is unknown, and the app does not turn unknown into failed. A bounded, opt-in retry policy is what I would add next.
- Contributor screens and sign-in: contributor replies come from a simulation panel kept apart from operator actions; one operator, no accounts.
- A real matching algorithm: the simulator picks the least loaded eligible contributor; the brief says matching quality is not the point.
- Deployment, several servers, an external queue: one process and one SQLite file, started with `cargo run`.
- Editing contributors: the panel is seed data.
