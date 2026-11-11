import { replaceNotesSelection } from "./meeting-workspace-test-controls.mjs";
import { selectMeetingPane, openMeetingTools } from "./meeting-workspace-test-controls.mjs";
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}
function edit(input, value) {
  if (input.applyUserChange) { input.applyUserChange(value); return; } input.value = value; input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value })); }
function tab(root, name) { selectMeetingPane(root, name); }
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  await until(() => document.querySelector(".live-workspace .live-notes"), "The live workspace did not mount.");
  const root = document.querySelector(".live-workspace"); tab(root, "transcript");
  return { fixture: window.__PENNEL_FIXTURE__, root, toggle: root.querySelector("[data-live-speaker-toggle]") };
}
async function ask(root, fixture) {
  const count = fixture.workspace.requests().length;
  tab(root, "questions"); edit(root.querySelector(".live-question-input"), "What does the finalized source say?");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length > count, "Ordinary questions did not remain available.");
  return fixture.workspace.requests().at(-1);
}

export async function checkLiveSpeakerControls() {
  const { fixture, root, toggle } = await owned();
  await openMeetingTools(root, "speakers");
  const { t } = await import("/src/i18n.ts");
  assert(toggle && !toggle.checked && !toggle.disabled, "Live speakers did not start off with an explicit available control.");
  const configuration = JSON.stringify((await window.__PENNEL_PLATFORM__.invoke("app_info")).settings);
  fixture.workspace.liveSpeakerDelay(350);
  toggle.click();
  assert(toggle.disabled && !toggle.checked && root.querySelector(".live-speaker-controls").getAttribute("aria-busy") === "true", "The pending toggle was not acknowledged separately.");
  toggle.dispatchEvent(new Event("change", { bubbles: true }));
  assert(fixture.calls.filter(command => command === "set_live_speaker_attribution").length === 1, "A duplicate toggle started another request.");
  await until(() => toggle.checked && !toggle.disabled, "Explicit live attribution did not become enabled.");
  assert(root.querySelector(".live-speaker-destination").textContent.includes(t("meetings.live_speakers_destination_local_short")), "The processing destination is missing.");
  const request = await ask(root, fixture);
  const first = root.querySelector('[data-segment-key="utterance:qa-utterance-0"]');
  const microphone = root.querySelector('[data-segment-key="utterance:qa-utterance-1"]');
  const sources = [first.dataset.sourceId, microphone.dataset.sourceId];
  const text = [first.querySelector(".transcript-copy").textContent, microphone.querySelector(".transcript-copy").textContent];
  tab(root, "notes");
  const note = root.querySelector(".live-notes"); note.focus(); note.setSelectionRange(2, 10, "backward");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  replaceNotesSelection(note, "samengesteld");
  const before = { value: note.value, start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection, scroll: note.scrollTop };
  fixture.workspace.attribution(0, { attribution: { kind: "single", voices: [2], provisional: true } });
  fixture.workspace.attribution(1, { attribution: { kind: "unknown", voices: [], provisional: true } });
  await until(() => !first.querySelector(".live-speaker-provisional").hidden && !microphone.querySelector(".live-speaker-provisional").hidden, "Both physical tracks did not show provisional attribution.");
  assert(first.querySelector(".speaker").textContent === t("meetings.speaker_number", { number: 2 }) && microphone.querySelector(".speaker").textContent === t("meetings.attribution_unknown"), "Known and unknown voices were not distinguished.");
  fixture.workspace.attribution(0, { attribution: { kind: "mixed", voices: [1, 2], provisional: true } });
  fixture.workspace.attribution(1, { attribution: { kind: "overlap", voices: [1, 2], provisional: true } });
  await until(() => first.querySelector(".speaker").textContent === t("meetings.attribution_mixed") && microphone.querySelector(".speaker").textContent === t("meetings.attribution_overlap"), "Mixed and overlapping voices were forced into one speaker.");
  for (const state of ["busy", "paused", "failed"]) {
    fixture.workspace.liveSpeakers({ state, reason_code: state, enabled: true, available: state !== "failed" });
    await until(() => root.querySelector(".live-speaker-controls").dataset.liveSpeakerState === state, `The ${state} state did not render.`);
    assert(!toggle.disabled, `The ${state} enabled state could not be switched off.`);
  }
  fixture.workspace.liveSpeakers({ state: "running", reason_code: null, enabled: true, available: true, revision: 100 });
  await until(() => root.querySelector(".live-speaker-controls").dataset.liveSpeakerState === "running", "Live processing did not recover its running status.");
  fixture.workspace.liveSpeakers({ state: "off", enabled: false, revision: 99 });
  await pause(650);
  assert(toggle.checked && root.querySelector(".live-speaker-controls").dataset.liveSpeakerState === "running", "An older status revision replaced acknowledged live processing.");
  fixture.workspace.attribution(0, { confirmed_speaker_name: "Confirmed synthetic person", attribution: { kind: "single", voices: [3], provisional: true } });
  fixture.workspace.attribution(1, { attribution: { kind: "unknown", voices: [], provisional: false } });
  fixture.workspace.complete(request.request_id);
  await until(() => first.querySelector(".speaker").textContent === "Confirmed synthetic person" && !root.querySelector(".live-question-history").textContent.includes("Writing an answer"), "A confirmed name or answer was lost.");
  await until(() => root.querySelector(".citation"), "The source-linked answer did not arrive.");
  assert(first.querySelector(".live-speaker-provisional").hidden && microphone.querySelector(".live-speaker-provisional").hidden, "Confirmed or final labels remained provisional.");
  assert(root.querySelector('[data-segment-key="utterance:qa-utterance-0"]') === first && root.querySelector('[data-segment-key="utterance:qa-utterance-1"]') === microphone
    && first.dataset.sourceId === sources[0] && microphone.dataset.sourceId === sources[1]
    && first.querySelector(".transcript-copy").textContent === text[0] && microphone.querySelector(".transcript-copy").textContent === text[1], "Live attribution changed row identity, physical source or spoken text.");
  assert(document.activeElement === note.contentDOM && root.querySelector(".live-notes") === note && note.value === before.value && note.selectionStart === before.start
    && note.selectionEnd === before.end && note.selectionDirection === before.direction && note.scrollTop === before.scroll, "Live labels or answers changed composition, focus or selection.");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  tab(root, "questions"); root.querySelector(".citation").click();
  await until(() => !root.querySelector(".live-source-notice").hidden, "Speaker revisions did not disclose the changed source context.");
  assert(root.querySelector(".live-source-notice blockquote")?.textContent === request.anchors[0].source_text && !root.querySelector(".live-source-highlight"),
    "A speaker-revised source lost its frozen excerpt or was presented as current.");
  root.querySelector(".live-source-back").click();
  assert(!root.querySelector('[data-live-pane="questions"]').hidden, "Source return lost the original question.");
  tab(root, "transcript");
  for (let index = 0; index < 60; index++) fixture.workspace.append({ start_ms: 30000 + index * 1000, end_ms: 31000 + index * 1000,
    speaker: "others", voice: 1, text: `Owned live speaker reading row ${index}.`, failed: false });
  const output = root.querySelector(".live-transcript");
  await until(() => output.textContent.includes("reading row 59."), "The scrolling transcript did not render.");
  output.scrollTop = Math.max(0, output.scrollHeight - output.clientHeight - 160); output.dispatchEvent(new Event("scroll"));
  const reading = [...output.querySelectorAll(".transcript-line")].find(row => row.getBoundingClientRect().bottom > output.getBoundingClientRect().top);
  const readingText = reading.querySelector(".transcript-copy").firstChild;
  const range = document.createRange(); range.setStart(readingText, 0); range.setEnd(readingText, Math.min(8, readingText.length));
  const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range); document.dispatchEvent(new Event("selectionchange"));
  const selected = selection.toString(); const position = reading.getBoundingClientRect().top - output.getBoundingClientRect().top;
  fixture.workspace.attribution(1, { attribution: { kind: "unknown", voices: [], provisional: true } });
  await until(() => !microphone.querySelector(".live-speaker-provisional").hidden, "A newer provisional revision did not arrive while reading.");
  const readingDelta = reading.getBoundingClientRect().top - output.getBoundingClientRect().top - position;
  assert(selection.toString() === selected && reading.isConnected && Math.abs(readingDelta) < 2,
    `A new provisional badge changed transcript selection or reading position (selection=${selection.toString() === selected}, connected=${reading.isConnected}, delta=${readingDelta}).`);
  fixture.workspace.liveSpeakers({ state: "paused", reason_code: "foreground_work", enabled: true, available: true, revision: 101 });
  await until(() => root.querySelector(".live-speaker-controls").dataset.liveSpeakerState === "paused", "Foreground speech admission did not pause optional attribution.");
  assert(selection.toString() === selected && Math.abs(reading.getBoundingClientRect().top - output.getBoundingClientRect().top - position) < 2,
    "A foreground admission message changed transcript reading position.");
  tab(root, "questions");
  const latest = await window.__PENNEL_PLATFORM__.invoke("meeting_status");
  const original = latest.transcript[0];
  fixture.workspace.replaceTranscript([{ ...original, utterance_id: "split-first", end_ms: 4000, text: "We ronden de planning ",
    confirmed_speaker_name: null, correction_review_required: true, lineage: [original.utterance_id] },
    { ...original, utterance_id: "split-second", start_ms: 4000, text: "maandag af.",
      confirmed_speaker_name: null, correction_review_required: true, lineage: [original.utterance_id] }, ...latest.transcript.slice(1)]);
  await until(() => !first.isConnected, "The verified split did not replace retired content identity.");
  root.querySelector(".citation").click();
  await until(() => !root.querySelector(".live-source-notice").hidden, "Changed utterance identity did not explain source uncertainty.");
  assert(root.querySelector(".live-source-notice blockquote")?.textContent === request.anchors[0].source_text && !root.querySelector(".live-source-highlight"), "A changed source was silently reassociated or its frozen excerpt was lost.");
  root.querySelector(".live-source-back").click();
  tab(root, "transcript");
  fixture.workspace.liveSpeakers({ enabled: true, state: "running", reason_code: null, available: true, revision: 102 });
  await pause(550); await openMeetingTools(root, "speakers"); toggle.click();
  await until(() => !toggle.checked && !toggle.disabled, "Live attribution could not be switched off explicitly.");
  assert(JSON.stringify((await window.__PENNEL_PLATFORM__.invoke("app_info")).settings) === configuration, "The live control changed the saved provider or settings.");
  root.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording(), "Stop did not quiesce after live attribution.");
  const next = fixture.workspace.restart();
  fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The next meeting did not open.");
  const nextRoot = document.querySelector(".live-workspace"); tab(nextRoot, "transcript");
  await openMeetingTools(nextRoot, "speakers");
  assert(!nextRoot.querySelector("[data-live-speaker-toggle]").checked, "A new meeting inherited automatic live speaker processing.");
  assert(!fixture.calls.some(command => ["save_settings", "download_model", "start_voice_enrollment", "remember_speaker"].includes(command)), "Live attribution changed configuration, downloaded assets or enrolled a voice.");
  return { explicitOptIn: true, pendingAcknowledgement: true, nextMeetingOff: true, bothSources: true, honestProvisionalFinalAndManual: true,
    staleStatusRejected: true, compositionRetained: true, transcriptReadingRetained: true, sourceReturnRetained: true, changedSourceExplained: true, noProviderOrEnrollmentChanges: true, syntheticOnly: true };
}

