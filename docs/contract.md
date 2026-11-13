# Product contract

Written by the orchestrator before slice S1. Every coding card points at sections of this file. It
turns the approved plan (`docs/plan.html`) into rules a test can check. If a coder finds this file
wrong or unclear, they report it; they do not quietly build something else.

## 1. Records

| Table | Holds | Notes |
|---|---|---|
| `contributors` | id, name, areas (one to three), capacity (1 or 2) | Seeded. No editing screen. |
| `tasks` | id, title, area, priority (`high`, `normal`), state, version, attention_reason, created_at_ms, updated_at_ms | `version` starts at 1. |
| `routing_requests` | id, task_id, status (`pending`, `resolved`, `superseded`), started_at_ms, ended_at_ms, excluded contributor ids, result_kind (`match`, `no_match`, null), result_contributor_id, returned_at_ms | One row per attempt. A superseded row keeps its place and may get a result later. |
| `assignments` | id, task_id, contributor_id, status (`offered`, `accepted`, `declined`, `failed`, `withdrawn`), source (`engine`, `operator`), reason_code, request_id, created_at_ms, ended_at_ms | Partial unique index on `task_id` where status is `offered` or `accepted`. |
| `events` | id, task_id, at_ms, kind, actor (`operator`, `engine`, `contributor`, `system`), summary, data (JSON) | Append only. `summary` is a finished sentence for the timeline. |
| `sim_schedule` | request_id, task_id, due_at_ms (null means never), outcome, contributor_id, delivered | Owned by the simulator. The app never reads it. |
| `meta` | key, value | Seed, boot count. |

Task states: `unassigned`, `routing`, `offered`, `assigned`, `needs_attention`, `closed`.
Attention reasons: `declined`, `offer_failed`, `no_match`, `selection_unavailable`.
Reason codes for operator overrides: `engine_too_slow`, `better_fit_known`,
`contributor_unavailable`, `priority`, `other`.

All times are wall-clock milliseconds from one `Clock` value handed to the app, never read
straight from the system inside `domain`, `store` or `app`. Tests run on tokio's paused clock; the
seeder uses a clock it moves by hand.

## 2. Commands and the transition table

`domain.rs` is pure: given what is stored about one task and one command, it returns the events and
record changes to write, or a refusal with a reason. No IO, no clock reads.

| Command | From | To | Rules |
|---|---|---|---|
| `submit` | unassigned | routing | Opens a pending request and hands it to the engine. |
| `route_again` | routing | routing | Needs a reason code. The pending request becomes superseded; a new one opens. |
| `route_again` | needs_attention | routing | No reason code needed. `exclude_previous: true` leaves out every contributor whose offer for this task was declined or failed. |
| `cancel_routing` | routing | unassigned | Needs a reason code. The pending request becomes superseded. |
| `assign` | routing | offered | Needs a reason code. The pending request becomes superseded. |
| `assign` | needs_attention | offered | Needs a reason code. |
| `assign` | offered, assigned | offered | Needs a reason code. The current assignment becomes withdrawn. Assigning the contributor who already holds it is refused. |
| `close` | needs_attention | closed | |
| contributor `accept` | offered | assigned | Routing for the task is complete. |
| contributor `decline` | offered | needs_attention | Reason `declined`. |
| contributor `fail` | offered | needs_attention | Reason `offer_failed` (the offer could not be delivered). |
| engine returned, request pending, match, contributor has room | routing | offered | Assignment with source `engine`. |
| engine returned, request pending, match, contributor is full | routing | needs_attention | Reason `selection_unavailable`. |
| engine returned, request pending, no match | routing | needs_attention | Reason `no_match`. |
| engine returned, request superseded | any | unchanged | Result stored on the request row, one `late_result_ignored` event, nothing else. |

Anything not in the table is refused. `assign` is refused when the contributor lacks the task's
area or has no room (active load equals capacity): expertise and capacity are hard limits; the
operator overrides the engine's choice among eligible people, not the limits.

Every operator command carries `expected_version`. If it differs from the stored version the
command is refused as stale and the reply carries the current task. The version goes up by one
each time a command changes the task's state or its active assignment. Events that only add
information (`late_result_ignored`, `server_restarted`) do not change the version, so a note
arriving cannot make an operator's next action stale.

One command is one SQLite transaction: the check, the record changes and the events are written
together or not at all.

## 3. The routing engine

```
trait RoutingEngine: accepts (request id, task, eligible contributors with their load)
                     later reports (request id, match contributor | no match), or never reports
```

Eligible means: has the task's area, and is not in the request's excluded list. The engine gets
this list when the request starts. The app applies a result when it arrives, by the table above.

The app never marks a pending request failed and has no timeout. "Past the normal window" means
more than 20 000 ms since the request started; it is worked out when a snapshot is built and is
never stored.

The simulator:

- decides each request's outcome from a hash of the seed and the request id, never from call order;
- in `auto`: 20 in 100 never return; the rest return after 5 to 20 seconds with the least loaded
  eligible contributor, or no match when the list is empty;
- per-task scenarios override that: `fast_match` (6 s), `slow_match` (26 s), `no_match` (8 s),
  `never_returns`;
