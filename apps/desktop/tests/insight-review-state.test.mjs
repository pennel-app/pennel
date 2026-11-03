import assert from "node:assert/strict";
import test from "node:test";
import { InsightDraft, appendInsightMarkdown, insertionStillCurrent, insightSourceMatches, sourceCandidates, safeInsightDestination } from "../../../assets/contracts/meeting-insights.ts";

const source = { meeting_uid: "meeting", utterance_id: "utterance", source_fingerprint: "a".repeat(64), quote: "café 😀", start_byte: 2, end_byte: 12, start_ms: 0, end_ms: 1000 };
const row = { utterance_id: "utterance", source_fingerprint: "a".repeat(64), text: "A café 😀", start_ms: 0, end_ms: 1000, failed: false };

test("source matching uses exact UTF-8 quote boundaries and durable identity", () => {
  assert.equal(insightSourceMatches(source, row, "meeting"), true);
  assert.equal(insightSourceMatches({ ...source, start_byte: 3 }, row, "meeting"), false);
  assert.equal(insightSourceMatches(source, { ...row, utterance_id: "later" }, "meeting"), false);
  assert.equal(insightSourceMatches(source, { ...row, text: "A cafe 😀" }, "meeting"), false);
  assert.equal(insightSourceMatches(source, row, "other"), false);
});

test("explicit reassociation offers exact quotes without guessing repeated occurrences", () => {
  const candidates = sourceCandidates(source, [{ ...row, utterance_id: "new", text: "café 😀 and café 😀" }], "meeting");
  assert.equal(candidates.length, 2);
  assert.deepEqual(candidates.map(candidate => [candidate.start_byte, candidate.end_byte]), [[0, 10], [15, 25]]);
  assert.equal(sourceCandidates(source, [{ ...row, text: "Different words" }], "meeting").length, 0);
});

test("insertion preserves exact personal notes and aborts after text, cursor or composition changes", () => {
  assert.equal(appendInsightMarkdown("Exact note  \n", "- [ ] Accepted item\n"), "Exact note  \n\n- [ ] Accepted item\n");
  const before = { text: "Exact note", revision: 1, selection_start: 3, selection_end: 4, selection_revision: 2, composing: false };
  assert.equal(insertionStillCurrent(before, { ...before }), true);
  for (const change of [{ text: "Changed" }, { revision: 2 }, { selection_start: 0 }, { selection_revision: 3 }, { composing: true }]) {
    assert.equal(insertionStillCurrent(before, { ...before, ...change }), false);
  }
});

test("asynchronous review acknowledgements preserve typing and conflicting drafts", () => {
  const draft = new InsightDraft("item", "claim", "Original", 1);
  draft.edit("First edit"); const revision = draft.local_revision;
  draft.edit("Typed while saving"); draft.acknowledge("First edit", 2, revision, true);
  assert.equal(draft.text, "Typed while saving");
  assert.equal(draft.dirty, true);
  assert.equal(draft.claim_id, null);
  draft.conflict = true;
  assert.equal(draft.text, "Typed while saving");
});

test("processing destination never exposes URL credentials, queries or deceptive loopback domains", () => {
  assert.deepEqual(safeInsightDestination("https://name:secret@api.example.test/v1?key=secret"), { host: "api.example.test", on_device: false });
  assert.equal(safeInsightDestination("http://127.example.test:8000/v1").on_device, false);
  assert.equal(safeInsightDestination("http://127.0.0.1:8000/v1").on_device, true);
});
