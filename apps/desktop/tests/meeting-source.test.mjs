import assert from "node:assert/strict";
import test from "node:test";
import { segmentAudioAvailable, attributionNoticeKey, attributionEditable } from "../src/meeting-source.ts";

test("durable playback uses physical source independently of changing speaker labels", () => {
  const microphone = { utterance_id: "u-microphone", source_id: "microphone", speaker: "others", voice: 2 };
  const tracks = [{ speaker: "me", source_id: "microphone", available: true }, { speaker: "others", source_id: "system", available: false }];
  assert.equal(segmentAudioAvailable(microphone, tracks), true);
  assert.equal(segmentAudioAvailable({ ...microphone, speaker: "me", voice: null }, tracks), true);
  assert.equal(segmentAudioAvailable({ ...microphone, source_id: "system" }, tracks), false);
  assert.equal(segmentAudioAvailable({ ...microphone, source_id: null }, tracks), false);
  assert.equal(segmentAudioAvailable({ speaker: "me" }, tracks), true);
  assert.equal(segmentAudioAvailable({ speaker: "others" }, tracks), false);
  assert.equal(segmentAudioAvailable(microphone, tracks, "unsupported"), false);
  assert.equal(segmentAudioAvailable({ speaker: "me" }, tracks, "stale"), false);
  assert.equal(segmentAudioAvailable({ speaker: "me" }, tracks, "absent"), true);
});

test("unavailable metadata is explained without replacing the transcript", () => {
  assert.equal(attributionNoticeKey(undefined), null);
  assert.equal(attributionNoticeKey("valid"), null);
  for (const status of ["absent", "pending", "stale", "unsupported", "corrupt"]) assert.equal(attributionNoticeKey(status), `meetings.attribution_metadata_${status}`);
  for (const status of [undefined, "valid", "absent"]) assert.equal(attributionEditable(status), true);
  for (const status of ["pending", "stale", "unsupported", "corrupt"]) assert.equal(attributionEditable(status), false);
});
