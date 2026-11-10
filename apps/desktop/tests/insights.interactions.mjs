import { replaceNotesSelection, selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) { for (let attempt = 0; attempt < 260; attempt++) { if (await check()) return; await pause(); } throw Error(message); }
function edit(input, value) {
  if (input.applyUserChange) { input.applyUserChange(value); return; } input.value = value; input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value })); }
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic review fixture.");
  await until(() => document.querySelector(".insight-review .insight-card"), "Reviewed suggestions did not mount.");
  const root = document.querySelector(".insight-review");
  root.querySelector(".insight-analysis-details").open = true;
  root.querySelectorAll(".insight-card-details").forEach(details => { details.open = true; });
  return { fixture: window.__PENNEL_FIXTURE__, root, api: window.__PENNEL_PLATFORM__ };
}
async function store(api) { return api.invoke("insights_snapshot", { id: window.__PENNEL_FIXTURE__.options.meeting_id }); }
async function choose(root, key) {
  const { t } = await import("/src/i18n.ts"); root.querySelector(".insight-language .choice").click();
  await until(() => [...document.querySelectorAll('[role="option"]')].some(option => option.textContent === t(key)), "The explicit content-language choices did not open.");
  [...document.querySelectorAll('[role="option"]')].find(option => option.textContent === t(key)).click();
  await pause(50);
}
function tab(name) {
  const workspace = document.querySelector(".live-workspace:has(.meeting-document)");
  if (workspace && (name === "notes" || name === "summary")) { selectMeetingPane(workspace, name); return; }
  const button = document.querySelector(`[data-live-tab="${name === "ask" ? "questions" : name === "insights" ? "suggestions" : name}"]`) || document.querySelector(`.detail-tabs [data-tab="${name}"]`);
  if (button?.getAttribute("aria-selected") !== "true") button?.click();
}