- writes each decision to `sim_schedule` when the request starts, and on start-up re-arms every
  undelivered row (an overdue row is delivered straight away), the way an outside service would
  have kept running while the app was down.

On start-up the app writes one `server_restarted` event on every task that has a pending request.

## 4. Invariants (checked after every step of the random command runs)

1. A task has at most one assignment with status `offered` or `accepted`.
2. A task has at most one `pending` routing request.
3. State agrees with the records: `routing` has exactly one pending request and no active
   assignment; `offered` has an active assignment with status `offered` and no pending request;
   `assigned` has one with status `accepted` and no pending request; `unassigned`,
   `needs_attention` and `closed` have neither. `needs_attention` has an attention reason; no
   other state does.
4. No contributor's active load (offered plus accepted) exceeds their capacity.
5. An active assignee has the task's area.
6. A task's version never goes down, and it changes if and only if the state or the active
   assignment changed.
7. A result for a superseded request changes nothing except its own request row and one event.
8. Events are append only, ordered by time, and the first event of every task is `task_created`.
9. `closed` is final: nothing moves a task out of it.

## 5. HTTP

| Route | Body | Reply |
|---|---|---|
| `GET /` and static files | | The page, embedded in the binary. |
| `GET /api/snapshot` | | The snapshot below. |
| `GET /api/events` | | Server-sent events: a `changed` event after every committed change. The page then fetches the snapshot again. |
| `POST /api/tasks` | `{title, area, priority, submit}` | 201 with the task. |
| `POST /api/tasks/{id}/commands` | `{type, expected_version, contributor_id?, reason_code?, note?, exclude_previous?}` where type is `submit`, `route_again`, `cancel_routing`, `assign` or `close` | 200 with the task. 409 `{error:"stale_version", message, task}`. 422 `{error:"refused", message}`. 404 for an unknown task. |
| `POST /api/sim/assignments/{id}/reply` | `{reply}` where reply is `accept`, `decline` or `fail` | 200, or 422 when the assignment is not the active offer. |
| `POST /api/sim/tasks/{id}/scenario` | `{scenario}` | 200. Applies to requests started afterwards. |
| `POST /api/sim/reset` | `{seed?}` | 200. Wipes the data and seeds again. |

Snapshot:

```json
{
  "now_ms": 0,
  "boot": {"count": 1, "started_at_ms": 0},
  "sim": {"seed": 42, "scenarios": ["auto", "fast_match", "slow_match", "no_match", "never_returns"]},
  "normal_window_ms": {"min": 5000, "max": 20000},
  "areas": ["finance", "healthcare", "data_analysis", "operations", "veterinary"],
  "reason_codes": [{"code": "engine_too_slow", "label": "Engine too slow"}],
  "contributors": [
    {"id": 1, "name": "", "areas": ["finance"], "capacity": 2, "active_load": 1}
  ],
  "tasks": [
    {
      "id": 1, "title": "", "area": "healthcare", "priority": "high",
      "state": "routing", "version": 2, "attention_reason": null,
      "created_at_ms": 0, "updated_at_ms": 0,
      "pending_request": {"id": 7, "started_at_ms": 0, "elapsed_ms": 26000, "past_normal_window": true},
      "active_assignment": {"id": 4, "contributor_id": 2, "status": "offered", "source": "engine", "reason_code": null},
      "scenario": "auto",
      "allowed_commands": ["route_again", "cancel_routing", "assign"],
      "events": [
        {"id": 1, "at_ms": 0, "kind": "task_created", "actor": "operator", "summary": "", "data": {}}
      ]
    }
  ]
}
```

`pending_request` and `active_assignment` are null when absent. `allowed_commands` comes from
`domain.rs`, so the page never carries its own copy of the transition table. Event kinds:
`task_created`, `routing_started`, `engine_matched`, `engine_no_match`,
`engine_selection_unavailable`, `operator_assigned`, `operator_rerouted`,
`operator_cancelled_routing`, `late_result_ignored`, `contributor_accepted`,
`contributor_declined`, `offer_failed`, `task_closed`, `server_restarted`.

## 6. Running

`cargo run` starts everything: it creates `routing-console.db` in the working directory if it is
missing, seeds it when empty, and serves `http://localhost:3000`. `PORT` and `ROUTING_DB` override
the port and the database path. The page's files are compiled into the binary with `include_str!`.

## 7. Seed

Eight invented contributors, each with one to three areas and capacity 1 or 2. Nobody has
`veterinary`. Six tasks, reaching these states by replaying real commands through the app against
a hand-moved clock (no rows inserted directly):

| Task | Area | Priority | State after seeding |
|---|---|---|---|
| Grade 40 model answers on hospital discharge planning | healthcare | high | routing, pending for longer than the normal window, scenario `never_returns` |
| Red-team a tax-advice assistant, 25 prompts | finance | high | needs_attention, `declined` |
| 30-minute interview on month-end close | finance | normal | assigned |
| Label 300 support tickets by root cause | data_analysis | normal | offered |
| Survey on warehouse shift scheduling tools | operations | normal | unassigned |
| Annotate dairy-herd sensor clips for lameness | veterinary | high | needs_attention, `no_match` |
