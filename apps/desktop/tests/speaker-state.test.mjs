import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";
import { setInterfaceLanguage } from "../src/i18n.ts";

beforeEach(() => setInterfaceLanguage("nl"));
import { combinedProgress, enrollmentButton, enrollmentStatus, enrollmentStep, enrollmentTone, percent, recognitionSummary, speakerModelSizes, speakerModelsReady, trainedLabel, speakerProcessingNotice, voiceNamingNotice } from "../src/speaker-state.ts";

const ids = Object.keys(speakerModelSizes);
const [segmentation, embedding] = ids;
const total = speakerModelSizes[segmentation] + speakerModelSizes[embedding];

test("the two speaker models count as one download", () => {
  assert.deepEqual(combinedProgress(ids, {}, {}), { downloaded: 0, total });
  const halfway = combinedProgress(ids, { [segmentation]: { kind: "downloaded" } }, { [embedding]: { downloaded: 1_000_000, total: speakerModelSizes[embedding] } });
  assert.equal(halfway.downloaded, speakerModelSizes[segmentation] + 1_000_000);
  assert.equal(halfway.total, total);
  assert.equal(percent({ downloaded: 0, total: 0 }), 0);
  assert.equal(percent({ downloaded: total, total }), 100);
});

test("a resumed partial download counts its bytes until progress arrives", () => {
  const partial = combinedProgress(ids, { [segmentation]: { kind: "partial", bytes: 500 } }, {});
  assert.equal(partial.downloaded, 500);
});

test("speaker models are ready only when both are downloaded", () => {
  assert.equal(speakerModelsReady(ids, { [segmentation]: { kind: "downloaded" } }), false);
  assert.equal(speakerModelsReady(ids, { [segmentation]: { kind: "downloaded" }, [embedding]: { kind: "downloaded" } }), true);
  assert.equal(speakerModelsReady([], {}), false);
});

test("enrollment records, then learns, and keeps elapsed time monotonic", () => {
  const clock = (seconds) => `00:${String(seconds).padStart(2, "0")}`;
  let state = { phase: "idle" };
  assert.equal(enrollmentButton(state, clock), "Start opname");
  state = enrollmentStep(state, { kind: "start" });
  state = enrollmentStep(state, { kind: "tick", seconds: 12 });
  state = enrollmentStep(state, { kind: "tick", seconds: 11 });
  assert.deepEqual(state, { phase: "recording", seconds: 12 });
  assert.equal(enrollmentButton(state, clock), "Stop · 00:12");
  assert.deepEqual(enrollmentStep(state, { kind: "start" }), state);
  state = enrollmentStep(state, { kind: "finish" });
  assert.equal(state.phase, "working");
  assert.equal(enrollmentStep(state, { kind: "cancel" }).phase, "working");
  const profile = { created_iso: "2026-10-26T10:00:00Z", speech_seconds: 22 };
  state = enrollmentStep(state, { kind: "succeeded", profile });
  assert.deepEqual(state, { phase: "done", profile });
  assert.equal(enrollmentStatus(state), "Pennel kent je stem nu.");
  assert.equal(enrollmentTone(state), "success");
});

test("every enrollment click shows its own state before the app answers", () => {
  const clock = String;
  let state = enrollmentStep({ phase: "idle" }, { kind: "request" });
  assert.deepEqual(state, { phase: "starting" });
  assert.equal(enrollmentButton(state, clock), "Microfoon starten…");
  assert.equal(enrollmentStep(state, { kind: "request" }), state);
  state = enrollmentStep(state, { kind: "start" });
  assert.equal(state.phase, "recording");
  state = enrollmentStep(state, { kind: "finish" });
  assert.equal(enrollmentButton(state, clock), "Stem verwerken…");
  assert.equal(enrollmentStatus(state), "Pennel verwerkt je stem. Dat duurt enkele seconden.");
  assert.equal(enrollmentTone(state), "neutral");
  assert.equal(enrollmentStep({ phase: "starting" }, { kind: "cancel" }).phase, "idle");
  assert.equal(enrollmentStep({ phase: "done", profile: {} }, { kind: "request" }).phase, "starting");
});

