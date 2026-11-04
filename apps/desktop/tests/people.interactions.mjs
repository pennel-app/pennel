import { openMeetingTool } from "./meeting-test-controls.mjs";

const assert = (value, message) => { if (!value) throw Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 260; attempt++) { if (await check()) return; await pause(); }
  throw Error(message);
}
function edit(input, text) {
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}
function tab(name) {
  const button = document.querySelector(`[data-live-tab="${name}"]`) || document.querySelector(`.detail-tabs [data-tab="${name}"]`);
  if (button?.getAttribute("aria-selected") !== "true") button?.click();
}
function action(root, text) { return [...root?.querySelectorAll("button") || []].find(button => button.textContent.trim() === text); }
function cancel(dialog) { dialog.dispatchEvent(new Event("cancel", { cancelable: true })); }
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic people fixture.");
  await until(() => document.querySelector(".meeting-detail, .live-workspace"), "The meeting workspace did not mount.");
  const { t } = await import("/src/i18n.ts");
  return { fixture: window.__PENNEL_FIXTURE__, api: window.__PENNEL_PLATFORM__, t };
}
async function open(control = ".people-manage") {
  await until(() => document.querySelector(control), "Participant controls did not mount.");
  if (document.querySelector(".live-workspace") && !document.querySelector(control).getClientRects().length) {
    await openMeetingTool("meetings.workspace_people");
  }
  const disclosure = document.querySelector(control).closest("details");
  if (disclosure && !disclosure.open) disclosure.querySelector("summary").click();
  document.querySelector(control).scrollIntoView({ block: "nearest" });
  assert(document.querySelector(control).getClientRects().length > 0, "Participant control is hidden from the user.");
  document.querySelector(control).focus();
  document.querySelector(control).click();
  await until(() => document.querySelector(".people-picker .people-choice"), "Participant choices did not load.");
  return document.querySelector(".people-picker");
}

export async function prepareRememberVoiceDialog() {
  const { fixture, t } = await owned();
  await openMeetingTool("meetings.workspace_speaker_controls");
  await until(() => [...document.querySelectorAll(".meeting-voice-row button")].some(button => button.textContent.trim() === t("people.remember") && !button.disabled), "The available remember-voice action did not load.");
  const button = [...document.querySelectorAll(".meeting-voice-row button")].find(button => button.textContent.trim() === t("people.remember") && !button.disabled);
  button.scrollIntoView({ block: "nearest" }); button.focus(); button.click();
  await until(() => document.querySelector("dialog[open] form input"), "Remembering a voice did not ask for a reviewable name.");
  assert(!fixture.calls.includes("remember_speaker"), "Opening the name dialog already changed a remembered voice.");
  return accepted({ explicitRememberDialog: true, noSubmission: true, syntheticOnly: true });
}
async function noteEditor() {
  tab("notes");
  await until(() => document.querySelector(".live-notes, .own-notes textarea"), "The note editor did not mount.");
  const editor = document.querySelector(".live-notes, .own-notes textarea"), disclosure = editor.closest("details");
  if (disclosure && !disclosure.open) disclosure.querySelector("summary").click();
  assert(editor.getBoundingClientRect().height > 0, "The note editor is not visible.");
  return editor;
}
function choose(dialog, id) {
  const choice = dialog.querySelector(`[data-person-id="${id}"]`);
  assert(choice, "The requested stable person was missing from the picker.");
  choice.click();
}
async function saveManual(dialog, t, name, key = "people.manual") {
  const revision = (await window.__PENNEL_FIXTURE__.people.read()).revision;
  action(dialog, t(key)).click();
  await until(() => document.querySelector("dialog:not(.people-picker) form"), "Manual person entry did not open.");
  const form = document.querySelector("dialog:not(.people-picker) form"), input = form.querySelector("input");
  assert(document.activeElement === input, "Manual entry did not receive keyboard focus.");
  edit(input, name);
  form.dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  await until(async () => !form.isConnected && !dialog.querySelector("input[type=search]").disabled
    && ((await window.__PENNEL_FIXTURE__.people.read()).revision > revision || dialog.querySelector(".error")), "Manual person entry did not finish.");
}
async function speakerPicker(t) {
  tab("transcript");
  if (document.querySelector(".live-workspace")) {
    await until(() => document.querySelector(".people-speaker-action"), "The live speaker action is missing.");
    document.querySelector(".people-speaker-action").click();
  } else {
    await until(() => document.querySelector(".speaker-picker"), "The review speaker action is missing.");
    document.querySelector(".speaker-picker").click();
    await until(() => action(document.querySelector(".choice-list"), t("people.link_title")), "Speaker linking was absent from the speaker menu.");
    action(document.querySelector(".choice-list"), t("people.link_title")).click();
  }
  await until(() => document.querySelector(".people-picker .people-choice"), "The speaker participant picker did not load.");
  return document.querySelector(".people-picker");
}
function accepted(values) { return window.__PENNEL_PEOPLE_ACCEPTANCE__ = { ...values, syntheticOnly: true }; }

