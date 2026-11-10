import { replaceNotesSelection } from "./meeting-workspace-test-controls.mjs";
import { selectMeetingPane, openMeetingTools } from "./meeting-workspace-test-controls.mjs";
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}
function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  return window.__PENNEL_FIXTURE__;
}
function edit(input, value) {
  if (input.applyUserChange) { input.applyUserChange(value); return; } input.value = value; input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value })); }
function tab(root, name) { selectMeetingPane(root, name); }

export async function checkDurableSpeakerTurns() {
  const fixture = owned();
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".live-workspace .live-notes"), "The live workspace did not mount.");
  const root = document.querySelector(".live-workspace");
  const note = root.querySelector(".live-notes");
  const first = root.querySelector('[data-segment-key="utterance:qa-utterance-0"]');
  assert(first, "The transcript did not use durable row identity.");
  const originalText = first.querySelector(".transcript-copy").textContent;
  const originalSource = first.dataset.sourceId;
  tab(root, "questions");
  edit(root.querySelector(".live-question-input"), "Show the exact finalized source.");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length === 1, "The question did not start.");
  const request = fixture.workspace.requests()[0];
  assert(request.anchors[0].segment_keys[0] === "utterance:qa-utterance-0", "The question retained a mutable speaker key.");
  tab(root, "notes");
  note.focus(); note.setSelectionRange(2, 10, "backward"); note.scrollTop = 10;
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  replaceNotesSelection(note, "samengesteld");
  const before = { text: note.value, start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection, scroll: note.scrollTop };
  for (const kind of ["mixed", "overlap", "unknown"]) {
    fixture.workspace.attribution(0, { speaker: "me", voice: null, attribution: { kind, voices: kind === "unknown" ? [] : [1, 2] } });
    await until(() => first.querySelector(".speaker").textContent === t(`meetings.attribution_${kind}`), `The ${kind} attribution was not shown honestly.`);
    assert(root.querySelector('[data-segment-key="utterance:qa-utterance-0"]') === first, "An attribution revision replaced the transcript row.");
    assert(first.dataset.sourceId === originalSource && first.querySelector(".transcript-copy").textContent === originalText, "An inferred speaker changed physical source or spoken text.");
  }
  fixture.workspace.complete(request.request_id);
  await until(() => root.querySelector(".citation"), "The frozen answer did not arrive.");
  assert(document.activeElement === note.contentDOM && root.querySelector(".live-notes") === note && note.value === before.text
    && note.selectionStart === before.start && note.selectionEnd === before.end && note.selectionDirection === before.direction && note.scrollTop === before.scroll,
  "Attribution or answer updates changed the composing notes editor.");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  fixture.workspace.attribution(0, { confirmed_speaker_name: "Confirmed synthetic person" });
  await until(() => first.querySelector(".speaker").textContent === "Confirmed synthetic person", "A confirmed manual name lost to automatic attribution.");
  tab(root, "questions");
  root.querySelector(".citation").click();
  await until(() => !root.querySelector(".live-source-notice").hidden, "Changed speaker context did not retain the original source excerpt.");
  assert(root.querySelector(".live-source-notice blockquote")?.textContent === request.anchors[0].source_text && !root.querySelector(".live-source-highlight"),
    "A speaker-revised source lost its frozen excerpt or was presented as current.");
  root.querySelector(".live-source-back").click();
  assert(!root.querySelector('[data-live-pane="questions"]').hidden, "Source return lost the question pane.");
  tab(root, "transcript");
  for (let index = 0; index < 60; index++) fixture.workspace.append({ start_ms: 30000 + index * 1000, end_ms: 31000 + index * 1000,
    speaker: "others", voice: 1, text: `Exact synthetic reading turn ${index}.`, failed: false });
  const output = root.querySelector(".live-transcript");
  await until(() => output.textContent.includes("reading turn 59."), "The long transcript did not render.");
  output.scrollTop = Math.max(0, output.scrollHeight - output.clientHeight - 150); output.dispatchEvent(new Event("scroll"));
  const reading = [...output.querySelectorAll(".transcript-line")].find(node => node.getBoundingClientRect().bottom > output.getBoundingClientRect().top);
  const text = reading.querySelector(".transcript-copy").firstChild;
  const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, Math.min(8, text.length));
  const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range); document.dispatchEvent(new Event("selectionchange"));
  const selected = selection.toString(); const scroll = output.scrollTop;
  fixture.workspace.attribution(55, { speaker: "me", voice: 3, attribution: { kind: "single", voices: [3] } });
  await until(() => root.querySelector('[data-segment-key="utterance:qa-utterance-55"] .speaker')?.textContent === t("meetings.speaker_number", { number: 3 }), "The newer attribution did not arrive.");
  assert(selection.toString() === selected && Math.abs(output.scrollTop - scroll) < 2 && reading.isConnected, "Attribution updates lost transcript selection or reading position.");
  fixture.workspace.metadata("unsupported");
  fixture.workspace.attribution(0, { correction_review_required: true });
  await until(() => !root.querySelector(".live-attribution-notice").hidden, "Unavailable metadata has no explanation.");
  assert(!first.querySelector(".transcript-correction-review").hidden, "An ambiguous correction has no review notice.");
  assert(output.textContent.includes(originalText), "Unavailable metadata removed readable transcript text.");
  return { durableRows: true, honestAttribution: true, physicalSourceRetained: true, exactTextRetained: true,
    compositionAndSelectionRetained: true, sourceSurvivesAttribution: true, readingPositionRetained: true, unavailableMetadataExplained: true, syntheticOnly: true };
}

