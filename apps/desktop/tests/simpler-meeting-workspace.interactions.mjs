const assert = (value, message) => { if (!value) throw Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 300; attempt++) { if (await check()) return; await pause(); }
  throw Error(message);
}
function edit(input, text) {
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  await until(() => document.querySelector(".live-workspace .live-notes"), "The unified meeting workspace did not open.");
  const root = document.querySelector(".live-workspace");
  return { root, fixture: window.__PENNEL_FIXTURE__, api: (await import("/src/api.ts")).api, id: () => root.dataset.meetingId };
}
async function summarySurface(root) {
  root.querySelector('[data-live-tab="summary"]').click();
  await until(() => !root.querySelector(".meeting-summary-editor").disabled, "The saved summary did not load.");
  assert(root.querySelector(".meeting-summary-rendered").getClientRects().length > 0, "Completed summary did not open as readable content.");
  root.querySelector(".meeting-summary-edit").click();
  await until(() => root.querySelector(".meeting-summary-editor").getClientRects().length > 0, "The explicit summary Edit action did not reveal its retained editor.");
  return root.querySelector(".meeting-summary-editor");
}
function record(name, result) { window.__PENNEL_LIVE_ACCEPTANCE__ ??= {}; window.__PENNEL_LIVE_ACCEPTANCE__[name] = result; return result; }

export async function checkNotesFirstWorkspace() {
  const { root, fixture } = await owned();
  const notes = root.querySelector(".live-notes");
  assert(root.querySelector('[data-live-pane="notes"]').getClientRects().length > 0, "Notes are not the initial writing surface.");
  for (const name of ["transcript", "questions"]) assert(root.querySelector(`[data-live-pane="${name}"]`).hidden, "A companion opened without a choice.");
  assert(root.querySelector(".live-note-controls").hidden, "Formatting details opened before they were requested.");
  edit(notes, "Synthetic notes while capture continues."); notes.focus(); notes.setSelectionRange(10, 15, "backward");
  root.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording() && root.dataset.recording === "false", "Stop did not finish capture in the same workspace.");
  assert(document.querySelector(".live-workspace") === root && root.querySelector(".live-notes") === notes, "Stop replaced the meeting or notes editor.");
  assert(notes.value === "Synthetic notes while capture continues." && notes.selectionStart === 10 && notes.selectionEnd === 15 && document.activeElement === notes,
    "Finalization changed notes, selection or focus.");
  assert(!root.querySelector(".live-review"), "Finalization introduced a separate review transition.");
  root.querySelector('[data-live-tab="questions"]').click();
  const questionPanel = root.querySelector('[data-live-pane="questions"]');
  assert(!questionPanel.hidden, "Questions did not open on request.");
  root.querySelector(".meeting-companion-close").click();
  assert(questionPanel.hidden && !root.classList.contains("live-companion-open"), "Closing the companion did not restore notes.");
  return record("notesFirstUnifiedLifecycle", { sameWorkspace: true, sameEditor: true, defaultClosed: true, stopPreservedSelection: true, syntheticOnly: true });
}

