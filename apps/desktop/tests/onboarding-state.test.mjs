import assert from "node:assert/strict";
import test from "node:test";
import { setInterfaceLanguage } from "../src/i18n.ts";
import { initialEngine, manualPracticeAllowed, onboardingFlow, onboardingGoal, onboardingProgressChange, onboardingResumeStep, onboardingRouteIntent, onboardingSteps, resumeStep, stepIndex } from "../src/onboarding-state.ts";

const settings = { providers: [{ id: "local", kind: "local" }], dictation_stt: { provider_id: "local", model: "whisper" } };

test("unknown microphone permission permits a real manual test without global shortcut rights", () => {
  assert.equal(manualPracticeAllowed(null, [{ kind: "microphone", status: "unknown" }, { kind: "accessibility", status: "denied" }], settings), true);
  assert.equal(manualPracticeAllowed({ configured: true, can_record: true, can_use_shortcut: false }, [], settings), true);
});

test("denied microphone and unavailable chosen recognizer do not falsely pass onboarding", () => {
  assert.equal(manualPracticeAllowed(null, [{ kind: "microphone", status: "denied" }], settings), false);
  assert.equal(manualPracticeAllowed({ configured: false, can_record: false }, [], settings), false);
});

test("Apple speech requires its own authorization, unlike local speech", () => {
  const apple = { providers: [{ id: "apple", kind: "apple_speech" }], dictation_stt: { provider_id: "apple", model: "dictation" } };
  assert.equal(manualPracticeAllowed(null, [{ kind: "microphone", status: "granted" }, { kind: "speech", status: "denied" }], apple), false);
  assert.equal(manualPracticeAllowed(null, [{ kind: "microphone", status: "granted" }, { kind: "speech", status: "granted" }], apple), true);
});

test("legacy named language and voice steps keep their meaning", () => {
  assert.deepEqual(onboardingSteps, ["language", "welcome", "engine", "permissions", "practice", "voice"]);
  assert.equal(stepIndex("welcome"), 1);
  assert.equal(stepIndex("practice"), 4);
  assert.equal(stepIndex("old-step"), 0);
});

test("new first use starts with a task choice and does not require optional voice enrollment", () => {
  assert.deepEqual(onboardingFlow(), ["welcome", "engine", "permissions", "practice"]);
  assert.deepEqual(onboardingFlow("language"), ["language", "welcome", "engine", "permissions", "practice"]);
  assert.deepEqual(onboardingFlow("voice"), ["welcome", "engine", "permissions", "practice", "voice"]);
});

test("old progress defaults to dictation while a selected meeting goal survives progress updates", () => {
  assert.equal(onboardingGoal({ version: 1, status: "pending", resume_step: "engine" }), "dictation");
  const meeting = { version: 1, status: "pending", resume_step: "welcome", goal: "meeting" };
  assert.deepEqual(onboardingProgressChange(meeting, "engine"), { ...meeting, resume_step: "engine" });
  assert.deepEqual(onboardingProgressChange(meeting, "permissions", undefined, "dictation"), { ...meeting, resume_step: "permissions", goal: "dictation" });
});

test("explicit dictation setup and permission repair ignore the saved meeting goal without rewriting it", () => {
  const saved = { version: 1, status: "completed", resume_step: "practice", goal: "meeting" };
  const before = structuredClone(saved);
  for (const route of ["practice", "dictation-setup", "dictation-permissions"]) {
    const intent = onboardingRouteIntent(route, saved);
    assert.equal(intent.goal, "dictation");
    assert.equal(intent.preserveProgress, true);
    assert.deepEqual(saved, before);
  }
  assert.deepEqual(onboardingRouteIntent("resume", saved), { goal: "meeting", preserveProgress: false, step: "resume" });
  assert.equal(onboardingRouteIntent("permissions", saved).goal, "meeting");
});

test("dictation access repair opens relevant permissions but repairs a missing recognizer first", () => {
  const saved = { version: 1, status: "completed", resume_step: "voice", goal: "meeting" };
  const permissions = onboardingRouteIntent("dictation-permissions", saved);
  assert.equal(onboardingResumeStep({ configured: true, resume_step: "practice" }, saved.resume_step, permissions), "permissions");
  assert.equal(onboardingResumeStep({ configured: false, resume_step: "engine" }, saved.resume_step, permissions), "engine");
  assert.equal(onboardingResumeStep({ configured: true, resume_step: "permissions" }, saved.resume_step, permissions), "permissions");
  assert.equal(onboardingResumeStep({ configured: true, resume_step: "practice" }, saved.resume_step, onboardingRouteIntent("dictation-setup", saved)), "practice");
  assert.equal(onboardingResumeStep({ configured: true, resume_step: "practice" }, saved.resume_step, onboardingRouteIntent("resume", saved)), "voice");
});

