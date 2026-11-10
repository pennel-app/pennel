import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}
function edit(input, text) {
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}
function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  return window.__PENNEL_FIXTURE__;
}
function tab(root, name) { selectMeetingPane(root, name); }

export async function checkLiveCaptions() {
  const fixture = owned();
  await until(() => document.querySelector(".live-notes"), "The live workspace did not open.");
  const root = document.querySelector(".live-workspace");
  const note = root.querySelector(".live-notes");
  const output = root.querySelector(".live-transcript");
  const caption = source => root.querySelector(`[data-preview-source="${source}"]`);
  const microphone = caption("me");
  const system = caption("others");
  assert(microphone && system, "Both physical source caption lines must exist.");
  edit(note, "# Synthetic meeting notes\n\nA saved note for the question.\n".repeat(8));
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  tab(root, "transcript");
  fixture.workspace.preview("me", { text: "Provisional microphone sentinel", chunk_start_ms: 21000, start_ms: 21000, end_ms: 24000 });
  fixture.workspace.preview("others", { text: "Provisional system sentinel", chunk_start_ms: 21000, start_ms: 21000, end_ms: 24000 });
  await until(() => microphone.textContent.includes("microphone sentinel") && system.textContent.includes("system sentinel"), "Both live source previews did not render.");
  tab(root, "questions");
  edit(root.querySelector(".live-question-input"), "What was decided in the finalized transcript?");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length === 1, "The question was not admitted during captions.");
  const request = fixture.workspace.requests()[0];
  assert(!JSON.stringify(request).includes("sentinel"), "Provisional captions entered the question evidence.");
  tab(root, "notes");
  note.focus(); note.setSelectionRange(15, 27, "backward"); note.scrollTop = 60;
  note.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  note.setRangeText("samengesteld", note.selectionStart, note.selectionEnd, "select");
  note.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, inputType: "insertCompositionText", data: "samengesteld" }));
  const before = { value: note.value, start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection, scroll: note.scrollTop };
  for (let index = 0; index < 8; index++) {
    fixture.workspace.preview("me", { text: `Newer microphone sentinel ${index}`, chunk_start_ms: 24000, start_ms: 24000, end_ms: 27000 + index });
    await pause(70);
  }
  fixture.workspace.finalizePreview("me", 21000, { start_ms: 21000, end_ms: 24000, speaker: "me", text: "Final microphone speech.", failed: false, voice: null });
  fixture.workspace.finalizePreview("others", 21000, { start_ms: 21000, end_ms: 24000, speaker: "others", text: "Final computer speech.", failed: false, voice: 1 });
  fixture.workspace.complete(request.request_id);
  await until(() => output.textContent.includes("Final computer speech.") && root.textContent.includes("Synthetic frozen answer"), "Final text and the frozen answer did not arrive.");
  assert(caption("me") === microphone && caption("others") === system, "A preview update replaced the physical source nodes.");
  assert(microphone.textContent === "Newer microphone sentinel 7" && system.textContent === "", "Finalizing an older chunk removed newer speech or retained a duplicate partial.");
  assert([...output.querySelectorAll(".transcript-copy")].filter(node => node.textContent === "Final microphone speech.").length === 1, "Final speech was duplicated.");
  assert(document.activeElement === note && root.querySelector(".live-notes") === note && note.value === before.value
    && note.selectionStart === before.start && note.selectionEnd === before.end && note.selectionDirection === before.direction && note.scrollTop === before.scroll,
  "Preview, final or answer updates changed the composing editor.");
  note.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "samengesteld" }));
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  tab(root, "transcript");
  for (let index = 0; index < 60; index++) fixture.workspace.append({ start_ms: 30000 + index * 15000, end_ms: 45000 + index * 15000,
    speaker: "others", text: `Synthetic finalized reading row ${index}.`, failed: false, voice: null });
  await until(() => output.textContent.includes("reading row 59."), "The reading fixture did not render.");
  output.scrollTop = Math.max(0, output.scrollHeight - output.clientHeight - 180);
  output.dispatchEvent(new Event("scroll"));
  const row = [...output.querySelectorAll(".transcript-line")].find(node => node.getBoundingClientRect().bottom > output.getBoundingClientRect().top);
  const text = row.querySelector(".transcript-copy").firstChild;
  const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, Math.min(8, text.length));
  const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
  document.dispatchEvent(new Event("selectionchange"));
  const selected = selection.toString(); const scroll = output.scrollTop;
  fixture.workspace.preview("me", { text: "Latest microphone sentinel", chunk_start_ms: 930000, start_ms: 930000, end_ms: 933000 });
  fixture.workspace.preview("others", { text: "Latest computer sentinel", chunk_start_ms: 930000, start_ms: 930000, end_ms: 933000 });
  await until(() => system.textContent === "Latest computer sentinel", "Preview refresh stopped while reading older text.");
  assert(selection.toString() === selected && Math.abs(output.scrollTop - scroll) < 2, "Preview updates changed transcript selection or reading position.");
  selection.removeAllRanges(); root.querySelector(".live-follow").click();
  fixture.workspace.preview("me", { text: "Growing live caption ".repeat(12), chunk_start_ms: 940000, start_ms: 940000, end_ms: 943000 });
  await until(() => microphone.textContent.includes("Growing live caption"), "The followed provisional line did not grow.");
  await new Promise(resolve => requestAnimationFrame(resolve));
  assert(Math.abs(output.scrollHeight - output.clientHeight - output.scrollTop) < 2, "Follow live did not include the growing inline provisional line.");
  tab(root, "questions");
  root.querySelector(".citation").click();
  await until(() => root.querySelector(".live-source-highlight"), "Caption updates invalidated a finalized source.");
  root.querySelector(".live-source-back").click();
  assert(!root.querySelector('[data-live-pane="questions"]').hidden, "Source return lost the originating question pane.");
  (root.classList.contains("live-overlay") ? document.querySelector(".toolbar-capture-stop") : root.querySelector(".live-stop")).click();
  await until(() => !fixture.workspace.recording() && root.dataset.recording === "false", "Stop did not quiesce during preview updates.");
  assert(root.querySelector(".live-captions").hidden && microphone.textContent === "" && system.textContent === "", "Stopped previews remain visible or retain caption text.");
  assert(!JSON.stringify(fixture.workspace.requests()).includes("sentinel"), "Interim text entered stored question evidence.");
  return { bothTracks: true, exactFinalReconciliation: true, sameEditorAndPreviewNodes: true, compositionRetained: true,
    scrollSelectionAndSourcesRetained: true, finalizedEvidenceOnly: true, stoppedDuringPreviews: true, syntheticOnly: true };
}

