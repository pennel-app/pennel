import { conversationRequests as requests } from "../../../assets/contracts/meeting-conversations.ts";

const assert = (value, message) => { if (!value) throw Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 360; attempt++) { if (await check()) return; await pause(); }
  throw Error(message);
}
function edit(input, text) { input.value = text; input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text })); }
function questions() {
  const tab = document.querySelector('[data-live-tab="questions"]') || document.querySelector('.detail-tabs [data-tab="ask"]');
  if (tab?.getAttribute("aria-selected") !== "true") tab?.click();
}
function action(root, text) { return [...root?.querySelectorAll("button") || []].find(button => button.textContent.trim() === text); }
function accepted(values) {
  const root = [...document.querySelectorAll(".conversation-panel")].find(root => root.offsetParent !== null);
  if (root) {
    const history = root.querySelector(".chat-history").getBoundingClientRect(), input = root.querySelector(".conversation-input").getBoundingClientRect();
    const picker = root.querySelector(".conversation-select").getBoundingClientRect();
    const submit = root.querySelector(".conversation-submit, .live-question-submit").getBoundingClientRect();
    assert(history.height >= 64, "The minimum window leaves no usable history viewport.");
    assert(input.height >= 40 && input.bottom <= innerHeight + 1, `The minimum window clips the composer: height=${input.height}, bottom=${input.bottom}, viewport=${innerHeight}.`);
    assert(submit.bottom <= innerHeight - 6, `The minimum window clips the submit action: bottom=${submit.bottom}, workspace inset=${innerHeight - 6}.`);
    assert(picker.width >= 120 && picker.height <= 40, "The conversation selector is squeezed into an unreadable control.");
  }
  return window.__PENNEL_CONVERSATION_ACCEPTANCE__ = { ...values, minimumWindowUsable: true, syntheticOnly: true };
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic conversation fixture.");
  await until(() => document.querySelector(".conversation-panel"), "The conversation panel did not mount.");
  questions();
  await until(() => document.querySelector(".conversation-panel .conversation-input:not(:disabled)"), "The conversation did not finish loading.");
  const { t, localizeBackendText } = await import("/src/i18n.ts");
  const fixture = window.__PENNEL_FIXTURE__, scope = fixture.options.tab === "chat" ? { kind: "library" } : { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  return { fixture, api: window.__PENNEL_PLATFORM__, root: document.querySelector(".conversation-panel"), scope, t, localizeBackendText };
}
const snapshot = (api, scope, id, cursors) => api.invoke("conversation_snapshot", { request: requests.snapshot(scope, id, cursors) });
async function menuAction(root, name) {
  const { t } = await import("/src/i18n.ts");
  root.querySelector(".conversation-actions").click();
  await until(() => action(document.querySelector(".choice-list"), t(`conversations.${name}`)), "The conversation action menu did not open.");
  action(document.querySelector(".choice-list"), t(`conversations.${name}`)).click();
}
async function draft(root, api, scope, text) {
  edit(root.querySelector(".conversation-input"), text);
  await until(async () => (await snapshot(api, scope)).conversation.draft === text, "The composer draft was not acknowledged by the durable store.");
}
async function rename(root, title) {
  await menuAction(root, "rename");
  await until(() => document.querySelector("dialog form input"), "Conversation renaming did not open.");
  const form = document.querySelector("dialog form"); edit(form.querySelector("input"), title);
  form.dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  await until(() => root.querySelector(".conversation-select").textContent === title, "Conversation renaming was not acknowledged.");
}
async function create(root, api, scope) {
  const previous = (await snapshot(api, scope)).conversation.id;
  root.querySelector(".conversation-new").click();
  await until(async () => (await snapshot(api, scope)).conversation.id !== previous && !root.querySelector(".conversation-select").disabled,
    "A new conversation did not become selectable.");
  return (await snapshot(api, scope)).conversation.id;
}
async function select(root, title) {
  root.querySelector(".conversation-select").click();
  await until(() => action(document.querySelector(".choice-list"), title), "The requested conversation did not appear in the picker.");
  action(document.querySelector(".choice-list"), title).click();
  await until(() => !root.querySelector(".conversation-select").disabled && root.querySelector(".conversation-select").textContent === title,
    "Switching conversation did not finish.");
}
async function remove(root, api, scope) {
  const previous = (await snapshot(api, scope)).conversation.id;
  await menuAction(root, "delete");
  await until(() => document.querySelector("dialog .danger"), "Conversation deletion did not ask about its specific conversation.");
  document.querySelector("dialog .danger").click();
  await until(async () => (await snapshot(api, scope)).conversation.id !== previous && !root.querySelector(".conversation-select").disabled,
    "Deleting the selected conversation did not finish.");
  return previous;
}
async function ask(root, fixture, question) {
  const previous = new Set(fixture.conversations.requests().map(request => request.request_id));
  edit(root.querySelector(".conversation-input"), question);
  await until(() => !(root.querySelector(".conversation-submit") || root.querySelector(".live-question-submit")).disabled, "The question could not be submitted.");
  (root.querySelector(".conversation-submit") || root.querySelector(".live-question-submit")).click();
  await until(() => fixture.conversations.requests().some(request => !previous.has(request.request_id)), "The core did not admit the durable question.");
  const request = fixture.conversations.requests().find(request => !previous.has(request.request_id));
  await until(() => root.querySelector(`[data-request-id="${request.request_id}"]`), "The pending answer did not appear.");
  return request;
}
async function complete(root, fixture, request, answer) {
  assert(await fixture.conversations.complete(request.request_id, answer), "The original request could not complete.");
  await until(() => root.querySelector(`[data-request-id="${request.request_id}"][data-message-status="completed"]`), "The completed answer was not restored from core storage.");
  return root.querySelector(`[data-request-id="${request.request_id}"]`);
}

export async function checkConversationLifecycle() {
  const { root, fixture, api, scope } = await owned(), originalNotes = fixture.workspace.notes();
  await rename(root, "Alpha synthetic conversation");
  const alpha = (await snapshot(api, scope)).conversation.id;
  await draft(root, api, scope, "Alpha synthetic draft");
  const beta = await create(root, api, scope); await rename(root, "Beta synthetic conversation");
  await draft(root, api, scope, "Beta synthetic draft");
  await select(root, "Alpha synthetic conversation");
  assert(root.querySelector(".conversation-input").value === "Alpha synthetic draft", "Switching did not restore the original composer draft.");
  await select(root, "Beta synthetic conversation");
  assert(root.querySelector(".conversation-input").value === "Beta synthetic draft", "The second conversation lost its own draft.");
  const deleted = await remove(root, api, scope), remaining = await snapshot(api, scope);
  assert(deleted === beta && remaining.conversation.id === alpha, "Deleting one conversation selected an unrelated identity.");
  assert(!remaining.conversations.some(conversation => conversation.id === beta), "The deleted conversation remained selectable.");
  const otherScope = scope.kind === "library" ? { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" } : { kind: "library" };
  const other = await snapshot(api, otherScope);
  assert(other.conversation.id !== alpha && other.conversation.draft === "", "A meeting and the library shared the same conversation or composer draft.");
  assert(fixture.workspace.notes() === originalNotes, "Conversation actions changed ordinary meeting notes.");
  return accepted({ explicitCreateSwitchRenameDelete: true, stableConversationIdentity: true, perConversationDrafts: true, meetingLibraryScopesIsolated: true, meetingNotesPreserved: true });
}

export async function checkConversationBackgroundAndCancellation() {
  const { root, fixture, api, scope, t } = await owned();
  await rename(root, "Original synthetic conversation");
  const original = (await snapshot(api, scope)).conversation.id, request = await ask(root, fixture, "A background synthetic question");
  await create(root, api, scope); await rename(root, "Different synthetic conversation");
  await draft(root, api, scope, "Draft in a different conversation");
  assert(await fixture.conversations.complete(request.request_id), "Background completion was discarded despite its original conversation existing.");
  await until(async () => (await snapshot(api, scope, original)).messages.some(message => message.request_id === request.request_id && message.status === "completed" && message.role === "assistant"), "Background completion was not saved to its original conversation.");
  assert(!root.textContent.includes("Synthetic frozen answer") && root.querySelector(".conversation-input").value === "Draft in a different conversation",
    "A background answer or composer draft crossed into the selected conversation.");
  await select(root, "Original synthetic conversation");
  await until(() => root.textContent.includes("Synthetic frozen answer"), "Returning to the original conversation lost its background answer.");
  const cancelled = await ask(root, fixture, "Cancel this synthetic question");
  (root.querySelector(".conversation-cancel") || root.querySelector(".live-question-cancel")).click();
  await until(() => root.querySelector(`[data-request-id="${cancelled.request_id}"][data-message-status="cancelled"]`), "Cancellation was not displayed as incomplete.");
  assert(await fixture.conversations.complete(cancelled.request_id, { text: "A cancelled late answer", citations: [] }) === false, "A cancelled request accepted a late completion.");
  assert(!root.textContent.includes("A cancelled late answer"), "A cancelled late answer appeared in history.");
  const failed = await ask(root, fixture, "Fail this synthetic question");
  await fixture.conversations.complete(failed.request_id, { error: "Synthetic provider unavailable.", partial_text: "Incomplete synthetic text" });
  await until(() => root.querySelector(`[data-request-id="${failed.request_id}"][data-message-status="failed"]`), "The failed answer looked complete.");
  assert(root.textContent.includes(t("conversations.status_failed")), "The incomplete failure state was not localized.");
  return accepted({ backgroundSavedOnlyToOriginal: true, selectedComposerPreserved: true, cancellationExplicit: true, lateCancelledCompletionRejected: true, failedAnswerIncomplete: true });
}

export async function prepareConversationReload() {
  const { root, fixture, api, scope } = await owned();
  await rename(root, "Persistent alpha conversation");
  const request = await ask(root, fixture, "A question retained through restart");
  const answer = { text: "Persistent synthetic answer [1]", citations: [{ number: 1, passage: request.anchors[0].passage }] };
  await complete(root, fixture, request, answer);
  const alpha = await snapshot(api, scope), beta = await create(root, api, scope);
  await rename(root, "Selected beta conversation");
  await draft(root, api, scope, "A selected draft retained through restart.");
  fixture.conversations.checkpoint({ alpha: alpha.conversation.id, beta, message: alpha.messages.at(-1).id,
    cutoff: alpha.messages.at(-1).evidence.included_through_ms });
}

export async function checkConversationAfterReload() {
  const { root, fixture, api, scope } = await owned(), checkpoint = fixture.conversations.checkpoint();
  const restored = await snapshot(api, scope);
  assert(restored.conversation.id === checkpoint.beta, "A full page reload changed the selected conversation identity.");
  assert(root.querySelector(".conversation-input").value === "A selected draft retained through restart.", "The composer draft did not survive full process-state replacement.");
  await select(root, "Persistent alpha conversation");
  await until(() => root.querySelector(`[data-message-id="${checkpoint.message}"]`)?.textContent.includes("Persistent synthetic answer"), "Reload lost a completed answer or its original message identity.");
  const alpha = await snapshot(api, scope);
  assert(alpha.conversation.id === checkpoint.alpha && alpha.messages.at(-1).evidence.included_through_ms === checkpoint.cutoff, "Restart changed conversation identity or the immutable answer cutoff.");
  return accepted({ fullReloadRestoresSelectedConversation: true, composerDraftRestored: true, completedMessageIdentityRestored: true, answerCutoffImmutable: true });
}

export async function prepareInterruptedConversationReload() {
  const { root, fixture, api, scope } = await owned(), request = await ask(root, fixture, "A pending synthetic question at restart");
  fixture.conversations.checkpoint({ conversation: (await snapshot(api, scope)).conversation.id, request: request.request_id });
}

export async function checkConversationInterruptedAfterReload() {
  const { root, fixture, api, scope, t } = await owned(), checkpoint = fixture.conversations.checkpoint();
  await until(() => root.querySelector(`[data-request-id="${checkpoint.request}"][data-message-status="interrupted"]`), "A pending reply was not marked interrupted after restart.");
  const state = await snapshot(api, scope), row = root.querySelector(`[data-request-id="${checkpoint.request}"]`);
  assert(state.conversation.id === checkpoint.conversation && state.conversation.active_request_id === null, "Restart resumed or retargeted an interrupted request.");
  assert(state.messages.at(-1).answer === null && row.textContent.includes(t("conversations.status_interrupted")), "An interrupted reply was displayed as a completed answer.");
  assert(!fixture.calls.includes("begin_conversation_question"), "Reopening automatically re-sent a pending question.");
  action(row, t("common.retry")).click();
  await until(() => root.querySelector(".conversation-input").value === "A pending synthetic question at restart", "Explicit retry did not recover the original question as a draft.");
  assert(!fixture.calls.includes("begin_conversation_question"), "Recovering a retry draft sent a question without explicit submission.");
  return accepted({ pendingBecomesInterrupted: true, noAutomaticResubmission: true, incompleteReplyExplicit: true, explicitRetryRecoversQuestion: true });
}

export async function checkConversationStorageRecovery() {
  const { root, fixture, api, scope, localizeBackendText } = await owned();
  assert((await snapshot(api, scope)).storage_status === "unsaved", "The storage-outage fixture did not start unsaved.");
  const question = "An unsaved question must remain usable", request = await ask(root, fixture, question);
  await complete(root, fixture, request);
  const before = await snapshot(api, scope);
  assert(before.storage_status === "unsaved" && root.querySelector(".conversation-storage-status").textContent === localizeBackendText(before.storage_error), "A completed answer falsely claimed durability or hid its storage reason.");
  await menuAction(root, "copy");
  await until(() => fixture.copiedTexts().length, "The retained conversation could not be copied.");
  assert(fixture.copiedTexts().at(-1).includes(question) && fixture.copiedTexts().at(-1).includes("Synthetic frozen answer"), "Recovery copy lost the question or completed answer.");
  fixture.conversations.failStorage(false); root.querySelector(".conversation-retry").click();
  await until(async () => (await snapshot(api, scope)).storage_status === "saved" && root.querySelector(".conversation-retry").hidden, "Retry did not acknowledge durable storage recovery.");
  const after = await snapshot(api, scope);
  assert(JSON.stringify(after.messages.map(message => message.id)) === JSON.stringify(before.messages.map(message => message.id)), "Retry duplicated or replaced retained messages.");
  return accepted({ storageOutageKeepsAskUsable: true, honestUnsavedStatus: true, recoveryCopyIncludesQuestionAndAnswer: true, retrySavesWithoutDuplicate: true });
}

export async function checkConversationDeletedRequest() {
  const { root, fixture, api, scope } = await owned(), notes = fixture.workspace.notes();
  const pending = await ask(root, fixture, "Delete this pending synthetic conversation"), deleted = await remove(root, api, scope);
  assert(await fixture.conversations.complete(pending.request_id, { text: "A late deleted answer", citations: [] }) === false, "A deleted conversation accepted a late answer.");
  const current = await snapshot(api, scope);
  assert(current.conversation.id !== deleted && !current.conversations.some(conversation => conversation.id === deleted) && !current.messages.length,
    "A late answer recreated deleted conversation data.");
  assert(fixture.workspace.notes() === notes && !root.textContent.includes("A late deleted answer"), "Conversation deletion changed notes or crossed a late answer into its replacement.");
  return accepted({ deletionScopedToConversation: true, lateDeletedCompletionRejected: true, deletedConversationNotRecreated: true, meetingNotesPreserved: true });
}

export async function checkConversationSources() {
  let { root, fixture, api, scope, t } = await owned();
  const request = await ask(root, fixture, "Which exact synthetic source supports the answer?");
  const row = await complete(root, fixture, request), messageId = row.dataset.messageId, conversationId = (await snapshot(api, scope)).conversation.id;
  row.querySelector(".citation").click();
  if (root.classList.contains("live-conversation-panel")) {
    await until(() => document.querySelector(".live-source-highlight"), "The exact live transcript anchor did not open.");
    assert(document.querySelector(".live-source-highlight").dataset.segmentKey === request.anchors[0].segment_keys[0], "The citation opened another utterance.");
    document.querySelector(".live-source-back").click();
  } else {
    await until(() => document.querySelector(".source-highlight") && document.querySelector(".meeting-source-return"), "The reviewed transcript source did not open with a return path.");
    document.querySelector(".meeting-source-return").click();
  }
  await until(() => [...document.querySelectorAll(".conversation-panel")].some(panel => panel.offsetParent !== null && panel.querySelector(`[data-message-id="${messageId}"]`)), "Citation return lost the original conversation or message.");
  root = [...document.querySelectorAll(".conversation-panel")].find(panel => panel.offsetParent !== null);
  assert((await snapshot(api, scope)).conversation.id === conversationId, "Citation return switched the selected conversation.");
  fixture.workspace.attribution(0, { confirmed_speaker_name: "Changed synthetic label" });
  await until(() => root.querySelector(`[data-message-id="${messageId}"] .citation`)?.textContent.includes(t("conversations.source_stale")), "Speaker-label changes did not mark stored references stale.");
  root.querySelector(`[data-message-id="${messageId}"] .citation`).click();
  await until(() => document.querySelector(".conversation-source-notice:not([hidden]), .live-source-notice:not([hidden])"), "Stale citation navigation did not explain its preserved source.");
  assert(!document.querySelector(".live-source-highlight, .source-highlight"), "A stale reference highlighted an unverified current source.");
  const detail = await api.invoke("meeting_detail", { id: fixture.options.meeting_id });
  fixture.workspace.replaceTranscript(detail.transcript.filter(segment => segment.utterance_id !== request.anchors[0].passage.utterance_ids[0]));
  await until(() => root.querySelector(`[data-message-id="${messageId}"] .citation`)?.textContent.includes(t("conversations.source_unavailable")), "A replaced transcript retained a falsely valid citation.");
  assert(root.querySelector(`[data-message-id="${messageId}"]`).textContent.includes("Synthetic frozen answer"), "Source changes removed the readable original answer.");
  if (scope.kind === "library") {
    await api.invoke("delete_meeting", { id: fixture.options.meeting_id });
    assert((await snapshot(api, scope)).messages.some(message => message.id === messageId), "Deleting a cited meeting deleted the library conversation.");
  }
  return accepted({ exactCitationAndReturn: true, conversationAndMessageRestored: true, speakerRevisionMarksStale: true, staleNavigationRevalidated: true,
    transcriptReplacementMarksUnavailable: true, originalAnswerRemainsReadable: true });
}

export async function checkConversationFinalizationAndRename() {
  const { root, fixture, api, scope } = await owned(), status = await api.invoke("meeting_status");
  await rename(root, "Recording conversation");
  const id = (await snapshot(api, scope)).conversation.id, request = await ask(root, fixture, "Keep the recording answer through finalization");
  fixture.workspace.append({ start_ms: 22000, end_ms: 29000, speaker: "others", voice: 1, text: "Later synthetic speech.", failed: false });
  document.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording() && document.querySelector(".live-workspace").dataset.recording === "false", "Finalization waited for the pending answer.");
  const renamed = await api.invoke("rename_meeting", { id: status.id, title: "Automatically renamed synthetic meeting" });
  fixture.emit("meeting-finished", { id: renamed, meeting_uid: status.meeting_uid, session_id: status.session_id, error: null, flush_warning: null });
  await complete(root, fixture, request);
  await draft(root, api, scope, "Review draft after automatic rename");
  const restored = await snapshot(api, scope), answer = restored.messages.find(message => message.request_id === request.request_id && message.role === "assistant");
  assert(restored.conversation.id === id && restored.scope.meeting_uid === status.meeting_uid && restored.current_meeting_id === renamed,
    "Recording finalization or directory rename changed the conversation identity.");
  assert(answer.evidence.included_through_ms === 21000 && answer.evidence.sources.every(source => source.included_through_ms === 21000), "Later speech changed the old answer's transcript cutoff.");
  return accepted({ stopIndependentFromAnswer: true, liveToReviewConversationContinuous: true, renameKeepsStableMeetingAndConversation: true, immutableCutoffAfterAppend: true, reviewDraftPreserved: true });
}

export async function checkConversationReplacedSession() {
  await owned();
  const fixture = window.__PENNEL_FIXTURE__, api = window.__PENNEL_PLATFORM__, original = await api.invoke("meeting_status");
  const oldRoot = document.querySelector(".conversation-panel"), pending = await ask(oldRoot, fixture, "A question from a replaced recording session");
  const next = fixture.workspace.restart({ id: "qa-replaced-session", uid: original.meeting_uid, session: "qa-replacement-token", title: "Replacement synthetic session", notes: "Replacement note sentinel", transcript: [] });
  fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The replacement recording session did not mount.");
  questions();
  assert(await fixture.conversations.complete(pending.request_id, { text: "An invalid replaced-session answer", citations: [] }) === false, "The replaced recording session accepted a late completion.");
  const root = document.querySelector(".conversation-panel");
  await until(() => root.querySelector(`[data-request-id="${pending.request_id}"][data-message-status="interrupted"]`), "The replaced request was not displayed as incomplete.");
  assert(!root.textContent.includes("An invalid replaced-session answer") && fixture.workspace.notes() === "Replacement note sentinel", "A replaced-session completion entered current notes or history.");
  return accepted({ replacedSessionRejectsLateCompletion: true, interruptedStateVisible: true, replacementSourceTextPreserved: true });
}

export async function checkConversationDraftLifecycle() {
  let { root, fixture, api, scope } = await owned();
  const text = "First line\nA Dutch/Flemish draft: géén automatische vertaling.", id = (await snapshot(api, scope)).conversation.id;
  await draft(root, api, scope, text);
  const input = root.querySelector(".conversation-input"); input.focus(); input.setSelectionRange(4, 16, "backward");
  const requestId = 1137001; fixture.emit("prepare-hide", requestId);
  await until(() => fixture.frontendFlushes().some(reply => reply.requestId === requestId), "Hiding did not acknowledge the conversation draft flush.");
  assert(fixture.frontendFlushes().find(reply => reply.requestId === requestId).success, "Conversation storage prevented ordinary hide.");
  fixture.emit("hide-flush-complete", requestId); fixture.emit("workspace-visibility-changed", { visible: false });
  fixture.emit("workspace-visibility-changed", { visible: true });
  await until(() => input.value === text && !input.disabled, "Hiding and reopening lost the composer draft.");
  fixture.emit("workspace-navigate", { section: "settings", tab: "overview", meeting_id: null, jump_ms: null });
  await until(() => document.querySelector(".settings-back"), "Settings navigation did not remain available.");
  document.querySelector(".settings-back").click();
  await until(() => document.querySelector(".conversation-panel .conversation-input"), "Returning from Settings lost the conversation panel.");
  questions(); root = document.querySelector(".conversation-panel");
  await until(() => root.querySelector(".conversation-input").value === text, "Returning from Settings restored another draft.");
  assert((await snapshot(api, scope)).conversation.id === id, "Navigation changed the selected conversation identity.");
  assert(!fixture.calls.includes("begin_conversation_question"), "Saving or restoring a draft sent it for inference.");
  return accepted({ hideFlushAcknowledged: true, reopenKeepsComposer: true, settingsReturnKeepsConversationAndDraft: true, contentLanguageUnchanged: true, noInferenceFromDraft: true });
}

export async function checkConversationPaginationAndReturn() {
  let { root, fixture, api, scope } = await owned();
  await rename(root, "Archive conversation");
  const origin = (await snapshot(api, scope)).conversation.id;
  for (let index = 0; index < 54; index++) await api.invoke("update_conversation", { request: requests.create(scope, `Synthetic archive ${index}`) });
  await until(() => (root.querySelector(".conversation-actions").disabled === false), "The conversation toolbar did not stay available.");
  await pause(900); await menuAction(root, "more");
  await until(() => {
    root.querySelector(".conversation-select").click();
    const ready = document.querySelectorAll('.choice-list [role="option"]').length === 55;
    if (!ready) document.querySelector(".choice-list")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    return ready;
  }, "Loading more conversations omitted or duplicated a cursor page.");
  document.querySelector(".choice-list").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  for (let index = 0; index < 30; index++) {
    const requestId = crypto.randomUUID();
    await api.invoke("begin_conversation_question", { request: requests.begin(scope, origin, requestId, `Archive question ${index}`) });
    await fixture.conversations.complete(requestId);
  }
  await until(() => !root.querySelector(".conversation-older").hidden, "Long conversation history had no bounded older-message action.");
  root.querySelector(".conversation-older").click();
  await until(() => root.querySelectorAll(".conversation-assistant").length === 30, "The older message page did not join without duplicates.");
  const oldRow = root.querySelector(".conversation-assistant"), oldMessage = oldRow.dataset.messageId;
  oldRow.querySelector(".citation").click();
  await until(() => document.querySelector(".meeting-source-return") && document.querySelector(".source-highlight"), "An older answer's exact source did not open.");
  document.querySelector(".meeting-source-return").click();
  await until(() => [...document.querySelectorAll(".conversation-panel")].some(panel => panel.offsetParent !== null && panel.querySelector(`[data-message-id="${oldMessage}"]`)), "Source return did not load the original older message through its bounded focus request.");
  root = [...document.querySelectorAll(".conversation-panel")].find(panel => panel.offsetParent !== null);
  assert(document.activeElement?.dataset.messageId === oldMessage, "Returning from an old citation did not focus its originating message.");
  await pause(1600);
  const focused = root.querySelector(`[data-message-id="${oldMessage}"]`).getBoundingClientRect(), history = root.querySelector(".chat-history").getBoundingClientRect();
  assert(focused.bottom > history.top && focused.top < history.bottom, "Subsequent polling scrolled away from the originating older answer.");
  return accepted({ conversationsPaged: true, messagesPagedWithoutDuplicates: true, oldCitationLoadsExactMessage: true, sourceReturnFocusRestored: true });
}

export async function prepareConversationKeyboard() {
  const state = await owned(), { root, api, scope } = state;
  await rename(root, "Keyboard alpha"); await draft(root, api, scope, "Keyboard alpha draft");
  await create(root, api, scope); await rename(root, "Keyboard beta"); await draft(root, api, scope, "Keyboard beta draft");
  await select(root, "Keyboard alpha"); root.querySelector(".conversation-select").focus();
  window.__PENNEL_CONVERSATION_KEYBOARD__ = state;
}
export function checkConversationKeyboardMenu() { assert(document.activeElement?.getAttribute("role") === "option", "Enter did not open the conversation picker with keyboard focus."); }
export async function checkConversationKeyboardSelection() {
  const { root } = window.__PENNEL_CONVERSATION_KEYBOARD__;
  await until(() => root.querySelector(".conversation-input").value === "Keyboard beta draft", "Arrow/Enter did not switch to the focused conversation and its draft.");
  root.querySelector(".conversation-actions").focus();
}
export function checkConversationKeyboardActions() { assert(document.activeElement?.getAttribute("role") === "menuitem", "The native-style action menu did not receive keyboard focus."); }
export function checkConversationKeyboardDialog() { assert(document.activeElement?.matches("dialog form input"), "Keyboard Rename did not focus its name field."); }
export function finishConversationKeyboard() {
  assert(!document.querySelector("dialog") && document.activeElement?.classList.contains("conversation-actions"), "Escape did not cancel renaming and restore its invoking action control.");
  delete window.__PENNEL_CONVERSATION_KEYBOARD__;
  return accepted({ realKeyboardPickerTraversal: true, conversationDraftRestoredByKeyboard: true, realKeyboardActionMenu: true, renameEscapeRestoresFocus: true });
}

async function preparationProbe() {
  const state = await owned(), { root, fixture } = state;
  document.querySelector('[data-live-tab="notes"]').click();
  const editor = document.querySelector(".live-notes");
  fixture.workspace.notesDelay(30000); edit(editor, "Acknowledged notes prepared for this synthetic question.");
  questions();
  const question = "The original question before slow preparation";
  edit(root.querySelector(".conversation-input"), question);
  await until(() => !root.querySelector(".live-question-submit").disabled, "The preflight question was unavailable.");
  const previous = new Set(fixture.conversations.requests().map(request => request.request_id)), begins = fixture.calls.filter(command => command === "begin_conversation_question").length;
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.pendingNoteSaves() > 0, "The note preflight did not enter the controlled save gate.");
  return { ...state, question, previous, begins };
}

