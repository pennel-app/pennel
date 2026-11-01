import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { visualModels } from "./model-catalog.mjs";

const script = await readFile(new URL("./desktop-platform.js", import.meta.url), "utf8");
const cases = JSON.parse(await readFile(new URL("./visual-cases.json", import.meta.url), "utf8"));
function fixture(name, overrides = {}, storage) {
  const item = cases.find(item => item.name === name);
  const context = vm.createContext({ location: { hostname: "127.0.0.1" }, structuredClone, queueMicrotask, localStorage: storage,
    __PENNEL_FIXTURE_OPTIONS__: { ...item, appearance: "dark", ...overrides } });
  vm.runInContext(script, context);
  return context;
}

test("every visual scenario has a unique stable identifier and valid entry", () => {
  assert.equal(new Set(cases.map(item => item.name)).size, cases.length);
  for (const item of cases) {
    assert.match(item.name, /^[a-z][a-z0-9-]+$/);
    assert.ok(["workspace", "settings", "history", "dictation", "onboarding", "pill"].includes(item.window));
    if (item.window === "pill") assert.ok(["listening", "working"].includes(item.pill_state));
    else assert.ok(item.section || item.step);
  }
});

test("saved-audio retranscription reports local progress and marks the summary outdated", async () => {
  const context=fixture("meeting-transcript",{saved_audio:true});
  const client=context.__PENNEL_PLATFORM__;
  const detail=await client.invoke("meeting_detail",{id:"2026-10-24 0900 Projectoverleg"});
  assert.equal(detail.retranscription_plan.available,true);
  assert.equal(detail.retranscription_plan.recognition.provider_kind,"local");
  const events=[];
  await client.listen("meeting-retranscription-changed",event=>events.push(event.payload));
  const count=await client.invoke("retranscribe_meeting_audio",{id:detail.info.id});
  assert.ok(count>0);
  assert.ok(events.some(event=>event.status.running));
  const after=await client.invoke("meeting_detail",{id:detail.info.id});
  assert.equal(after.summary_outdated,true);
  assert.equal(after.retranscription_status.running,false);
  assert.equal(after.retranscription_status.progress.phase,"completed");
  assert.equal(after.notes,detail.notes);
});

test("fixture app information is synthetic and never returns an API key", async () => {
  const context = fixture("onboarding-engine-custom");
  const info = await context.__PENNEL_PLATFORM__.invoke("app_info");
  assert.equal(info.settings.appearance, "dark");
  assert.equal(info.settings.dictation_stt.provider_id, "openai");
  assert.equal(info.settings.mode, "fast");
  assert.equal(info.preview, true);
  assert.equal(info.capabilities.local_models.status, "available");
  assert.ok(Object.values(info.keys).every(value => typeof value === "boolean"));
  await context.__PENNEL_PLATFORM__.invoke("set_key", { account: "openai", key: "synthetic-only" });
  assert.equal(JSON.stringify(await context.__PENNEL_PLATFORM__.invoke("app_info")).includes("synthetic-only"), false);
});