test("a successful initial practice snapshot completes once without an identical persistence loop", () => {
  const pending = { version: 1, status: "pending", resume_step: "practice" };
  const completed = onboardingProgressChange(pending, "practice", "completed");
  assert.deepEqual(completed, { version: 1, status: "completed", resume_step: "practice" });
  assert.equal(onboardingProgressChange(completed, "practice", "completed"), null);
  assert.equal(onboardingProgressChange(completed, "practice"), null);
});

test("replaying or skipping a completed wizard never downgrades completion", () => {
  const completed = { version: 1, status: "completed", resume_step: "practice" };
  assert.deepEqual(onboardingProgressChange(completed, "engine", "skipped"), { version: 1, status: "completed", resume_step: "engine" });
});

test("model chooser keeps a valid selection and defaults to the recommended model", async () => {
  const { defaultModelId, modelOptionLabel } = await import("../src/onboarding-state.ts");
  const models = [{ id: "a", name: "Klein", size_bytes: 150e6, recommended: false }, { id: "b", name: "Groot", size_bytes: 1.2e9, recommended: true }];
  assert.equal(defaultModelId(models, "a"), "a");
  assert.equal(defaultModelId(models, "zzz"), "b");
  assert.equal(defaultModelId([], "x"), "x");
  const size = (bytes) => `${bytes / 1e6} MB`;
  setInterfaceLanguage("nl");
  assert.equal(modelOptionLabel(models[1], size), "Groot · 1200 MB · Aanbevolen");
  setInterfaceLanguage("en");
  assert.equal(modelOptionLabel(models[1], size), "Groot · 1200 MB · Recommended");
  assert.equal(defaultModelId(models, "a"), "a");
  assert.equal(modelOptionLabel(models[0], size), "Klein · 150 MB");
});

test("new model defaults use the same compatible recommendation as Settings while saved choices stay intact", async () => {
  const { defaultModelId } = await import("../src/onboarding-state.ts");
  const models = [{ id: "unsupported", recommended: true }, { id: "compatible", recommended: true }];
  assert.equal(defaultModelId(models, "missing", [models[1]]), "compatible");
  assert.equal(defaultModelId(models, "unsupported", [models[1]]), "unsupported");
  assert.equal(defaultModelId(models, "missing", []), "missing");
});

test("saved optional voice enrollment remains addressable after practice", () => {
  assert.deepEqual(onboardingSteps.slice(-2), ["practice", "voice"]);
  assert.equal(stepIndex("voice"), 5);
});

test("meeting readiness fallback inspects the selected meeting recognizer", () => {
  const meeting = { ...settings, providers: [...settings.providers, { id: "apple", kind: "apple_speech" }], meeting_stt: { provider_id: "apple", model: "dictation" } };
  const permissions = [{ kind: "microphone", status: "granted" }, { kind: "speech", status: "denied" }];
  assert.equal(manualPracticeAllowed(null, permissions, meeting, "dictation"), true);
  assert.equal(manualPracticeAllowed(null, permissions, meeting, "meeting"), false);
});

test("resume returns to the voice step once the practice is done", () => {
  assert.equal(resumeStep("practice", "voice"), "voice");
  assert.equal(resumeStep("practice", "practice"), "practice");
  assert.equal(resumeStep("engine", "voice"), "engine");
  assert.equal(resumeStep("permissions", undefined), "permissions");
});

test("finishing or skipping the voice step completes the onboarding", () => {
  const pending = { version: 1, status: "pending", resume_step: "voice" };
  assert.deepEqual(onboardingProgressChange(pending, "voice", "completed"), { version: 1, status: "completed", resume_step: "voice" });
  assert.deepEqual(onboardingProgressChange({ ...pending, resume_step: "practice" }, "voice"), pending);
});

test("a processor too old for local models starts onboarding on a provider", () => {
  assert.equal(initialEngine(undefined, true), "local");
  assert.equal(initialEngine("local", true), "local");
  assert.equal(initialEngine(undefined, false), "custom");
  assert.equal(initialEngine("local", false), "custom");
  assert.equal(initialEngine("apple_speech", false), "apple");
  assert.equal(initialEngine("open_ai", true), "custom");
});

test("language step persistence preserves completed status without redoing setup", () => {
  const current = { version: 1, status: "completed", resume_step: "voice" };
  assert.deepEqual(onboardingProgressChange(current, "language"), { version: 1, status: "completed", resume_step: "language" });
  assert.equal(resumeStep("engine", "language"), "engine");
});
