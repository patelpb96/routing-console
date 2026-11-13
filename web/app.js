// The page keeps no domain state: every change on the server sends `changed`, the page fetches one
// snapshot and `render` redraws from it. View state is only the selected task (the URL hash), the
// drafts of open form fields, and which in-page confirmation is open.
"use strict";

const $ = (sel) => document.querySelector(sel);

// Plain words for server codes. An unknown code shows as itself, so a new server value still shows.
const LABELS = {
  state: { unassigned: "Unassigned", routing: "Routing", offered: "Offered", assigned: "Assigned",
    needs_attention: "Needs attention", closed: "Closed" },
  attention: { declined: "Contributor declined", offer_failed: "Offer could not be delivered",
    no_match: "Engine found no match", selection_unavailable: "The engine's pick was no longer available" },
  reason: { engine_too_slow: "Engine too slow", better_fit_known: "Better fit known",
    contributor_unavailable: "Contributor unavailable", priority: "Priority", other: "Other" },
  area: { finance: "Finance", healthcare: "Healthcare", data_analysis: "Data analysis",
    operations: "Operations", veterinary: "Veterinary" },
  priority: { high: "High priority", normal: "Normal priority" },
  actor: { operator: "Operator", engine: "Engine", contributor: "Contributor", system: "System" },
  assignment: { offered: "offered, no reply yet", accepted: "accepted" },
  source: { engine: "engine's pick", operator: "assigned by hand" },
  command: { submit: "Submit for assignment", assign: "Assign by hand", route_again: "Route again",
    cancel_routing: "Cancel routing", close: "Close task" },
  scenario: { auto: "Auto (normal behaviour)", fast_match: "Fast match (6 s)", slow_match: "Slow match (26 s)",
    no_match: "No match (8 s)", never_returns: "Never returns" },
};
const label = (group, code) => (LABELS[group] && LABELS[group][code]) || String(code);
const TONE = { routing: "accent", offered: "accent", assigned: "ok", needs_attention: "bad" };
const COMMAND_ORDER = ["submit", "assign", "route_again", "cancel_routing", "close"];
const SCALE_MS = 30000;

// The one rule the page must know about the server: which commands carry a reason code.
const needsReason = (cmd, t) =>
  cmd === "assign" || (t.state === "routing" && (cmd === "route_again" || cmd === "cancel_routing"));

let last = null;                 // the snapshot drawn last
let win = { min: 5000, max: 20000 };
let offset = 0;                  // server now_ms minus performance.now(), so the clock ticks locally
let names = new Map();
let confirming = null;           // key of the open in-page confirmation
let queueKey = "";
const drafts = new Map();        // data-key -> value the operator typed or picked
const hooks = [];

const now = () => performance.now() + offset;
const selectedId = () => { const m = /^#task-(\d+)$/.exec(location.hash); return m ? Number(m[1]) : null; };

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) node.append(c);
  return node;
}

// A form control that keeps what the operator typed across redraws.
function field(tag, key, attrs, ...children) {
  const node = el(tag, { ...attrs, "data-key": key }, ...children);
  if (drafts.has(key)) {
    const v = drafts.get(key);
    if (node.type === "checkbox") node.checked = v;
    else if (tag !== "select" || [...node.options].some((o) => o.value === v)) node.value = v;
  }
  return node;
}
function keepDraft(e) {
  const n = e.target;
  if (!n.dataset || !n.dataset.key || "live" in n.dataset) return;
  if (!["INPUT", "SELECT", "TEXTAREA"].includes(n.tagName)) return;
  drafts.set(n.dataset.key, n.type === "checkbox" ? n.checked : n.value);
}
function clearDrafts(prefix) { for (const k of [...drafts.keys()]) if (k.startsWith(prefix)) drafts.delete(k); }