export async function checkConversationDraftDuringPreparation() {
  const { root, fixture, api, scope, question, previous } = await preparationProbe();
  const newer = "A newer unsent composer draft while notes are saving";
  edit(root.querySelector(".conversation-input"), newer);
  fixture.workspace.notesDelay(0); fixture.workspace.releaseNotes();
  await until(() => fixture.conversations.requests().some(request => !previous.has(request.request_id)), "The original prepared question was not admitted.");
  const request = fixture.conversations.requests().find(request => !previous.has(request.request_id));
  await until(async () => (await snapshot(api, scope)).conversation.draft === newer, "Question preparation cleared the newer composer draft.");
  const state = await snapshot(api, scope);
  assert(state.messages.find(message => message.request_id === request.request_id && message.role === "user")?.text === question,
    "Slow preparation submitted the newer draft instead of the captured question.");
  await complete(root, fixture, request);
  assert(root.querySelector(".conversation-input").value === newer, "Completing the original answer cleared the newer composer text.");
  return accepted({ originalQuestionCapturedBeforePreparation: true, newerDraftSurvivesSlowNotes: true, laterAnswerPreservesComposer: true });
}

export async function checkConversationCancelDuringPreparation() {
  const { root, fixture, question, previous, begins } = await preparationProbe();
  assert(!root.querySelector(".live-question-cancel").hidden, "Preparing an answer had no cancellation action.");
  root.querySelector(".live-question-cancel").click();
  fixture.workspace.notesDelay(0); fixture.workspace.releaseNotes();
  await until(() => root.querySelector(".live-question-cancel").hidden && !root.querySelector(".live-question-submit").disabled,
    "Cancelling before admission did not release the original composer.");
  assert(fixture.calls.filter(command => command === "begin_conversation_question").length === begins
    && fixture.conversations.requests().every(request => previous.has(request.request_id)), "A cancelled preflight dispatched inference.");
  assert(root.querySelector(".conversation-input").value === question, "Preflight cancellation lost the submitted question draft.");
  assert(fixture.workspace.notes() === "Acknowledged notes prepared for this synthetic question.", "Cancelling a question discarded the independent note save.");
  return accepted({ preparationCancellationVisible: true, noProviderDispatchAfterCancel: true, questionDraftRetained: true, ordinaryNoteSaveIndependent: true });
}

