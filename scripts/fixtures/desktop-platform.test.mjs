import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { webcrypto } from "node:crypto";
import { visualModels } from "./model-catalog.mjs";
import * as conversationContract from "../../assets/contracts/meeting-conversations.ts";
import { createConversationFixture } from "./meeting-conversations.mjs";

const script = await readFile(new URL("./desktop-platform.js", import.meta.url), "utf8");
const cases = JSON.parse(await readFile(new URL("./visual-cases.json", import.meta.url), "utf8"));

test("failed transcription fixture exposes retained-chunk eligibility separately from a full archive", async () => {
  const context = fixture("meeting-notes", { failed_recovery: true, saved_audio: false });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_detail", { id });
  assert.equal(before.transcript.filter(row => row.failed).length, 2);
  assert.equal(before.failed_recovery.retryable_count, 2);
  assert.equal(before.audio_tracks.some(track => track.available), false);
  assert.equal(await api.invoke("retranscribe_meeting", { id }), 2);
  assert.equal((await api.invoke("meeting_detail", { id })).failed_recovery.failed_count, 0);
});

test("failed-only fixture preserves partial progress, zero results and notes on later retry", async () => {
  const context = fixture("meeting-notes", { failed_recovery: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, id = control.options.meeting_id;
  const notes = (await api.invoke("meeting_detail", { id })).notes;
  control.recovery.mode("zero");
  assert.equal(await api.invoke("retranscribe_meeting", { id }), 0);
  control.recovery.mode("partial_error");
  await assert.rejects(api.invoke("retranscribe_meeting", { id }), /Synthetic speech/);
  assert.equal((await api.invoke("meeting_detail", { id })).failed_recovery.failed_count, 1);
  control.recovery.mode("success");
  assert.equal(await api.invoke("retranscribe_meeting", { id }), 1);
  assert.equal((await api.invoke("meeting_detail", { id })).notes, notes);
});

test("partially eligible failed-recovery fixture retries only retained passages", async () => {
  const context = fixture("meeting-failed-recovery-partial-check");
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = (await api.invoke("meeting_detail", { id })).failed_recovery;
  assert.equal(before.failed_count, 2);
  assert.equal(before.retryable_count, 1);
  assert.equal(before.reason_code, "no_retained_audio");
  assert.equal(await api.invoke("retranscribe_meeting", { id }), 1);
  const after = (await api.invoke("meeting_detail", { id })).failed_recovery;
  assert.equal(after.failed_count, 1);
  assert.equal(after.retryable_count, 0);
  assert.equal(after.status, "unavailable");
});

test("summary fixtures require current revisions and preserve an edited version for recovery", async () => {
  const api = fixture("meeting-notes").__PENNEL_PLATFORM__, id = "2026-10-24 0900 Projectoverleg";
  const before = await api.invoke("meeting_summary", { id });
  const edited = "## Exact edit\n\nThe owner's **Markdown**.\n";
  const saved = await api.invoke("save_meeting_summary", { id, expectedRevision: before.revision, expectedSourceRevision: before.source_revision, markdown: edited });
  assert.equal(saved.markdown, edited);
  await assert.rejects(api.invoke("save_meeting_summary", { id, expectedRevision: before.revision, expectedSourceRevision: before.source_revision, markdown: "stale" }));
  const history = await api.invoke("meeting_summary_history", { id });
  assert.equal(history[0].markdown, before.markdown);
  await api.invoke("set_meeting_notes", { text: "Changed source notes" });
  await assert.rejects(api.invoke("save_meeting_summary", { id, expectedRevision: saved.revision, expectedSourceRevision: saved.source_revision, markdown: "stale source" }));
  assert.equal((await api.invoke("meeting_summary", { id })).markdown, edited);
});

test("shareable preview never falls back to private notes or legacy owner output", async () => {
  const privateFact = "PRIVATE_NOTE_SENTINEL_7a21";
  const api = fixture("meeting-notes", { private_notes: privateFact, summary_empty: true }).__PENNEL_PLATFORM__;
  const id = "2026-10-24 0900 Projectoverleg";
  assert.equal((await api.invoke("preview_meeting_export", { id, scope: "personal", includeTranscript: false })).markdown, privateFact);
  await assert.rejects(api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: false }));
  const owner = await api.invoke("generate_meeting_summary", { id });
  assert.ok(owner.markdown.includes(privateFact));
  const safe = await api.invoke("generate_shareable_meeting_summary", { id });
  assert.equal(safe.markdown.includes(privateFact), false);
  const preview = await api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: true });
  assert.equal(preview.markdown.includes(privateFact), false);
  await api.invoke("set_meeting_notes", { text: "Changed sources" });
  assert.equal((await api.invoke("shareable_meeting_summary", { id })).outdated, false);
  await api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: false });
});

test("shareable fixture freshness follows transcript attribution and source metadata", async () => {
  const context = fixture("meeting-notes", { rich_utterances: true, configured_text_provider: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, id = control.options.meeting_id;
  await api.invoke("generate_shareable_meeting_summary", { id });
  control.workspace.attribution(0, { confirmed_speaker_name: "Canonical name" });
  assert.equal((await api.invoke("shareable_meeting_summary", { id })).outdated, true);
  await assert.rejects(api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: false }));
});

test("shareable export localizes only appended labels and preserves canonical generation", async () => {
  const results = [];
  for (const language of ["en", "nl"]) {
    const context = fixture("meeting-transcript", { locale: language, ui_language: language, rich_utterances: true, configured_text_provider: true });
    const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, id = control.options.meeting_id;
    control.workspace.attribution(2, { confirmed_speaker_name: "System audio" });
    const generated = await api.invoke("generate_shareable_meeting_summary", { id });
    results.push(generated.markdown);
    const preview = await api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: true, language });
    const appended = preview.markdown.slice(generated.markdown.length);
    assert.equal(preview.markdown.slice(0, generated.markdown.length), generated.markdown);
    assert.ok(appended.includes(language === "nl" ? "Anderen 1 (Computeraudio)" : "Speaker 1 (Computer audio)"));
    assert.ok(appended.includes(language === "nl" ? "Onbekende spreker (Microfoon)" : "Unknown speaker (Microphone)"));
    assert.ok(appended.includes(language === "nl" ? "System audio (Computeraudio)" : "System audio (Computer audio)"));
    assert.ok(appended.includes("We ronden de planning maandag af."));
    assert.ok(appended.includes("Ik bevestig de volgende afspraak."));
  }
  assert.equal(results[0], results[1], "Interface language changed canonical generation input.");
});

test("legacy export translates owned marker labels without translating source words", async () => {
  const context = fixture("meeting-notes", { locale: "nl", configured_text_provider: true });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const generated = await api.invoke("generate_shareable_meeting_summary", { id });
  for (const language of ["en", "nl"]) {
    const preview = await api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: true, language });
    const appended = preview.markdown.slice(generated.markdown.length);
    assert.ok(appended.includes(language === "nl" ? "Anderen 1 (Onbekende bron)" : "Speaker 1 (Unknown source)"));
    assert.ok(appended.includes(language === "nl" ? "Ik (Onbekende bron)" : "Me (Unknown source)"));
    assert.ok(appended.includes("Ik bevestig de volgende afspraak."));
  }
});

test("new and upgraded synthetic profiles leave automatic summaries unanswered", async () => {
  const api = fixture("meeting-notes", { configured_text_provider: true }).__PENNEL_PLATFORM__;
  const info = await api.invoke("app_info");
  assert.equal(info.settings.summary_automation, "unanswered");
  await api.invoke("save_settings", { settings: { ...info.settings, summary_automation: "manual" } });
  assert.equal((await api.invoke("app_info")).settings.summary_automation, "manual");
});