function dur(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`;
  const m = Math.floor(s / 60);
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}
const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

function sentence(ms) {
  const normal = `Answers normally take ${win.min / 1000} to ${win.max / 1000} s.`;
  if (ms > win.max) return `No answer yet after ${dur(ms)}. ${normal} It may still answer.`;
  return `Waiting for the engine: ${dur(ms)} so far. ${normal}`;
}

// Time-dependent nodes carry their start time; `paint` is all the once-a-second tick does to them.
function timed(show, startedMs, tag = "span") {
  const node = el(tag, { "data-show": show, "data-started": startedMs });
  paint(node, now());
  return node;
}
function paint(node, at) {
  const ms = at - Number(node.dataset.started);
  const past = ms > win.max;
  const show = node.dataset.show;
  if (show === "ago" || show === "elapsed") node.textContent = dur(ms);
  else if (show === "sentence") node.textContent = sentence(ms);
  else if (show === "advice") node.textContent = past
    ? (node.dataset.assign === "1" ? "Your call: keep waiting, route again, or assign someone by hand."
      : "Your call: keep waiting or route again. Nobody who works in this area has room to assign by hand.")
    : "The engine is looking for a contributor. Nothing to do yet.";
  else if (show === "window") {
    node.textContent = past ? "Past the normal window" : ms < win.min ? "Before the normal window" : "Within the normal window";
  } else if (show === "marker") node.style.left = `${(Math.min(ms, SCALE_MS) / SCALE_MS) * 100}%`;
  if (show !== "ago") node.classList.toggle("past", past);
}

function say(text, kind = "info") {
  const box = $("#msg");
  box.textContent = text;
  box.className = text ? kind : "";
  $("#msg-close").hidden = !text;
}

// Resolves to the reply body (or `{}`) when the server accepted, to null when it did not.
async function post(url, body, okText) {
  let res;
  try {
    res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  } catch (_) {
    say("The server is not reachable. Try again when it is back.", "error");
    return null;
  }
  let data = null;
  try { data = await res.json(); } catch (_) { /* empty or plain-text body */ }
  if (res.ok) {
    if (okText) say(okText, "info");
    refresh();
    return data || {};
  }
  if (res.status === 409) say("This task changed since you looked. Showing the current state.", "error");
  else if (data && data.message) say(data.message, "error");
  else say(`The server refused this (status ${res.status}).`, "error");
  refresh();
  return null;
}

let asked = 0;
let drawn = 0;
async function refresh() {
  const n = ++asked;
  try {
    const res = await fetch("/api/snapshot", { cache: "no-store" });
    if (!res.ok) throw new Error(String(res.status));
    const snap = await res.json();
    if (n <= drawn) return;      // an older reply arriving late must not undo a newer one
    drawn = n;
    offset = snap.now_ms - performance.now();
    render(snap);
  } catch (_) {
    if (n > drawn) say("Could not load the current state: the server is not reachable.", "error");
  }
}

// Redrawing replaces nodes; put focus (and the caret) back on the control the operator was using.
function keepFocus(draw) {
  const active = document.activeElement;
  const key = active && active.dataset ? active.dataset.key : null;
  let range = null;
  try { if (key && typeof active.selectionStart === "number") range = [active.selectionStart, active.selectionEnd]; } catch (_) { /* no caret */ }
  draw();
  if (!key) return;
  const n = document.querySelector(`[data-key="${CSS.escape(key)}"]`);
  if (!n || n === active) return;
  n.focus({ preventScroll: true });
  if (range && n.setSelectionRange) try { n.setSelectionRange(...range); } catch (_) { /* no caret */ }
}
const focusKey = (key) => { const n = document.querySelector(`[data-key="${CSS.escape(key)}"]`); if (n) n.focus(); };
const redraw = () => { if (last) render(last); };

function render(snap) {
  last = snap;
  win = snap.normal_window_ms;
  names = new Map(snap.contributors.map((c) => [c.id, c.name]));
  keepFocus(() => {
    drawQueue(snap);
    drawTasks(snap);
    drawDetail(snap);
    drawPeople(snap);
    fillAreas(snap);
    for (const fn of hooks) fn(snap);
  });
}

const waitingSince = (t) => (t.pending_request ? t.pending_request.started_at_ms : t.updated_at_ms);
const pastWindow = (t, at) => t.state === "routing" && !!t.pending_request && at - t.pending_request.started_at_ms > win.max;
function queueOf(snap, at) {
  return snap.tasks
    .filter((t) => t.state === "needs_attention" || pastWindow(t, at))
    .sort((a, b) => (a.priority === "high" ? 0 : 1) - (b.priority === "high" ? 0 : 1) || waitingSince(a) - waitingSince(b));
}

function drawHeader(snap, queue) {
  const count = (state) => snap.tasks.filter((t) => t.state === state).length;
  const pairs = [["Needs you", queue.length], ["Routing", count("routing")], ["Offered", count("offered")]];
  $("#counts").replaceChildren(...pairs.map(([k, v]) => el("div", {}, el("dt", {}, k), el("dd", {}, String(v)))));
  const since = clock(snap.boot.started_at_ms);
  $("#boot").textContent = snap.boot.count > 1 ? `Server restarted at ${since}` : `Running since ${since}`;
}

function taskLink(t, prefix, ...lines) {
  const a = el("a", { href: `#task-${t.id}`, class: "row", "data-key": `${prefix}-${t.id}` },
    el("span", { class: "title" }, t.title), ...lines);
  if (t.id === selectedId()) a.setAttribute("aria-current", "true");
  return el("li", { class: t.state === "closed" ? "closed" : null }, a);
}
const stateTag = (t) => el("span", { class: `tag ${TONE[t.state] || "quiet"}` }, label("state", t.state));
const prioTag = (t) => el("span", { class: t.priority === "high" ? "tag strong" : "tag quiet" }, label("priority", t.priority));
function holder(t) {
  const a = t.active_assignment;
  return a ? `${names.get(a.contributor_id) || `Contributor ${a.contributor_id}`} (${label("assignment", a.status)})` : null;
}