export async function checkLiveSpeakerStopPending() {
  const { fixture, root, toggle } = await owned();
  const previousId = root.dataset.meetingId;
  await openMeetingTools(root, "speakers");
  fixture.workspace.liveSpeakerDelay(1000); fixture.workspace.failSaves(true);
  toggle.click(); assert(toggle.disabled, "The delayed live request did not become pending.");
  tab(root, "notes");
  const note = root.querySelector(".live-notes"); edit(note, "Owned dirty notes while live attribution is pending.");
  root.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording(), "Stop waited for live attribution or a failed note save.");
  await until(() => !toggle.checked && root.querySelector(".live-speaker-controls").dataset.liveSpeakerState === "stopped", "Stop did not close live publication controls.");
  const next = fixture.workspace.restart(); fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The next meeting did not open while the old request drained.");
  await pause(1100);
  const nextRoot = document.querySelector(".live-workspace"); tab(nextRoot, "transcript");
  await openMeetingTools(nextRoot, "speakers");
  assert(!nextRoot.querySelector("[data-live-speaker-toggle]").checked && !toggle.checked, "A late old-session toggle activated another meeting.");
  assert(note.value === "Owned dirty notes while live attribution is pending.", "Stop lost the dirty notes.");
  fixture.workspace.failSaves(false);
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  const savedPrevious = await window.__PENNEL_PLATFORM__.invoke("meeting_detail", { id: previousId });
  const current = await window.__PENNEL_PLATFORM__.invoke("meeting_status");
  assert(savedPrevious.notes === note.value && current.notes === next.notes, "Recovering the old saved draft changed the new recording or lost the old notes.");
  return { captureFirstStop: true, failedSaveRetained: true, lateToggleRejected: true, nextMeetingOff: true, oldDraftRecoveredIndependently: true, syntheticOnly: true };
}