export async function checkSummaryEditingAndCandidate() {
  const { root, fixture, api, id } = await owned();
  const editor = await summarySurface(root);
  edit(editor, "Synthetic private summary before generation.");
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  assert((await api.meetingSummary(id())).markdown === editor.value, "Editable summary was not durably acknowledged.");
  const generate = root.querySelector(".meeting-summary-generate");
  await until(() => !generate.disabled, "Summary generation is unavailable in the configured fixture.");
  const previous = fixture.calls.filter(command => command === "generate_meeting_summary").length;
  generate.click();
  await until(() => fixture.calls.filter(command => command === "generate_meeting_summary").length > previous, "Summary generation did not start.");
  editor.focus(); edit(editor, "Later synthetic writing must survive generation."); editor.setSelectionRange(6, 15, "backward");
  editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  const before = { value: editor.value, start: editor.selectionStart, end: editor.selectionEnd, direction: editor.selectionDirection, scroll: editor.scrollTop };
  await until(() => !root.querySelector(".meeting-summary-candidate").hidden, "The generated candidate was not reviewable.");
  assert(root.querySelector(".meeting-summary-editor") === editor && document.activeElement === editor && editor.value === before.value
    && editor.selectionStart === before.start && editor.selectionEnd === before.end && editor.selectionDirection === before.direction && editor.scrollTop === before.scroll,
    "Summary completion replaced active text, composition, selection, scroll or focus.");
  assert(root.querySelector(".meeting-summary-accept").disabled, "A candidate overwrote newer typing.");
  editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  root.querySelector(".meeting-summary-candidate .btn:not(.primary)").click();
  await until(() => !generate.disabled, "A fresh summary draft could not be requested."); generate.click();
  await until(() => !root.querySelector(".meeting-summary-candidate").hidden && !root.querySelector(".meeting-summary-accept").disabled, "The fresh candidate could not be reviewed.");
  const expected = root.querySelector(".meeting-summary-candidate-body").textContent;
  assert(expected, "Candidate preview was empty.");
  root.querySelector(".meeting-summary-accept").click();
  await until(() => root.querySelector(".meeting-summary-candidate").hidden, "Accepting the reviewed candidate did not save it.");
  assert((await api.meetingSummary(id())).markdown === editor.value, "Accepted candidate differs from the saved editor.");
  root.querySelector(".meeting-summary-edit").click();
  await until(() => editor.hidden && root.querySelector(".meeting-summary-rendered").getClientRects().length > 0, "Done did not return to the readable summary.");
  return record("summaryEditingCandidate", { autosaveAcknowledged: true, generationPreservesInput: true, staleCandidateRejected: true, explicitAcceptance: true, readableDefaultAndDone: true, syntheticOnly: true });
}

export async function checkSummaryRecoveryAndHistory() {
  const { root, api, id } = await owned(); const editor = await summarySurface(root);
  const client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client); let fail = true;
  client.invoke = async (command, args) => { if (command === "save_meeting_summary" && fail) throw Error("Synthetic summary write failed."); return original(command, args); };
  try {
    edit(editor, "Retained synthetic summary after a failed save.");
    await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts()).catch(() => undefined);
    assert(root.querySelector(".meeting-summary-surface").dataset.summaryState === "error" && editor.value.includes("Retained synthetic"), "Failed save lost the summary draft or acknowledgement state.");
    fail = false; root.querySelector(".meeting-summary-retry").click();
    await until(async () => (await api.meetingSummary(id())).markdown === editor.value, "Retry did not durably save the retained edit.");
    const earlier = editor.value; edit(editor, "A later synthetic summary revision.");
    await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
    const disclosure = root.querySelector(".meeting-summary-history"); disclosure.open = true;
    await until(() => disclosure.querySelector(".meeting-summary-version"), "Saved summary history is unavailable.");
    const version = disclosure.querySelector(".meeting-summary-version"); version.open = true; version.querySelector("button").click();
    await until(() => editor.value === earlier, "Restoring history did not restore the selected summary text.");
    assert((await api.meetingSummaryHistory(id())).some(entry => entry.markdown === "A later synthetic summary revision."), "Restore discarded the overwritten summary.");
    return record("summaryRecoveryHistory", { retainedFailure: true, retryAcknowledged: true, restoreRetainsHistory: true, syntheticOnly: true });
  } finally { client.invoke = original; }
}