test("accepted review export retains provenance while note staging keeps the exact edit", async () => {
  const context = fixture("insights-review-flow");
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("insights_snapshot", { id }), item = before.items[0];
  const edited = "- [ ] The user's exact **edited** Markdown.\n";
  const reviewed = await api.invoke("review_insight", { id, request: { store_revision: before.revision,
    item_id: item.id, item_revision: item.revision, update: { review_state: "accepted", edited_text: edited } } });
  const note = await api.invoke("insight_markdown", { id, itemId: item.id, itemRevision: reviewed.items[0].revision });
  assert.equal(note, edited);
  const exported = await api.invoke("insights_export", { id });
  assert.ok(exported.startsWith(edited));
  assert.ok(exported.includes("[Transcript](transcript.md)"));
  assert.ok(exported.includes(item.sources[0].quote));
  assert.ok(exported.includes("Evidence cutoff:"));
  assert.ok(exported.includes("decision.example.invalid"));
  assert.equal(await api.invoke("insight_markdown", { id, itemId: item.id, itemRevision: reviewed.items[0].revision }), edited);
});
function fixture(name, overrides = {}, storage, browser = {}) {
  const item = cases.find(item => item.name === name);
  const context = vm.createContext({ location: { hostname: "127.0.0.1", href: "http://127.0.0.1:4310/index.html", origin: "http://127.0.0.1:4310" }, URL, structuredClone, queueMicrotask, setTimeout, clearTimeout, TextEncoder, TextDecoder, crypto: webcrypto, localStorage: storage, ...browser,
    __PENNEL_CONVERSATION_CONTRACT__: conversationContract,
    __PENNEL_CREATE_CONVERSATION_FIXTURE__: createConversationFixture,
    __PENNEL_FIXTURE_OPTIONS__: { ...item, appearance: "dark", ...overrides } });
  vm.runInContext(script, context);
  return context;
}

const reviewStorage = () => {
  const values = new Map();
  return { values, get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
};

test("owner review starts and stops distinct meetings and reopens saved work after reload", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-lifecycle", configured_text_provider: true, rich_utterances: true };
  const first = fixture("simple-task-shell", options, storage), api = first.__PENNEL_PLATFORM__;
  assert.equal((await api.invoke("meeting_status")).recording, false);
  const firstId = await api.invoke("start_meeting", { title: "First review conversation" });
  const status = await api.invoke("meeting_status");
  assert.equal(status.recording, true);
  assert.equal(status.id, firstId);
  assert.equal(status.title, "First review conversation");
  await api.invoke("set_meeting_notes", { text: "Keep this exact note.", sessionId: status.session_id });
  await api.invoke("stop_meeting", { sessionId: status.session_id });
  assert.equal((await api.invoke("meeting_status")).recording, false);
  await api.invoke("save_meeting_notes", { id: firstId, text: "Continued after Stop." });
  assert.equal((await api.invoke("meeting_detail", { id: firstId })).notes, "Continued after Stop.");
  const secondId = await api.invoke("start_meeting", { title: "Second review conversation" });
  assert.notEqual(firstId, secondId);
  assert.notEqual((await api.invoke("meeting_status")).session_id, status.session_id);
  const retained = await api.invoke("meeting_detail", { id: firstId });
  assert.equal(retained.notes, "Continued after Stop.");
  assert.equal(retained.info.title, "First review conversation");
  await api.invoke("stop_meeting");
  const reopened = fixture("simple-task-shell", options, storage).__PENNEL_PLATFORM__;
  assert.equal((await reopened.invoke("meeting_detail", { id: firstId })).notes, "Continued after Stop.");
  assert.ok((await reopened.invoke("meetings", { query: "" })).some(info => info.id === secondId));
});

test("owner review dictation completion retains one result and cancelled late work cannot return", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-dictation", dictation_manual: true };
  const context = fixture("simple-task-shell", options, storage), api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__;
  const before = (await api.invoke("history")).length, session = await api.invoke("open_dictation_session", { kind: "pad" });
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  assert.equal((await api.invoke("get_dictation_session", { sessionId: session.session_id })).phase, "listening");
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  assert.equal((await api.invoke("get_dictation_session", { sessionId: session.session_id })).phase, "working");
  assert.equal(control.review.completeDictation(), 1);
  const done = await api.invoke("get_dictation_session", { sessionId: session.session_id });
  assert.equal(done.phase, "done");
  assert.equal((await api.invoke("history")).length, before + 1);
  assert.equal(control.review.completeDictation(), 0);
  const reopenedContext = fixture("simple-task-shell", options, storage), reopened = reopenedContext.__PENNEL_PLATFORM__;
  assert.equal(await reopened.invoke("pad_text"), done.text);
  assert.equal((await reopened.invoke("history")).length, before + 1);
  const next = await reopened.invoke("open_dictation_session", { kind: "pad" });
  await reopened.invoke("toggle_dictation_session", { sessionId: next.session_id });
  await reopened.invoke("toggle_dictation_session", { sessionId: next.session_id });
  reopenedContext.__PENNEL_FIXTURE__.review.completeDictation();
  const reopenedHistory = await reopened.invoke("history");
  assert.equal(new Set(reopenedHistory.map(entry => entry.id)).size, reopenedHistory.length);
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  await api.invoke("toggle_dictation_session", { sessionId: session.session_id });
  await api.invoke("cancel_dictation_session", { sessionId: session.session_id });
  assert.equal(control.review.completeDictation(), 0);
  assert.equal((await api.invoke("history")).length, before + 1);
});

test("owner review prepared output keeps labels while late transcript changes reject stale generation", async () => {
  const context = fixture("simple-task-shell", { owner_review: true, rich_utterances: true, configured_text_provider: true, summary_generation_delay_ms: 20 });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, id = control.options.meeting_id;
  control.workspace.attribution(0, { confirmed_speaker_name: "Confirmed speaker" });
  const pending = api.invoke("generate_shareable_meeting_summary", { id });
  await new Promise(resolve => setTimeout(resolve, 0));
  control.workspace.attribution(0, { confirmed_speaker_name: "Changed speaker" });
  await assert.rejects(pending, /transcript changed/);
  assert.equal(await api.invoke("shareable_meeting_summary", { id }), null);
  await api.invoke("generate_shareable_meeting_summary", { id });
  const output = await api.invoke("preview_meeting_export", { id, scope: "shareable", includeTranscript: true });
  assert.match(output.markdown, /\[00:00\] Changed speaker \(System audio\): We ronden/);
  assert.ok(output.html.includes("Changed speaker (System audio)"));
  await api.invoke("set_meeting_notes", { text: "PRIVATE_CHANGED_NOTE" });
  assert.equal((await api.invoke("shareable_meeting_summary", { id })).outdated, false);
  control.workspace.attribution(0, { source_id: "room_microphone" });
  assert.equal((await api.invoke("shareable_meeting_summary", { id })).outdated, true);
});

test("owner review ignores a delayed note save after the capture session changes", async () => {
  const context = fixture("simple-task-shell", { owner_review: true, note_save_delay_ms: 60_000 });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__;
  await api.invoke("start_meeting", { title: "First delayed session" });
  const first = await api.invoke("meeting_status");
  const pending = api.invoke("set_meeting_notes", { text: "Old session note", sessionId: first.session_id });
  await api.invoke("stop_meeting");
  await api.invoke("start_meeting", { title: "Current session" });
  control.workspace.releaseNotes();
  await assert.rejects(pending, /session changed/);
  assert.equal((await api.invoke("meeting_status")).notes, "");
});

test("owner review search filters titles and returns matching source passages", async () => {
  const api = fixture("simple-task-shell", { owner_review: true }).__PENNEL_PLATFORM__;
  const title = await api.invoke("search_library", { query: "Studio Noord" });
  assert.deepEqual(Array.from(title.meetings, info => info.title), ["Klantgesprek Studio Noord"]);
  const passage = await api.invoke("search_library", { query: "planning maandag" });
  assert.ok(passage.passages.length > 0);
  assert.ok(passage.passages.every(row => row.kind === "transcript" && row.text.includes("planning") && row.meeting_id));
  const missing = await api.invoke("search_library", { query: "no-such-review-term" });
  assert.equal(missing.meetings.length, 0);
  assert.equal(missing.passages.length, 0);
});

test("owner review reset and frontend state are confined to one synthetic profile", async () => {
  const storage = reviewStorage();
  storage.setItem("ordinary-app-setting", "preserve");
  storage.setItem("pennel-synthetic-fixture:other-review", "preserve");
  const context = fixture("simple-task-shell", { owner_review: true, profile: "review-one" }, storage);
  context.localStorage.setItem("pennel.workspace.presentation.v1", JSON.stringify({ visible: false }));
  assert.equal(JSON.parse(context.localStorage.getItem("pennel.workspace.presentation.v1")).visible, false);
  const other = fixture("simple-task-shell", { owner_review: true, profile: "review-two" }, storage);
  assert.equal(other.localStorage.getItem("pennel.workspace.presentation.v1"), null);
  other.localStorage.setItem("pennel.workspace.presentation.v1", "other presentation");
  storage.setItem("pennel-synthetic-fixture:review-one:conversations:v1", "scratch");
  context.__PENNEL_FIXTURE__.review.resetProfile();
  assert.equal(storage.getItem("pennel-synthetic-fixture:review-one:ui:pennel.workspace.presentation.v1"), null);
  assert.equal(storage.getItem("pennel-synthetic-fixture:review-one:conversations:v1"), null);
  assert.equal(storage.getItem("ordinary-app-setting"), "preserve");
  assert.equal(storage.getItem("pennel-synthetic-fixture:other-review"), "preserve");
  assert.equal(other.localStorage.getItem("pennel.workspace.presentation.v1"), "other presentation");
});

