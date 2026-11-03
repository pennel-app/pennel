const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 30) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 200; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}
function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture server.");
  return window.__PENNEL_FIXTURE__;
}
function edit(input, text) {
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}
function tab(name) { document.querySelector(`[data-live-tab="${name}"]`)?.click(); }

export async function checkLiveConcurrentEditing() {
  const fixture = owned();
  await until(() => document.querySelector(".live-workspace .live-notes"), "The live workspace did not open.");
  const root = document.querySelector(".live-workspace");
  const note = root.querySelector(".live-notes");
  edit(note, "# Synthetic notes\n\nThe saved decision is Monday.\n".repeat(12));
  tab("questions");
  const question = root.querySelector(".live-question-input");
  edit(question, "What was decided?");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace?.requests().length === 1, "The question was not admitted.");
  const frozen = fixture.workspace.requests()[0];
  tab("notes");
  note.focus();
  note.setSelectionRange(17, 26, "backward");
  note.scrollTop = 72;
  note.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
  note.setRangeText("samengesteld", note.selectionStart, note.selectionEnd, "select");
  note.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, inputType: "insertCompositionText", data: "samengesteld" }));
  const before = { value: note.value, start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection, scroll: note.scrollTop };
  fixture.workspace.append({ start_ms: 22_000, end_ms: 28_000, speaker: "others", text: "Later synthetic speech must not enter the old answer.", failed: false, voice: 1 });
  fixture.workspace.complete(frozen.request_id);
  await until(() => root.textContent.includes("Synthetic frozen answer"), "The frozen answer did not arrive.");
  await pause(1100);
  assert(document.querySelector(".live-notes") === note, "An asynchronous update replaced the editor.");
  assert(note.value === before.value && note.selectionStart === before.start && note.selectionEnd === before.end && note.selectionDirection === before.direction && note.scrollTop === before.scroll, "Concurrent updates changed composition, selection, text or scroll.");
  assert(frozen.included_through_ms === 21_000, "The question included later transcript rows.");
  note.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "samengesteld" }));
  note.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "samengesteld" }));
  await until(() => fixture.workspace.notes() === before.value, "Completed composition was not saved.");
  return { sameEditor: true, compositionRetained: true, sourceSnapshotFrozen: true, syntheticOnly: true };
}

export async function checkLiveStopWithFailedSave() {
  const fixture = owned();
  await until(() => document.querySelector(".live-notes"), "The live workspace did not open.");
  const note = document.querySelector(".live-notes");
  fixture.workspace.failSaves(true);
  edit(note, "Synthetic draft retained after a failed write.");
  document.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording(), "A failed note save kept capture running.");
  assert(document.querySelector(".live-notes") === note && note.value.includes("retained after a failed write"), "Stop discarded or replaced the dirty editor.");
  fixture.workspace.failSaves(false);
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  assert(fixture.workspace.notes() === note.value, "The stopped meeting could not recover its note save.");
  return { captureStoppedBeforeSave: true, dirtyDraftRetained: true, retrySaved: true, syntheticOnly: true };
}

export async function checkLiveSettingsRetention() {
  const fixture = owned();
  await until(() => document.querySelector(".live-notes"), "The live workspace did not open.");
  const note = document.querySelector(".live-notes");
  fixture.workspace.failSaves(true);
  edit(note, "Synthetic unsaved notes across Settings.");
  note.setSelectionRange(3, 14, "backward");
  document.querySelector(".library-settings").click();
  await until(() => document.querySelector(".settings-back"), "Failed note storage blocked Settings.");
  document.querySelector(".settings-back").click();
  await until(() => document.querySelector(".live-notes"), "Settings did not return to the meeting.");
  assert(document.querySelector(".live-notes") === note && note.value === "Synthetic unsaved notes across Settings.", "Settings replaced the retained draft editor.");
  assert(note.selectionStart === 3 && note.selectionEnd === 14, "Settings lost note selection.");
  fixture.workspace.failSaves(false);
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  return { sameEditor: true, failedSaveDoesNotBlockSettings: true, syntheticOnly: true };
}

