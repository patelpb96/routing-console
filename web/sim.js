// The simulation panel stands in for the engine and the contributors. It lives in its own script
// and its own region so nothing here can be mistaken for an operator action.
"use strict";

(() => {
  const C = window.Console;
  const { el, field, post, label } = C;
  let sig = "";

  function fillCreateScenarios(snap) {
    const sel = document.getElementById("create-scenario");
    const want = snap.sim.scenarios.join();
    if (sel.dataset.filled === want) return;
    const keep = sel.value;
    sel.replaceChildren(...snap.sim.scenarios.map((s) => el("option", { value: s }, label("scenario", s))));
    if (snap.sim.scenarios.includes(keep)) sel.value = keep;
    sel.dataset.filled = want;
  }

  function scenarioBlock(t, snap) {
    const head = el("h3", {}, "Engine scenario");
    if (!t) return el("div", {}, head, el("p", { class: "small muted" }, "Choose a task to set how the simulated engine answers it."));
    // Sent as soon as it changes, so it is not a draft: the snapshot is what it shows.
    const pick = el("select", { "data-key": "sim-scenario", "data-live": true,
      onchange: async (e) => {
        const ok = await post(`/api/sim/tasks/${t.id}/scenario`, { scenario: e.target.value },
          `Engine scenario for task ${t.id}: ${label("scenario", e.target.value)}.`);
        if (!ok) e.target.value = t.scenario;
      } }, ...snap.sim.scenarios.map((s) => el("option", { value: s }, label("scenario", s))));
    pick.value = t.scenario;
    return el("div", {}, head,
      el("label", { class: "f" }, `For task ${t.id}, ${t.title}`, pick),
      el("p", { class: "small muted" }, "Applies to requests started after you set it. A request already waiting keeps the outcome it was given."));
  }

  function offersBlock(snap) {
    const people = new Map(snap.contributors.map((c) => [c.id, c.name]));
    const offers = snap.tasks.filter((t) => t.active_assignment && t.active_assignment.status === "offered");
    const head = el("h3", {}, "Contributor replies");
    if (!offers.length) return el("div", {}, head, el("p", { class: "small muted" }, "No offer is waiting for a reply."));
    const reply = (a, r) => () => post(`/api/sim/assignments/${a.id}/reply`, { reply: r });
    return el("div", {}, head, el("ul", { class: "offers" }, ...offers.map((t) => {
      const a = t.active_assignment;
      const who = people.get(a.contributor_id) || `Contributor ${a.contributor_id}`;
      return el("li", { role: "group", "aria-label": `${who}'s reply to task ${t.id}` },
        el("p", { class: "small" }, `${who}, offered task ${t.id}: ${t.title}`),
        el("div", { class: "buttons" },
          el("button", { type: "button", "data-key": `sim-${a.id}-accept`, onclick: reply(a, "accept") }, "Accept"),
          el("button", { type: "button", "data-key": `sim-${a.id}-decline`, onclick: reply(a, "decline") }, "Decline"),
          el("button", { type: "button", "data-key": `sim-${a.id}-fail`, onclick: reply(a, "fail") }, "Not delivered")));
    })));
  }

  async function reset() {
    const raw = String(document.querySelector('[data-key="sim-seed"]').value).trim();
    if (raw !== "" && !/^\d+$/.test(raw)) { C.say("The seed must be a whole number.", "error"); return; }
    const ok = await post("/api/sim/reset", raw === "" ? {} : { seed: Number(raw) }, "Data wiped and seeded again.");
    if (!ok) return;
    // Task ids start again after a reset, so a kept selection or draft would point at a different task.
    C.clearDrafts("");
    if (location.hash) location.hash = "";
  }

  function resetBlock(snap) {
    return el("div", {}, el("h3", {}, "Seed and reset"),
      el("label", { class: "f" }, "Seed", field("input", "sim-seed",
        { type: "text", inputmode: "numeric", value: String(snap.sim.seed), autocomplete: "off" })),
      el("p", { class: "small muted" }, `Current seed: ${snap.sim.seed}. Reset wipes every task, offer and history line, then seeds again.`),
      C.confirmBlock("sim-reset", "Reset data", "Wipe all data and seed again? This cannot be undone.", "Yes, wipe and reseed", reset));
  }

  C.onRender((snap) => {
    fillCreateScenarios(snap);
    const t = snap.tasks.find((x) => x.id === C.selectedId());
    const offers = snap.tasks.filter((x) => x.active_assignment && x.active_assignment.status === "offered")
      .map((x) => [x.id, x.title, x.active_assignment]);
    // Rebuild only when the panel's content changed, so an open select or a typed seed survives.
    const next = JSON.stringify([t && [t.id, t.title, t.scenario], offers, snap.sim, snap.contributors.map((c) => c.name),
      C.confirming()]);
    if (next === sig) return;
    sig = next;
    document.getElementById("sim-body").replaceChildren(scenarioBlock(t, snap), offersBlock(snap), resetBlock(snap));
  });

  // The panel can be folded away; remember that per browser. Storage may be blocked, which is fine.
  const panel = document.getElementById("sim");
  try { if (localStorage.getItem("sim-open") === "0") panel.open = false; } catch (_) { /* default stays open */ }
  panel.addEventListener("toggle", () => {
    try { localStorage.setItem("sim-open", panel.open ? "1" : "0"); } catch (_) { /* not remembered */ }
  });
})();