test("owner review profile reset cancels its pending global dictation without recreating stored work", async () => {
  const storage = reviewStorage(), context = fixture("simple-task-shell", { owner_review: true, profile: "review-reset-pending", dictation_manual: true }, storage);
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__;
  await api.invoke("toggle_dictation");
  assert.equal((await api.invoke("dictation_state")).state, "listening");
  await api.invoke("toggle_dictation");
  assert.equal((await api.invoke("dictation_state")).state, "working");
  control.review.resetProfile();
  assert.equal(control.review.completeDictation(), 0);
  assert.equal(storage.getItem("pennel-synthetic-fixture:review-reset-pending:owner-review:v1"), null);
});

test("owner review download and save failures can be repaired without replacing useful work", async () => {
  const context = fixture("simple-task-shell", { owner_review: true, configured_text_provider: true }), api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__;
  const id = await api.invoke("start_meeting", { title: "Recoverable review" });
  await api.invoke("set_meeting_notes", { text: "Saved note" });
  control.review.failSaves(true);
  await assert.rejects(api.invoke("set_meeting_notes", { text: "Unsaved replacement" }));
  assert.equal((await api.invoke("meeting_detail", { id })).notes, "Saved note");
  control.review.failSaves(false);
  await api.invoke("set_meeting_notes", { text: "Recovered note" });
  assert.equal((await api.invoke("meeting_detail", { id })).notes, "Recovered note");
  const model = (await api.invoke("local_models"))[0];
  await api.invoke("delete_model", { id: model.id });
  control.review.failDownloads("Synthetic download unavailable");
  await api.invoke("download_model", { id: model.id });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.notEqual((await api.invoke("model_state", { id: model.id })).kind, "downloaded");
  control.review.failDownloads(null);
  await api.invoke("download_model", { id: model.id });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal((await api.invoke("model_state", { id: model.id })).kind, "downloaded");
});

test("owner review completed model downloads and explicit deletion survive profile reload", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-model-reload" };
  const first = fixture("onboarding-welcome", options, storage), api = first.__PENNEL_PLATFORM__;
  const id = (await api.invoke("local_models"))[0].id;
  assert.equal((await api.invoke("model_state", { id })).kind, "not_downloaded");
  await api.invoke("download_model", { id });
  assert.equal((await api.invoke("model_state", { id })).kind, "downloaded");
  const reopened = fixture("onboarding-welcome", options, storage).__PENNEL_PLATFORM__;
  assert.equal((await reopened.invoke("model_state", { id })).kind, "downloaded");
  await reopened.invoke("delete_model", { id });
  assert.equal((await fixture("onboarding-welcome", options, storage).__PENNEL_PLATFORM__.invoke("model_state", { id })).kind, "not_downloaded");
  const other = fixture("onboarding-welcome", { ...options, profile: "review-model-other" }, storage).__PENNEL_PLATFORM__;
  assert.equal((await other.invoke("model_state", { id })).kind, "not_downloaded");
});

test("owner review summary history remains recoverable after profile reload", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-summary-reload", configured_text_provider: true };
  const first = fixture("meeting-notes", options, storage), api = first.__PENNEL_PLATFORM__, id = first.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_summary", { id });
  const edited = await api.invoke("save_meeting_summary", { id, expectedRevision: before.revision, expectedSourceRevision: before.source_revision, markdown: "Exact recoverable edit." });
  assert.equal((await api.invoke("meeting_summary_history", { id })).length, 1);
  const reopened = fixture("meeting-notes", options, storage).__PENNEL_PLATFORM__;
  const history = await reopened.invoke("meeting_summary_history", { id });
  assert.equal(history.length, 1);
  assert.equal(history[0].markdown, before.markdown);
  const restored = await reopened.invoke("restore_meeting_summary", { id, expectedRevision: edited.revision,
    expectedSourceRevision: edited.source_revision, revision: history[0].revision });
  assert.equal(restored.markdown, before.markdown);
  assert.equal((await reopened.invoke("meeting_summary_history", { id }))[0].markdown, "Exact recoverable edit.");
});

test("owner review reset in another tab fences pending dictation, notes, summaries and settings", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-cross-tab-reset", dictation_manual: true,
    note_save_delay_ms: 200, summary_generation_delay_ms: 20, configured_text_provider: true };
  storage.setItem("unrelated-user-setting", "preserve");
  storage.setItem("pennel-synthetic-fixture:other-review:owner-review:v1", "preserve");
  const tab = fixture("meeting-notes", options, storage), api = tab.__PENNEL_PLATFORM__, control = tab.__PENNEL_FIXTURE__;
  await api.invoke("save_pad_text", { text: "Old synthetic draft" });
  await api.invoke("toggle_dictation"); await api.invoke("toggle_dictation");
  const notes = api.invoke("save_meeting_notes", { id: control.options.meeting_id, text: "Late old notes" });
  const summary = api.invoke("generate_shareable_meeting_summary", { id: control.options.meeting_id });
  const summaryOutcome = summary.then(() => null, error => error);
  const info = await api.invoke("app_info");
  const resetTab = fixture("meeting-notes", options, storage);
  resetTab.__PENNEL_FIXTURE__.review.resetProfile();
  assert.equal(control.review.completeDictation(), 0);
  control.workspace.releaseNotes();
  await assert.rejects(notes, /reset|Reload/i);
  assert.match(String(await summaryOutcome), /reset|Reload/i);
  await assert.rejects(api.invoke("save_settings", { settings: info.settings }), /reset|Reload/i);
  assert.throws(() => tab.localStorage.setItem("pennel.workspace.presentation.v1", "old presentation"), /reset|Reload/i);
  assert.equal(storage.getItem("pennel-synthetic-fixture:review-cross-tab-reset:owner-review:v1"), null);
  assert.equal(storage.getItem("pennel-synthetic-fixture:review-cross-tab-reset:reset-generation"), "1");
  assert.equal(storage.getItem("unrelated-user-setting"), "preserve");
  assert.equal(storage.getItem("pennel-synthetic-fixture:other-review:owner-review:v1"), "preserve");
  const fresh = fixture("meeting-notes", options, storage).__PENNEL_PLATFORM__;
  await fresh.invoke("save_pad_text", { text: "Fresh synthetic draft" });
  assert.equal(await fresh.invoke("pad_text"), "Fresh synthetic draft");
  assert.ok(storage.getItem("pennel-synthetic-fixture:review-cross-tab-reset:owner-review:v1"));
});

test("owner review practice opens an isolated local window sharing only its review profile", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-practice-window", locale: "en", appearance: "light", dictation_manual: true };
  const parent = fixture("simple-task-shell", options, storage), api = parent.__PENNEL_PLATFORM__, opened = [];
  parent.open = (url, name) => { opened.push({ url, name }); return { focus() {} }; };
  await api.invoke("save_pad_text", { text: "Keep the parent draft." });
  await api.invoke("show_window", { kind: "onboarding", tab: "practice" });
  assert.equal(opened.length, 1);
  const url = new URL(opened[0].url);
  assert.equal(url.origin, parent.location.origin);
  assert.equal(url.searchParams.get("qa"), "owner-review-practice");
  assert.equal(url.searchParams.get("window"), "onboarding");
  assert.equal(url.searchParams.get("tab"), "practice");
  assert.equal(url.searchParams.get("profile"), options.profile);
  assert.equal(url.searchParams.get("locale"), "en");
  assert.equal(url.searchParams.get("theme"), "light");
  const practice = fixture("onboarding-practice", options, storage), practiceApi = practice.__PENNEL_PLATFORM__;
  const session = await practiceApi.invoke("open_dictation_session", { kind: "practice" });
  const historyBefore = (await api.invoke("history")).length;
  await practiceApi.invoke("toggle_dictation_session", { sessionId: session.session_id });
  await practiceApi.invoke("toggle_dictation_session", { sessionId: session.session_id });
  practice.__PENNEL_FIXTURE__.review.completeDictation();
  assert.equal((await practiceApi.invoke("get_dictation_session", { sessionId: session.session_id })).phase, "done");
  assert.equal(await api.invoke("pad_text"), "Keep the parent draft.");
  assert.equal((await api.invoke("history")).length, historyBefore);
  await api.invoke("show_window", { kind: "onboarding", tab: "dictation-permissions" });
  assert.equal(new URL(opened[1].url).searchParams.get("qa"), "owner-review-permissions");
  assert.equal(new URL(opened[1].url).searchParams.get("tab"), "dictation-permissions");
  parent.open = () => null;
  await assert.rejects(api.invoke("show_window", { kind: "onboarding", tab: "practice" }), /pop.up/i);
  await assert.rejects(api.invoke("show_window", { kind: "unknown-native-window" }), /review|supported/i);
});