test("template fixtures use canonical English definitions without translating meeting content", async () => {
  const api = fixture("meeting-notes").__PENNEL_PLATFORM__;
  const templates = await api.invoke("meeting_templates");
  assert.deepEqual(Array.from(templates, value => value.name), ["General", "One-to-one", "Client meeting", "Standup"]);
  assert.match(await api.invoke("template_source", { id: "algemeen" }), /## Summary\n## Decisions\n## Action items/);
  const detail = await api.invoke("meeting_detail", { id: "2026-10-24 0900 Projectoverleg" });
  assert.equal(detail.notes, "- Planning afronden\n- Volgende afspraak bevestigen");
});

test("first-use fixtures preserve the chosen task without requiring voice enrollment", async () => {
  const meeting = fixture("onboarding-welcome", { goal: "meeting" });
  const api = meeting.__PENNEL_PLATFORM__;
  const info = await api.invoke("app_info");
  assert.equal(info.settings.onboarding.goal, "meeting");
  const next = structuredClone(info.settings);
  next.onboarding.resume_step = "permissions";
  await api.invoke("save_settings", { settings: next });
  assert.equal((await api.invoke("app_info")).settings.onboarding.goal, "meeting");
});

test("the task-choice fixture IPC updates one slot while preserving current preferences and unfinished choices", async () => {
  const context = fixture("guided-setup-draft-interaction", {
    model_choices: {
      dictation_stt: { provider_id: "local", model: "synthetic-missing-dictation-model" },
      meeting_stt: { provider_id: "local", model: "synthetic-missing-meeting-model" },
    },
  });
  const api = context.__PENNEL_PLATFORM__;
  const latest = (await api.invoke("app_info")).settings;
  latest.language = "en";
  latest.history_retention_days = 7;
  await api.invoke("save_settings", { settings: latest });
  const after = await api.invoke("apply_task_model_choice", { slot: "llm", choice: { provider_id: "groq", model: "  synthetic-draft-model  " } });
  assert.deepEqual(after.settings.llm, { provider_id: "groq", model: "synthetic-draft-model" });
  assert.deepEqual(after.settings.dictation_stt, latest.dictation_stt);
  assert.deepEqual(after.settings.meeting_stt, latest.meeting_stt);
  assert.equal(after.settings.language, "en");
  assert.equal(after.settings.history_retention_days, 7);
  assert.equal(context.__PENNEL_FIXTURE__.setupTrace.appliedTasks.length, 1);
  assert.equal(context.__PENNEL_FIXTURE__.setupTrace.savedChoices.length, 2);
  assert.deepEqual(Object.keys(context.__PENNEL_FIXTURE__.setupTrace.appliedTasks[0]).sort(), ["choice", "slot"]);
});

test("task-choice fixture validation rejects invalid slots, providers, tasks and undownloaded local models without writes", async () => {
  const context = fixture("guided-setup-draft-interaction");
  const api = context.__PENNEL_PLATFORM__;
  const before = JSON.stringify((await api.invoke("app_info")).settings);
  for (const request of [
    { slot: "unknown", choice: { provider_id: "groq", model: "synthetic" } },
    { slot: "llm", choice: { provider_id: "missing", model: "synthetic" } },
    { slot: "llm", choice: { provider_id: "local", model: "whisper-large-v3-turbo" } },
    { slot: "llm", choice: { provider_id: "groq", model: "  " } },
    { slot: "dictation_stt", choice: { provider_id: "local", model: "synthetic-not-downloaded" } },
    { slot: "meeting_stt", choice: { provider_id: "local", model: "diar_streaming_sortformer_4spk-v2.1" } },
  ]) {
    await assert.rejects(api.invoke("apply_task_model_choice", request));
    assert.equal(JSON.stringify((await api.invoke("app_info")).settings), before);
  }
  assert.equal(context.__PENNEL_FIXTURE__.setupTrace.appliedTasks.length, 0);
  assert.equal(context.__PENNEL_FIXTURE__.setupTrace.savedChoices.length, 0);
});

test("Windows and Linux fixtures retain native unknown microphone state after an ineffective request", async () => {
  for (const platform of ["windows", "linux"]) {
    const api = fixture("settings-models", { platform, permission: "unknown" }).__PENNEL_PLATFORM__;
    assert.equal((await api.invoke("app_info")).platform, platform);
    const permissions = await api.invoke("request_permission", { kind: "microphone" });
    assert.equal(permissions.find(permission => permission.kind === "microphone").status, "unknown");
    assert.equal((await api.invoke("onboarding_readiness", { target: "meeting" })).can_record, true);
  }
});

test("pad/practice sessions emit to the fixture listener and never invoke native transport", async () => {
  const context = fixture("dictation-pad");
  const api = context.__PENNEL_PLATFORM__;
  const events = [];
  await api.listen("dictation-session", event => events.push(event.payload));
  const session = await api.invoke("open_dictation_session", { kind: "pad" });
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  assert.equal(events[0].phase, "listening");
  assert.equal(events[1].phase, "done");
  assert.match(await api.invoke("pad_text"), /Zullen we morgen/);
  assert.equal(context.__PENNEL_FIXTURE__.unknown.length, 0);
});

test("denied and empty states are explicit and unknown commands fail closed", async () => {
  const denied = fixture("onboarding-permission-denied");
  const readiness = await denied.__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "practice" });
  assert.equal(readiness.can_record, false);
  const info = await denied.__PENNEL_PLATFORM__.invoke("app_info");
  assert.equal(info.capabilities.global_hotkey.status, "permission_required");
  assert.match(info.capabilities.global_hotkey.reason, /fn in andere apps/);
  const empty = fixture("meetings-empty");
  assert.equal((await empty.__PENNEL_PLATFORM__.invoke("meetings")).length, 0);
  await assert.rejects(() => empty.__PENNEL_PLATFORM__.invoke("unexpected_real_service"), /Onbekend fixturecommando/);
});