export async function checkUnavailableLiveSpeakers() {
  const { fixture, root, toggle } = await owned();
  await openMeetingTools(root, "speakers");
  assert(toggle.disabled && !toggle.checked, "An unavailable live route was enabled.");
  const explanation = root.querySelector(".live-speaker-state");
  assert(!explanation.hidden && explanation.textContent.trim(), "The unavailable route has no explanation.");
  assert(!root.querySelector(".live-notes").disabled, "Unavailable live attribution disabled ordinary notes.");
  const request = await ask(root, fixture); fixture.workspace.complete(request.request_id);
  await until(() => root.querySelector(".citation"), "Unavailable attribution disabled ordinary questions.");
  assert(!fixture.calls.some(command => ["set_live_speaker_attribution", "save_settings", "download_model", "start_voice_enrollment"].includes(command)), "An unavailable live route triggered setup or processing automatically.");
  await openMeetingTools(root, "speakers");
  return { explanation: true, offAndDisabled: true, ordinaryNotesAndQuestionsUsable: true, noAutomaticProcessing: true, syntheticOnly: true };
}

export async function showProvisionalLiveSpeakers() {
  const { fixture, root, toggle } = await owned();
  fixture.workspace.attribution(0, { attribution: { kind: "overlap", voices: [1, 2], provisional: true } });
  fixture.workspace.attribution(1, { attribution: { kind: "unknown", voices: [], provisional: true } });
  fixture.workspace.attribution(2, { attribution: { kind: "mixed", voices: [1, 2], provisional: true } });
  await until(() => [...root.querySelectorAll(".live-speaker-provisional")].filter(node => !node.hidden).length === 3, "Provisional attribution states did not appear.");
  assert(toggle.checked, "The already opted-in fixture did not retain its selected state.");
  await openMeetingTools(root, "speakers");
  root.querySelector(".live-speaker-details").open = true;
  assert(root.querySelector(".live-speaker-destination").getClientRects().length > 0, "The destination and model details cannot be reached through More.");
  tab(root, "transcript");
  return { qualitativeProvisionalStates: true, destinationAndModels: true, syntheticOnly: true };
}