function drawQueue(snap) {
  const queue = queueOf(snap, now());
  queueKey = queue.map((t) => t.id).join();
  drawHeader(snap, queue);
  const rows = queue.map((t) => {
    const why = t.state === "routing"
      ? el("span", { class: "why past" }, "No answer for ", timed("elapsed", t.pending_request.started_at_ms), ". It may still answer.")
      : el("span", { class: "why" }, `${label("attention", t.attention_reason)}. Waiting `, timed("ago", t.updated_at_ms), ".");
    return taskLink(t, "q", el("span", { class: "meta" }, prioTag(t), stateTag(t), label("area", t.area)), why);
  });
  $("#queue").replaceChildren(...(rows.length ? rows : [el("li", { class: "empty" }, "Nothing needs you right now.")]));
}

// The 0 to 30 s scale in three segments drawn to scale; `labels` only on the full-size bar.
const segments = (labels) => [
  el("span", { class: "early", style: `flex:${win.min}` }, labels ? "early" : ""),
  el("span", { class: "normal", style: `flex:${win.max - win.min}` }, labels ? "normal window" : ""),
  el("span", { class: "over", style: `flex:${SCALE_MS - win.max}` }, labels ? "past window" : ""),
];

function elapsedChip(p) {
  return el("span", { class: "chip" },
    el("span", { class: "mini", "aria-hidden": "true" }, ...segments(false), timed("marker", p.started_at_ms, "i")),
    timed("elapsed", p.started_at_ms), " · ", timed("window", p.started_at_ms));
}

function drawTasks(snap) {
  const tasks = [...snap.tasks].sort((a, b) => (a.state === "closed") - (b.state === "closed") || a.id - b.id);
  $("#tasks").replaceChildren(...tasks.map((t) => taskLink(t, "t",
    el("span", { class: "meta" }, stateTag(t), label("area", t.area), prioTag(t),
      el("span", {}, "created ", timed("ago", t.created_at_ms), " ago"),
      holder(t) && el("span", {}, `held by ${holder(t)}`)),
    t.pending_request && elapsedChip(t.pending_request))));
}

function drawPeople(snap) {
  $("#people").replaceChildren(...snap.contributors.map((c) => {
    const full = c.active_load >= c.capacity;
    return el("li", {},
      el("span", { class: "name" }, c.name),
      el("span", { class: "muted" }, c.areas.map((a) => label("area", a)).join(", ")),
      el("span", { class: "load" }, `${c.active_load} of ${c.capacity}`, full && el("span", { class: "tag quiet" }, "Full")));
  }));
}

function fillAreas(snap) {
  const sel = $("#create select[name=area]");
  if (sel.dataset.filled === snap.areas.join()) return;
  sel.replaceChildren(...snap.areas.map((a) => el("option", { value: a }, label("area", a))));
  sel.dataset.filled = snap.areas.join();
}