test("owner review workspace bridges preserve exact History and settings destinations", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-route-window" };
  const parent = fixture("simple-task-shell", options, storage), delivered = [], api = parent.__PENNEL_PLATFORM__;
  await api.listen("workspace-navigate", event => delivered.push(event.payload));
  await api.invoke("save_pad_text", { text: "Stable parent draft" });
  await api.invoke("show_window", { kind: "history", tab: "entry:qa-dictation-1" });
  assert.equal(delivered.at(-1).section, "history");
  assert.equal(delivered.at(-1).tab, "entry:qa-dictation-1");
  await api.invoke("show_window", { kind: "settings", tab: "appearance" });
  assert.equal(delivered.at(-1).section, "settings");
  assert.equal(delivered.at(-1).tab, "appearance");
  const practice = fixture("onboarding-practice", options, storage);
  practice.opener = parent;
  await practice.__PENNEL_PLATFORM__.invoke("show_workspace", { route: { section: "history", tab: "entry:qa-dictation-2", meeting_id: null, jump_ms: null } });
  assert.equal(delivered.at(-1).tab, "entry:qa-dictation-2");
  assert.equal(await api.invoke("pad_text"), "Stable parent draft");
  await assert.rejects(api.invoke("show_workspace", { route: { section: "history", tab: "entry:../not-a-result" } }), /route|history|destination/i);
});

test("owner review saved connections and credential presence survive reload without storing key text", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-connection-reload" };
  const context = fixture("simple-task-shell", options, storage), api = context.__PENNEL_PLATFORM__;
  const added = await api.invoke("add_provider", { kind: "custom" });
  const provider = added.settings.providers.find(provider => provider.id.startsWith("qa-custom"));
  const secret = "DO_NOT_PERSIST_SYNTHETIC_KEY_84f";
  await api.invoke("set_key", { account: provider.id, key: secret });
  const reopened = fixture("simple-task-shell", options, storage).__PENNEL_PLATFORM__, restored = await reopened.invoke("app_info");
  assert.ok(restored.settings.providers.some(item => item.id === provider.id));
  assert.equal(restored.keys[provider.id], true);
  assert.equal([...storage.values.values()].some(value => value.includes(secret)), false);
});

test("owner review permission repair in a local window refreshes the parent without replacing its draft", async () => {
  const storage = reviewStorage(), events = new Map(), options = { owner_review: true, profile: "review-permission-window", permission: "denied" };
  const parent = fixture("simple-task-shell", options, storage, { addEventListener: (name, handler) => events.set(name, handler) });
  const api = parent.__PENNEL_PLATFORM__;
  await api.invoke("save_pad_text", { text: "Retain the parent draft." });
  assert.equal((await api.invoke("app_info")).capabilities.global_hotkey.status, "permission_required");
  const repair = fixture("onboarding-dictation-permissions", options, storage).__PENNEL_PLATFORM__;
  await repair.invoke("request_permission", { kind: "accessibility" });
  await repair.invoke("request_permission", { kind: "microphone" });
  const key = "pennel-synthetic-fixture:review-permission-window:owner-review:v1";
  events.get("storage")({ key, newValue: storage.getItem(key) });
  assert.equal((await api.invoke("app_info")).capabilities.global_hotkey.status, "available");
  assert.equal((await api.invoke("onboarding_readiness", { target: "dictation" })).can_record, true);
  assert.equal(await api.invoke("pad_text"), "Retain the parent draft.");
});

test("owner review onboarding service writes preserve newer parent workspace and service changes", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "review-child-preserves-work", dictation_manual: true };
  const parent = fixture("simple-task-shell", options, storage), api = parent.__PENNEL_PLATFORM__;
  await api.invoke("save_pad_text", { text: "Original parent draft" });
  await api.invoke("start_meeting", { title: "Original meeting" });
  const child = fixture("onboarding-dictation-permissions", options, storage).__PENNEL_PLATFORM__;
  await api.invoke("stop_meeting");
  const newestId = await api.invoke("start_meeting", { title: "Latest parent meeting" });
  await api.invoke("set_meeting_notes", { text: "Latest parent notes" });
  await api.invoke("save_pad_text", { text: "Latest parent draft" });
  await api.invoke("toggle_dictation"); await api.invoke("toggle_dictation"); parent.__PENNEL_FIXTURE__.review.completeDictation();
  const latestHistory = await api.invoke("history");
  await api.invoke("set_key", { account: "openai", key: "PARENT_KEY_MUST_STAY_PRIVATE" });
  await child.invoke("request_permission", { kind: "microphone" });
  const modelId = (await child.invoke("local_models"))[0].id;
  await child.invoke("delete_model", { id: modelId }); await child.invoke("download_model", { id: modelId });
  await child.invoke("set_key", { account: "groq", key: "CHILD_KEY_MUST_STAY_PRIVATE" });
  const reopened = fixture("simple-task-shell", options, storage).__PENNEL_PLATFORM__;
  assert.equal(await reopened.invoke("pad_text"), "Latest parent draft");
  assert.equal((await reopened.invoke("meeting_status")).id, newestId);
  assert.equal((await reopened.invoke("meeting_status")).recording, true);
  assert.equal((await reopened.invoke("meeting_detail", { id: newestId })).notes, "Latest parent notes");
  assert.equal((await reopened.invoke("history"))[0].id, latestHistory[0].id);
  assert.equal((await reopened.invoke("model_state", { id: modelId })).kind, "downloaded");
  const restored = await reopened.invoke("app_info");
  assert.equal(restored.keys.openai, true);
  assert.equal(restored.keys.groq, true);
  assert.equal([...storage.values.values()].some(value => /PARENT_KEY_MUST_STAY_PRIVATE|CHILD_KEY_MUST_STAY_PRIVATE/.test(value)), false);
});

test("owner review practice readiness resumes to practice only with a usable engine and microphone", async () => {
  const ready = fixture("simple-task-shell", { owner_review: true }).__PENNEL_PLATFORM__;
  const allowed = await ready.invoke("onboarding_readiness", { target: "practice" });
  assert.equal(allowed.configured, true);
  assert.equal(allowed.can_record, true);
  assert.equal(allowed.resume_step, "practice");
  const denied = await fixture("simple-task-shell", { owner_review: true, permission: "denied" }).__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "practice" });
  assert.equal(denied.can_record, false);
  assert.equal(denied.resume_step, "permissions");
  const missing = await fixture("onboarding-welcome", { owner_review: true }).__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "practice" });
  assert.equal(missing.configured, false);
  assert.equal(missing.resume_step, "engine");
  const unsupported = await fixture("simple-task-shell", { owner_review: true, provider_availability: { local: { status: "unsupported", reason: "Unavailable synthetic engine" } } }).__PENNEL_PLATFORM__.invoke("onboarding_readiness", { target: "practice" });
  assert.equal(unsupported.configured, false);
  assert.equal(unsupported.resume_step, "engine");
});