export async function checkPeopleSpeakers() {
  const { fixture, api, t } = await owned();
  const id = fixture.options.meeting_id, original = await api.invoke("meeting_detail", { id });
  tab("notes");
  let dialog = await open();
  assert(document.activeElement === dialog.querySelector("input[type=search]"), "The picker search did not receive keyboard focus.");
  await saveManual(dialog, t, "Synthetic duplicate");
  await saveManual(dialog, t, "Synthetic duplicate");
  const duplicates = (await fixture.people.read()).people.filter(person => person.display_name === "Synthetic duplicate");
  assert(duplicates.length === 2 && duplicates[0].id !== duplicates[1].id, "Duplicate names were merged in manual entry.");
  cancel(dialog);
  dialog = await speakerPicker(t);
  choose(dialog, duplicates[0].id);
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => !dialog.isConnected, "Speaker linkage did not finish.");
  assert((await fixture.people.read()).speaker_links[0].person_id === duplicates[0].id, "The explicit speaker identity was not stored.");
  dialog = await speakerPicker(t);
  choose(dialog, duplicates[1].id);
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => !dialog.isConnected, "Speaker relinking did not finish.");
  assert((await fixture.people.read()).speaker_links[0].person_id === duplicates[1].id, "Relinking used the display name instead of the selected identity.");
  dialog = await speakerPicker(t);
  action(dialog.querySelector(".dialog-actions"), t("people.unlink")).click();
  await until(() => !dialog.isConnected, "Speaker unlinking did not finish.");
  assert((await fixture.people.read()).speaker_links.length === 0, "Speaker unlinking retained the association.");
  const after = await api.invoke("meeting_detail", { id });
  assert(JSON.stringify(after.speakers) === JSON.stringify(original.speakers), "Person linkage changed canonical speaker names.");
  assert(!fixture.calls.some(command => ["rename_speaker", "remember_speaker", "start_voice_enrollment"].includes(command)), "Person linkage renamed or enrolled a voice.");
  dialog = await speakerPicker(t); choose(dialog, duplicates[1].id);
  await saveManual(dialog, t, "Renamed synthetic person", "people.rename_person");
  assert((await fixture.people.read()).people.find(person => person.id === duplicates[1].id)?.display_name === "Renamed synthetic person", "Renaming a person replaced its stable identity.");
  assert(JSON.stringify((await api.invoke("meeting_detail", { id })).speakers) === JSON.stringify(original.speakers), "Renaming a private person changed canonical speaker labels.");
  return accepted({ separateDuplicatePeople: true, explicitLinkRelinkUnlink: true, stableIdentityAfterRename: true, canonicalSpeakerNamesPreserved: true, noVoiceEnrollment: true, initialKeyboardFocus: true });
}

