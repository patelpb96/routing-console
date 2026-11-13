// The page keeps no state: every change on the server sends `changed`, the page fetches one
// snapshot and `render` redraws everything from it.
"use strict";

const $ = (sel) => document.querySelector(sel);

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const c of children) node.append(c);
  return node;
}

function since(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  return `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`;
}

async function post(url, body) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) {
    let message = text;
    try { message = JSON.parse(text).message || text; } catch (_) { /* plain-text 500 */ }
    $("#status").textContent = `Refused (${res.status}): ${message}`;
  } else {
    $("#status").textContent = "";
  }
}

function render(snap) {
  const areaSelect = $("#create select[name=area]");
  if (areaSelect.options.length !== snap.areas.length) {
    areaSelect.replaceChildren(...snap.areas.map((a) => el("option", { value: a }, a)));
  }
  const names = new Map(snap.contributors.map((c) => [c.id, c.name]));
  const rows = snap.tasks.map((t) => {
    const actions = el("td");
    if (t.allowed_commands.includes("submit")) {
      actions.append(el("button", {
        onclick: () => post(`/api/tasks/${t.id}/commands`,
          { type: "submit", expected_version: t.version }),
      }, "Submit"));
    }
    const sim = el("td");
    const a = t.active_assignment;
    if (a && a.status === "offered") {
      sim.append(el("button", {
        class: "sim",
        onclick: () => post(`/api/sim/assignments/${a.id}/reply`, { reply: "accept" }),
      }, `Simulate: ${names.get(a.contributor_id) || "contributor"} accepts`));
    }
    const picker = el("select", {
      "aria-label": "Simulated engine scenario",
      onchange: (e) => post(`/api/sim/tasks/${t.id}/scenario`, { scenario: e.target.value }),
    }, ...snap.sim.scenarios.map((s) => el("option", { value: s }, s)));
    picker.value = t.scenario;
    sim.append(el("label", {}, "Scenario ", picker));

    let state = t.state;
    if (a) state += ` (${names.get(a.contributor_id) || a.contributor_id})`;
    return el("tr", {},
      el("td", {}, t.title), el("td", {}, t.area), el("td", {}, t.priority),
      el("td", {}, state), el("td", {}, since(snap.now_ms - t.created_at_ms)),
      actions, sim);
  });
  $("#tasks").replaceChildren(...rows);
}

async function refresh() {
  try {
    const res = await fetch("/api/snapshot");
    if (!res.ok) throw new Error(await res.text());
    render(await res.json());
  } catch (e) {
    $("#status").textContent = `Could not load the snapshot: ${e.message}`;
  }
}

$("#create").addEventListener("submit", (e) => {
  e.preventDefault();
  // `elements` because a form's own `title` and `submit` properties shadow named controls.
  const f = e.target.elements;
  post("/api/tasks", {
    title: f.namedItem("title").value,
    area: f.namedItem("area").value,
    priority: f.namedItem("priority").value,
    submit: f.namedItem("submit").checked,
  }).then(() => { f.namedItem("title").value = ""; });
});

new EventSource("/api/events").addEventListener("changed", refresh);
refresh();