test("conversation fixture preserves selected identities and per-conversation drafts through a full reopen", async () => {
  const values = new Map(), storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const options = { profile: "conversation-reopen", configured_text_provider: true, rich_utterances: true };
  const first = fixture("meeting-transcript", options, storage), api = first.__PENNEL_PLATFORM__;
  const scope = { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" }, requests = conversationContract.conversationRequests;
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  const originalId = initial.selected_conversation_id;
  await assert.rejects(api.invoke("update_conversation", { request: { scope, action: "draft", conversation_id: originalId, draft: "Missing CAS revision" } }));
  const saved = await api.invoke("update_conversation", { request: requests.draft(scope, originalId, "First synthetic draft", initial.conversation.draft_revision) });
  await assert.rejects(api.invoke("update_conversation", { request: requests.draft(scope, originalId, "Stale draft", initial.conversation.draft_revision) }));
  const second = await api.invoke("update_conversation", { request: requests.create(scope, "Second synthetic conversation") });
  assert.notEqual(second.selected_conversation_id, originalId);
  await api.invoke("update_conversation", { request: requests.draft(scope, second.selected_conversation_id, "Second synthetic draft", second.conversation.draft_revision) });
  const reopened = fixture("meeting-transcript", options, storage).__PENNEL_PLATFORM__;
  const restored = await reopened.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(restored.selected_conversation_id, second.selected_conversation_id);
  assert.equal(restored.conversation.draft, "Second synthetic draft");
  const selected = await reopened.invoke("update_conversation", { request: requests.select(scope, originalId) });
  assert.equal(selected.conversation.draft, "First synthetic draft");
  assert.equal(selected.conversation.draft_revision, saved.conversation.draft_revision);
  const library = await reopened.invoke("conversation_snapshot", { request: requests.snapshot({ kind: "library" }) });
  assert.notEqual(library.selected_conversation_id, originalId);
  assert.equal(library.conversation.draft, "");
});

test("conversation fixture keeps background replies in their original conversation and restores incomplete work honestly", async () => {
  const values = new Map(), storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const options = { profile: "conversation-requests", configured_text_provider: true, rich_utterances: true, live_question_manual: true };
  const context = fixture("meeting-transcript", options, storage), api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests;
  const scope = { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  const first = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), original = first.conversation.id;
  const requestId = "00000000-0000-7000-8000-000000009001";
  const pending = await api.invoke("begin_conversation_question", { request: requests.begin(scope, original, requestId, "Which synthetic decision?") });
  assert.equal(pending.conversation.active_request_id, requestId);
  assert.deepEqual(Array.from(pending.messages, message => message.status), ["completed", "pending"]);
  assert.equal(pending.messages[1].evidence.included_through_ms, 21000);
  const second = await api.invoke("update_conversation", { request: requests.create(scope, "A different conversation") });
  assert.equal(await context.__PENNEL_FIXTURE__.conversations.complete(requestId), true);
  const current = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(current.conversation.id, second.conversation.id);
  assert.equal(current.messages.length, 0);
  const completed = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope, original) });
  assert.equal(completed.messages[1].status, "completed");
  assert.equal(completed.messages[1].evidence.included_through_ms, 21000);
  const cancelledId = "00000000-0000-7000-8000-000000009002";
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, current.conversation.id, cancelledId, "Cancel this synthetic question") });
  const cancelled = await api.invoke("cancel_conversation_question", { request: requests.cancel(scope, current.conversation.id, cancelledId) });
  assert.equal(cancelled.messages.at(-1).status, "cancelled");
  assert.equal(await context.__PENNEL_FIXTURE__.conversations.complete(cancelledId), false);
  const interruptedId = "00000000-0000-7000-8000-000000009003";
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, current.conversation.id, interruptedId, "Pending at restart") });
  const reopened = fixture("meeting-transcript", options, storage);
  const restored = await reopened.__PENNEL_PLATFORM__.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(restored.conversation.active_request_id, null);
  assert.equal(restored.messages.at(-1).status, "interrupted");
  assert.equal(restored.messages.at(-1).answer, null);
});

test("conversation fixture resolves stable source identity after rename and marks changed labels or deleted meetings", async () => {
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, scope = { kind: "library" }, requests = conversationContract.conversationRequests;
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), requestId = "00000000-0000-7000-8000-000000009010";
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, initial.conversation.id, requestId, "Which source supports this?") });
  await control.conversations.complete(requestId);
  const answered = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), message = answered.messages[1];
  const source = requests.source(scope, initial.conversation.id, message.id, 1), original = await api.invoke("conversation_source", { request: source });
  assert.equal(original.status, "current");
  const renamed = await api.invoke("rename_meeting", { id: control.options.meeting_id, title: "Renamed synthetic meeting" });
  assert.notEqual(renamed, control.options.meeting_id);
  const moved = await api.invoke("conversation_source", { request: source });
  assert.equal(moved.current_meeting_id, renamed);
  assert.equal(moved.passage.meeting_uid, original.passage.meeting_uid);
  control.workspace.attribution(0, { confirmed_speaker_name: "Corrected synthetic speaker" });
  const stale = await api.invoke("conversation_source", { request: source });
  assert.equal(stale.status, "stale");
  assert.equal(stale.passage.text, original.passage.text);
  await api.invoke("delete_meeting", { id: renamed });
  const unavailable = await api.invoke("conversation_source", { request: source });
  assert.equal(unavailable.status, "unavailable");
  assert.equal(unavailable.current_meeting_id, null);
  assert.equal((await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) })).messages[1].text, message.text);
});

test("conversation fixture note anchors use the core's UTF-16 editor offsets around Unicode", async () => {
  const context = fixture("recording", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  await api.invoke("set_meeting_notes", { text: "Préface 😀\nRepeated source.\nRepeated source.\n", sessionId: "qa-recording" });
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  const pending = await api.invoke("begin_conversation_question", { request: requests.begin(scope, initial.conversation.id,
    "00000000-0000-7000-8000-000000009015", "Cite the second repeated note", { session_id: "qa-recording" }) });
  const repeated = pending.messages.at(-1).evidence.anchors.filter(anchor => anchor.source_text === "Repeated source.");
  assert.deepEqual(Array.from(repeated, anchor => [anchor.notes_start, anchor.notes_end]), [[11, 27], [28, 44]]);
});

test("conversation fixture uses the shared required wire fields and refuses stale or deleted requests", async () => {
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), conversationId = initial.conversation.id;
  for (const [command, request] of [
    ["conversation_snapshot", null], ["begin_conversation_question", { scope, conversation_id: conversationId, request_id: "request" }],
    ["cancel_conversation_question", { scope, conversation_id: conversationId }],
    ["conversation_source", { scope, conversation_id: conversationId, message_id: "message", citation_number: "1" }],
  ]) await assert.rejects(api.invoke(command, request === null ? {} : { request }));
  const requestId = "00000000-0000-7000-8000-000000009020";
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, conversationId, requestId, "Delete this conversation") });
  await api.invoke("update_conversation", { request: requests.delete(scope, conversationId) });
  assert.equal(await context.__PENNEL_FIXTURE__.conversations.complete(requestId), false);
  const after = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.notEqual(after.conversation.id, conversationId);
  assert.equal(after.messages.length, 0);
  await assert.rejects(api.invoke("conversation_snapshot", { request: requests.snapshot(scope, conversationId) }));
});

test("conversation fixture treats an identical request retry as idempotent and rejects a different question", async () => {
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "library" };
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  const request = requests.begin(scope, initial.conversation.id, "00000000-0000-7000-8000-000000009025", "An idempotent synthetic question");
  const first = await api.invoke("begin_conversation_question", { request });
  const retry = await api.invoke("begin_conversation_question", { request });
  assert.deepEqual(Array.from(retry.messages, message => message.id), Array.from(first.messages, message => message.id));
  await assert.rejects(api.invoke("begin_conversation_question", { request: { ...request, question: "A conflicting retry" } }));
  assert.equal((await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) })).messages.length, 2);
});

test("conversation fixture retains unsaved questions and answers and retries without duplicate messages", async () => {
  const values = new Map(), storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const options = { profile: "conversation-outage", configured_text_provider: true, rich_utterances: true, conversation_manual: true, conversation_storage_error: true };
  const context = fixture("meeting-transcript", options, storage), api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "library" };
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), requestId = "00000000-0000-7000-8000-000000009030";
  assert.equal(initial.storage_status, "unsaved");
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, initial.conversation.id, requestId, "Keep this unsaved question") });
  await context.__PENNEL_FIXTURE__.conversations.complete(requestId);
  const unsaved = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(unsaved.storage_status, "unsaved");
  assert.equal(unsaved.messages[1].status, "completed");
  context.__PENNEL_FIXTURE__.conversations.failStorage(false);
  const saved = await api.invoke("update_conversation", { request: requests.retry(scope, initial.conversation.id) });
  assert.equal(saved.storage_status, "saved");
  const reopened = fixture("meeting-transcript", { ...options, conversation_storage_error: false }, storage);
  const restored = await reopened.__PENNEL_PLATFORM__.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.deepEqual(Array.from(restored.messages, message => message.id), Array.from(unsaved.messages, message => message.id));
  assert.equal(restored.messages.length, 2);
});

test("conversation fixture preserves a specific temporary-storage reason without disabling ordinary questions", async () => {
  const reason = "This meeting's identity could not be saved. Conversations remain temporary; retry saving or copy text before quitting.";
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true, conversation_storage_error: true, conversation_storage_reason: reason });
  const api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(initial.storage_status, "unsaved"); assert.equal(initial.storage_error, reason);
  const requestId = "00000000-0000-7000-8000-000000009032";
  await api.invoke("begin_conversation_question", { request: requests.begin(scope, initial.conversation.id, requestId, "An ordinary temporary question") });
  await context.__PENNEL_FIXTURE__.conversations.complete(requestId);
  const answer = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(answer.storage_error, reason); assert.equal(answer.messages.at(-1).status, "completed");
});