export async function checkInsightReview() {
  const { fixture, root, api } = await owned();
  const { t } = await import("/src/i18n.ts");
  const initial = await store(api), first = root.querySelector(`[data-insight-id="${initial.items[0].id}"]`), second = root.querySelector(`[data-insight-id="${initial.items[1].id}"]`);
  const originalNotes = fixture.workspace.notes();
  assert(root.querySelector(".insight-find").disabled, "Interface language silently selected a content language.");
  await choose(root, "insights.language_en");
  await until(() => !root.querySelector(".insight-find").disabled, "An explicitly qualified fixture did not enable on-demand analysis.");
  assert(root.querySelector(".insight-destination").textContent.includes("decision.example.invalid"), "The actual processing destination is missing.");
  assert(second.querySelector(".insight-owner-due").textContent.includes(t("insights.unknown")), "Unsupported owner or due information was invented.");
  first.querySelector(".insight-accept").click();
  await until(async () => (await store(api)).items[0].review_state === "accepted", "Accepting the item did not persist.");
  assert(fixture.workspace.notes() === originalNotes, "Accepting an item silently changed personal notes.");
  first.querySelector(".insight-edit-action").click();
  await until(() => !first.querySelector(".insight-edit").hidden, "Editing did not claim and open the item.");
  const editor = first.querySelector(".insight-edit"); const revised = "- [ ] Reviewed action kept exactly.\n  Personal accepted edit.";
  editor.focus(); edit(editor, revised); first.querySelector(".insight-save-edit").focus(); first.querySelector(".insight-save-edit").click();
  await until(() => first.querySelector(".insight-edit").hidden, "Saving did not finish the reviewed edit.");
  second.querySelector(".insight-dismiss").click();
  await until(() => !second.isConnected, "Dismissed proposals did not leave the default review list.");
  assert((await store(api)).items[1].review_state === "dismissed", "Dismissal was not stored.");
  root.querySelector(".insight-dismissed-toggle").click();
  await until(() => root.querySelector(`[data-insight-id="${initial.items[1].id}"]`), "Dismissed proposals could not be reviewed again.");
  first.querySelector(".insight-source-open").click();
  await until(() => document.querySelector(".live-workspace .live-source-highlight, .meeting-detail .source-highlight"), "The exact transcript source was not revealed.");
  assert(getSelection().toString() === initial.items[0].sources[0].quote, "Source selection did not use the exact quote bytes.");
  document.querySelector(".live-source-back:not([hidden]), .insight-source-back").click();
  await until(() => root.isConnected, "Source return lost the review workspace.");
  first.querySelector(".insight-add-notes").click();
  await until(() => fixture.workspace.notes().includes(revised), "The explicit accepted item did not enter ordinary notes saving.");
  assert(fixture.workspace.notes().startsWith(originalNotes), "Insertion overwrote personal notes.");
  tab("insights");
  await until(() => first.querySelector(".insight-add-notes").disabled && first.querySelector(".insight-insertion-state").textContent.includes(t("insights.insertion_acknowledged")), "Insertion was claimed before durable acknowledgement.");
  first.querySelector(".insight-add-notes").click();
  assert(fixture.workspace.notes().split(revised).length - 1 === 1, "A repeated insertion duplicated accepted text.");
  tab("notes"); tab("insights");
  assert(root.querySelector(`[data-insight-id="${initial.items[0].id}"] .insight-item-text`).textContent === revised, "Reopening lost the accepted edit.");
  const exported = await api.invoke("insights_export", { id: fixture.options.meeting_id });
  assert(exported.includes(revised) && !exported.includes(originalNotes) && !exported.includes(initial.items[1].text), "Export included private notes, dismissed content or lost an accepted edit.");
  assert(!fixture.calls.some(command => ["download_model", "set_key", "save_settings", "start_voice_enrollment"].includes(command)), "Review changed providers, credentials or models.");
  return { explicitLanguage: true, destinationShown: true, acceptedEditedDismissedPersistent: true, exactSourcesAndReturn: true,
    explicitAcknowledgedInsertion: true, personalNotesPreserved: true, duplicateInsertionBlocked: true, syntheticOnly: true };
}

export async function checkUnqualifiedInsights() {
  const { root, fixture } = await owned(); await choose(root, "insights.language_nl");
  assert(root.querySelector(".insight-find").disabled, "An unqualified exact setup could run analysis.");
  assert(!fixture.calls.includes("begin_insights_analysis"), "Opening review sent meeting content for analysis.");
  root.querySelector(".insight-accept").click();
  await until(() => root.querySelector(".insight-add-notes"), "Existing items could not be reviewed without qualification.");
  return { unqualifiedUnavailable: true, existingReviewUsable: true, noInference: true, syntheticOnly: true };
}

export async function checkReadonlyInsights() {
  assert(window.__PENNEL_FIXTURE__, "Use the owned fixture.");
  await until(() => document.querySelector(".insight-review"), "The review pane did not mount.");
  const root = document.querySelector(".insight-review");
  await until(() => root.querySelector(".insight-availability").textContent.trim(), "The read-only format has no explanation.");
  assert(root.querySelector(".insight-find").disabled && root.querySelector(".insight-export").disabled, "Unknown metadata became an empty writable store.");
  return { preservedReadonlyFormat: true, explanation: true, syntheticOnly: true };
}