export async function checkConversationCancelBeforeAdmissionReply() {
  const { root, fixture, api, scope } = await owned();
  fixture.conversations.beginDelay(30000);
  const previous = new Set(fixture.conversations.requests().map(request => request.request_id));
  edit(root.querySelector(".conversation-input"), "A question whose admission reply is delayed");
  await until(() => !root.querySelector(".live-question-submit").disabled, "The delayed-admission question was not available.");
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.conversations.pendingBeginResponses() === 1, "The admitted question response was not held.");
  const request = fixture.conversations.requests().find(request => !previous.has(request.request_id));
  const newer = "A newer draft retained while cancelling the old question";
  edit(root.querySelector(".conversation-input"), newer);
  fixture.conversations.cancelDelay(30000);
  root.querySelector(".live-question-cancel").click();
  await until(async () => (await snapshot(api, scope)).messages.some(message => message.request_id === request.request_id && message.role === "assistant" && message.status === "cancelled"),
    "Cancel waited behind the unresolved admission response.");
  assert(fixture.conversations.pendingBeginResponses() === 1, "The cancellation check did not run before admission acknowledgement.");
  await until(() => fixture.conversations.pendingCancelResponses() === 1, "The cancel response was not held independently.");
  fixture.conversations.beginDelay(0); fixture.conversations.releaseBegins();
  await until(() => fixture.conversations.pendingBeginResponses() === 0, "The begin response did not release before cancellation acknowledgement.");
  await until(async () => (await snapshot(api, scope)).conversation.draft === newer, "The newer draft did not save after begin released while cancellation remained pending.");
  assert(root.querySelector(".live-question-submit").disabled && !root.querySelector(".live-question-cancel").hidden,
    "The original conversation became idle before cancellation was acknowledged.");
  fixture.conversations.cancelDelay(0); fixture.conversations.releaseCancels();
  await until(() => root.querySelector(`[data-request-id="${request.request_id}"][data-message-status="cancelled"]`) && root.querySelector(".live-question-cancel").hidden,
    "A late admission response resurrected the cancelled request.");
  await until(async () => (await snapshot(api, scope)).conversation.draft === newer, "Cancelling an old admission cleared the newer draft.");
  assert(root.querySelector(".conversation-input").value === newer, "A stale begin acknowledgement replaced the visible composer.");
  assert(await fixture.conversations.complete(request.request_id, { text: "A late answer after independent cancellation", citations: [] }) === false,
    "Independent cancellation left its provider result admissible.");
  return accepted({ cancellationBypassesUnresolvedBeginReply: true, busyUntilIndependentCancelAcknowledged: true, staleAdmissionAcknowledgementDiscarded: true, newerComposerDraftPreserved: true, lateCompletionRejected: true });
}

