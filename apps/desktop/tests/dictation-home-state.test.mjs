import assert from "node:assert/strict";
import test from "node:test";
import { matchingDictations, recentDictations, recommendedSpeechModel } from "../src/dictation-home-state.ts";
import { TaskSearchQueries } from "../src/workspace-search-state.ts";
import { defaultModelId } from "../src/onboarding-state.ts";

const entry = (id, time, status = "done", text = id) => ({ id, created_at_ms: time, status, text, raw: text, app_bundle_id: null, error: null, has_audio: false });

test("Recent contains the five newest actual nonempty results without failure placeholders", () => {
  const entries = [entry("failed", 12, "failed"), entry("empty", 11, "done", "  "), entry("unsaved", 10, "unsaved"), ...Array.from({length: 7}, (_, index) => entry(`result-${index}`, index))];
  assert.deepEqual(recentDictations(entries).map(item => item.id), ["unsaved", "result-6", "result-5", "result-4", "result-3"]);
  assert.equal(entries[0].id, "failed", "Reading Recent mutated history order");
  assert.deepEqual(recentDictations([]), []);
});

test("each task restores its own search without sharing a meeting query with dictation history", () => {
  const search = new TaskSearchQueries("dictation");
  search.set("dictated words");
  assert.equal(search.select("meetings"), "");
  search.set("planning meeting");
  assert.equal(search.select("dictation"), "dictated words");
  assert.equal(search.select("meetings"), "planning meeting");
  assert.deepEqual(matchingDictations([entry("first", 1, "done", "Dictated words")], "WORDS").map(item => item.id), ["first"]);
  assert.deepEqual(matchingDictations([entry("first", 1)], " "), []);
});

test("onboarding and task recovery share the recommended compatible speech choice", () => {
  const models = [{id: "legacy", recommended: true, legacy: true}, {id: "compatible", recommended: true, legacy: false}, {id: "other", recommended: false, legacy: false}];
  assert.equal(recommendedSpeechModel(models).id, "compatible");
  assert.equal(defaultModelId(models, "missing"), "compatible");
  assert.equal(defaultModelId(models, "other"), "other", "An existing user choice was replaced by a recommendation");
  assert.equal(recommendedSpeechModel([]), undefined);
});
