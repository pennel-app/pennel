import assert from "node:assert/strict";
import test from "node:test";
import { TaskModelDraft, TaskSetupScope, missingTaskAccess, processingLocation, taskConfiguration } from "../src/task-setup-state.ts";

function info() {
  return {
    settings: {
      providers: [{ id: "local", kind: "local", name: "Local", base_url: "" }, { id: "text", kind: "groq", name: "Groq", base_url: "https://api.groq.com/openai/v1" }],
      dictation_stt: { provider_id: "local", model: "speech" },
      meeting_stt: { provider_id: "local", model: "speech" },
      llm: { provider_id: "text", model: "text-model" },
      mode: "fast",
    },
    keys: { text: false },
    kinds: [
      { kind: "local", stt: true, llm: false, requires_key: false, is_cli_agent: false, availability: { status: "available", reason: null } },
      { kind: "groq", stt: true, llm: true, requires_key: true, is_cli_agent: false, availability: { status: "available", reason: null } },
    ],
  };
}

test("task readiness keeps speech independent of optional text processing", () => {
  const current = info();
  assert.equal(taskConfiguration(current, "dictation_stt", ["speech"]).state, "ready");
  assert.equal(taskConfiguration(current, "meeting_stt", ["speech"]).state, "ready");
  assert.equal(taskConfiguration(current, "llm", ["speech"]).state, "key");
  current.keys.text = true;
  assert.equal(taskConfiguration(current, "llm", ["speech"]).state, "ready");
});

test("a selected local model must actually be downloaded, and an unsupported task stays unavailable", () => {
  const current = info();
  assert.equal(taskConfiguration(current, "dictation_stt", []).state, "download");
  current.settings.llm = { provider_id: "local", model: "speech" };
  assert.equal(taskConfiguration(current, "llm", ["speech"]).state, "unsupported");
  current.kinds[0].availability = { status: "unsupported", reason: "Unavailable here" };
  assert.equal(taskConfiguration(current, "meeting_stt", ["speech"]).reason, "Unavailable here");
});

test("draft choices preserve saved assignments and remember each provider's model", () => {
  const current = info();
  const before = structuredClone(current.settings);
  const draft = new TaskModelDraft(current.settings.dictation_stt);
  draft.setModel("another-speech-model");
  draft.chooseProvider("text", "default-text-model");
  draft.setModel("custom-text-model");
  draft.chooseProvider("local", "default-speech-model");
  assert.deepEqual(draft.choice, { provider_id: "local", model: "another-speech-model" });
  draft.chooseProvider("text", "ignored-default");
  assert.equal(draft.choice.model, "custom-text-model");
  assert.deepEqual(current.settings, before);
  const copy = draft.choice;
  copy.model = "mutated-copy";
  assert.equal(draft.choice.model, "custom-text-model");
});

test("draft readiness checks the proposed provider and model without saving them", () => {
  const current = info();
  assert.equal(taskConfiguration(current, "dictation_stt", ["speech"], { provider_id: "text", model: "new-model" }).state, "key");
  assert.equal(taskConfiguration(current, "dictation_stt", ["speech"], { provider_id: "local", model: "not-downloaded" }).state, "download");
  assert.equal(taskConfiguration(current, "dictation_stt", ["speech"], { provider_id: "missing", model: "speech" }).state, "provider");
  assert.equal(current.settings.dictation_stt.provider_id, "local");
});

test("an unfinished server address cannot become a ready draft", () => {
  const current = info();
  current.keys.text = true;
  current.settings.providers[1].base_url = "";
  assert.equal(taskConfiguration(current, "llm", []).state, "address");
  current.settings.providers[1].base_url = "not-a-server";
  assert.equal(taskConfiguration(current, "llm", []).state, "address");
  current.settings.providers[1].base_url = "http://localhost:1234/v1";
  assert.equal(taskConfiguration(current, "llm", []).state, "ready");
});

test("processing location distinguishes local services from remote endpoints and subscriptions", () => {
  for (const kind of ["local", "apple_speech", "apple_intelligence", "ollama", "lm_studio"]) assert.equal(processingLocation(kind), "device");
  assert.equal(processingLocation("ollama", "http://127.0.0.1:11434/v1"), "device");
  assert.equal(processingLocation("lm_studio", "http://[::1]:1234/v1"), "device");
  assert.equal(processingLocation("ollama", "https://models.example/v1"), "own");
  assert.equal(processingLocation("pennel_cloud"), "cloud");
  for (const kind of ["open_ai", "open_router", "groq", "custom", "codex", "claude_code", "gemini_cli"]) assert.equal(processingLocation(kind), "own");
});

test("unknown microphone access on Windows and Linux does not block applying a speech model", () => {
  const permissions = [{ kind: "microphone", status: "unknown" }, { kind: "speech", status: "not_required" }];
  for (const task of ["dictation_stt", "meeting_stt"]) assert.equal(missingTaskAccess(task, "local", permissions), undefined);
});

test("denied microphone and unknown Apple speech access still require review", () => {
  const microphone = { kind: "microphone", status: "denied" };
  assert.equal(missingTaskAccess("dictation_stt", "local", [microphone]), microphone);
  const speech = { kind: "speech", status: "unknown" };
  assert.equal(missingTaskAccess("meeting_stt", "apple_speech", [{ kind: "microphone", status: "unknown" }, speech]), speech);
  assert.equal(missingTaskAccess("llm", "apple_intelligence", [microphone]), undefined);
});

test("setup scope ignores stale async views and keeps saves pending until settled", async () => {
  const scope = new TaskSetupScope();
  const first = scope.next();
  assert.equal(scope.current(first), true);
  const second = scope.next();
  assert.equal(scope.current(first), false);
  assert.equal(scope.current(second), true);
  let finish;
  const saving = scope.track(new Promise(resolve => { finish = resolve; }));
  assert.equal(scope.busy, true);
  let settled = false;
  void scope.settle().then(() => { settled = true; });
  await Promise.resolve();
  assert.equal(settled, false);
  finish();
  await saving;
  assert.equal(scope.busy, false);
  await scope.settle();
  assert.equal(settled, true);
  scope.dispose();
  assert.equal(scope.current(second), false);
});

test("failed saves are observable and a successful retry clears the failure", async () => {
  const scope = new TaskSetupScope();
  await assert.rejects(scope.track(Promise.reject(new Error("Cannot save"))), /Cannot save/);
  assert.match(String(scope.failure), /Cannot save/);
  await scope.track(Promise.resolve());
  assert.equal(scope.failure, null);
});

test("a flushed settings retry clears only its own failure, never a failed credential replacement", async () => {
  const scope = new TaskSetupScope();
  await assert.rejects(scope.track(Promise.reject(new Error("Settings unavailable")), "settings"));
  scope.settingsSaved();
  assert.equal(scope.failure, null);
  await assert.rejects(scope.track(Promise.reject(new Error("Credential save failed")), "mutation"));
  await assert.rejects(scope.track(Promise.reject(new Error("Settings unavailable")), "settings"));
  scope.settingsSaved();
  assert.match(String(scope.failure), /Credential save failed/);
  await scope.track(Promise.resolve(), "mutation");
  assert.equal(scope.failure, null);
});