test("visual model metadata uses the bundled core catalog rather than invented scores", async () => {
  const catalog = JSON.parse(await readFile(new URL("../../core/src/catalog.json", import.meta.url), "utf8"));
  const source = await readFile(new URL("../../core/src/catalog.rs", import.meta.url), "utf8");
  const models = visualModels(catalog, source);
  assert.ok(models.length > 20);
  assert.equal(models[0].id, "parakeet-tdt-0.6b-v3");
  const whisper = models.find(model => model.id === "whisper-large-v3-turbo");
  assert.equal(whisper.size_bytes, 886381760);
  assert.equal(whisper.speed, 32);
  assert.equal(whisper.accuracy, 67);
  assert.ok(models.every(model => model.speed >= 0 && model.speed <= 100 && (model.accuracy === null || (model.accuracy >= 0 && model.accuracy <= 100))));
});

test("missing global shortcut permission does not block an existing user's manual pad", async () => {
  const context = fixture("dictation-pad-shortcut-denied");
  const info = await context.__PENNEL_PLATFORM__.invoke("app_info");
  const readiness = await context.__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "dictation_pad" });
  assert.equal(info.settings.onboarding.status, "skipped");
  assert.equal(info.capabilities.microphone.status, "available");
  assert.equal(readiness.can_record, true);
  assert.equal(readiness.can_use_shortcut, false);
  assert.equal(readiness.can_paste_externally, false);
  await context.__PENNEL_PLATFORM__.invoke("request_permission", { kind: "accessibility" });
  assert.equal((await context.__PENNEL_PLATFORM__.invoke("app_info")).hotkey_error, null);
  assert.equal((await context.__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "dictation_pad" })).can_use_shortcut, true);
});

test("the optional pad has an explicit route separate from the direct-dictation home", () => {
  const home = cases.find(item => item.name === "dictation");
  const pad = cases.find(item => item.name === "dictation-pad");
  const deniedPad = cases.find(item => item.name === "dictation-pad-shortcut-denied");
  assert.equal(home.tab, undefined);
  assert.equal(pad.section, home.section);
  assert.equal(pad.tab, "pad");
  assert.equal(deniedPad.tab, "pad");
  assert.equal(deniedPad.shortcut_denied, true);
});

test("pill snapshots provide only synthetic display text with no capture or transport calls", async () => {
  const captions = cases.filter(item => item.window === "pill");
  assert.equal(captions.length, 4);
  for (const item of captions) {
    const context = fixture(item.name);
    const snapshot = await context.__PENNEL_PLATFORM__.invoke("dictation_state");
    assert.equal(snapshot.state, item.pill_state);
    assert.equal(snapshot.preview.session_id, "qa-" + item.name);
    assert.equal(snapshot.preview.text, item.preview_text || "");
    assert.equal(snapshot.preview.revision, item.preview_text ? 7 : 0);
    assert.deepEqual(Array.from(context.__PENNEL_FIXTURE__.calls), ["dictation_state"]);
    assert.equal(context.__PENNEL_FIXTURE__.unknown.length, 0);
  }
  const long = cases.find(item => item.name === "pill-long-preview");
  assert.match(long.preview_text, /<b>blijft letterlijk<\/b>/);
  assert.match(long.preview_text, /日本語/);
  assert.match(long.preview_text, /👨‍👩‍👧‍👦/);
});

