import assert from "node:assert/strict";
import test from "node:test";
import { setInterfaceLanguage } from "../src/i18n.ts";
import { liveQuestionMetadata } from "../src/live-question-scope.ts";

const snapshot = { request_id: "owned-question", meeting_uid: "owned-meeting", session_id: "owned-session",
  transcript_revision: 1, notes_revision: "owned-notes", included_through_ms: 100_000,
  pending_transcription: 0, anchors: [] };

test("live question metadata renders the declared selected scope in English and Dutch", () => {
  const selected = { ...snapshot, selection: { scope: "selected_excerpts", available_sources: 160, included_sources: 25,
    available_history_turns: 8, included_history_turns: 2, history_reduced: true } };
  setInterfaceLanguage("en");
  assert.equal(liveQuestionMetadata(selected, "01:40"),
    "Based on selected excerpts (25/160 sources) available through 01:40 · Earlier conversation shortened (2/8 turns).");
  setInterfaceLanguage("nl");
  assert.equal(liveQuestionMetadata(selected, "01:40"),
    "Gebaseerd op geselecteerde fragmenten (25/160 bronnen) beschikbaar tot 01:40 · Eerder gesprek ingekort (2/8 beurten).");
  setInterfaceLanguage("en");
});

test("shortened answers are disclosed even when all earlier turns fit", () => {
  setInterfaceLanguage("en");
  const whole = { ...snapshot, selection: { scope: "whole_meeting", available_sources: 2, included_sources: 2,
    available_history_turns: 6, included_history_turns: 6, history_reduced: true } };
  assert.equal(liveQuestionMetadata(whole, "01:40"),
    "Based on transcript through 01:40 · Earlier conversation shortened (6/6 turns).");
});

test("older snapshots retain cutoff and pending-speech metadata", () => {
  setInterfaceLanguage("en");
  assert.equal(liveQuestionMetadata({ ...snapshot, pending_transcription: 2 }, "01:40"),
    "Based on transcript through 01:40 · 2 speech blocks are still being processed.");
});