export async function checkPeopleMentions() {
  const { fixture, api, t } = await owned();
  tab("notes");
  const editor = await noteEditor();
  assert(editor, "The note editor is missing.");
  const original = "# 😀 Synthetic notes\n\n[plain Alex](https://example.invalid) and `@Alex` — HERE **kept**\n";
  editor.focus(); edit(editor, original);
  const start = original.indexOf("HERE"); editor.setSelectionRange(start, start + 4);
  editor.dispatchEvent(new Event("select"));
  let dialog = await open(".people-insert");
  const duplicatePeople = (await fixture.people.read()).people.filter(person => person.display_name === "Alex");
  choose(dialog, duplicatePeople[0].id);
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => !dialog.isConnected, "Mention insertion did not finish.");
  const expected = original.replace("HERE", "@Alex");
  assert(editor.value === expected && fixture.workspace.notes() === expected, "Mention insertion changed surrounding Markdown or was not acknowledged.");
  dialog = await open();
  action(dialog.querySelector(".people-mention-row"), t("people.relink")).click();
  choose(dialog, duplicatePeople[1].id);
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => !dialog.isConnected, "Mention relinking did not finish.");
  assert((await fixture.people.read()).mentions[0].person_id === duplicatePeople[1].id, "Mention relinking merged same-name identities.");
  assert(editor.value === expected && fixture.workspace.notes() === expected, "Mention relinking changed authored text.");
  dialog = await open();
  action(dialog.querySelector(".people-mention-row"), t("people.unlink")).click();
  await until(async () => (await fixture.people.read()).mentions.length === 0, "Mention unlinking did not finish.");
  assert(editor.value === expected && fixture.workspace.notes() === expected, "Mention unlinking removed or rewrote authored text.");
  const contact = (await fixture.people.read()).people.find(person => person.origin === "contact");
  choose(dialog, contact.id);
  const exportResult = await api.invoke("export_meeting", { id: fixture.options.meeting_id, includeTranscript: true });
  assert(!JSON.stringify(exportResult).includes(contact.email) && !JSON.stringify(exportResult).includes(contact.private_alias), "Export enriched content with private contact details.");
  return accepted({ exactMarkdown: true, explicitMentionRelinkUnlink: true, stableDuplicateIdentity: true, privateDetailsExcludedFromExport: true });
}

export async function checkPeopleContactSnapshot() {
  const { fixture, t } = await owned();
  tab("notes");
  const editor = await noteEditor(), original = editor.value;
  editor.focus(); editor.setSelectionRange(editor.value.length, editor.value.length); editor.dispatchEvent(new Event("select"));
  let dialog = await open(".people-insert");
  const count = (await fixture.people.read()).people.length;
  action(dialog, t("people.choose_contact")).click();
  await until(async () => (await fixture.people.read()).people.length === count + 1, "The chosen contact snapshot was not stored.");
  assert(dialog.querySelector(".people-visible-field input").value === "" && dialog.querySelector(".dialog-actions .primary").disabled,
    "A contact-only name was silently offered as authored Markdown.");
  assert(editor.value === original && fixture.workspace.notes() === original, "Choosing a contact changed notes before visible-name confirmation.");
  edit(dialog.querySelector(".people-visible-field input"), "Visible synthetic person");
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => !dialog.isConnected, "The confirmed contact mention did not finish.");
  assert(editor.value === `${original}@Visible synthetic person`, "The explicit visible name was not used verbatim.");
  dialog = await open();
  const contact = (await fixture.people.read()).people.at(-1); choose(dialog, contact.id);
  assert(dialog.querySelector(".people-selected").textContent.includes(t("people.contact_snapshot")), "The one-time contact snapshot was presented as live verified data.");
  dialog.querySelector(".people-selected").scrollIntoView({ block: "nearest" });
  return accepted({ explicitContactSelection: true, noImplicitContactNameInNotes: true, explicitVisibleName: true, snapshotStatus: true });
}