export async function checkPersistedSpeakerQualifier() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  const fixture = window.__PENNEL_FIXTURE__;
  await until(() => document.querySelector(".live-workspace .transcript-line"), "The finished review transcript did not mount.");
  const visibleMarkers = () => [...document.querySelectorAll(".live-workspace .live-speaker-provisional")].filter(node => !node.hidden);
  assert(visibleMarkers().length === 3, "Persisted provisional estimates were displayed as final in review.");
  fixture.workspace.attribution(0, { confirmed_speaker_name: "Confirmed review person" });
  fixture.emit("meeting-updated", fixture.options.meeting_id);
  await until(() => document.querySelector('.live-workspace [data-transcript-index="0"] .speaker')?.textContent === "Confirmed review person", "The confirmed review name did not appear.");
  assert(visibleMarkers().length === 2, "A confirmed manual name remained provisional in review.");
  return { persistedProvisionalQualifier: true, manualConfirmationWins: true, syntheticOnly: true };
}

export async function checkSelectedTranscriptSplit(release = "clear") {
  const { fixture, root } = await owned();
  const spoken = "Owned rapid turns keep these exact selected words together.";
  const parent = { start_ms: 0, end_ms: 30_000, speaker: "others", text: spoken, failed: false, voice: null,
    utterance_id: "owned-selected-parent", source_id: "microphone", attribution: { kind: "unknown", voices: [] }, attribution_revision: 1 };
  const later = Array.from({ length: 48 }, (_, index) => ({ start_ms: 30_000 + index * 1_000, end_ms: 31_000 + index * 1_000,
    speaker: "others", text: `Owned later reading passage ${index}.`, failed: false, voice: 2,
    utterance_id: `owned-reading-${index}`, source_id: "system_audio", attribution: { kind: "single", voices: [2] }, attribution_revision: 1 }));
  fixture.workspace.replaceTranscript([parent, ...later]);
  await until(() => root.querySelector('[data-segment-key="utterance:owned-selected-parent"]') && root.textContent.includes("passage 47."), "The owned coarse transcript did not render.");
  const request = await ask(root, fixture);
  tab(root, "notes");
  const note = root.querySelector(".live-notes");
  note.focus(); note.setSelectionRange(2, 9, "backward");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  replaceNotesSelection(note, "samengesteld");
  tab(root, "transcript");
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const output = root.querySelector(".live-transcript");
  output.scrollTop = 0; output.dispatchEvent(new Event("scroll"));
  const oldRow = output.querySelector('[data-segment-key="utterance:owned-selected-parent"]');
  const textNode = oldRow.querySelector(".transcript-copy").firstChild;
  const start = spoken.indexOf("these"); const end = spoken.indexOf(" together");
  const selectedWords = spoken.slice(start, end);
  const selection = getSelection(); const range = document.createRange();
  range.setStart(textNode, start); range.setEnd(textNode, end); selection.removeAllRanges(); selection.addRange(range);
  document.dispatchEvent(new Event("selectionchange"));
  const before = { top: output.scrollTop, value: note.value, start: note.selectionStart, end: note.selectionEnd,
    direction: note.selectionDirection, scroll: note.scrollTop, focus: document.activeElement };
  const cut = spoken.indexOf("selected");
  const children = [{ ...parent, utterance_id: "owned-selected-first", end_ms: 15_000, text: spoken.slice(0, cut),
    lineage: [parent.utterance_id], voice: 3, attribution: { kind: "single", voices: [3], provisional: true }, attribution_revision: 2 },
  { ...parent, utterance_id: "owned-selected-second", start_ms: 15_000, text: spoken.slice(cut),
    lineage: [parent.utterance_id], voice: 4, attribution: { kind: "single", voices: [4], provisional: true }, attribution_revision: 2 }];
  fixture.workspace.replaceTranscript([...children, ...later]);
  fixture.workspace.complete(request.request_id);
  await until(() => root.querySelector(".citation"), "The independent answer did not finish during the split.");
  await pause(700);
  assert(oldRow.isConnected && output.querySelector('[data-segment-key="utterance:owned-selected-parent"]') === oldRow,
    "A real timed split removed a selected transcript node.");
  assert(selection.anchorNode === textNode && selection.focusNode === textNode && selection.anchorOffset === start && selection.focusOffset === end,
    "A timed split changed the selected DOM Range.");
  assert(selection.toString() === selectedWords && selection.getRangeAt(0).cloneContents().textContent === selectedWords,
    "A timed split added timestamp/speaker text or changed the words selected for copy.");
  assert(Math.abs(output.scrollTop - before.top) < 2 && !root.querySelector(".live-follow").hidden, "A timed split moved the paused transcript or resumed following.");
  assert(root.querySelector(".live-notes") === note && note.value === before.value && note.selectionStart === before.start
    && note.selectionEnd === before.end && note.selectionDirection === before.direction && note.scrollTop === before.scroll
    && document.activeElement === before.focus, `A timed split or answer changed the composing notes editor (same=${root.querySelector(".live-notes") === note}, value=${note.value === before.value}, start=${note.selectionStart === before.start}, end=${note.selectionEnd === before.end}, direction=${note.selectionDirection === before.direction}, scroll=${note.scrollTop === before.scroll}, focus=${document.activeElement === before.focus}).`);
  const latest = [{ ...children[0], voice: 5, attribution: { kind: "single", voices: [5], provisional: true }, attribution_revision: 3 }, children[1]];
  fixture.workspace.replaceTranscript([...latest, ...later]);
  await pause(700);
  assert(oldRow.isConnected && selection.toString() === selectedWords, "A newer pending split changed the selected text.");
  let movedNode;
  let movedWords;
  if (release === "follow") root.querySelector(".live-follow").click();
  else if (release === "move") {
    movedNode = output.querySelector('[data-segment-key="utterance:owned-reading-0"] .transcript-copy').firstChild;
    movedWords = movedNode.textContent.slice(0, 10);
    const moved = document.createRange(); moved.setStart(movedNode, 0); moved.setEnd(movedNode, 10);
    selection.removeAllRanges(); selection.addRange(moved); document.dispatchEvent(new Event("selectionchange"));
  } else { selection.removeAllRanges(); document.dispatchEvent(new Event("selectionchange")); }
  await until(() => !oldRow.isConnected && output.querySelector('[data-segment-key="utterance:owned-selected-second"]'), "The latest split did not apply when the user left the selection.");
  const first = output.querySelector('[data-segment-key="utterance:owned-selected-first"]');
  const second = output.querySelector('[data-segment-key="utterance:owned-selected-second"]');
  assert(first.querySelector(".transcript-copy").textContent + second.querySelector(".transcript-copy").textContent === spoken,
    "Applying a deferred split dropped, duplicated or redistributed the spoken text.");
  const { t } = await import("/src/i18n.ts");
  assert(first.querySelector(".speaker").textContent === t("meetings.speaker_number", { number: 5 }), "The workspace applied an obsolete pending attribution instead of the latest snapshot.");
  if (release === "move") assert(selection.anchorNode === movedNode && selection.toString() === movedWords, "Flushing the split destroyed the user's new selection.");
  if (release === "follow") assert(selection.isCollapsed && root.querySelector(".live-follow").hidden && output.scrollTop >= output.scrollHeight - output.clientHeight - 2,
    "Follow live did not explicitly leave the selection and resume the latest transcript.");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  tab(root, "questions");
  root.querySelector(".citation").click();
  await until(() => !root.querySelector(".live-source-notice").hidden, "A retired source identity was silently reassociated through the split.");
  assert(root.querySelector(".live-source-notice blockquote")?.textContent === request.anchors[0].source_text
    && !root.querySelector(".live-source-highlight"), "A split source lost its frozen excerpt or gained a guessed source mapping.");
  return { exactSelectedWords: true, originalRangeRetained: true, transcriptScrollPaused: true, notesCompositionRetained: true,
    latestDeferredSnapshot: true, deferredSplitRelease: release, changedSourceExplained: true, syntheticOnly: true };
}