export async function checkConversationUnicodeNoteSource() {
  const { root, fixture } = await owned();
  document.querySelector('[data-live-tab="notes"]').click();
  const editor = document.querySelector(".live-notes");
  edit(editor, "Préface 😀\nRepeated source.\nRepeated source.\n"); questions();
  const request = await ask(root, fixture, "Which second repeated note supports this answer?");
  const repeated = request.anchors.flatMap((anchor, index) => anchor.source_text === "Repeated source." ? [{ anchor, number: index + 1 }] : []);
  assert(repeated.length === 2 && repeated[1].anchor.notes_start === 28, "The durable note anchor did not retain the core's exact UTF-16 range.");
  const row = await complete(root, fixture, request, { text: `The second repeated note [${repeated[1].number}]`,
    citations: [{ number: repeated[1].number, passage: repeated[1].anchor.passage }] });
  row.querySelector(".citation").click();
  await until(() => document.activeElement === editor && editor.selectionStart === 28 && editor.selectionEnd === 44,
    "A Unicode note citation selected another occurrence or mixed UTF-8 bytes with editor offsets.");
  assert(editor.value.slice(editor.selectionStart, editor.selectionEnd) === "Repeated source.", "The selected note range did not match the original exact source.");
  document.querySelector(".live-source-back").click();
  await until(() => root.offsetParent !== null && row.isConnected, "Returning from a Unicode source lost the originating answer.");
  return accepted({ unicodeEditorOffsetsMatchCore: true, repeatedNoteIdentityPreserved: true, exactSecondOccurrenceSelected: true, answerReturnPreserved: true });
}

export async function checkConversationTemporaryReason() {
  const { root, fixture, api, scope, localizeBackendText } = await owned();
  const initial = await snapshot(api, scope), reason = fixture.options.conversation_storage_reason;
  assert(initial.storage_status === "unsaved" && initial.storage_error === reason, "The unsaved conversation lost its specific recovery reason.");
  assert(root.querySelector(".conversation-storage-status").textContent === localizeBackendText(reason), "The temporary-storage reason was not shown in the selected interface language.");
  const question = "An ordinary question while conversation storage is unavailable", request = await ask(root, fixture, question);
  await complete(root, fixture, request);
  assert((await snapshot(api, scope)).storage_status === "unsaved", "A temporary answer falsely claimed durable storage.");
  await menuAction(root, "copy");
  await until(() => fixture.copiedTexts().at(-1)?.includes(question), "The temporary answer could not be copied for recovery.");
  return accepted({ specificTemporaryReasonVisible: true, selectedInterfaceLanguageUsedForReason: true, ordinaryAskRemainsUsable: true, temporaryAnswerCopyAvailable: true });
}