// An irreversible button asks in the page first; which question is open is view state, so it
// survives a redraw.
function confirmBlock(key, buttonText, question, yesText, onYes) {
  if (confirming !== key) {
    return el("button", { type: "button", "data-key": `${key}-ask`,
      onclick: () => { confirming = key; redraw(); focusKey(`${key}-yes`); } }, buttonText);
  }
  return el("div", { class: "confirm", role: "group", "aria-label": question },
    el("p", {}, question),
    el("button", { type: "button", class: "danger", "data-key": `${key}-yes`,
      onclick: () => { confirming = null; redraw(); onYes(); } }, yesText),
    el("button", { type: "button", "data-key": `${key}-no`,
      onclick: () => { confirming = null; redraw(); focusKey(`${key}-ask`); } }, "Keep it"));
}

let detailSig = "";
function drawDetail(snap) {
  const box = $("#detail");
  const id = selectedId();
  const t = snap.tasks.find((x) => x.id === id);
  // Rebuilding would close a select the operator has open, so only redraw when what it shows changed.
  const sig = JSON.stringify([id, t && [t.title, t.version, t.state, t.attention_reason, t.events.length,
    t.allowed_commands, t.assignable_contributor_ids, t.active_assignment, t.pending_request && t.pending_request.id],
  snap.contributors, snap.reason_codes, confirming]);
  if (sig === detailSig) return;
  detailSig = sig;
  if (!t) {
    box.replaceChildren(el("h2", { id: "detail-h", tabindex: "-1" }, "Task detail"),
      el("p", { class: "muted" }, id ? `Task ${id} is not in the current list.` : "Choose a task to see its state, actions and history."));
    return;
  }
  const fact = (k, ...v) => el("div", {}, el("dt", {}, k), el("dd", {}, ...v));
  const next = nextStep(t);
  // Optional parts are null when absent; replaceChildren would print the word "null" for them.
  box.replaceChildren(...[
    el("a", { href: "#", class: "back" }, "Back to the list"),
    el("p", { class: "eyebrow" }, `Task ${t.id}`),
    el("h2", { id: "detail-h", tabindex: "-1" }, t.title),
    el("dl", { class: "facts" },
      fact("State", stateTag(t), t.attention_reason && ` ${label("attention", t.attention_reason)}`),
      fact("Area", label("area", t.area)), fact("Priority", label("priority", t.priority)),
      fact("Held by", holder(t) || "Nobody"),
      fact("Created", `${clock(t.created_at_ms)}, `, timed("ago", t.created_at_ms), " ago")),
    t.active_assignment && el("p", { class: "small muted" }, `How: ${label("source", t.active_assignment.source)}` +
      (t.active_assignment.reason_code ? `, reason: ${label("reason", t.active_assignment.reason_code)}.` : ".")),
    t.pending_request && scale(t.pending_request),
    next.text,
    el("h3", {}, "Your decision"),
    actions(t, snap, next.primary),
    el("h3", {}, "History"),
    history(t)].filter(Boolean));
}

function scale(p) {
  const s = p.started_at_ms;
  const pct = (ms) => `left:${(ms / SCALE_MS) * 100}%`;
  return el("div", { class: "scale" },
    el("div", { class: "bar", "aria-hidden": "true" }, ...segments(true), timed("marker", s, "i")),
    el("div", { class: "ticks", "aria-hidden": "true" },
      el("span", { style: pct(0) }, "0 s"), el("span", { style: pct(win.min) }, `${win.min / 1000} s`),
      el("span", { style: pct(win.max) }, `${win.max / 1000} s`), el("span", { style: pct(SCALE_MS) }, `${SCALE_MS / 1000} s`)),
    timed("sentence", s, "p"));
}

// Each action keeps one name: the button, the confirmation and the history all say the same thing.
const DONE = { submit: "Submitted for assignment.", assign: "Assigned by hand. Waiting for them to accept.", route_again: "Routing again.",
  cancel_routing: "Routing cancelled.", close: "Task closed." };
async function command(t, type, extra) {
  const ok = await post(`/api/tasks/${t.id}/commands`, { type, expected_version: t.version, ...extra }, DONE[type]);
  if (ok) clearDrafts(`t${t.id}-`);
}