test("conversation fixture can hold acknowledged-note preparation before question admission", async () => {
  const context = fixture("recording", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, before = control.workspace.notes();
  control.workspace.notesDelay(3000);
  const pending = api.invoke("set_meeting_notes", { text: "New acknowledged synthetic notes", sessionId: "qa-recording" });
  try {
    for (let attempt = 0; attempt < 100 && !control.workspace.pendingNoteSaves(); attempt++) await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(control.workspace.pendingNoteSaves(), 1);
    assert.equal(control.workspace.notes(), before);
  } finally { control.workspace.notesDelay(0); control.workspace.releaseNotes(); }
  await pending;
  assert.equal(control.workspace.notes(), "New acknowledged synthetic notes");
});

test("conversation fixture permits scoped cancellation before its delayed admission response returns", async () => {
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, requests = conversationContract.conversationRequests, scope = { kind: "library" };
  const initial = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) }), requestId = "00000000-0000-7000-8000-000000009035";
  control.conversations.beginDelay(3000);
  control.conversations.cancelDelay(3000);
  let responded = false, cancelResponded = false, pendingCancel;
  const pending = api.invoke("begin_conversation_question", { request: requests.begin(scope, initial.conversation.id, requestId, "Delayed synthetic admission") })
    .then(result => { responded = true; return result; });
  try {
    for (let attempt = 0; attempt < 100 && !control.conversations.pendingBeginResponses(); attempt++) await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(control.conversations.pendingBeginResponses(), 1);
    pendingCancel = api.invoke("cancel_conversation_question", { request: requests.cancel(scope, initial.conversation.id, requestId) })
      .then(result => { cancelResponded = true; return result; });
    for (let attempt = 0; attempt < 100 && !control.conversations.pendingCancelResponses(); attempt++) await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(control.conversations.pendingCancelResponses(), 1);
    assert.equal((await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) })).messages.at(-1).status, "cancelled");
    assert.equal(responded, false);
    control.conversations.beginDelay(0); control.conversations.releaseBegins(); await pending;
    assert.equal(cancelResponded, false);
  } finally { control.conversations.beginDelay(0); control.conversations.releaseBegins(); control.conversations.cancelDelay(0); control.conversations.releaseCancels(); }
  await pending; await pendingCancel;
  assert.equal((await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) })).messages.at(-1).status, "cancelled");
});

test("conversation fixture bounds conversation and message pages using opaque cursors", async () => {
  const context = fixture("meeting-transcript", { configured_text_provider: true, rich_utterances: true, conversation_manual: true });
  const api = context.__PENNEL_PLATFORM__, requests = conversationContract.conversationRequests, scope = { kind: "library" };
  let state = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  for (let index = 0; index < 54; index++) state = await api.invoke("update_conversation", { request: requests.create(scope, `Synthetic conversation ${index}`) });
  assert.equal(state.conversations.length, 50);
  const olderConversations = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope, undefined, { conversations_before: state.next_conversations_cursor }) });
  assert.equal(olderConversations.conversations.length, 5);
  assert.equal(olderConversations.next_conversations_cursor, null);
  for (let index = 0; index < 30; index++) {
    const requestId = `00000000-0000-7000-8000-${String(9200 + index).padStart(12, "0")}`;
    await api.invoke("begin_conversation_question", { request: requests.begin(scope, state.conversation.id, requestId, `Synthetic question ${index}`) });
    await context.__PENNEL_FIXTURE__.conversations.complete(requestId);
  }
  const latest = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope) });
  assert.equal(latest.messages.length, 50);
  const oldest = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope, undefined, { messages_before: latest.next_messages_cursor }) });
  assert.equal(oldest.messages.length, 10);
  assert.equal(oldest.next_messages_cursor, null);
  assert.equal(new Set([...latest.messages, ...oldest.messages].map(message => message.id)).size, 60);
  const focused = await api.invoke("conversation_snapshot", { request: requests.snapshot(scope, undefined, { focus_message_id: oldest.messages[1].id }) });
  assert.ok(focused.messages.some(message => message.id === oldest.messages[1].id));
  assert.ok(focused.messages.length <= 50);
});

test("people fixture keeps duplicate identities and speaker links separate from names and enrollment", async () => {
  const context = fixture("meeting-transcript", { people_seeded: true, rich_utterances: true });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_detail", { id });
  let people = await api.invoke("meeting_people_snapshot", { id });
  const duplicates = people.people.filter(person => person.display_name === "Alex");
  assert.equal(duplicates.length, 2);
  assert.notEqual(duplicates[0].id, duplicates[1].id);
  const result = await api.invoke("update_meeting_people", { id, request: { action: "link_speaker", expected_revision: people.revision,
    expected_source_revision: people.source_revision, person_id: duplicates[1].id, utterance_ids: [before.transcript[0].utterance_id] } });
  assert.equal(result.snapshot.speaker_links[0].person_id, duplicates[1].id);
  const after = await api.invoke("meeting_detail", { id });
  assert.deepEqual(after.speakers, before.speakers);
  assert.equal(context.__PENNEL_FIXTURE__.calls.includes("remember_speaker"), false);
  assert.equal(context.__PENNEL_FIXTURE__.calls.includes("rename_speaker"), false);
  await assert.rejects(api.invoke("update_meeting_people", { id, request: { action: "unlink_speaker", expected_revision: people.revision,
    link_id: result.snapshot.speaker_links[0].id } }));
  people = await api.invoke("meeting_people_snapshot", { id });
  assert.equal(people.speaker_links.length, 1);
});

test("people fixture inserts only the chosen Markdown range and unlink preserves authored text", async () => {
  const context = fixture("meeting-transcript", { people_seeded: true, rich_utterances: true });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const original = "# 😀 Notes\r\n\r\n[plain Alex](https://example.invalid) and `@Alex` — HERE **kept**\r\n";
  await api.invoke("save_meeting_notes", { id, text: original });
  const before = await api.invoke("meeting_people_snapshot", { id });
  const start = new TextEncoder().encode(original.slice(0, original.indexOf("HERE"))).length;
  const result = await api.invoke("update_meeting_people", { id, request: { action: "insert_mention", expected_revision: before.revision,
    expected_notes_revision: before.notes_revision, person_id: before.people[0].id, visible_name: "Alex", notes: original, start_byte: start, end_byte: start + 4 } });
  const expected = original.replace("HERE", "@Alex");
  assert.equal(result.notes, expected);
  assert.equal((await api.invoke("meeting_detail", { id })).notes, expected);
  const unlink = await api.invoke("update_meeting_people", { id, request: { action: "unlink_mention", expected_revision: result.snapshot.revision,
    mention_id: result.snapshot.mentions[0].id } });
  assert.equal(unlink.snapshot.mentions.length, 0);
  assert.equal((await api.invoke("meeting_detail", { id })).notes, expected);
  await assert.rejects(api.invoke("update_meeting_people", { id, request: { action: "insert_mention", expected_revision: unlink.snapshot.revision,
    expected_notes_revision: before.notes_revision, person_id: before.people[0].id, visible_name: "Other", notes: original, start_byte: start, end_byte: start + 4 } }));
  assert.equal((await api.invoke("meeting_detail", { id })).notes, expected);
});

test("people fixture relinking requires the current note revision and an untouched mention anchor", async () => {
  const context = fixture("meeting-transcript", { people_seeded: true, rich_utterances: true });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_people_snapshot", { id }), notes = (await api.invoke("meeting_detail", { id })).notes;
  const end = new TextEncoder().encode(notes).length;
  const inserted = await api.invoke("update_meeting_people", { id, request: { action: "insert_mention", expected_revision: before.revision,
    expected_notes_revision: before.notes_revision, person_id: before.people[0].id, visible_name: "Alex", notes, start_byte: end, end_byte: end } });
  const request = { action: "relink_mention", expected_revision: inserted.snapshot.revision,
    mention_id: inserted.snapshot.mentions[0].id, person_id: before.people[1].id };
  await assert.rejects(api.invoke("update_meeting_people", { id, request }));
  await assert.rejects(api.invoke("update_meeting_people", { id, request: { ...request, expected_notes_revision: before.notes_revision } }));
  const relinked = await api.invoke("update_meeting_people", { id, request: { ...request, expected_notes_revision: inserted.snapshot.notes_revision } });
  assert.equal(relinked.snapshot.mentions[0].person_id, before.people[1].id);
  assert.equal((await api.invoke("meeting_detail", { id })).notes, inserted.notes);
  await api.invoke("save_meeting_notes", { id, text: `${inserted.notes}\nExternal edit.` });
  const changed = await api.invoke("meeting_people_snapshot", { id });
  await assert.rejects(api.invoke("update_meeting_people", { id, request: { ...request,
    expected_revision: changed.revision, expected_notes_revision: changed.notes_revision, person_id: before.people[0].id } }));
  assert.equal((await api.invoke("meeting_people_snapshot", { id })).mentions[0].person_id, before.people[1].id);
});

