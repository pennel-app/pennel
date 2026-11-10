import assert from "node:assert/strict";
import test from "node:test";
import { DictationSessionState, HomePracticeDraft, OwnedSessionLifecycle, PadDraftState } from "../src/dictation-session-state.ts";

const snapshot = (id, kind = "practice", phase = "idle") => ({ session_id: id, kind, phase, text: "", error: null, target_name: null, external_paste_available: false, external_paste_reason: null });

test("home practice appends each completed capture once while preserving editable text and recoverable results", () => {
  const draft = new HomePracticeDraft();
  draft.value = "My message:";
  draft.accept(snapshot("home", "home_practice", "listening"));
  draft.accept({ ...snapshot("home", "home_practice", "done"), text: "Hello." });
  assert.equal(draft.value, "My message: Hello.");
  draft.value += " Edited.";
  draft.accept({ ...snapshot("home", "home_practice", "done"), text: "Hello." });
  assert.equal(draft.value, "My message: Hello. Edited.");
  draft.accept({ ...snapshot("home", "home_practice", "listening"), text: "Hello." });
  draft.accept({ ...snapshot("home", "home_practice", "error"), text: "Hello.", error: "No speech" });
  assert.equal(draft.value, "My message: Hello. Edited.");
  draft.accept({ ...snapshot("home", "home_practice", "listening"), text: "Hello." });
  draft.accept({ ...snapshot("home", "home_practice", "error"), text: "Recovered.", error: "Saving failed" });
  assert.equal(draft.value, "My message: Hello. Edited. Recovered.");
});

test("home practice disposal rejects late events and busy leave preserves its owner", async () => {
  let emit; let phase = "listening"; const closed = [];
  const driver = {
    open: async kind => snapshot("home", kind, phase), get: async () => snapshot("home", "home_practice", phase),
    toggle: async () => {}, cancel: async () => {}, close: async id => { closed.push(id); },
    subscribe: async listener => { emit = listener; return () => {}; },
  };
  const updates = [];
  const lifecycle = new OwnedSessionLifecycle("home_practice", value => updates.push(value.phase), driver);
  await lifecycle.open();
  assert.equal(await lifecycle.leave(), false);
  assert.deepEqual(closed, []);
  assert.equal(lifecycle.state.accept(snapshot("home", "practice", "done")), false);
  await lifecycle.close();
  emit(snapshot("home", "home_practice", "done"));
  assert.deepEqual(updates, ["listening"]);
  assert.deepEqual(closed, ["home"]);
});

test("home practice retains completion even when an opening race delivers only the final snapshot", () => {
  const draft = new HomePracticeDraft();
  const done = { ...snapshot("home", "home_practice", "done"), text: "Fast result." };
  draft.accept(done);
  draft.accept(done);
  assert.equal(draft.value, "Fast result.");
  draft.accept({ ...snapshot("home", "home_practice", "listening"), text: "Fast result." });
  draft.accept(done);
  assert.equal(draft.value, "Fast result. Fast result.");
});

test("late results from a closed or replaced owned session are rejected", () => {
  const state = new DictationSessionState();
  state.bind(snapshot("old"));
  assert.equal(state.close(), "old");
  assert.equal(state.accept(snapshot("old", "practice", "done")), false);
  state.bind(snapshot("new"));
  assert.equal(state.accept(snapshot("old", "practice", "done")), false);
  assert.equal(state.snapshot.session_id, "new");
});

test("pad and practice results cannot cross ownership and busy phases are explicit", () => {
  const state = new DictationSessionState();
  state.bind(snapshot("owned", "pad"));
  assert.equal(state.accept(snapshot("owned", "practice", "done")), false);
  assert.equal(state.accept(snapshot("owned", "pad", "listening")), true);
  assert.equal(state.busy, true);
  state.accept(snapshot("owned", "pad", "working"));
  assert.equal(state.busy, true);
  state.accept(snapshot("owned", "pad", "done"));
  assert.equal(state.busy, false);
});

