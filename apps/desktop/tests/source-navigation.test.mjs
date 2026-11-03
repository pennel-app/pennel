import assert from "node:assert/strict";
import test from "node:test";
import { sourceRange, sourceBlocks, transcriptSourceBlocks } from "../src/source-navigation.ts";
import { WorkspaceNavigation, parseWorkspaceRoute, workspaceRoute } from "../src/workspace-state.ts";

test("passages match full source text with whitespace normalization and original selection offsets", () => {
  const text = "  # Besluit\n\n- Het café blijft open.\n";
  const range = sourceRange(text, "# Besluit\n- Het cafe\u0301 blijft open.");
  assert.deepEqual(range, { start: 2, end: text.lastIndexOf(".") + 1 });
  assert.equal(sourceRange(text, "Het café sluit."), null);
});

test("ambiguous repeated source text is never assigned to an unrelated occurrence", () => {
  assert.equal(sourceRange("Een besluit.\nEen besluit.", "Een besluit."), null);
  assert.equal(sourceRange("Een besluit.", ""), null);
  assert.equal(sourceRange("De planning wijzigt.", "plan"), null);
  assert.deepEqual(sourceBlocks(["# Besluiten", "- We bellen morgen.", "# Acties"], "# Besluiten\n- We bellen morgen."), { first: 0, last: 1 });
});

test("transcript citations verify packed speaker-labelled text at the cited timestamp", () => {
  const segments = [
    { start_ms: 0, end_ms: 4000, text: "Noor: De planning is klaar." },
    { start_ms: 4000, end_ms: 8000, text: "Ik: Ik stuur het voorstel." },
    { start_ms: 8000, end_ms: 12000, text: "Noor: Dank je." },
  ];
  assert.deepEqual(transcriptSourceBlocks(segments, { start_ms: 0, text: "Noor: De planning is klaar.\nIk: Ik stuur het voorstel." }), { first: 0, last: 1 });
  assert.equal(transcriptSourceBlocks(segments, { start_ms: 0, text: "Noor: Dank je." }), null);
  assert.equal(transcriptSourceBlocks(segments, { start_ms: 0, text: "Noor: De planning was gisteren klaar." }), null);
  assert.equal(transcriptSourceBlocks(segments, { start_ms: 50_000, text: "Noor: Dank je." }), null);
});

test("source routes validate targets, keep their text literal and clear one-shot jumps when remembered", () => {
  const target = { kind: "notes", start_ms: null, text: "<b>Letterlijke notitie</b>" };
  const route = workspaceRoute("meetings", null, "Projectoverleg", null, target);
  assert.deepEqual(parseWorkspaceRoute(route), route);
  assert.equal(route.source_target.text, target.text);
  const nav = new WorkspaceNavigation();
  nav.set(route);
  nav.set(workspaceRoute("history"));
  assert.deepEqual(nav.select("meetings"), workspaceRoute("meetings", null, "Projectoverleg"));
  for (const invalid of [{ kind: "other", text: "x", start_ms: null }, { kind: "notes", text: "", start_ms: null }, { kind: "transcript", text: "x", start_ms: -1 }]) {
    assert.equal(parseWorkspaceRoute({ ...route, source_target: invalid }), null);
  }
  assert.equal(parseWorkspaceRoute({ ...route, section: "history" }), null);
});

test("durable transcript citations resolve exact utterances despite mutable speaker text", () => {
  const segments = [
    { utterance_id: "u-first", source_fingerprint: "a".repeat(64), start_ms: 0, end_ms: 4000, text: "New name: The exact first words." },
    { utterance_id: "u-second", source_fingerprint: "b".repeat(64), start_ms: 4000, end_ms: 8000, text: "Unknown speaker: The exact next words." },
  ];
  const source = { start_ms: 0, text: "Previous name: The exact first words.", utterance_ids: ["u-first"], utterance_fingerprints: ["a".repeat(64)] };
  assert.deepEqual(transcriptSourceBlocks(segments, source), { first: 0, last: 0 });
  assert.equal(transcriptSourceBlocks(segments, { ...source, utterance_ids: ["missing"] }), null);
  assert.equal(transcriptSourceBlocks([{ ...segments[0], source_fingerprint: "c".repeat(64) }], source), null);
  assert.equal(transcriptSourceBlocks([segments[0], segments[0]], source), null);
});

test("source routes preserve durable identity and refuse malformed identity without downgrading", () => {
  const target = { kind: "transcript", start_ms: 0, text: "Frozen source", meeting_uid: "stable-meeting",
    utterance_ids: ["u-first"], utterance_fingerprints: ["a".repeat(64)] };
  const route = workspaceRoute("meetings", null, "Projectoverleg", null, target);
  assert.deepEqual(parseWorkspaceRoute(route), route);
  for (const change of [{ utterance_ids: [] }, { utterance_ids: ["u-first", "u-first"] }, { utterance_fingerprints: [] }, { utterance_fingerprints: ["invalid"] }]) {
    assert.equal(parseWorkspaceRoute({ ...route, source_target: { ...target, ...change } }), null);
  }
});