export async function checkDurablePlayback() {
  const fixture = owned();
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".live-workspace .transcript-line"), "The finished transcript did not mount.");
  fixture.workspace.attribution(0, { speaker: "me", voice: 3, attribution: { kind: "single", voices: [3] } });
  fixture.emit("meeting-updated", fixture.options.meeting_id);
  await until(() => document.querySelector('.live-workspace .transcript-line[data-transcript-index="0"] .speaker')?.textContent === t("meetings.speaker_number", { number: 3 }), "The changed inferred speaker did not render.");
  const row = document.querySelector('.live-workspace .transcript-line[data-transcript-index="0"]');
  row.querySelector(".transcript-stamp").click();
  await until(() => fixture.workspace.playback(), "Playback did not use the durable utterance resolver.");
  const playback = fixture.workspace.playback();
  assert(playback.utteranceId === "qa-utterance-0" && playback.source_id === "system", "Playback followed the changed person instead of the physical source.");
  assert(fixture.calls.includes("play_meeting_utterance_audio") && !fixture.calls.includes("play_meeting_audio"), "A durable row used legacy mutable-speaker playback.");
  return { durablePlayback: true, physicalSourceIndependent: true, syntheticOnly: true };
}

export async function checkUnavailableSpeakerMetadata() {
  owned();
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".live-workspace .transcript-line"), "The readable transcript did not mount.");
  const root = document.querySelector(".live-workspace");
  assert(root.querySelectorAll(".transcript-line").length === 3, "Unavailable metadata removed spoken text.");
  assert([...root.querySelectorAll(".transcript-stamp")].every(stamp => stamp.disabled), "Unavailable metadata allowed source guessing for playback.");
  assert(!root.querySelector(".speaker-picker"), "Unsupported speaker metadata exposed a writable naming picker.");
  assert(root.querySelector(".live-attribution-notice")?.textContent === t("meetings.attribution_metadata_unsupported"), "Unsupported metadata has no usable explanation.");
  await openMeetingTools(root, "speakers");
  await until(() => [...root.querySelectorAll("button")].some(button => button.textContent === t("meetings.recognize_speakers") && button.disabled), "Unsupported speaker controls did not finish loading.");
  for (const key of ["meetings.retranscribe_audio", "meetings.recognize_speakers"]) {
    const button = [...root.querySelectorAll("button")].find(button => button.textContent === t(key));
    assert(button?.disabled, `Unsupported metadata left ${key} writable.`);
  }
  return { readableTranscript: true, playbackSourceNotGuessed: true, metadataReadOnly: true, explanation: true, syntheticOnly: true };
}