export async function checkLiveFinalizationFocus() {
  const fixture = owned();
  const client = window.__PENNEL_PLATFORM__;
  await until(() => document.querySelector(".live-notes"), "The live workspace did not open.");
  const note = document.querySelector(".live-notes");
  const root = document.querySelector(".live-workspace");
  const original = client.invoke.bind(client);
  let releaseRead;
  const pending = new Promise(resolve => { releaseRead = resolve; });
  let reading = false;
  client.invoke = async (command, args) => {
    const result = await original(command, args);
    if (command === "meeting_status" && !reading) { reading = true; await pending; }
    return result;
  };
  try {
    fixture.workspace.append({ start_ms: 22_000, end_ms: 28_000, speaker: "others", text: "Final synthetic chunk.", failed: false, voice: 1 });
    await until(() => reading, "The delayed transcript read did not begin.");
    note.focus();
    note.setSelectionRange(2, 9, "backward");
    note.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    await original("stop_meeting");
    await until(() => root.dataset.recording === "false", "Finalization did not reach the retained workspace.");
    await pause(100);
    assert(document.activeElement === note && document.querySelector(".live-workspace") === root, "Finalization detached the focused composing editor.");
    releaseRead();
    await pause(700);
    assert(root.dataset.recording === "false", "A late transcript read resurrected recording.");
    fixture.emit("meeting-updated", fixture.options.meeting_id);
    await pause(100);
    assert(document.activeElement === note && note.selectionStart === 2 && note.selectionEnd === 9 && note.selectionDirection === "backward", "Review refresh lost composition focus or selection.");
    note.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    return { compositionFocusAcrossFinalization: true, lateStatusSuppressed: true, sameRoot: true, syntheticOnly: true };
  } finally { releaseRead(); client.invoke = original; }
}

export async function checkLiveToolbarStopSessions() {
  const fixture = owned();
  await until(() => document.querySelector(".live-workspace"), "The live workspace did not open.");
  const original = await window.__PENNEL_PLATFORM__.invoke("meeting_status");
  document.querySelector(".library-settings").click();
  await until(() => document.querySelector(".settings-back"), "Settings did not open.");
  const globalStop = document.querySelector(".toolbar-capture-stop");
  await until(() => !globalStop.hidden && !globalStop.disabled, "Settings has no available capture Stop.");
  globalStop.click();
  await until(() => !fixture.workspace.recording(), "The first global Stop did not stop capture.");
  const next = fixture.workspace.restart();
  fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The next meeting did not open.");
  fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: original.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === original.session_id, "The original meeting did not reopen.");
  await until(() => !globalStop.hidden && !globalStop.disabled, "Reviewing the old workspace hid or disabled Stop for the next meeting.");
  globalStop.click();
  await until(() => !fixture.workspace.recording(), "The next session could not use global Stop.");
  return { oldWorkspaceKeepsActiveStop: true, stopReenabledForNextSession: true, syntheticOnly: true };
}

export async function checkLiveDuplicateNotesReview() {
  const fixture = owned();
  await until(() => document.querySelector(".live-notes"), "The live workspace did not open.");
  const root = document.querySelector(".live-workspace");
  const note = root.querySelector(".live-notes");
  const repeated = "Repeated synthetic note.";
  edit(note, repeated + "\n" + repeated + "\n");
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  tab("questions");
  edit(root.querySelector(".live-question-input"), "Which repeated note supports this answer?");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length === 1, "The question did not start.");
  const snapshot = fixture.workspace.requests()[0];
  const index = snapshot.anchors.findIndex(anchor => anchor.passage.kind === "notes" && anchor.notes_start === repeated.length + 1);
  assert(index >= 0, "The second repeated note has no distinct source anchor.");
  fixture.emit("live-question-result", { snapshot, answer: { text: `The second note [${index + 1}]`, citations: [{ number: index + 1, passage: snapshot.anchors[index].passage }] }, error: null });
  await until(() => root.querySelector(".citation"), "The repeated note citation did not appear.");
  root.querySelector(".citation").click();
  await until(() => note.selectionStart === repeated.length + 1, "The second note citation opened the first occurrence.");
  assert(note.selectionEnd === repeated.length * 2 + 1, "The repeated note source range is wrong.");
  root.querySelector(".live-source-back").click();
  await window.__PENNEL_PLATFORM__.invoke("stop_meeting");
  await until(() => !root.querySelector(".live-review").hidden, "The finished workspace has no review/export route.");
  root.querySelector(".live-review").click();
  await until(() => document.querySelector(".meeting-detail .meeting-footer .share-menu"), "Existing review/export controls are unavailable.");
  const details = document.querySelector(".own-notes");
  details.open = true;
  const changed = "Acknowledged edits made in the existing review editor.";
  edit(details.querySelector("textarea"), changed);
  document.querySelector(".meeting-workspace-return").click();
  await until(() => document.querySelector(".live-workspace") === root, "Review did not return to the retained workspace.");
  assert(root.querySelector(".live-notes") === note && note.value === changed, "Review lost workspace identity or its confirmed edits.");
  return { exactDuplicateSource: true, existingReviewAndExport: true, acknowledgedReviewEditsRetained: true, syntheticOnly: true };
}