test("cloud scenarios are synthetic: signed out by default, signed in only when requested", async () => {
  const signedOut = fixture("settings-account");
  await assert.rejects(() => signedOut.__PENNEL_PLATFORM__.invoke("cloud_account"), (error) => error.code === "not_signed_in");
  assert.equal((await signedOut.__PENNEL_PLATFORM__.invoke("cloud_sync_last")).status.kind, "signed_out");
  assert.equal((await signedOut.__PENNEL_PLATFORM__.invoke("app_info")).settings.cloud_sync, false);
  const synced = fixture("settings-account-synced");
  assert.match((await synced.__PENNEL_PLATFORM__.invoke("cloud_account")).email, /@example\.com$/);
  assert.equal((await synced.__PENNEL_PLATFORM__.invoke("app_info")).settings.cloud_sync, true);
  const conflict = fixture("meeting-sync-conflict");
  assert.equal((await conflict.__PENNEL_PLATFORM__.invoke("meeting_sync_states"))[0].state, "conflict");
  assert.equal((await conflict.__PENNEL_PLATFORM__.invoke("meeting_conflict_files")).length, 1);
  for (const context of [signedOut, synced, conflict]) assert.equal(context.__PENNEL_FIXTURE__.unknown.length, 0);
});

test("unsaved history stays temporary until an explicit save and recovery never runs recognition", async () => {
  const context = fixture("history-unsaved");
  const api = context.__PENNEL_PLATFORM__;
  const info = await api.invoke("app_info");
  const entry = (await api.invoke("history")).find(entry => entry.status === "unsaved");
  assert.ok(entry?.text);
  assert.equal(info.unsaved_history_count, 1);
  await api.invoke("copy_history", { id: entry.id });
  assert.equal((await api.invoke("app_info")).unsaved_history_count, 1);
  await assert.rejects(() => api.invoke("copy_history", { id: "not-a-fixture-entry" }), /Onbekend/);
  await api.invoke("save_history", { id: entry.id });
  const after = await api.invoke("history");
  assert.equal(after.filter(value => value.id === entry.id).length, 1);
  assert.equal(after.find(value => value.id === entry.id).status, "done");
  assert.equal(after.find(value => value.id === entry.id).text, entry.text);
  assert.equal((await api.invoke("app_info")).unsaved_history_count, 0);
  assert.ok(!context.__PENNEL_FIXTURE__.calls.some(command => ["retry_history", "toggle_dictation", "download_model"].includes(command)));
});

test("credential revisions invalidate task verification even when key availability stays true", async () => {
  const api = fixture("settings-models-mixed").__PENNEL_PLATFORM__;
  const initial = await api.invoke("app_info");
  assert.equal(initial.keys.groq, true);
  await api.invoke("set_key", { account: "groq", key: "synthetic-first" });
  const first = await api.invoke("app_info");
  await api.invoke("set_key", { account: "groq", key: "synthetic-second" });
  const second = await api.invoke("app_info");
  assert.equal(second.keys.groq, true);
  assert.ok(second.provider_revisions.groq > first.provider_revisions.groq);
  assert.ok(first.provider_revisions.groq > initial.provider_revisions.groq);
  assert.doesNotMatch(JSON.stringify(second), /synthetic-first|synthetic-second/);
});