export async function checkInsightNoteFailure() {
  const { fixture, root, api } = await owned(); const initial = await store(api), first = root.querySelector(`[data-insight-id="${initial.items[0].id}"]`);
  first.querySelector(".insight-accept").click(); await until(async () => (await store(api)).items[0].review_state === "accepted", "Acceptance failed.");
  fixture.workspace.failSaves(true); const previous = fixture.workspace.notes();
  first.querySelector(".insight-add-notes").click();
  await until(() => document.querySelector(".live-notes")?.value.includes(initial.items[0].text), "A failed note write did not retain the inserted draft.");
  await until(() => !root.querySelector(".insight-error").hidden, "A failed insertion save had no explanation.");
  assert(!document.querySelector(".live-save-error").hidden, "The current notes pane hid its failed-save recovery.");
  assert(fixture.workspace.notes() === previous, "A failed save was reported as durable.");
  const note = document.querySelector(".live-notes"), retained = note.value;
  fixture.workspace.failSaves(false); note.focus(); note.dispatchEvent(new FocusEvent("blur", { bubbles: false }));
  await until(() => fixture.workspace.notes() === retained, "The ordinary notes writer could not acknowledge the retained insertion.");
  tab("insights"); await until(async () => (await store(api)).insertion_receipts[0]?.state === "acknowledged", "The original insertion receipt was not acknowledged.");
  const attempts = (await store(api)).insertion_receipts.length; first.querySelector(".insight-add-notes").click();
  assert((await store(api)).insertion_receipts.length === attempts, "Retry created another insertion attempt after acknowledgement.");
  assert(note.undo(), "Suggestion insertion is missing from editor history.");
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  assert(note.value === previous && fixture.workspace.notes() === previous, "Undo did not persist the pre-suggestion notes.");
  assert(note.redo(), "Suggestion insertion cannot be redone.");
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  assert(note.value === retained && fixture.workspace.notes() === retained && (await store(api)).insertion_receipts.length === attempts, "Redo changed insertion identity or failed to save exact notes.");
  return { failedSaveRetainsEditableDraft: true, ordinaryWriterRetry: true, durableAck: true, noDuplicateAttempt: true, undoRedo: true, syntheticOnly: true };
}

export async function checkInsightInsertionRace() {
  const { fixture, root, api } = await owned(); const initial = await store(api), first = root.querySelector(`[data-insight-id="${initial.items[0].id}"]`);
  first.querySelector(".insight-accept").click(); await until(async () => (await store(api)).items[0].review_state === "accepted", "Acceptance failed.");
  first.querySelector(".insight-add-notes").click();
  await until(() => fixture.calls.includes("prepare_insight_insertion"), "The insertion was not prepared.");
  tab("notes");
  const note = document.querySelector(".live-notes"), userText = "Private notes typed while insertion was preparing.";
  note.focus(); edit(note, userText); note.setSelectionRange(3, 12, "backward");
  await until(() => !root.querySelector(".insight-error").hidden, "Concurrent typing did not stop stale insertion.");
  assert(note.value === userText && note.selectionStart === 3 && note.selectionEnd === 12, "Insertion overwrote current text or selection.");
  await pause(700); note.dispatchEvent(new FocusEvent("blur"));
  await until(() => fixture.workspace.notes() === userText, "The independent private note edit was not saved.");
  tab("insights"); root.querySelector(".insight-toolbar .text-button").click();
  await until(async () => (await store(api)).insertion_receipts.length === 1, "The interrupted preparation lost its receipt.");
  first.querySelector(".insight-check-insertion").click();
  await until(() => !first.querySelector(".insight-insertion-review").hidden, "Ambiguous recovery did not require explicit review.");
  assert(fixture.workspace.notes() === userText, "Recovery appended text automatically.");
  first.querySelector(".insight-resolve-absent").click();
  await until(async () => (await store(api)).insertion_receipts[0].state === "not_inserted", "The explicit review decision was not saved.");
  assert(fixture.workspace.notes() === userText, "Resolving recovery changed note text.");
  return { typingAndCursorPreserved: true, pendingReceiptRetained: true, explicitAmbiguousReview: true, noAutomaticInsertion: true, syntheticOnly: true };
}