export async function checkPeopleLateAndChangedInput() {
  const { fixture, api, t } = await owned();
  tab("notes");
  let dialog = await open();
  const before = await fixture.people.read(); fixture.people.contactDelay(450);
  action(dialog, t("people.choose_contact")).click();
  await until(() => dialog.querySelector("input[type=search]").disabled, "Pending native selection was not acknowledged.");
  cancel(dialog); await pause(650);
  assert((await fixture.people.read()).people.length === before.people.length, "A contact result applied after its picker closed.");
  const editor = await noteEditor();
  editor.focus(); edit(editor, "Original **Markdown** to keep."); editor.setSelectionRange(0, 8); editor.dispatchEvent(new Event("select"));
  dialog = await open(".people-insert");
  choose(dialog, before.people[0].id);
  editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  editor.value = "Original composed text **kept**.";
  editor.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, inputType: "insertCompositionText", data: "composed" }));
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => dialog.querySelector(".error")?.textContent.includes(t("people.selection_changed")), "Changed composing input did not refuse mention insertion.");
  assert(editor.value === "Original composed text **kept**." && !(await fixture.people.read()).mentions.length, "A late mention overwrote composition or created a false anchor.");
  cancel(dialog);
  editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "composed" }));
  editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "composed" }));
  await until(() => fixture.workspace.notes() === editor.value, "Composition did not retain its ordinary save path.");
  dialog = await open(); fixture.people.failWrites(true);
  await saveManual(dialog, t, "Unsaved synthetic person");
  await until(() => dialog.querySelector(".error")?.textContent.includes("Synthetic participant storage unavailable"), "Participant storage failure was hidden.");
  assert(!(await fixture.people.read()).people.some(person => person.display_name === "Unsaved synthetic person"), "A failed participant save was claimed durable.");
  fixture.people.failWrites(false); cancel(dialog);
  dialog = await speakerPicker(t); choose(dialog, before.people[0].id);
  const detail = await api.invoke("meeting_detail", { id: fixture.options.meeting_id });
  detail.transcript[0].text += " Changed synthetic source."; fixture.workspace.replaceTranscript(detail.transcript);
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => dialog.querySelector(".error")?.textContent.includes("The speaker source changed"), "A stale speaker selection was not rejected.");
  assert(!(await fixture.people.read()).speaker_links.length, "A stale source produced a speaker association.");
  cancel(dialog); tab("notes"); dialog = await open();
  return accepted({ lateContactDiscarded: true, composingTextPreserved: true, changedSelectionRejected: true, ordinaryNotesRecovery: true, honestStorageFailure: true, staleSpeakerSourceRejected: true });
}

export async function checkPeopleUnavailableContacts() {
  const { fixture, t } = await owned();
  tab("notes");
  const dialog = await open();
  assert(action(dialog, t("people.choose_contact")).disabled, "An unsupported platform enabled native contact access.");
  assert(dialog.textContent.includes(t("people.contact_unavailable")), "Unavailable contacts have no explanation.");
  await saveManual(dialog, t, "Manual offline guest");
  assert((await fixture.people.read()).people.some(person => person.display_name === "Manual offline guest"), "Manual entry depended on native contacts.");
  assert(!fixture.calls.includes("pick_meeting_contact"), "Unsupported contact access was attempted.");
  return accepted({ visibleUnavailableContactControl: true, manualEntryAvailable: true, noNativeContactRequest: true });
}

export async function openPeopleForKeyboard() {
  await owned(); await noteEditor();
  await until(() => document.querySelector(".people-manage"), "Participant controls did not mount.");
  document.querySelector(".people-manage").focus();
  const dialog = await open();
  assert(document.activeElement === dialog.querySelector("input[type=search]"), "The picker did not focus its search field.");
}

export function checkPeopleKeyboardChoice() {
  assert(document.activeElement?.classList.contains("people-choice"), "Tab did not reach the first participant choice.");
}

export function checkPeopleKeyboardSelected() {
  assert(document.activeElement?.getAttribute("aria-pressed") === "true", `Enter did not select the focused participant: active=${document.activeElement?.outerHTML.slice(0, 200)}; selected=${document.querySelector(".people-choice.selected")?.outerHTML.slice(0, 200)}`);
}

export function checkPeopleKeyboardReturn() {
  assert(!document.querySelector(".people-picker"), "Escape did not close the participant picker.");
  assert(document.activeElement?.classList.contains("people-manage"), `Closing the picker did not return focus to its invoking control: ${document.activeElement?.outerHTML.slice(0, 220)}`);
  return accepted({ browserTabTraversal: true, browserEnterSelection: true, browserEscapeCancellation: true, invokerFocusRestored: true });
}