test("manual edits survive in-flight refreshes and saved text survives session refresh", () => {
  const draft = new PadDraftState();
  draft.hydrate("Bewaarde tekst");
  draft.edit("Mijn eigen wijziging");
  assert.equal(draft.accept({ phase: "idle", text: "Bewaarde tekst" }), false);
  assert.equal(draft.value, "Mijn eigen wijziging");
  draft.acknowledge("Mijn eigen wijziging");
  draft.accept({ phase: "idle", text: "Mijn eigen wijziging" });
  assert.equal(draft.value, "Mijn eigen wijziging");
  assert.equal(draft.dirty, false);
});

test("recognized text retained after a storage error becomes an unsaved client draft", () => {
  const draft = new PadDraftState();
  draft.hydrate("Eerste zin.");
  draft.accept({ phase: "error", text: "Eerste zin.\nNieuwe woorden." });
  assert.equal(draft.value, "Eerste zin.\nNieuwe woorden.");
  assert.equal(draft.dirty, true);
  assert.equal(draft.accept({ phase: "idle", text: "Eerste zin." }), false);
  draft.acknowledge("Eerste zin.\nNieuwe woorden.");
  assert.equal(draft.dirty, false);
});

test("reopening a retained unsaved backend pad still requires a successful save", () => {
  const draft = new PadDraftState();
  draft.hydrate("Herstelde, nog niet bewaarde tekst.");
  draft.accept({ phase: "error", text: "Herstelde, nog niet bewaarde tekst." });
  assert.equal(draft.dirty, true);
});

test("closing during an open and immediately reopening binds a fresh replacement session", async () => {
  let resolveFirst;
  let opens = 0;
  const closed = [];
  const updates = [];
  const first = new Promise((resolve) => { resolveFirst = resolve; });
  const driver = {
    open: async () => { opens += 1; return opens === 1 ? first : snapshot("replacement"); },
    get: async () => snapshot("replacement"),
    toggle: async () => {}, cancel: async () => {},
    close: async (id) => { closed.push(id); },
    subscribe: async () => () => {},
  };
  const lifecycle = new OwnedSessionLifecycle("practice", (value) => updates.push(value.session_id), driver);
  const opening = lifecycle.open();
  await Promise.resolve();
  await lifecycle.close();
  const reopening = lifecycle.open();
  resolveFirst(snapshot("stale"));
  await Promise.all([opening, reopening]);
  assert.equal(opens, 2);
  assert.deepEqual(closed, ["stale"]);
  assert.deepEqual(updates, ["replacement"]);
  assert.equal(lifecycle.state.snapshot.session_id, "replacement");
});

test("a delayed state request cannot regress a completed event to an older busy phase", async () => {
  let emit;
  let resolveRefresh;
  const delayed = new Promise((resolve) => { resolveRefresh = resolve; });
  const driver = {
    open: async () => snapshot("owned"), get: async () => delayed,
    toggle: async () => {}, cancel: async () => {}, close: async () => {},
    subscribe: async (listener) => { emit = listener; return () => {}; },
  };
  const lifecycle = new OwnedSessionLifecycle("practice", () => {}, driver);
  await lifecycle.open();
  const reading = lifecycle.refresh();
  emit({ ...snapshot("owned", "practice", "done"), text: "Klaar" });
  resolveRefresh(snapshot("owned", "practice", "working"));
  await reading;
  assert.equal(lifecycle.state.snapshot.phase, "done");
  assert.equal(lifecycle.state.snapshot.text, "Klaar");
});

test("a get started before a local edit/save cannot overwrite the acknowledged pad text", async () => {
  let resolveRefresh;
  const delayed = new Promise((resolve) => { resolveRefresh = resolve; });
  const stored = { ...snapshot("pad", "pad"), text: "Oude tekst" };
  const draft = new PadDraftState();
  const driver = {
    open: async () => stored, get: async () => delayed,
    toggle: async () => {}, cancel: async () => {}, close: async () => {},
    subscribe: async () => () => {},
  };
  const lifecycle = new OwnedSessionLifecycle("pad", (value) => draft.accept(value), driver);
  await lifecycle.open();
  const reading = lifecycle.refresh();
  lifecycle.invalidateRefresh();
  draft.edit("Mijn bewaarde wijziging");
  lifecycle.acceptLocalSavedText("Mijn bewaarde wijziging");
  draft.acknowledge("Mijn bewaarde wijziging");
  resolveRefresh(stored);
  await reading;
  assert.equal(lifecycle.state.snapshot.text, "Mijn bewaarde wijziging");
  assert.equal(draft.value, "Mijn bewaarde wijziging");
  assert.equal(draft.dirty, false);
});