// What the operator can do now, in one sentence, and which button (if any) is the likely one. A
// request still waiting gets no likely button: unknown is not failed, so waiting stays a fair choice.
function nextStep(t) {
  const step = (text, primary = null) => ({ text: el("p", { class: "next" }, text), primary });
  const who = (t.active_assignment && names.get(t.active_assignment.contributor_id)) || "The contributor";
  // Only name the options this task really has: assigning needs the command and someone with room.
  const canAssign = t.allowed_commands.includes("assign") && t.assignable_contributor_ids.length > 0;
  const orAssign = canAssign ? ", or assign someone by hand" : "";
  if (t.state === "unassigned") return step("Not sent to the engine yet. Submit it when it is ready.", "submit");
  if (t.state === "routing" && t.pending_request) {
    const node = el("p", { class: "next", "data-show": "advice", "data-started": t.pending_request.started_at_ms,
      "data-assign": canAssign ? "1" : "0" });
    paint(node, now());
    return { text: node, primary: null };
  }
  if (t.state === "offered") return step(`Waiting for ${who} to accept. Nothing to do yet.`);
  if (t.state === "assigned") return step(`${who} has accepted. Routing is complete.`);
  if (t.state === "closed") return step("Closed. Nothing more can happen to this task.");
  if (t.state !== "needs_attention") return step("Nothing to do yet.");
  if (t.attention_reason === "no_match") {
    return canAssign ? step("The engine found nobody. Assign someone by hand, or close the task.", "assign")
      : step("The engine found nobody, and nobody who works in this area has room. Route again later, or close the task.");
  }
  if (t.attention_reason === "selection_unavailable") return step(`The engine's pick filled up first. Route again${orAssign}.`, "route_again");
  if (t.attention_reason === "declined") return step(`The contributor declined. Route again without them${orAssign}.`, "route_again");
  return step(`The offer could not be delivered. Route again without them${orAssign}.`, "route_again");
}

// One form for the whole decision: one reason, shared by every action that needs one.
function actions(t, snap, primary) {
  const k = `t${t.id}`;
  const allowed = COMMAND_ORDER.filter((c) => t.allowed_commands.includes(c));
  if (!allowed.length) return el("p", { class: "muted" }, "No actions in this state.");
  const button = (cmd, text) => el("button", { type: "submit", name: "cmd", value: cmd, "data-key": `${k}-${cmd}-go`,
    class: cmd === primary ? "primary" : null }, text || label("command", cmd));
  const parts = [];
  const usable = (c) => c !== "close" && (c !== "assign" || t.assignable_contributor_ids.length > 0);
  const reasoned = allowed.filter((c) => usable(c) && needsReason(c, t));
  if (reasoned.length) {
    const verbs = reasoned.map((c) => label("command", c).replace(/ by hand$/, "").toLowerCase());
    const list = verbs.length > 1 ? `${verbs.slice(0, -1).join(", ")} or ${verbs.at(-1)}` : verbs[0];
    parts.push(el("label", { class: "f" }, `Reason for stepping in (needed to ${list})`,
      field("select", `${k}-reason`, { name: "reason" }, el("option", { value: "" }, "Choose a reason"),
        ...snap.reason_codes.map((r) => el("option", { value: r.code }, LABELS.reason[r.code] || r.label || r.code)))));
  }
  if (allowed.includes("assign")) {
    const ids = t.assignable_contributor_ids;
    const people = new Map(snap.contributors.map((c) => [c.id, c]));
    const opt = (id) => { const c = people.get(id); return el("option", { value: id }, c ? `${c.name} (${c.active_load} of ${c.capacity} tasks)` : `Contributor ${id}`); };
    parts.push(ids.length
      ? el("div", { class: "line" },
        el("label", { class: "f" }, "Contributor", field("select", `${k}-assign-who`, { name: "contributor" }, ...ids.map(opt))),
        el("label", { class: "f" }, "Note (optional)", field("input", `${k}-assign-note`, { name: "note", maxlength: "500", autocomplete: "off" })),
        button("assign", t.active_assignment ? "Reassign by hand" : "Assign by hand"))
      : el("p", { class: "muted" }, "Nobody who works in this area has room, so nobody can be assigned by hand."));
  }
  const rest = allowed.filter((c) => c !== "assign" && c !== "close");
  if (rest.includes("route_again") && t.state === "needs_attention") {
    const lost = t.attention_reason === "declined" || t.attention_reason === "offer_failed";
    parts.push(el("label", { class: "check" }, field("input", `${k}-route-exclude`, { type: "checkbox", name: "exclude", checked: lost }),
      " When routing again, leave out anyone who declined or could not be reached"));
  }
  if (rest.length) parts.push(el("div", { class: "buttons" }, ...rest.map((c) => button(c))));
  const form = el("form", { class: "decide", onsubmit: (e) => {
    e.preventDefault();
    const cmd = e.submitter && e.submitter.value;
    const f = e.target.elements;
    if (!cmd) return;
    const extra = {};
    if (needsReason(cmd, t)) {
      extra.reason_code = f.namedItem("reason").value;
      if (!extra.reason_code) { say(`Choose a reason first: "${label("command", cmd)}" needs one.`, "error"); focusKey(`${k}-reason`); return; }
    }
    if (cmd === "assign") {
      extra.contributor_id = Number(f.namedItem("contributor").value);
      extra.note = f.namedItem("note").value.trim() || undefined;
    }
    if (cmd === "route_again" && t.state === "needs_attention") extra.exclude_previous = f.namedItem("exclude").checked;
    command(t, cmd, extra);
  } }, ...parts);
  const close = allowed.includes("close") && confirmBlock(`${k}-close`, "Close task",
    "Close this task? A closed task cannot be reopened.", "Yes, close it", () => command(t, "close", {}));
  return el("div", { class: "stack" }, parts.length ? form : null, close || null);
}

