import assert from "node:assert/strict";
import test from "node:test";
import { LiveSpeakerControl, liveSpeakerStatusKey } from "../src/live-speaker-state.ts";

const status = (revision = 1, changes = {}) => ({ revision, enabled: false, available: true, state: "off", reason_code: null,
  model: "Owned segmentation + embedding models", destination: "local", ...changes });

test("live attribution defaults off for each meeting and never shares the previous session choice", () => {
  const first = new LiveSpeakerControl("first", status());
  assert.equal(first.checked, false);
  const request = first.begin(true);
  assert.ok(request);
  assert.equal(first.complete(request, status(2, { enabled: true, state: "running" })), true);
  assert.equal(first.checked, true);
  const next = new LiveSpeakerControl("next", status());
  assert.equal(next.checked, false);
  assert.equal(new LiveSpeakerControl("older-backend").checked, false);
});

test("a pending toggle prevents duplicate requests and changes the displayed choice only after acknowledgement", () => {
  const control = new LiveSpeakerControl("session", status());
  const request = control.begin(true);
  assert.equal(control.checked, false);
  assert.equal(control.pending, true);
  assert.equal(control.begin(true), null);
  assert.equal(control.complete(request, status(2, { enabled: true, state: "warming" })), true);
  assert.equal(control.checked, true);
  assert.equal(control.pending, false);
});

test("stale status revisions, foreign sessions and late toggle acknowledgements cannot revive stopped work", () => {
  const control = new LiveSpeakerControl("session", status(5, { enabled: true, state: "running" }));
  assert.equal(control.observe("other", status(6)), false);
  assert.equal(control.observe("session", status(4)), false);
  assert.equal(control.checked, true);
  const request = control.begin(true);
  control.stop();
  assert.equal(control.checked, false);
  assert.equal(control.complete(request, status(7, { enabled: true, state: "running" })), false);
  assert.equal(control.observe("session", status(8, { enabled: true, state: "running" })), false);
  assert.equal(control.canChange(true), false);
});

test("unavailable setups cannot be enabled while enabled, failed or paused work can always be disabled", () => {
  for (const reason_code of ["local_required", "models_missing", "unsupported", "configuration_changed"]) {
    const control = new LiveSpeakerControl("session", status(1, { available: false, state: "unavailable", reason_code }));
    assert.equal(control.begin(true), null);
    assert.equal(liveSpeakerStatusKey(control.value), `meetings.live_speakers_${reason_code}`);
  }
  for (const state of ["busy", "paused", "failed"]) {
    const control = new LiveSpeakerControl("session", status(1, { enabled: true, available: false, state }));
    assert.ok(control.begin(true));
  }
});

test("failed toggles retain the acknowledged choice and a new explicit request clears the error", () => {
  const control = new LiveSpeakerControl("session", status());
  const request = control.begin(true);
  assert.equal(control.fail(request, "Owned failure"), true);
  assert.equal(control.checked, false);
  assert.equal(control.error, "Owned failure");
  assert.ok(control.begin(true));
  assert.equal(control.error, null);
});

test("a newer busy snapshot wins over an older toggle acknowledgement", () => {
  const control = new LiveSpeakerControl("session", status());
  const request = control.begin(true);
  control.observe("session", status(5, { state: "busy", reason_code: "busy" }));
  assert.equal(control.complete(request, status(2, { enabled: true, state: "running" })), true);
  assert.equal(control.value.revision, 5);
  assert.equal(control.checked, false);
  assert.equal(control.canChange(true), false);
});

test("foreground speech admission and unknown backend reasons have useful localized states", () => {
  assert.equal(liveSpeakerStatusKey(status(1, { state: "paused", reason_code: "foreground_work" })), "meetings.live_speakers_foreground_work");
  assert.equal(liveSpeakerStatusKey(status(1, { state: "unavailable", reason_code: "future_reason" })), "meetings.live_speakers_unavailable");
});