export async function checkPeoplePendingConfirmation() {
  const { fixture, t } = await owned(); tab("notes");
  const editor = await noteEditor(), original = editor.value;
  editor.focus(); editor.setSelectionRange(original.length, original.length); editor.dispatchEvent(new Event("select"));
  const dialog = await open(".people-insert"), person = (await fixture.people.read()).people[0];
  choose(dialog, person.id); dialog.querySelector(".dialog-actions .primary").click();
  await until(() => dialog.isConnected && dialog.textContent.includes(t("people.confirmation_pending")), "Pending confirmation closed the picker or hid recovery status.");
  const written = `${original}@${person.display_name}`;
  assert(editor.value === written && fixture.workspace.notes() === written, "Written note text was not acknowledged during metadata confirmation failure.");
  assert(dialog.querySelector(".dialog-actions .primary").hidden, "The picker offered duplicate insertion while confirmation was pending.");
  action(dialog, t("common.retry")).click();
  await until(() => !dialog.textContent.includes(t("people.confirmation_pending")), "Retry did not reconcile pending mention confirmation.");
  assert(editor.value === written && fixture.workspace.notes() === written && (await fixture.people.read()).mentions.length === 1, "Confirmation retry duplicated or replaced the written mention.");
  return accepted({ writtenNotesAcknowledged: true, confirmationPendingVisible: true, duplicateInsertionUnavailable: true, retryReconcilesWithoutDuplicate: true });
}

export async function beginPeopleDelayedInsert() {
  const { fixture, api, t } = await owned(), editor = await noteEditor(), original = editor.value;
  editor.focus(); editor.setSelectionRange(original.length, original.length); editor.dispatchEvent(new Event("select"));
  const dialog = await open(".people-insert"), person = (await fixture.people.read()).people[0];
  choose(dialog, person.id); fixture.people.insertDelay(30000);
  const expected = `${original}@${person.display_name}`;
  window.__PENNEL_PEOPLE_PENDING__ = { fixture, api, t, editor, dialog, expected, visibleName: person.display_name };
  dialog.querySelector(".dialog-actions .primary").click();
  await until(() => fixture.people.pendingInsertResponses() === 1, "The fixture did not hold the written mention response.");
  assert(fixture.workspace.notes() === expected, "The delayed response did not follow a completed durable note write.");
}

export function checkPeopleDelayedDismissal() {
  const { dialog, t } = window.__PENNEL_PEOPLE_PENDING__;
  assert(dialog.isConnected, "Escape dismissed an in-flight canonical Markdown insertion.");
  assert(dialog.querySelector("input[type=search]").disabled && dialog.querySelector(".dialog-actions .primary").disabled,
    "The pending Markdown operation lost its explicit busy state.");
  action(dialog.querySelector(".dialog-actions"), t("common.cancel")).click();
  assert(dialog.isConnected, "Cancel dismissed an in-flight canonical Markdown insertion.");
}

export async function finishPeopleDelayedInsert() {
  const { fixture, editor, dialog, expected, visibleName } = window.__PENNEL_PEOPLE_PENDING__;
  fixture.people.releaseInserts();
  await until(() => editor.value === expected && !dialog.isConnected, "The durable mention response was not acknowledged in its original editor.");
  const next = `${expected}\nA following ordinary edit.`;
  editor.focus(); edit(editor, next);
  await until(() => fixture.workspace.notes() === next, "The ordinary writer did not preserve the acknowledged mention.");
  assert(fixture.workspace.notes().split(`@${visibleName}`).length - 1 === 1, "The next ordinary save removed or duplicated the mention.");
  delete window.__PENNEL_PEOPLE_PENDING__;
  return accepted({ delayedDurableResponse: true, escapeAndCancelPreserveOwnedAcknowledgement: true, nextOrdinarySavePreservesSingleMention: true });
}