export async function checkScopedCopyPreview() {
  const { root, fixture } = await owned(); const editor = await summarySurface(root);
  const privateSummary = "PRIVATE_SYNTHETIC_OWNER_SUMMARY";
  edit(editor, privateSummary);
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  root.querySelector('[data-live-tab="notes"]').click();
  root.querySelector(".meeting-export-action").click();
  const preview = root.querySelector(".meeting-export-preview"), body = preview.querySelector(".meeting-export-copy");
  await until(() => !preview.hidden && !preview.querySelector(".meeting-export-generate").disabled, "The default shareable preview did not open.");
  assert(preview.dataset.exportScope === "shareable" && !fixture.calls.includes("export_meeting"), "Copy defaulted to a legacy personal export.");
  preview.querySelector(".meeting-export-generate").click();
  await until(() => !preview.querySelector(".meeting-export-rendered").hidden && body.value.trim() && !preview.querySelector(".meeting-export-copy-markdown").disabled, "A verified shareable preview was not prepared.");
  const exact = body.value;
  assert(!exact.includes(privateSummary) && !exact.includes("PRIVATE_SYNTHETIC_NOTE"), "Private summary or note facts entered the shareable preview.");
  const notes = root.querySelector(".live-notes"); edit(notes, notes.value + "\nPRIVATE_SYNTHETIC_NOTE_AFTER_PREVIEW");
  preview.querySelector(".meeting-export-copy-markdown").click();
  await until(() => fixture.copiedTexts().length > 0, "The exact preview could not be copied.");
  assert(fixture.copiedTexts().at(-1) === exact && body.value === exact, "Copy silently regenerated the reviewed preview.");
  preview.querySelector('[data-export-scope="personal"]').click();
  await until(() => preview.dataset.exportScope === "personal" && !preview.querySelector(".meeting-export-rendered").hidden && body.value.includes(privateSummary), "Explicit Personal copy did not include the owner summary.");
  return record("scopedCopyPreview", { shareableDefault: true, privateFactsExcluded: true, exactPreparedCopy: true, personalExplicit: true, syntheticOnly: true });
}

export async function checkSummaryManualReview() {
  const { root, fixture, api, id } = await owned();
  await until(() => !root.querySelector(".meeting-summary-editor").disabled, "The saved outdated summary did not load.");
  const button = root.querySelector(".meeting-summary-mark-reviewed");
  const body = (await api.meetingSummary(id())).markdown;
  assert(button.getClientRects().length && !button.disabled, "Manual review requires a provider or is unavailable for a clean saved summary.");
  const client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client);
  let race = null, attempts = 0;
  client.invoke = async (command, args) => {
    if (command === "accept_meeting_summary") {
      attempts++;
      const change = race; race = null;
      if (change === "notes") {
        const saved = await original("meeting_detail", { id: args.id });
        await original("save_meeting_notes", { id: args.id, text: saved.notes + "\nNew private source before review acknowledgement." });
      } else if (change === "transcript") fixture.workspace.append({ start_ms: 25_000, end_ms: 29_000, speaker: "others", voice: null,
        text: "A new transcript source before review acknowledgement.", failed: false });
    }
    return original(command, args);
  };
  try {
    for (const change of ["notes", "transcript"]) {
      race = change; const expected = attempts + 1;
      button.click();
      await until(() => attempts === expected && root.querySelector(".meeting-summary-surface").dataset.summaryState === "error", `Changed ${change} did not reject the stale review acknowledgement.`);
      const saved = await api.meetingSummary(id());
      assert(saved.outdated && saved.markdown === body && !root.querySelector(".meeting-summary-outdated").hidden,
        `A stale ${change} acknowledgement cleared the warning or changed summary text.`);
      root.querySelector(".meeting-summary-recover").click();
      await until(() => !root.querySelector(".meeting-summary-recover").disabled, "The current source revision could not be checked for manual review.");
    }
    button.click();
    await until(async () => !(await api.meetingSummary(id())).outdated && root.querySelector(".meeting-summary-outdated").hidden,
      "A fresh manual review could not acknowledge the unchanged private summary.");
    assert((await api.meetingSummary(id())).markdown === body, "Manual review rewrote the summary.");
    assert(await api.shareableMeetingSummary(id()) === null, "Manual owner review created a shareable summary.");
    assert(!fixture.calls.some(command => ["generate_meeting_summary", "generate_shareable_meeting_summary", "summarize_meeting"].includes(command)), "Manual review triggered provider generation.");
    return record("summaryManualReview", { noProviderRequired: true, notesAndTranscriptRaceRejected: true, unchangedOwnerText: true, shareableSeparationPreserved: true, syntheticOnly: true });
  } finally { client.invoke = original; }
}