function history(t) {
  const evs = [...t.events].sort((a, b) => a.at_ms - b.at_ms || a.id - b.id);
  return el("ol", { class: "history" }, ...evs.map((ev) => {
    const note = ev.kind === "late_result_ignored" || ev.kind === "server_restarted";
    return el("li", { class: note ? "note" : null },
      el("time", { datetime: new Date(ev.at_ms).toISOString() }, clock(ev.at_ms)),
      el("span", { class: "actor" }, label("actor", ev.actor)),
      el("span", { class: "summary" }, note && el("strong", {}, "Note: "), ev.summary,
        ev.data && typeof ev.data.note === "string" && ev.data.note && el("span", { class: "muted" }, ` Operator's note: ${ev.data.note}`)));
  }));
}

// A scenario only applies to requests started after it is set, so a task with one is created
// unsubmitted, given its scenario, and only then submitted.
async function createTask(body, scenario) {
  if (!scenario || scenario === "auto") return post("/api/tasks", body, "Task created.");
  const task = await post("/api/tasks", { ...body, submit: false });
  if (!task) return null;
  if (!(await post(`/api/sim/tasks/${task.id}/scenario`, { scenario }))) return task;
  if (body.submit && !(await post(`/api/tasks/${task.id}/commands`, { type: "submit", expected_version: task.version }))) return task;
  say(`Task created with engine scenario: ${label("scenario", scenario)}.`, "info");
  return task;
}

$("#create").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  // `elements` because a form's own `title` and `submit` properties shadow named controls.
  const f = form.elements;
  const button = form.querySelector("button[type=submit]");
  button.disabled = true;
  try {
    const task = await createTask({ title: f.namedItem("title").value.trim(), area: f.namedItem("area").value,
      priority: f.namedItem("priority").value, submit: f.namedItem("submit").checked }, f.namedItem("scenario").value);
    if (task) { form.reset(); if (task.id) location.hash = `task-${task.id}`; }
  } finally { button.disabled = false; }
});

document.addEventListener("input", keepDraft);
document.addEventListener("change", keepDraft);
$("#msg-close").addEventListener("click", () => say(""));
window.addEventListener("hashchange", () => {
  confirming = null;
  redraw();
  const h = $("#detail-h");
  if (h && selectedId()) h.focus();
});

// Crossing 20 s commits nothing on the server, so no event arrives: the page moves its own clock.
setInterval(() => {
  if (!last) return;
  const at = now();
  for (const n of document.querySelectorAll("[data-started]")) paint(n, at);
  if (queueOf(last, at).map((t) => t.id).join() !== queueKey) keepFocus(() => drawQueue(last));
}, 1000);

const stream = new EventSource("/api/events");
stream.addEventListener("open", () => { $("#conn").textContent = "Live"; refresh(); });
stream.addEventListener("error", () => { $("#conn").textContent = "Reconnecting to the server"; });
stream.addEventListener("changed", refresh);

// What sim.js may use; it draws its panel from the same snapshot after every render.
window.Console = {
  el, field, post, say, label, confirmBlock, clearDrafts, selectedId,
  confirming: () => confirming,
  onRender(fn) { hooks.push(fn); if (last) fn(last); },
};
refresh();