export async function checkPeopleDelayedHide() {
  await beginPeopleDelayedInsert();
  const { fixture, editor, dialog, expected } = window.__PENNEL_PEOPLE_PENDING__, requestId = 1027001;
  fixture.emit("prepare-hide", requestId);
  await until(() => fixture.frontendFlushes().some(reply => reply.requestId === requestId), "Hiding waited for the delayed mention IPC response.");
  assert(fixture.frontendFlushes().find(reply => reply.requestId === requestId).success, "Hiding failed while the canonical mention response was pending.");
  assert(fixture.people.pendingInsertResponses() === 1, "The hide probe did not run during the pending canonical response.");
  fixture.emit("hide-flush-complete", requestId);
  fixture.emit("workspace-visibility-changed", { visible: false });
  fixture.people.releaseInserts();
  await until(() => editor.value === expected && !dialog.isConnected, "The hidden window did not acknowledge its original durable mention.");
  fixture.emit("workspace-visibility-changed", { visible: true });
  await finishPeopleDelayedInsert();
  return accepted({ delayedDurableResponse: true, hideFlushRemainsAvailable: true, hiddenWindowAcknowledgesOriginalDraft: true, nextOrdinarySavePreservesSingleMention: true });
}

export async function checkPeopleDelayedNavigation() {
  await beginPeopleDelayedInsert();
  const { fixture, api, dialog, expected, visibleName } = window.__PENNEL_PEOPLE_PENDING__, id = fixture.options.meeting_id;
  fixture.emit("workspace-navigate", { section: "settings", tab: "overview", meeting_id: null, jump_ms: null });
  await until(() => document.querySelector(".settings-back"), "Settings navigation waited for the delayed mention response.");
  assert(fixture.people.pendingInsertResponses() === 1, "Navigation did not occur during the pending canonical response.");
  fixture.people.releaseInserts();
  await until(() => !dialog.isConnected, "An obsolete insertion dialog remained after acknowledging its owner.");
  fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.meetingId === id, "The original meeting did not reopen.");
  const editor = await noteEditor();
  assert(editor.value === expected, "Remounting the original editor lost its delayed durable mention.");
  const next = `${expected}\nAn edit after returning.`; editor.focus(); edit(editor, next);
  await until(async () => (await api.invoke("meeting_detail", { id })).notes === next, "The returning editor overwrote its acknowledged mention.");
  assert(next.split(`@${visibleName}`).length - 1 === 1, "Navigation duplicated the authored mention.");
  delete window.__PENNEL_PEOPLE_PENDING__;
  return accepted({ navigationRemainsAvailable: true, detachedOwnerAcknowledgesDurableNotes: true, originalDraftRestored: true, nextSavePreservesSingleMention: true });
}

export async function checkPeopleDelayedSessionReplacement() {
  await beginPeopleDelayedInsert();
  const { fixture, api, dialog, expected, visibleName } = window.__PENNEL_PEOPLE_PENDING__;
  const original = await api.invoke("meeting_status");
  await api.invoke("stop_meeting", { sessionId: original.session_id });
  assert(!fixture.workspace.recording() && fixture.people.pendingInsertResponses() === 1, "Global Stop waited for the delayed mention response.");
  const otherNotes = "Different synthetic meeting must stay unchanged.";
  const next = fixture.workspace.restart({ id: "qa-other-meeting", uid: "qa-other-stable-uid", session: "qa-other-recording", title: "Other synthetic meeting", notes: otherNotes, transcript: [] });
  fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The replacement session did not open.");
  const otherEditor = await noteEditor(); assert(otherEditor.value === otherNotes, "The replacement editor did not show its own notes.");
  fixture.people.releaseInserts();
  await until(() => !dialog.isConnected, "The obsolete insertion dialog remained in the replacement session.");
  assert(otherEditor.value === otherNotes && fixture.workspace.notes() === otherNotes, "A late mention result changed the replacement meeting.");
  fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: original.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === original.session_id, "The original stopped workspace did not reopen.");
  const editor = await noteEditor(); assert(editor.value === expected, "The original stopped editor lost its delayed mention acknowledgement.");
  const text = `${expected}\nAn original-meeting edit.`; editor.focus(); edit(editor, text);
  await until(async () => (await api.invoke("meeting_detail", { id: original.id })).notes === text, "Saving the original editor lost its acknowledged mention.");
  assert(text.split(`@${visibleName}`).length - 1 === 1 && fixture.workspace.notes() === otherNotes, "The following save duplicated the mention or retargeted another meeting.");
  delete window.__PENNEL_PEOPLE_PENDING__;
  return accepted({ stopRemainsImmediate: true, replacementSessionIsolated: true, lateResultAcknowledgesOriginalOnly: true, originalNextSavePreservesMention: true });
}