test("recording storage failure retains the session and transcript until explicit persistence retry", async () => {
  const context = fixture("recording-storage-error");
  const api = context.__PENNEL_PLATFORM__;
  const failed = await api.invoke("meeting_status");
  assert.equal(failed.phase, "error");
  assert.equal(failed.recording, false);
  assert.equal(failed.busy, false);
  assert.equal(failed.id, context.__PENNEL_FIXTURE__.options.meeting_id);
  assert.ok(failed.session_id);
  assert.ok(failed.persistence_error);
  assert.ok(failed.terminal_error);
  assert.equal(failed.can_retry_persistence, true);
  assert.ok(failed.transcript.length > 0);
  assert.equal(failed.recognition.provider_kind, "local");
  assert.equal(failed.recognition.language, "nl");
  await assert.rejects(() => api.invoke("retry_meeting_persistence", { sessionId: "stale-session" }), /sessie/);
  await api.invoke("retry_meeting_persistence", { sessionId: failed.session_id });
  const saved = await api.invoke("meeting_status");
  assert.equal(saved.phase, "idle");
  assert.equal(saved.can_retry_persistence, false);
  assert.equal(saved.persistence_error, null);
  assert.equal(saved.terminal_error, null);
  assert.ok(!context.__PENNEL_FIXTURE__.calls.some(command => ["start_meeting", "retranscribe_meeting"].includes(command)));
});

test("runtime metadata preserves recording recovery without copying user content", async () => {
  const context = fixture("recording-storage-error");
  const api = context.__PENNEL_PLATFORM__;
  const full = await api.invoke("meeting_status");
  const metadata = await api.invoke("meeting_runtime_status");
  const { notes, transcript, ...expected } = full;
  assert.deepEqual(JSON.parse(JSON.stringify(metadata)), JSON.parse(JSON.stringify(expected)));
  assert.ok(transcript.length);
  assert.equal(Object.hasOwn(metadata, "transcript"), false);
  assert.equal(Object.hasOwn(metadata, "notes"), false);
  assert.equal(metadata.can_retry_persistence, true);
});

test("paused transcription is nonfatal while recording and recovery audio stay available", async () => {
  const context = fixture("recording-transcription-paused");
  const status = await context.__PENNEL_PLATFORM__.invoke("meeting_status");
  assert.equal(status.phase, "recording");
  assert.equal(status.recording, true);
  assert.equal(status.busy, true);
  assert.ok(status.transcription_warning);
  assert.equal(status.persistence_error, null);
  assert.equal(status.terminal_error, null);
  assert.equal(status.can_retry_persistence, false);
  assert.ok(status.pending > 0);
});

test("active model fixtures join atomically, replay progress and do not restart after cancellation", async () => {
  const context = fixture("onboarding-download-active");
  const api = context.__PENNEL_PLATFORM__;
  const info = await api.invoke("app_info");
  const id = info.settings.dictation_stt.model;
  assert.deepEqual(Array.from(info.active_model_downloads), [id]);
  assert.equal((await api.invoke("model_state", { id })).kind, "partial");
  const progress = [];
  await api.listen("download-progress", event => progress.push(event.payload));
  const joining = api.invoke("join_model_download", { id });
  const cancelled = assert.rejects(joining, /cancelled/);
  assert.equal(progress.length, 1);
  assert.equal(progress[0].id, id);
  assert.ok(progress[0].downloaded > 0 && progress[0].downloaded < progress[0].total);
  await api.invoke("cancel_download", { id });
  await cancelled;
  assert.equal((await api.invoke("app_info")).active_model_downloads.length, 0);
  assert.equal(await api.invoke("join_model_download", { id }), false);
  assert.equal((await api.invoke("model_state", { id })).kind, "partial");
  assert.ok(!context.__PENNEL_FIXTURE__.calls.includes("download_model"));
});