test("an enrollment error shows the core's message and can be retried", () => {
  const failed = enrollmentStep({ phase: "working" }, { kind: "failed", message: "Pennel hoorde te weinig spraak." });
  assert.equal(enrollmentStatus(failed), "Pennel hoorde te weinig spraak.");
  assert.equal(enrollmentTone(failed), "error");
  assert.equal(enrollmentButton(failed, String), "Opnieuw proberen");
  assert.equal(enrollmentStep(failed, { kind: "start" }).phase, "recording");
  assert.equal(enrollmentStep({ phase: "idle" }, { kind: "succeeded", profile: {} }).phase, "idle");
});

test("recognition results read as one short line", () => {
  assert.equal(recognitionSummary({ voices: 3, named: 1, me_found: false }), "3 stemmen gevonden · 1 herkend");
  assert.equal(recognitionSummary({ voices: 1, named: 0, me_found: true }), "1 stem gevonden · jouw stem herkend");
  assert.equal(recognitionSummary({ voices: 0, named: 0, me_found: false }), "Geen andere stemmen gevonden");
});

test("capacity exhaustion stays visible instead of reporting ordinary success", () => {
  const result = { voices: 12, named: 0, me_found: false, complete: false, unassigned_voices: 3 };
  assert.match(recognitionSummary(result), /Sprekerlabels onvolledig: 3 stemmen zonder label/);
  setInterfaceLanguage("en");
  assert.match(recognitionSummary(result), /Speaker labels incomplete: 3 voices without a label/);
});

test("an incomplete no-speech result does not imply successful labeling", () => {
  assert.match(recognitionSummary({ voices: 0, named: 0, me_found: false, complete: false, unassigned_voices: 0 }), /Sprekerlabels onvolledig/);
});

test("saved processing failures and changed transcripts retain actionable guidance", () => {
  assert.match(speakerProcessingNotice({ phase: "failed", reason: "private provider payload" }), /opnieuw/);
  assert.doesNotMatch(speakerProcessingNotice({ phase: "failed", reason: "private provider payload" }), /private provider payload/);
  assert.match(speakerProcessingNotice({ phase: "outdated" }), /opnieuw/);
  assert.equal(speakerProcessingNotice(null), "");
});

test("low evidence remains visible separately from exhausted identity capacity", () => {
  const status = { phase: "complete", reason: "limited_evidence", complete: true, unassigned_voices: 0 };
  assert.match(speakerProcessingNotice(status), /passages.*geen sprekerlabel/);
  assert.doesNotMatch(speakerProcessingNotice(status), /stemmen zonder label/);
  assert.match(speakerProcessingNotice({ ...status, phase: "incomplete", unassigned_voices: 2 }), /2 stemmen zonder label/);
  setInterfaceLanguage("en");
  assert.match(speakerProcessingNotice(status), /short or unclear passages/);
});

test("missing and ambiguous Teams evidence never claims that names were applied", () => {
  for (const status of ["unbound", "unavailable", "insufficient_evidence", "conflict"]) {
    assert.ok(voiceNamingNotice({ status, reason: "untrusted payload" }));
    assert.doesNotMatch(voiceNamingNotice({ status }), /automatisch toegepast/);
  }
  setInterfaceLanguage("en");
  assert.match(voiceNamingNotice({ status: "applied" }), /applied automatically/);
});

test("the trained date is shown in Dutch", () => {
  assert.equal(trainedLabel(null), "Nog niet getraind");
  assert.match(trainedLabel({ created_iso: "2026-10-26T10:00:00Z", speech_seconds: 20 }), /^Getraind op 26 okt/);
  assert.equal(trainedLabel({ created_iso: "kapot", speech_seconds: 20 }), "Getraind");
});


test("speaker labels follow the interface without changing enrollment state", () => {
  const state = { phase: "recording", seconds: 12 };
  setInterfaceLanguage("en");
  assert.equal(enrollmentButton(state, (seconds) => `00:${seconds}`), "Stop · 00:12");
  assert.equal(enrollmentStatus({ phase: "done", profile: {} }), "Pennel now recognizes your voice.");
  assert.equal(recognitionSummary({ voices: 2, named: 1, me_found: true }), "2 voices found · 1 recognized · your voice recognized");
  assert.equal(trainedLabel(null), "Not trained yet");
  setInterfaceLanguage("nl");
  assert.equal(enrollmentButton(state, (seconds) => `00:${seconds}`), "Stop · 00:12");
  assert.deepEqual(state, { phase: "recording", seconds: 12 });
});
