/** Run only in the owned loopback fixture server; no native windows or user data are used. */
import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
function ownedFixture() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) {
    throw new Error("Unified workspace checks require the owned synthetic fixture server.");
  }
  return window.__PENNEL_FIXTURE__;
}

const assert = (value, message) => { if (!value) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 125; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 40));
  }
  throw new Error(message);
}
function visible(node) {
  if (!node?.getClientRects().length || node.closest("[hidden]")) return false;
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    if (parent.tagName === "DETAILS" && !parent.open && !parent.querySelector("summary")?.contains(node)) return false;
  }
  return true;
}
function click(node) {
  assert(visible(node) && !node.disabled, "The requested workspace action is unavailable.");
  node.click();
}
function edit(node, value) {
  assert(node && !node.disabled, "The owned fixture has no editable field.");
  node.focus(); if (node.applyUserChange) { node.applyUserChange(value); return; } node.value = value;
  node.dispatchEvent(new Event("input", { bubbles: true }));
}
function observeCommands() {
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const operations = [];
  client.invoke = async (command, args = {}) => {
    const operation = { command, args: structuredClone(args) };
    operations.push(operation);
    const value = await original(command, args);
    if (command === "open_dictation_session") operation.result = structuredClone(value);
    return value;
  };
  return { operations, restore: () => { client.invoke = original; } };
}
function section(name) {
  return document.querySelector(`.library-sidebar [data-workspace-section="${name}"]`);
}
function assertMain(owner, operations) {
  assert(document === owner && document.body.dataset.window === "workspace", "A product section changed its owning window.");
  assert(!operations.some(operation => ["show_window", "show_workspace"].includes(operation.command)), "Local navigation requested a separate window.");
}
async function showPad() {
  click(section("dictation"));
  await until(() => document.querySelector(".dictation-home") || document.querySelector("textarea.dictation-pad"), "Dictation did not open inside main.");
  if (!document.querySelector("textarea.dictation-pad")) click(document.querySelector(".dictation-pad-option button"));
  await until(() => document.querySelector("textarea.dictation-pad:not(:disabled)"), "The editable dictation pad did not become ready.");
  return document.querySelector("textarea.dictation-pad");
}

export async function checkUnifiedWorkspaceFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  await until(() => document.querySelector(".live-workspace"), "Use the saved meeting-notes fixture.");
  const owner = document;
  const id = fixture.options.meeting_id;
  const beforeMeeting = await api.meetingDetail(id);
  const beforePad = await api.padText();
  const originalTitle = document.querySelector(".live-workspace .live-title").value;
  const observed = observeCommands();
  try {
    for (const name of ["dictation", "meetings"]) assert(visible(section(name)), `The ${name} section is not discoverable.`);
    assert(!visible(section("history")), "History must stay nested under Dictation.");
    selectMeetingPane(document.querySelector(".live-workspace"), "notes"); await settle();
    const meetingNote = "Owned meeting draft during unified navigation. This is synthetic text.";
    edit(document.querySelector(".live-notes"), meetingNote);

    const pad = await showPad();
    const padText = "Owned draft in the unified Pennel pad.\nThis text must survive section switches.";
    edit(pad, padText);
    click(section("history"));
    await until(() => document.querySelector(".history-view"), "History did not open inside main.");
    assert((await api.padText()) === padText, "Leaving the pad lost its pending text.");
    assert((await api.meetingDetail(id)).notes === meetingNote, "Leaving the meeting lost its draft.");

    const target = (await api.history(200)).find(entry => entry.status === "done");
    assert(target, "The owned History fixture has no retained result.");
    fixture.emit("workspace-navigate", { section: "history", tab: `entry:${target.id}`, meeting_id: null, jump_ms: null });
    await until(() => document.querySelector(".history-target")?.dataset.historyId === target.id, "Exact History recovery lost its requested result.");
    await until(() => document.querySelector(".history-target")?.contains(document.activeElement), "Exact History recovery did not reveal the retained result.");
    const copied = document.querySelector('.history-target [data-history-action="copy"]');
    assert(copied, "The retained result has no copy action."); copied.focus(); await settle();
    click(document.querySelector('[data-dictation-section="pad"]'));
    await until(() => document.querySelector("textarea.dictation-pad"), "History could not return to the saved pad.");
    click(section("history"));
    await until(() => document.querySelector('.history-selected[aria-current="true"]')?.dataset.historyId === target.id, "Returning to History lost the selected result.");
    await until(() => document.activeElement?.closest("[data-history-id]")?.dataset.historyId === target.id
      && document.activeElement?.dataset.historyAction === "copy", "Returning to History lost its selected action.");
    const other = (await api.history(200)).find(entry => entry.status === "done" && entry.id !== target.id);
    assert(other, "The owned fixture needs a second retained result.");
    fixture.emit("workspace-navigate", { section: "history", tab: `entry:${other.id}`, meeting_id: null, jump_ms: null });
    await until(() => document.querySelector(".history-target")?.dataset.historyId === other.id, "A fresh recovery target did not override remembered History selection.");

    click(section("meetings"));
    await until(() => document.querySelector(".live-workspace .live-title")?.value === originalTitle, "Meetings did not restore the previous meeting.");
    assert(document.querySelector(".live-notes").value === meetingNote, "Returning to Meetings lost the personal note.");
    click(section("dictation"));
    await until(() => document.querySelector(".history-view"), "Returning to Dictation forgot its last History surface.");
    click(document.querySelector('[data-dictation-section="pad"]'));
    await until(() => document.querySelector("textarea.dictation-pad"), "Write in Pennel did not restore the existing pad.");
    assert(document.querySelector("textarea.dictation-pad").value === padText, "Returning to Dictation lost its draft.");
    assert(section("dictation").getAttribute("aria-current") === "page", "The active Dictation section has no navigation state.");

    click(document.querySelector(".library-settings"));
    await until(() => document.querySelector(".settings-workspace"), "Settings did not open in the shared workspace.");
    click(document.querySelector(".settings-back"));
    await until(() => document.querySelector("textarea.dictation-pad"), "Settings Back did not restore Dictation's pad.");
    assert(document.querySelector("textarea.dictation-pad").value === padText, "The Settings roundtrip lost pad text.");

    click(section("history"));
    await until(() => document.querySelector(".history-view"), "The History section did not reopen.");
    assert(section("history").getAttribute("aria-current") === "page", "The active History section has no navigation state.");
    click(document.querySelector(".library-settings"));
    await until(() => document.querySelector(".settings-workspace"), "History Settings did not stay in main.");
    click(document.querySelector(".settings-back"));
    await until(() => document.querySelector(".history-view"), "Settings Back did not restore History.");
    assertMain(owner, observed.operations);
    assert(!observed.operations.some(operation => ["start_meeting", "stop_meeting", "toggle_dictation", "toggle_dictation_session"].includes(operation.command)), "Browsing product sections started or stopped capture.");
    assert(!fixture.unknown.length, "Unified navigation used an unsupported fixture operation.");
    return { sameMainWindow: true, padDraftPreserved: true, meetingPreserved: true, historyTargetPreserved: true, sectionMemory: true, settingsReturn: true, externalServicesUsed: false, userDataUsed: false };
  } finally {
    await api.savePadText(beforePad);
    await api.saveMeetingNotes(id, beforeMeeting.notes);
    observed.restore();
  }
}

export async function checkUnifiedWorkspaceRecordingFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  await until(() => document.querySelector(".live-notes"), "Use the active recording fixture.");
  const owner = document;
  const before = await api.meetingStatus();
  assert(before.recording, "The recording fixture is not active.");
  const observed = observeCommands();
  try {
    const title = "Owned recording across the shared workspace";
    const notes = "Owned active recording notes. This content is synthetic.";
    edit(document.querySelector(".live-title"), title);
    edit(document.querySelector(".live-notes"), notes);
    click(section("dictation"));
    await until(() => document.querySelector(".dictation-home") || document.querySelector("textarea.dictation-pad"), "Dictation did not share the recording's main window.");
    click(section("history"));
    await until(() => document.querySelector(".history-view"), "History did not share the recording's main window.");
    const during = await api.meetingStatus();
    assert(during.recording && during.session_id === before.session_id && during.id === before.id, "Browsing replaced or stopped the active recording.");
    assert(during.title === title && during.notes === notes, "Browsing lost the recording title or notes.");

    click(document.querySelector(".library-record"));
    await until(() => document.querySelector(".live-notes"), "The active-recording action did not return to capture.");
    assert(document.querySelector(".live-title").value === title && document.querySelector(".live-notes").value === notes, "Returning to capture lost its drafts.");
    assertMain(owner, observed.operations);
    assert(!observed.operations.some(operation => ["start_meeting", "stop_meeting", "toggle_dictation", "toggle_dictation_session"].includes(operation.command)), "Product navigation restarted or stopped capture.");
    assert(!fixture.unknown.length, "Unified recording navigation used an unsupported fixture operation.");
    return { sameMainWindow: true, sameRecordingSession: true, draftsPreserved: true, noCaptureRestart: true, externalServicesUsed: false, userDataUsed: false };
  } finally {
    await api.setMeetingTitle(before.title ?? "", before.session_id);
    await api.saveMeetingNotes(before.id, before.notes);
    observed.restore();
  }
}

export async function checkUnifiedPadNavigationGuard() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  const owner = document;
  const before = await api.padText();
  const observed = observeCommands();
  let sessionId;
  try {
    const pad = await showPad();
    const text = "Owned draft before a guarded section switch.";
    edit(pad, text);
    const start = [...document.querySelectorAll(".pad-workspace .page-heading button")].find(node => node.textContent.trim() === t("dictation.pad.start"));
    click(start);
    await until(() => observed.operations.some(operation => operation.command === "open_dictation_session" && operation.result?.kind === "pad"), "The owned pad session was not created.");
    sessionId = observed.operations.filter(operation => operation.command === "open_dictation_session" && operation.result?.kind === "pad").at(-1).result.session_id;
    await until(async () => (await api.getDictationSession(sessionId)).phase === "listening", "The owned fixture did not begin listening.");
    click(section("history")); await settle();
    await until(() => document.querySelector(".pad-status")?.textContent.includes(t("dictation.pad.finish_before_leaving")), "The busy pad did not explain how to leave safely.");
    assert(document.querySelector("textarea.dictation-pad") && !document.querySelector(".history-view"), "Navigation abandoned a listening pad.");
    assert((await api.getDictationSession(sessionId)).phase === "listening", "The attempted switch silently cancelled capture.");
    click([...document.querySelectorAll(".pad-actions button")].find(node => node.textContent.trim() === t("common.cancel")));
    await until(async () => (await api.getDictationSession(sessionId)).phase === "idle", "Explicit cancellation did not finish the owned session.");
    click(section("history"));
    await until(() => document.querySelector(".history-view"), "The pad stayed trapped after explicit cancellation.");
    click(document.querySelector('[data-dictation-section="pad"]'));
    await until(() => document.querySelector("textarea.dictation-pad:not(:disabled)"), "The saved pad did not reopen after cancellation.");
    assert(document.querySelector("textarea.dictation-pad").value === text, "Guarded navigation lost the previously typed draft.");
    assertMain(owner, observed.operations);
    assert(!fixture.unknown.length, "The guarded pad used an unsupported fixture operation.");
    return { sameMainWindow: true, activeSessionRetained: true, cancellationExplicit: true, draftPreserved: true, externalServicesUsed: false, userDataUsed: false };
  } finally {
    if (sessionId) { try { await api.cancelDictationSession(sessionId); } catch {} }
    await api.savePadText(before);
    observed.restore();
  }
}