test("an active join stays pending and completion updates model ownership before the done event", async () => {
  const context = fixture("onboarding-download-active");
  const api = context.__PENNEL_PLATFORM__;
  const id = (await api.invoke("app_info")).settings.dictation_stt.model;
  let settled = false;
  const joining = api.invoke("join_model_download", { id }).then(value => { settled = true; return value; });
  await Promise.resolve();
  assert.equal(settled, false);
  const snapshots = [];
  await api.listen("download-done", () => { snapshots.push(api.invoke("app_info")); });
  assert.equal(context.__PENNEL_FIXTURE__.completeDownload(id), true);
  assert.equal(await joining, true);
  const done = await snapshots[0];
  assert.equal(done.active_model_downloads.length, 0);
  assert.equal((await api.invoke("model_state", { id })).kind, "downloaded");
});

test("task tests are explicit synthetic requests and reject unsupported task names", async () => {
  const context = fixture("settings-models-mixed");
  const api = context.__PENNEL_PLATFORM__;
  await api.invoke("app_info");
  await api.invoke("local_models", {});
  assert.ok(!context.__PENNEL_FIXTURE__.calls.includes("test_provider_task"));
  const result = await api.invoke("test_provider_task", { slot: "llm" });
  assert.equal(result.verified, true);
  assert.match(result.message, /voorbeeld/);
  await assert.rejects(() => api.invoke("test_provider_task", { slot: "unexpected-service" }), /taak/);
  assert.equal(context.__PENNEL_FIXTURE__.unknown.length, 0);
});


test("interface language defaults to the first supported system preference without changing speech", async () => {
  for (const [languages, expected] of [
    [["nl-BE", "en-US"], "nl"], [["fr-FR", "EN_us.UTF-8"], "en"],
    [["fr", "nl_NL.UTF-8"], "nl"], [["fr", "de"], "en"], [[], "en"],
  ]) {
    const context = fixture("onboarding-language", { system_languages: languages });
    const info = await context.__PENNEL_PLATFORM__.invoke("app_info");
    assert.equal(info.settings.ui_language, "system");
    assert.equal(info.system_interface_language, expected);
    assert.equal(info.interface_language, expected);
    assert.equal(info.settings.language, "nl");
    assert.equal(info.settings.onboarding.resume_step, "language");
  }
});

test("saving interface preferences applies immediately, emits settings changes and retains active sessions", async () => {
  const context = fixture("dictation-pad", { locale: "nl-BE" });
  const api = context.__PENNEL_PLATFORM__;
  const before = await api.invoke("app_info");
  const session = await api.invoke("open_dictation_session", { kind: "pad" });
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  const events = [];
  await api.listen("settings-changed", event => events.push(event.payload));
  const settings = { ...before.settings, ui_language: "en" };
  const after = await api.invoke("save_settings", { settings });
  assert.equal(after.interface_language, "en");
  assert.equal(after.system_interface_language, "nl");
  assert.equal(events.length, 1);
  assert.deepEqual(after.settings, settings);
  assert.equal((await api.invoke("get_dictation_session", { sessionId: session.session_id })).phase, "listening");
  assert.equal(after.settings.language, "nl");
  assert.deepEqual(after.settings.dictation_stt, before.settings.dictation_stt);
  await api.invoke("save_settings", { settings: { ...settings, ui_language: "system" } });
  assert.equal((await api.invoke("app_info")).interface_language, "nl");
  assert.ok(!context.__PENNEL_FIXTURE__.calls.includes("close_dictation_session"));
});

test("an explicitly named synthetic fixture profile retains language after reopening", async () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const first = fixture("onboarding-language", { locale: "nl", profile: "language-persistence" }, storage);
  const info = await first.__PENNEL_PLATFORM__.invoke("app_info");
  await first.__PENNEL_PLATFORM__.invoke("save_settings", { settings: { ...info.settings, ui_language: "en" } });
  const reopened = fixture("onboarding-language", { locale: "nl", profile: "language-persistence" }, storage);
  const after = await reopened.__PENNEL_PLATFORM__.invoke("app_info");
  assert.equal(after.interface_language, "en");
  assert.equal(after.system_interface_language, "nl");
  assert.equal(after.settings.ui_language, "en");
  assert.equal(after.settings.language, "nl");
});