export async function checkInsightGenerationEditing() {
  const { root, api } = await owned(); const initial = await store(api), first = root.querySelector(`[data-insight-id="${initial.items[0].id}"]`);
  await choose(root, "insights.language_en"); root.querySelector(".insight-find").click();
  await until(() => !root.querySelector(".insight-cancel").hidden, "Explicit analysis did not start.");
  first.querySelector(".insight-edit-action").click(); await until(() => !first.querySelector(".insight-edit").hidden, "An in-flight item could not be protected for editing.");
  const editor = first.querySelector(".insight-edit"); editor.focus();
  editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })); edit(editor, "Owned composed edit preserved during regeneration.");
  editor.setSelectionRange(6, 14, "backward"); const value = editor.value;
  await until(() => root.querySelectorAll(".insight-card").length === 1, "The synthetic regeneration did not replace unreviewed proposals.");
  assert(first.isConnected && document.activeElement === editor && editor.value === value && editor.selectionStart === 6 && editor.selectionEnd === 14 && editor.selectionDirection === "backward", "Regeneration lost the claimed edit, composition or selection.");
  editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })); first.querySelector(".insight-save-edit").click();
  await until(() => first.querySelector(".insight-edit").hidden, "The retained edit could not be saved after generation.");
  assert((await store(api)).items[0].edited_text === value, "The edit was not durable.");
  return { claimBeforeGeneration: true, concurrentCompositionRetained: true, unreviewedReplaceable: true, savedEditPersistent: true, syntheticOnly: true };
}

export async function checkInsightStaleSource() {
  const { fixture, root, api } = await owned(); const initial = await store(api), first = root.querySelector(`[data-insight-id="${initial.items[0].id}"]`);
  const detail = await api.invoke("meeting_detail", { id: fixture.options.meeting_id });
  const replacement = { ...detail.transcript[0], utterance_id: "00000000-0000-7000-8000-000000009999", start_ms: 1000, end_ms: 9000 };
  fixture.workspace.replaceTranscript([replacement, ...detail.transcript.slice(1)]);
  root.querySelector(".insight-toolbar .text-button").click();
  await until(() => !first.querySelector(".insight-reassociate-action").hidden, "A replaced source was not marked stale.");
  first.querySelector(".insight-source-open").click(); await pause(100);
  assert(!first.querySelector(".insight-source-notice").hidden && first.querySelector("blockquote").textContent === initial.items[0].sources[0].quote, "A missing source was guessed or its frozen quote was lost.");
  first.querySelector(".insight-reassociate-action").click();
  await until(() => first.querySelector(".insight-reassociate .choice"), "Exact source reassociation did not offer a selection.");
  first.querySelector(".insight-reassociate .choice").click();
  await until(() => document.querySelectorAll('[role="option"]').length > 1, "The exact source choices did not open.");
  [...document.querySelectorAll('[role="option"]')].find(option => option.textContent.includes(initial.items[0].sources[0].quote)).click();
  [...first.querySelectorAll(".insight-reassociate button")].find(button => !button.classList.contains("choice")).click();
  await until(async () => (await store(api)).items[0].source_reassociated, "The explicit source link was not stored.");
  const after = (await store(api)).items[0];
  assert(after.source_history[0].utterance_id === initial.items[0].sources[0].utterance_id && after.sources[0].utterance_id === replacement.utterance_id, "Reassociation lost the earlier source or guessed another identity.");
  const { t } = await import("/src/i18n.ts"); assert(first.querySelector(".insight-source-notice").textContent === t("insights.source_link_reviewed"), "A reviewed source link still claimed to be unavailable.");
  return { staleSourceExplained: true, frozenQuoteKept: true, explicitExactReassociation: true, sourceHistory: true, syntheticOnly: true };
}

export async function checkInsightCancel() {
  const { root, api } = await owned(); const before = await store(api);
  await choose(root, "insights.language_en"); root.querySelector(".insight-find").click();
  await until(() => !root.querySelector(".insight-cancel").hidden, "Explicit analysis did not start."); root.querySelector(".insight-cancel").click();
  assert(root.querySelector(".insight-find").disabled, "Logical cancellation immediately admitted replacement work.");
  await until(() => !root.querySelector(".insight-find").disabled, "Physical completion did not restore admission.");
  assert((await store(api)).items.length === before.items.length, "Cancellation removed reviewed data.");
  return { cancellationRetainsItems: true, physicalWorkerAccountingVisible: true, explicitRetryOnly: true, syntheticOnly: true };
}