test("people fixture distinguishes written notes from pending mention confirmation without duplicate insertion", async () => {
  const context = fixture("meeting-transcript", { people_seeded: true, rich_utterances: true, people_confirmation_pending: true });
  const api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_people_snapshot", { id }), notes = (await api.invoke("meeting_detail", { id })).notes;
  const end = new TextEncoder().encode(notes).length;
  const written = await api.invoke("update_meeting_people", { id, request: { action: "insert_mention", expected_revision: before.revision,
    expected_notes_revision: before.notes_revision, person_id: before.people[0].id, visible_name: "Alex", notes, start_byte: end, end_byte: end } });
  assert.equal(written.notes, `${notes}@Alex`);
  assert.equal(written.snapshot.recovery_status, "confirmation_pending");
  const recovered = await api.invoke("meeting_people_snapshot", { id });
  assert.equal(recovered.recovery_status, "ready");
  assert.equal(recovered.mentions.length, 1);
  assert.equal((await api.invoke("meeting_detail", { id })).notes, `${notes}@Alex`);
});

test("people fixture can delay a durable mention response independently of its note write", async () => {
  const context = fixture("meeting-transcript", { people_seeded: true, rich_utterances: true });
  const api = context.__PENNEL_PLATFORM__, control = context.__PENNEL_FIXTURE__, id = control.options.meeting_id;
  const before = await api.invoke("meeting_people_snapshot", { id }), notes = (await api.invoke("meeting_detail", { id })).notes;
  const end = new TextEncoder().encode(notes).length;
  control.people.insertDelay(3000);
  let responded = false;
  const pending = api.invoke("update_meeting_people", { id, request: { action: "insert_mention", expected_revision: before.revision,
    expected_notes_revision: before.notes_revision, person_id: before.people[0].id, visible_name: "Alex", notes, start_byte: end, end_byte: end } })
    .then(result => { responded = true; return result; });
  try {
    for (let attempt = 0; attempt < 100 && !control.people.pendingInsertResponses(); attempt++) await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(control.people.pendingInsertResponses(), 1);
    assert.equal(control.workspace.notes(), `${notes}@Alex`);
    assert.equal(responded, false);
  } finally { control.people.releaseInserts(); }
  assert.equal((await pending).notes, `${notes}@Alex`);
  assert.equal(control.people.pendingInsertResponses(), 0);
});

test("every visual scenario has a unique stable identifier and valid entry", () => {
  assert.equal(new Set(cases.map(item => item.name)).size, cases.length);
  for (const item of cases) {
    assert.match(item.name, /^[a-z][a-z0-9-]+$/);
    assert.ok(["workspace", "settings", "history", "dictation", "onboarding", "pill"].includes(item.window));
    if (item.window === "pill") assert.ok(["listening", "hands_free", "command", "no_sound", "working", "inserted", "copied", "problem"].includes(item.pill_state));
    else assert.ok(item.section || item.step);
  }
});

test("live fixture freezes admitted notes and transcript through Stop and later appends", async () => {
  const context = fixture("recording", { configured_text_provider: true, live_question_manual: true });
  const api = context.__PENNEL_PLATFORM__;
  const control = context.__PENNEL_FIXTURE__.workspace;
  const status = await api.invoke("meeting_status");
  const snapshot = await api.invoke("begin_live_question", { requestId: "owned-question", meetingUid: status.meeting_uid,
    sessionId: status.session_id, expectedNotesText: status.notes, question: "What was decided?", history: [] });
  control.append({ start_ms: 22_000, end_ms: 28_000, speaker: "others", text: "Later synthetic statement.", failed: false, voice: 1 });
  await api.invoke("save_meeting_notes", { id: status.id, text: "Newer synthetic note" });
  await api.invoke("stop_meeting");
  let result;
  await api.listen("live-question-result", event => { result = event.payload; });
  control.complete(snapshot.request_id);
  assert.equal(control.recording(), false);
  assert.equal(snapshot.included_through_ms, 21_000);
  assert.equal(result.snapshot.request_id, "owned-question");
  assert.equal(result.snapshot.meeting_uid, status.meeting_uid);
  assert.ok(result.snapshot.anchors.every(anchor => !anchor.source_text.includes("Later synthetic") && !anchor.source_text.includes("Newer synthetic")));
});

test("selected live fixture declares exactly its sent anchors and freezes scope through append", async () => {
  const context = fixture("live-question-selected-scope-wide");
  const api = context.__PENNEL_PLATFORM__;
  const control = context.__PENNEL_FIXTURE__.workspace;
  const status = await api.invoke("meeting_status");
  const snapshot = await api.invoke("begin_live_question", { requestId: "owned-selected-question", meetingUid: status.meeting_uid,
    sessionId: status.session_id, expectedNotesText: status.notes, question: "Which evidence was sent?", history: [] });
  assert.equal(snapshot.selection.scope, "selected_excerpts");
  assert.equal(snapshot.selection.included_sources, snapshot.anchors.length);
  assert.ok(snapshot.selection.available_sources > snapshot.selection.included_sources);
  let delivered;
  await api.listen("live-question-result", event => { delivered = event.payload.snapshot; });
  control.append({ start_ms: 30_000, end_ms: 38_000, speaker: "others", text: "Later source.", failed: false, voice: null });
  control.complete(snapshot.request_id);
  assert.deepEqual(delivered.selection, snapshot.selection);
  assert.equal(delivered.included_through_ms, snapshot.included_through_ms);
});

test("live fixture refuses mismatched acknowledged notes and cancels without delivering late completion", async () => {
  const context = fixture("recording", { configured_text_provider: true, live_question_manual: true });
  const api = context.__PENNEL_PLATFORM__;
  const status = await api.invoke("meeting_status");
  const args = { requestId: "owned-cancel", meetingUid: status.meeting_uid, sessionId: status.session_id,
    expectedNotesText: status.notes, question: "Question", history: [] };
  await assert.rejects(api.invoke("begin_live_question", { ...args, expectedNotesText: "Unacknowledged" }));
  const snapshot = await api.invoke("begin_live_question", args);
  let delivered = false;
  await api.listen("live-question-result", () => { delivered = true; });
  await api.invoke("cancel_live_question", { requestId: snapshot.request_id, meetingUid: status.meeting_uid, sessionId: status.session_id });
  context.__PENNEL_FIXTURE__.workspace.complete(snapshot.request_id);
  assert.equal(delivered, false);
});

test("two-track preview fixtures stay out of question evidence and reconcile only their own chunk", async () => {
  const context = fixture("live-captions-wide", { configured_text_provider: true, live_question_manual: true });
  const api = context.__PENNEL_PLATFORM__;
  const control = context.__PENNEL_FIXTURE__.workspace;
  control.preview("me", { text: "Provisional microphone sentinel", chunk_start_ms: 21000, start_ms: 21000, end_ms: 24000 });
  control.preview("others", { text: "Provisional system sentinel", chunk_start_ms: 21000, start_ms: 21000, end_ms: 24000 });
  const status = await api.invoke("meeting_status");
  assert.equal(status.preview.tracks.length, 2);
  assert.equal(status.preview.transcript_revision, status.transcript_revision);
  const question = await api.invoke("begin_live_question", { requestId: "preview-question", meetingUid: status.meeting_uid,
    sessionId: status.session_id, expectedNotesText: status.notes, question: "Use finalized evidence only", history: [] });
  assert.equal(JSON.stringify(question).includes("sentinel"), false);
  control.preview("me", { text: "Newer microphone sentinel", chunk_start_ms: 24000, start_ms: 24000, end_ms: 27000 });
  control.finalizePreview("me", 21000, { start_ms: 21000, end_ms: 24000, speaker: "me", text: "Final microphone speech", failed: false, voice: null });
  const after = await api.invoke("meeting_status");
  assert.equal(after.preview.tracks.find(track => track.source === "me").text, "Newer microphone sentinel");
  assert.equal(after.preview.tracks.find(track => track.source === "others").text, "Provisional system sentinel");
  assert.equal(after.transcript.filter(segment => segment.text === "Final microphone speech").length, 1);
  await api.invoke("stop_meeting");
  assert.equal(JSON.stringify(await api.invoke("meeting_detail", { id: status.id })).includes("sentinel"), false);
});