export async function checkLiveCaptionStates() {
  const fixture = owned();
  await until(() => document.querySelector(".live-captions"), "The live captions did not mount.");
  const root = document.querySelector(".live-workspace");
  tab(root, "transcript");
  for (const state of ["warming", "busy", "failed", "disabled", "unsupported"]) {
    fixture.workspace.preview("me", { state, mode: state === "unsupported" ? "after_recording" : "local", text: "" });
    await until(() => root.querySelector('[data-caption-source="me"]').dataset.previewState === state, `The ${state} caption status did not render.`);
    const reason = root.querySelector('[data-caption-source="me"] .live-caption-state');
    assert(!reason.hidden && reason.textContent.trim(), `The ${state} caption status has no explanation.`);
  }
  assert(root.querySelector('[data-caption-source="me"]').textContent.includes("15–30"), "Unsupported batch providers have no accurate chunk cadence.");
  fixture.workspace.append({ start_ms: 22000, end_ms: 25000, speaker: "me", text: "Final transcription survives unavailable captions.", failed: false, voice: null });
  await until(() => root.querySelector(".live-transcript").textContent.includes("survives unavailable"), "Unavailable preview disabled finalized transcription.");
  return { allAvailabilityStatesExplained: true, unsupportedCadence: true, finalTranscriptionRetained: true, syntheticOnly: true };
}

export async function checkLiveTextPreference() {
  const fixture = owned();
  const { t } = await import("/src/i18n.ts");
  await until(() => [...document.querySelectorAll("label")].some(node => node.textContent.includes(t("settings.live_text.enabled"))), "The shared live-text preference did not mount.");
  const row = [...document.querySelectorAll("label")].find(node => node.textContent.includes(t("settings.live_text.enabled")));
  const input = row.querySelector('input[type="checkbox"]');
  assert(input.checked, "Existing profiles did not preserve enabled live text.");
  input.click();
  await until(async () => !(await window.__PENNEL_PLATFORM__.invoke("app_info")).settings.live_text_enabled, "The shared live-text setting was not saved.");
  assert(!fixture.calls.some(command => command === "download_model" || command === "test_provider_task" || command === "start_meeting"), "Changing display preference started unrelated work.");
  return { sharedPreferenceSaved: true, defaultPreserved: true, noInferenceOrDownloads: true, syntheticOnly: true };
}
