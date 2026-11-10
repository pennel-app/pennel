/** Run only in the owned loopback fixture server; no native windows or user data are used. */
import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
function ownedFixture() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) {
    throw new Error("Inline Settings checks require the owned synthetic fixture server.");
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
  throw new Error(typeof message === "function" ? message() : message);
}
function visible(node) {
  if (!node?.getClientRects().length || node.closest("[hidden]")) return false;
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    if (parent.tagName === "DETAILS" && !parent.open && !parent.querySelector("summary")?.contains(node)) return false;
  }
  return true;
}
function click(node) {
  assert(visible(node) && !node.disabled, "The requested inline Settings action is unavailable.");
  node.click();
}
function button(owner, label) {
  return [...owner.querySelectorAll("button")].find(node => node.textContent.trim() === label);
}
function edit(node, value) {
  assert(node && !node.disabled, "The fixture has no editable note or title.");
  node.focus(); node.value = value;
  node.dispatchEvent(new Event("input", { bubbles: true }));
}
function observeCommands({ longTranscript = false } = {}) {
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const operations = [];
  client.invoke = async (command, args = {}) => {
    operations.push({ command, args: structuredClone(args) });
    const value = await original(command, args);
    if (longTranscript && command === "meeting_detail" && Array.isArray(value.transcript)) {
      return { ...value, transcript: Array.from({ length: 24 }, (_, index) => value.transcript.map(segment => ({
        ...segment, start_ms: segment.start_ms + index * 30_000, end_ms: segment.end_ms + index * 30_000,
      }))).flat() };
    }
    return value;
  };
  return { operations, restore: () => { client.invoke = original; } };
}
function assertNoWindowHop(operations) {
  assert(!operations.some(operation => ["show_window", "show_workspace"].includes(operation.command)), "Local Settings navigation opened or requested another window.");
}
function settingsShortcut(platform) {
  window.dispatchEvent(new KeyboardEvent("keydown", {
    key: ",", code: "Comma", metaKey: platform === "macos", ctrlKey: platform !== "macos", bubbles: true, cancelable: true,
  }));
}
function settingsRoute(tab) {
  return { section: "settings", tab, meeting_id: null, jump_ms: null };
}

export async function checkInlineSettingsFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".live-workspace"), "Use the saved meeting-notes fixture.");
  const owner = document;
  const id = fixture.options.meeting_id;
  const before = await api.meetingDetail(id);
  const title = document.querySelector(".live-workspace .live-title").value;
  const observed = observeCommands({ longTranscript: true });
  try {
    // A longer owned transcript makes reading-position restoration observable.
    fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: id, jump_ms: null });
    await until(() => document.querySelectorAll('.live-transcript .transcript-line').length >= before.transcript.length * 24, "The prepared transcript did not refresh in its retained workspace.");
    const transcriptTab = () => document.querySelector('.live-workspace [role="tab"][data-live-tab="transcript"]');
    selectMeetingPane(document.querySelector(".live-workspace"), "notes"); await settle();
    const note = document.querySelector(".live-notes");
    const literal = "Owned inline Settings draft. This is synthetic test text.";
    edit(note, literal); note.setSelectionRange(6, 18);
    click(transcriptTab()); await settle();
    const panel = document.querySelector(".live-workspace .live-transcript");
    assert(panel.scrollHeight > panel.clientHeight, "The prepared transcript needs a scrollable reading area.");
    panel.scrollTop = Math.min(120, panel.scrollHeight - panel.clientHeight); await settle();
    const readingPosition = panel.scrollTop;
    const operationStart = observed.operations.length;

    click(document.querySelector(".library-settings"));
    await until(() => document.querySelector(".settings-workspace") && visible(document.querySelector(".settings-categories")), "Settings did not open inside the main window.");
    assert(document === owner && document.body.dataset.window === "workspace", "Settings changed the owning workspace.");
    assert(!visible(document.querySelector(".library-sidebar")), "The library sidebar competes with inline Settings navigation.");
    assert((await api.meetingDetail(id)).notes === literal, "Entering Settings lost the unsaved meeting draft.");
    assert(visible(document.querySelector('.settings-back[data-action="back-to-work"]')), "Settings lacks a visible way back to the prior task.");

    click(button(document.querySelector(".settings-categories"), t("navigation.appearance")));
    const appearanceReady = () => button(document.querySelector(".settings-categories"), t("navigation.appearance"))?.getAttribute("aria-current") === "page"
      && [...document.querySelectorAll(".settings-pane .choice[aria-label]")].some(node => node.getAttribute("aria-label").startsWith(t("interface.language") + ":"));
    await until(appearanceReady, "The appearance category did not finish opening.");
    click(document.querySelector(".settings-back"));
    await until(() => transcriptTab()?.getAttribute("aria-selected") === "true", "Returning from Settings lost the selected Transcript view.");
    await until(() => Math.abs(document.querySelector(".live-transcript").scrollTop - readingPosition) <= 1, "Returning from Settings lost the reading position.");
    assert(document.querySelector(".live-workspace .live-title").value === title, "Returning selected a different meeting.");
    selectMeetingPane(document.querySelector(".live-workspace"), "notes"); await settle();
    const restored = document.querySelector(".live-notes");
    assert(restored.value === literal && !document.querySelector('[data-live-pane="notes"]').hidden, "Returning from Settings lost the note or its selected pane.");
    assert(restored.selectionStart === 6 && restored.selectionEnd === 18, "Returning from Settings lost the note selection.");

    settingsShortcut(fixture.info().platform);
    await until(() => document.querySelector(".settings-workspace"), "The standard Settings shortcut did not navigate inline.");
    await until(appearanceReady, () => `Reopening Settings forgot its category: ${document.querySelector('.settings-categories [aria-current="page"]')?.dataset.settingsCategory ?? "none"}.`);
    fixture.emit("workspace-navigate", settingsRoute("models"));
    await until(() => document.querySelector(".models-settings"), "A Settings deep link did not open in the main window.");
    click(document.querySelector(".settings-back"));
    await until(() => document.querySelector('.live-workspace[data-document-layout="continuous"] .live-notes-pane:not([hidden])'), "A category deep link overwrote the previous task.");
    assert(document.querySelector(".live-notes").value === literal, "The deep-link roundtrip lost the note.");
    assertNoWindowHop(observed.operations.slice(operationStart));
    assert(!observed.operations.some(operation => ["start_meeting", "stop_meeting"].includes(operation.command)), "Settings navigation started or stopped recording.");
    assert(!fixture.unknown.length, "Inline Settings used an unsupported fixture operation.");
    return { sameMainWindow: true, categoryRemembered: true, meetingPreserved: true, draftPreserved: true, tabPreserved: true, readingPositionPreserved: true, noteSelectionPreserved: true, shortcutAndDeepLinkInline: true, externalServicesUsed: false, userDataUsed: false };
  } finally {
    await api.saveMeetingNotes(id, before.notes);
    observed.restore();
  }
}

export async function checkInlineSettingsRecordingFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".live-notes"), "Use the active recording fixture.");
  const before = await api.meetingStatus();
  assert(before.recording, "The recording fixture is not active.");
  const observed = observeCommands();
  try {
    const literal = "Owned active recording note for an inline Settings detour.";
    const title = "Owned recording title";
    edit(document.querySelector(".live-title"), title);
    edit(document.querySelector(".live-notes"), literal);
    click(document.querySelector(".library-settings"));
    await until(() => document.querySelector(".settings-workspace"), "Recording Settings did not stay in the main window.");
    await until(() => visible(document.querySelector(".settings-recording")), "Inline Settings hides the active recording state.");
    assert(document.querySelector(".settings-recording").textContent.includes(t("meetings.return_recording")), "Inline Settings has no useful active-recording return action.");
    fixture.emit("workspace-navigate", settingsRoute("recording"));
    await until(() => document.querySelector('[data-settings-category="meeting-settings"][aria-current="page"]') && document.querySelector('[data-settings-detail="recording"][open]'), "The recording preference subsection is not available.");
    click(document.querySelector(".settings-recording"));
    await until(() => document.querySelector(".live-notes"), "The visible recording action did not return to the same active task.");
    click(document.querySelector(".library-settings"));
    await until(() => document.querySelector(".settings-workspace"), "Settings could not reopen during recording.");
    click(document.querySelector(".settings-back"));
    await until(() => document.querySelector(".live-notes"), "Returning from Settings did not restore the active recording.");
    const after = await api.meetingStatus();
    assert(after.recording && after.id === before.id && after.session_id === before.session_id, "Settings replaced or stopped the active recording.");
    assert(document.querySelector(".live-notes").value === literal, "Settings lost the active recording note.");
    assert(document.querySelector(".live-title").value === title, "Settings lost the active recording title.");
    assertNoWindowHop(observed.operations);
    assert(!observed.operations.some(operation => ["start_meeting", "stop_meeting"].includes(operation.command)), "The Settings detour started or stopped capture.");
    assert(!fixture.unknown.length, "Recording Settings used an unsupported fixture operation.");
    return { sameMainWindow: true, sameRecording: true, notePreserved: true, titlePreserved: true, noCaptureRestart: true, externalServicesUsed: false, userDataUsed: false };
  } finally {
    await api.setMeetingNotes(before.notes, before.session_id);
    if (before.title != null) await api.setMeetingTitle(before.title, before.session_id);
    observed.restore();
  }
}