test("durable fixture sources keep identity and content fingerprints through attribution revisions", async () => {
  const context = fixture("speaker-turns-live", { configured_text_provider: true, live_question_manual: true });
  const api = context.__PENNEL_PLATFORM__;
  const control = context.__PENNEL_FIXTURE__.workspace;
  const before = await api.invoke("meeting_status");
  assert.ok(before.transcript[0].utterance_id);
  const snapshot = await api.invoke("begin_live_question", { requestId: "durable-source", meetingUid: before.meeting_uid, sessionId: before.session_id,
    expectedNotesText: before.notes, question: "Where is the exact source?", history: [] });
  assert.equal(snapshot.anchors[0].segment_keys[0], `utterance:${before.transcript[0].utterance_id}`);
  control.attribution(0, { speaker: "me", voice: 3, attribution: { kind: "single", voices: [3] } });
  const after = await api.invoke("meeting_status");
  assert.equal(after.transcript[0].utterance_id, before.transcript[0].utterance_id);
  assert.equal(after.transcript[0].source_id, before.transcript[0].source_id);
  assert.equal(after.transcript[0].source_fingerprint, before.transcript[0].source_fingerprint);
  assert.equal(snapshot.anchors[0].segment_fingerprints[0], before.transcript[0].source_fingerprint);
});

test("rich fixture fingerprints preserve BOM while collapsing only Unicode White_Space", async () => {
  const context = fixture("speaker-turns-live", { configured_text_provider: true, live_question_manual: true });
  const control = context.__PENNEL_FIXTURE__.workspace;
  control.replaceTranscript([{ utterance_id: "u-bom", source_id: "microphone", start_ms: 0, end_ms: 1000, speaker: "others", voice: null,
    failed: false, text: "\u00a0\ufeffalpha\u2003beta\n😀\ufeff\u00a0" }]);
  const status = await context.__PENNEL_PLATFORM__.invoke("meeting_status");
  const tuple = ["u-bom", "microphone", 0, 1000, "\ufeffalpha beta 😀\ufeff", false];
  const expected = Buffer.from(await webcrypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(tuple)))).toString("hex");
  assert.equal(expected, "8a1840c80592f1692d9de9405f944333752b7b3fbb8e3d9674b03ceea425ff96");
  assert.equal(status.transcript[0].source_fingerprint, expected);
});

test("live speaker fixtures require explicit session selection and reset off for the next meeting", async () => {
  const context = fixture("live-speakers-controls", { configured_text_provider: true, live_question_manual: true });
  const api = context.__PENNEL_PLATFORM__;
  const control = context.__PENNEL_FIXTURE__.workspace;
  const before = await api.invoke("meeting_status");
  assert.equal(before.live_speaker_attribution.enabled, false);
  assert.equal(before.live_speaker_attribution.destination, "local");
  const enabled = await api.invoke("set_live_speaker_attribution", { sessionId: before.session_id, enabled: true });
  assert.equal(enabled.enabled, true);
  assert.equal(context.__PENNEL_FIXTURE__.calls.includes("save_settings"), false);
  const next = control.restart();
  assert.equal((await api.invoke("meeting_status")).live_speaker_attribution.enabled, false);
  await assert.rejects(api.invoke("set_live_speaker_attribution", { sessionId: before.session_id, enabled: true }));
  assert.notEqual(next.session_id, before.session_id);
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
  const displays = captions.filter(item => !item.steps?.length);
  for (const state of ["listening", "hands_free", "command", "no_sound", "working", "inserted", "copied", "problem"]) {
    assert.ok(displays.some(item => item.pill_state === state), `Missing ${state} pill display coverage`);
  }
  assert.ok(displays.some(item => item.pill_state === "listening" && !item.preview_text), "Missing empty listening display");
  for (const state of ["listening", "hands_free", "working"]) {
    assert.ok(displays.some(item => item.pill_state === state && item.preview_text), `Missing ${state} preview display`);
  }
  for (const name of ["pill-status-announcements", "pill-status-crossfade", "pill-preview-lifecycle", "pill-status-motion", "pill-status-motion-reduced"]) {
    const interaction = captions.find(item => item.name === name);
    assert.ok(interaction?.steps?.some(step => typeof step.eval === "string" && step.eval.trim()), `Missing executable ${name} interaction`);
    assert.equal(interaction.pill_state, "listening", `${name} must begin from a visible listening snapshot`);
  }
  assert.equal(captions.find(item => item.name === "pill-status-motion-reduced").reduced_motion, true);
  assert.notEqual(captions.find(item => item.name === "pill-status-motion").reduced_motion, true);
  for (const item of captions) {
    const context = fixture(item.name);
    const snapshot = await context.__PENNEL_PLATFORM__.invoke("dictation_state");
    assert.equal(snapshot.state, item.pill_state);
    for (const [field, value] of Object.entries(item.pill_fields || {})) assert.equal(snapshot[field], value);
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

test("saved library note writes persist by meeting without replacing active recording notes", async () => {
  const api = fixture("native-navigation-interaction").__PENNEL_PLATFORM__;
  const activeId = "2026-10-24 0900 Projectoverleg";
  const savedId = "2026-10-14 1400 Klantgesprek Studio Noord";
  const activeBefore = await api.invoke("meeting_detail", {id: activeId});
  const written = "Owned saved-meeting Markdown.\nIt must survive reading the document again.";
  await api.invoke("save_meeting_notes", {id: savedId, text: written});
  assert.equal((await api.invoke("meeting_detail", {id: savedId})).notes, written);
  assert.equal((await api.invoke("meeting_detail", {id: activeId})).notes, activeBefore.notes);
});

test("denied microphone fixture keeps recording repair reachable until access refreshes", async () => {
  const api = fixture("recording-capsule-blocked").__PENNEL_PLATFORM__;
  assert.equal((await api.invoke("app_info")).capabilities.microphone.status, "permission_required");
  await api.invoke("request_permission", { kind: "microphone" });
  assert.equal((await api.invoke("app_info")).capabilities.microphone.status, "available");
});

test("summary consent fixture mirrors durable per-meeting dismissal without changing global policy", async () => {
  const storage = reviewStorage(), options = { owner_review: true, profile: "summary-consent-A", summary_empty: true, summary_automation: "unanswered" };
  const context = fixture("meeting-document-minimum", options, storage), api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  assert.equal(await api.invoke("meeting_summary_prompt_dismissed", { id }), false);
  const before = await api.invoke("meeting_summary", { id });
  const saved = await api.invoke("dismiss_meeting_summary_prompt", { id, expectedRevision: before.revision, expectedSourceRevision: before.source_revision });
  assert.equal(saved.markdown, before.markdown); assert.notEqual(saved.revision, before.revision);
  assert.equal((await api.invoke("app_info")).settings.summary_automation, "unanswered");
  assert.equal(await fixture("meeting-document-minimum", options, storage).__PENNEL_PLATFORM__.invoke("meeting_summary_prompt_dismissed", { id }), true);
  assert.equal(await fixture("meeting-document-minimum", { ...options, profile: "summary-consent-B" }, storage).__PENNEL_PLATFORM__.invoke("meeting_summary_prompt_dismissed", { id }), false);
  assert.ok(!context.__PENNEL_FIXTURE__.calls.some(command => ["summarize_meeting", "generate_meeting_summary", "save_settings"].includes(command)));
});

test("summary consent fixture direct create saves only the expected empty unedited body", async () => {
  const context = fixture("meeting-document-minimum", { owner_review: true, summary_empty: true, summary_automation: "unanswered" }), api = context.__PENNEL_PLATFORM__, id = context.__PENNEL_FIXTURE__.options.meeting_id;
  const before = await api.invoke("meeting_summary", { id }); assert.equal(before.edited, false);
  await api.invoke("summarize_meeting", { id, templateId: "algemeen", expectedRevision: before.revision, expectedSourceRevision: before.source_revision });
  const saved = await api.invoke("meeting_summary", { id }); assert.ok(saved.markdown.trim()); assert.equal(saved.edited, false);
  await assert.rejects(api.invoke("summarize_meeting", { id, templateId: "algemeen", expectedRevision: saved.revision, expectedSourceRevision: saved.source_revision }));
  assert.equal((await api.invoke("meeting_summary", { id })).markdown, saved.markdown);
  assert.equal((await api.invoke("app_info")).settings.summary_automation, "unanswered");
});