test("a hidden pad rebinds once when reopened and rejects results from its closed session", async () => {
  const listeners = [];
  const updates = [];
  let opens = 0;
  const closed = [];
  const driver = {
    open: async () => snapshot(`pad-${++opens}`, "pad"),
    get: async (id) => snapshot(id, "pad"),
    toggle: async () => {}, cancel: async () => {},
    close: async (id) => { closed.push(id); },
    subscribe: async (listener) => { listeners.push(listener); return () => {}; },
  };
  const lifecycle = new OwnedSessionLifecycle("pad", (value) => updates.push(value.session_id), driver);
  await lifecycle.open();
  await lifecycle.close();
  await Promise.all([lifecycle.open(), lifecycle.open()]);
  listeners[0]({ ...snapshot("pad-1", "pad", "done"), text: "Te laat" });
  assert.equal(opens, 2);
  assert.deepEqual(closed, ["pad-1"]);
  assert.deepEqual(updates, ["pad-1", "pad-2"]);
  assert.equal(lifecycle.state.snapshot.session_id, "pad-2");
});

test("leaving a busy pad keeps its capture and listener owned until explicit finish or cancellation", async () => {
  let emit;
  const closed = [];
  const driver = {
    open: async () => snapshot("pad", "pad", "listening"),
    get: async () => snapshot("pad", "pad", "listening"),
    toggle: async () => {}, cancel: async () => {},
    close: async id => { closed.push(id); },
    leave: async id => { closed.push(`leave:${id}`); },
    subscribe: async listener => { emit = listener; return () => {}; },
  };
  const lifecycle = new OwnedSessionLifecycle("pad", () => {}, driver);
  await lifecycle.open();
  assert.equal(await lifecycle.leave(), false);
  assert.deepEqual(closed, []);
  emit({ ...snapshot("pad", "pad", "done"), text: "Preserved dictation" });
  assert.equal(lifecycle.state.snapshot.text, "Preserved dictation");
  assert.equal(await lifecycle.leave(), true);
  assert.deepEqual(closed, ["leave:pad"]);
  assert.equal(lifecycle.state.snapshot, null);
});

test("a native capture beginning during leave is not closed or detached", async () => {
  let emit;
  let current = snapshot("pad", "pad");
  let unsubscribed = false;
  const driver = {
    open: async () => current, get: async () => current,
    toggle: async () => {}, cancel: async () => {}, close: async () => {},
    leave: async () => { current = snapshot("pad", "pad", "listening"); emit(current); throw new Error("Busy"); },
    subscribe: async listener => { emit = listener; return () => { unsubscribed = true; }; },
  };
  const lifecycle = new OwnedSessionLifecycle("pad", () => {}, driver);
  await lifecycle.open();
  assert.equal(await lifecycle.leave(), false);
  assert.equal(lifecycle.state.snapshot.phase, "listening");
  assert.equal(unsubscribed, false);
});

test("failed idle leave preserves the session and can be retried without opening or closing another owner", async () => {
  let fail = true;
  let opens = 0;
  const driver = {
    open: async () => { opens++; return snapshot("pad", "pad"); }, get: async () => snapshot("pad", "pad"),
    toggle: async () => {}, cancel: async () => {}, close: async () => { throw new Error("Wrong close policy"); },
    leave: async () => { if (fail) throw new Error("Transport failed"); }, subscribe: async () => () => {},
  };
  const lifecycle = new OwnedSessionLifecycle("pad", () => {}, driver);
  await lifecycle.open();
  await assert.rejects(lifecycle.leave(), /Transport failed/);
  assert.equal(lifecycle.state.snapshot.session_id, "pad");
  fail = false;
  assert.equal(await lifecycle.leave(), true);
  await lifecycle.close();
  assert.equal(opens, 1);
});