export async function checkFinishedInsightWorkspace() {
  assert(window.__PENNEL_FIXTURE__, "Use the owned fixture."); const fixture = window.__PENNEL_FIXTURE__;
  await until(() => document.querySelector(".live-workspace .live-notes"), "The active workspace did not mount.");
  const root = document.querySelector(".live-workspace"), note = root.querySelector(".live-notes");
  root.querySelector('[data-live-tab="suggestions"]').click();
  assert(!root.querySelector(".insight-review") && !fixture.calls.includes("insights_snapshot"), "Finished review activated live suggestions before qualification.");
  note.focus(); note.setSelectionRange(2, 10, "backward"); note.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  replaceNotesSelection(note, "samengesteld");
  const before = { value: note.value, start: note.selectionStart, end: note.selectionEnd };
  root.querySelector(".live-stop").click();
  await until(() => root.querySelectorAll(".insight-review .insight-card").length === 4, "Finished-only review did not reuse the suggestions pane.");
  assert(document.activeElement === note.contentDOM && root.querySelector(".live-notes") === note && note.value === before.value && note.selectionStart === before.start && note.selectionEnd === before.end, "Finishing and mounting review changed the composing notes.");
  assert(!fixture.calls.includes("begin_insights_analysis"), "Finishing automatically analyzed meeting content.");
  note.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  return { live97RemainsGated: true, finishedComponentReused: true, compositionRetained: true, noAutomaticAnalysis: true, syntheticOnly: true };
}

export async function checkInsightExternalUpdate() {
  const { fixture, root, api } = await owned();
  const initial = await store(api), item = initial.items[2], card = root.querySelector(`[data-insight-id="${item.id}"]`);
  card.querySelector(".insight-edit-action").click(); await until(() => !card.querySelector(".insight-edit").hidden, "The protected editor did not open.");
  const editor = card.querySelector(".insight-edit"); editor.focus();
  editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })); edit(editor, "Unfinished composed review text retained."); editor.setSelectionRange(5, 14, "backward");
  const detail = await api.invoke("meeting_detail", { id: fixture.options.meeting_id });
  fixture.workspace.replaceTranscript([{ ...detail.transcript[0], text: "An unrelated source was corrected." }, ...detail.transcript.slice(1)]);
  fixture.emit("meeting-updated", fixture.options.meeting_id);
  await until(() => !root.querySelector(`[data-insight-id="${initial.items[0].id}"] .insight-source-notice`).hidden, "An external source update did not invalidate its source link.");
  assert(document.querySelector(`[data-insight-id="${item.id}"] .insight-edit`) === editor && document.activeElement === editor
    && editor.value === "Unfinished composed review text retained." && editor.selectionStart === 5 && editor.selectionEnd === 14 && editor.selectionDirection === "backward",
  "An external source update replaced or blurred the composing review edit.");
  editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  return { externalSourceChange: true, dirtyCompositionFocusSelectionRetained: true, syntheticOnly: true };
}

export async function checkInsightUnresolvedLink() {
  const { root, api } = await owned(), initial = await store(api);
  const change = root.querySelector(`[data-insight-id="${initial.items[3].id}"]`);
  assert(!change.querySelector(".insight-linkage").hidden && !change.querySelector(".insight-prior-statements").hidden, "An unresolved change was displayed as an established relationship.");
  assert(change.querySelector(".insight-prior-statements blockquote").textContent === initial.items[0].sources[0].quote, "The unresolved related statement lost its frozen evidence.");
  return { linkageNotAssumed: true, relatedStatementVisible: true, syntheticOnly: true };
}
