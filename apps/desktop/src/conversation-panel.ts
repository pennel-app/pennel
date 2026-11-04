import { api, type AppInfo } from "./api";
import { askText, confirmDialog, controlButton, errorText, formatClock, h, markdown, showPopupMenu } from "./dom";
import { bindLocalization, l, localizeBackendText, t } from "./i18n";
import { languageModelConfigured } from "./presentation";
import { VisibilityPoll } from "./window-lifecycle";
import { liveQuestionMetadata } from "./live-question-scope";
import { meetingDate } from "./meeting-labels";
import { ConversationController, type ConversationTransport } from "../../../assets/contracts/meeting-conversation-controller.ts";
import { conversationScopeKey, type ConversationMessage, type ConversationResolvedSource, type ConversationScope } from "../../../assets/contracts/meeting-conversations.ts";
import type { Context } from "./main";
import "./conversations.css";

interface ConversationHost {
  scope: ConversationScope;
  live?: boolean;
  prepare?(): Promise<{ expected_notes_revision?: string; session_id?: string }>;
  canAsk?(): boolean;
  source?(source: ConversationResolvedSource, origin: HTMLElement, message: ConversationMessage, restore: () => void): Promise<boolean> | boolean;
}
const controllers = new Map<string, WeakRef<ConversationController>>();
const recoveryControllers = new Map<string, ConversationController>();
const requestedFocus = new WeakMap<ConversationController, string>();
const contextKey = (info: AppInfo) => `${info.data_dir}\u0000${info.settings.meetings_dir}`;

function controllerFor(ctx: Context, scope: ConversationScope): ConversationController {
  const context = contextKey(ctx.info), key = `${context}\u0000${conversationScopeKey(scope)}`;
  for (const [key, reference] of controllers) if (!reference.deref()) controllers.delete(key);
  const existing = recoveryControllers.get(key) ?? controllers.get(key)?.deref();
  if (existing) return existing;
  const guarded = <T, A>(work: (request: A) => Promise<T>) => async (request: A): Promise<T> => {
    if (contextKey(ctx.info) !== context) throw new Error("conversation_context_changed");
    const result = await work(request);
    if (contextKey(ctx.info) !== context) throw new Error("conversation_context_changed");
    return result;
  };
  const transport: ConversationTransport = { snapshot: guarded(api.conversationSnapshot), update: guarded(api.updateConversation),
    begin: guarded(api.beginConversationQuestion), cancel: guarded(api.cancelConversationQuestion), source: guarded(api.conversationSource) };
  const model = new ConversationController(scope, transport);
  controllers.set(key, new WeakRef(model));
  model.subscribe(() => {
    const canonical = `${context}\u0000${conversationScopeKey(model.scope)}`;
    controllers.set(canonical, new WeakRef(model));
    // Only unacknowledged IPC text needs frontend retention; history belongs to the core.
    if (model.dirty) { recoveryControllers.set(key, model); recoveryControllers.set(canonical, model); }
    else {
      if (recoveryControllers.get(key) === model) recoveryControllers.delete(key);
      if (recoveryControllers.get(canonical) === model) recoveryControllers.delete(canonical);
    }
  });
  return model;
}

export async function flushConversations(): Promise<void> {
  const retained = [...controllers.values()].flatMap(reference => reference.deref() ?? []);
  await Promise.all([...new Set([...retained, ...recoveryControllers.values()])].map(model => model.flush().catch(() => undefined)));
}

export async function restoreConversationPanel(ctx: Context, scope: ConversationScope, conversationId: string, messageId: string): Promise<string | null> {
  const model = controllerFor(ctx, scope);
  await model.select(conversationId); await model.focus(messageId);
  if (model.snapshot?.conversation.id !== conversationId || !model.snapshot.messages.some(message => message.id === messageId)) throw new Error(t("conversations.source_unavailable"));
  requestedFocus.set(model, messageId);
  return model.snapshot.current_meeting_id;
}

export function conversationPanel(initialContext: Context, host: ConversationHost) {
  let ctx = initialContext, binding = 0, sourceNavigation = 0, composing = false, preparing = false;
  let poll: VisibilityPoll | null = null, unsubscribe: (() => void) | null = null;
  let currentConversation: string | null = null;
  const model = controllerFor(ctx, host.scope);
  let focusMessage: string | null = requestedFocus.get(model) ?? null;
  let followLatest = focusMessage === null, intendedScroll: number | null = null;
  const root = h("section", { class: `chat conversation-panel${host.live ? " live-conversation-panel" : ""}` });
  const picker = controlButton({ class: "btn conversation-select", "aria-label": l("conversations.select"), "aria-haspopup": "menu", onclick: () => {
    const items = model.snapshot?.conversations.map(conversation => ({ value: conversation.id, label: conversation.title || l("conversations.untitled"),
      run: () => { void model.select(conversation.id); } })) ?? [];
    showPopupMenu(picker, items, { current: model.conversationId ?? "", role: "listbox" });
  } }, l("conversations.loading"));
  const create = controlButton({ class: "btn compact conversation-new", title: l("conversations.new"), "aria-label": l("conversations.new"), onclick: () => { void model.create(); } }, l("conversations.new_short"));
  const renameConversation = async () => {
    const name = await askText(l("conversations.rename"), model.snapshot?.conversation.title ?? "", l("conversations.rename_help"));
    if (name !== null) await model.rename(name);
  };
  const deleteConversation = async () => {
    if (await confirmDialog(l("conversations.delete_title"), l("conversations.delete_help"))) await model.delete();
  };
  const actions = controlButton({ class: "btn compact conversation-actions", "aria-label": l("conversations.actions"), "aria-haspopup": "menu", onclick: () => {
    showPopupMenu(actions, [
      { value: "rename", label: l("conversations.rename"), disabled: !model.snapshot, run: renameConversation },
      { value: "delete", label: l("conversations.delete"), disabled: !model.snapshot, run: deleteConversation },
      { value: "more", label: l("conversations.more"), disabled: !model.snapshot?.next_conversations_cursor, run: () => model.loadMore() },
      { value: "copy", label: l("conversations.copy"), disabled: !model.snapshot, run: copyConversation },
    ]);
  } }, "…");
  const toolbar = h("div", { class: "conversation-toolbar row" }, picker, create, actions);
  const storageText = h("span", { class: "conversation-storage-status", role: "status", "aria-live": "polite" });
  const retry = controlButton({ class: "text-button conversation-retry", hidden: true, onclick: () => { void model.retrySave(); } }, l("conversations.retry"));
  const copied = h("span", { class: "help", role: "status" });
  async function copyConversation() {
    const text = [...model.snapshot?.messages ?? []].map(message => message.status === "completed" ? message.text : `${message.text}\n[${t(`conversations.status_${message.status}` as "conversations.status_pending")}]`).filter(Boolean).join("\n\n");
    try { await api.copyText([text, model.draft].filter(Boolean).join("\n\n")); copied.textContent = t("conversations.copy_done"); }
    catch (error) { ctx.report(error); }
  }
  const copy = controlButton({ class: "text-button conversation-copy", onclick: () => { void copyConversation(); } }, l("conversations.copy"));
  const error = h("p", { class: "error conversation-error", hidden: true, role: "status" });
  const sourceNotice = h("div", { class: "conversation-source-notice", hidden: true, role: "status" });
  const history = h("div", { class: `chat-history${host.live ? " live-question-history" : ""}`, role: "log", "aria-live": "polite", tabindex: 0 });
  const older = controlButton({ class: "text-button conversation-older", hidden: true, onclick: () => { followLatest = false; void model.loadOlder(); } }, l("conversations.older"));
  const input = h("textarea", { class: `conversation-input${host.live ? " live-question-input" : ""}`, rows: 2,
    placeholder: l(host.scope.kind === "library" ? "meetings.ask_placeholder_all" : "meetings.ask_placeholder_this"), "aria-label": l("meetings.your_question") });
  const submit = controlButton({ class: `btn primary${host.live ? " live-question-submit" : " conversation-submit"}`, onclick: () => { void ask(); } }, l("meetings.ask"));
  const cancel = controlButton({ class: `btn${host.live ? " live-question-cancel" : " conversation-cancel"}`, hidden: true, onclick: () => { void model.cancel(); } }, l("common.cancel"));
  const setup = h("div", { class: "meeting-setup chat-setup help", role: "status" }, h("span", {}, l("meetings.questions_setup_help")),
    h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_questions")));
  const composer = h("div", { class: host.live ? "live-question-composer" : "chat-composer conversation-composer" }, input, h("div", { class: "row" }, submit, cancel));
  const storage = h("div", { class: "conversation-storage row" }, storageText, retry, copy, copied);
  history.append(sourceNotice);
  root.append(toolbar, storage, error, older, history, setup, composer);
  const rows = new Map<string, { element: HTMLElement; signature: string }>();
  const positions = new Map<string, number>();
  history.addEventListener("scroll", () => {
    if (currentConversation) positions.set(currentConversation, history.scrollTop);
    if (intendedScroll !== null && Math.abs(history.scrollTop - intendedScroll) < 2) intendedScroll = null;
    else followLatest = history.scrollHeight - history.scrollTop - history.clientHeight < 40;
  });
  input.addEventListener("input", () => { model.editDraft(input.value); readiness(); });
  input.addEventListener("compositionstart", () => { composing = true; readiness(); });
  input.addEventListener("compositionend", () => { composing = false; model.editDraft(input.value); readiness(); });
  input.addEventListener("keydown", event => { if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void ask(); } });
  input.addEventListener("blur", () => { void flush(); });

  function readiness() {
    const ready = languageModelConfigured(ctx.info), state = model.getSnapshot();
    input.disabled = state.loading || !state.snapshot;
    submit.disabled = state.loading || !ready || state.busy || preparing || composing || host.canAsk?.() === false || !input.value.trim() || !state.conversationId;
    cancel.hidden = !state.busy;
    setup.hidden = ready;
  }
  async function ask() {
    const question = input.value.trim();
    if (!question || submit.disabled) return;
    followLatest = true; focusMessage = null; requestedFocus.delete(model);
    history.scrollTop = history.scrollHeight; intendedScroll = history.scrollTop;
    preparing = true; readiness();
    try {
      await model.ask(question, crypto.randomUUID(), async () => await host.prepare?.() ?? {});
    } catch (failure) { error.textContent = errorText(failure); error.hidden = false; }
    finally { preparing = false; readiness(); }
  }
  function citationLabel(message: ConversationMessage, number: number): string {
    const citation = message.answer?.citations.find(citation => citation.number === number);
    if (!citation) return `[${number}]`;
    const source = message.evidence?.sources.find(source => source.number === number);
    const passage = citation.passage;
    const place = passage.kind === "transcript" && passage.start_ms !== null ? formatClock(passage.start_ms / 1000) : t(`meetings.source_${passage.kind}`);
    const context = host.scope.kind === "library" ? `${passage.title} · ${meetingDate(passage.start_iso)} · ` : "";
    const suffix = source && source.status !== "current" ? ` · ${t(source.status === "stale" ? "conversations.source_stale" : "conversations.source_unavailable")}` : "";
    return `[${number}] ${context}${place}${suffix}`;
  }
  function drawMessage(message: ConversationMessage, element: HTMLElement) {
    element.replaceChildren();
    if (message.role === "assistant") element.dataset.requestId = message.request_id;
    element.dataset.messageId = message.id;
    element.dataset.messageStatus = message.status;
    if (message.role === "user") { element.append(h("p", { class: "chat-question" }, message.text)); return; }
    const metadata = h("p", { class: "live-answer-cutoff help" });
    const evidence = message.evidence;
    if (evidence) metadata.textContent = evidence.meeting_uid ? liveQuestionMetadata({ ...evidence, meeting_uid: evidence.meeting_uid, session_id: evidence.session_id ?? "" }, formatClock(evidence.included_through_ms / 1000)) : t("conversations.empty_help");
    const response = h("div", { class: "live-answer", role: "status" });
    if (message.status === "pending") response.append(h("progress", { "aria-label": l("meetings.writing_answer") }));
    else if (message.status === "completed") response.append(markdown(message.answer?.text ?? message.text));
    else {
      response.append(h("div", {}, h("p", { class: "help conversation-incomplete" }, l(`conversations.status_${message.status}`)),
        message.text ? h("p", {}, message.text) : null,
        message.error ? h("p", { class: "error" }, localizeBackendText(message.error)) : null));
      const question = model.snapshot?.messages.find(candidate => candidate.id === message.reply_to)?.text;
      if (question) response.append(controlButton({ class: "text-button", onclick: () => { model.editDraft(question); input.focus(); } }, l("common.retry")));
    }
    if (message.status === "completed" && message.answer) {
      const citations = h("div", { class: "citations" });
      for (const citation of message.answer.citations) {
        citations.append(h("button", { class: "text-button citation", "data-citation-number": citation.number, onclick: () => { void openSource(message, citation.number, element); } }, citationLabel(message, citation.number)));
      }
      response.append(citations);
    }
    element.append(metadata, response);
  }
  async function openSource(message: ConversationMessage, number: number, origin: HTMLElement) {
    const navigation = ++sourceNavigation, ownerBinding = binding;
    try {
      await flush();
      const source = await model.source(message.id, number, message.conversation_id);
      if (navigation !== sourceNavigation || ownerBinding !== binding || model.conversationId !== message.conversation_id || !root.isConnected) return;
      const restore = () => { void model.select(message.conversation_id).then(() => focus(message.id)); };
      if (await host.source?.(source, origin, message, restore)) return;
      sourceNotice.replaceChildren(h("p", { class: "help" }, l(source.status === "unavailable" ? "conversations.source_unavailable" : "conversations.source_stale")),
        h("blockquote", {}, source.anchor?.source_text ?? source.passage.text)); history.append(sourceNotice); sourceNotice.hidden = false;
      sourceNotice.scrollIntoView({ block: "nearest" });
    } catch (failure) { error.textContent = errorText(failure); error.hidden = false; }
  }
  function render() {
    const state = model.getSnapshot(), snapshot = state.snapshot;
    const switched = state.conversationId !== currentConversation;
    if (switched) { ++sourceNavigation; sourceNotice.hidden = true; }
    if (switched && currentConversation) positions.set(currentConversation, history.scrollTop);
    currentConversation = state.conversationId;
    if (!composing && input.value !== state.draft) {
      const start = input.selectionStart, end = input.selectionEnd, direction = input.selectionDirection;
      input.value = state.draft; if (!switched) input.setSelectionRange(start, end, direction);
    }
    picker.textContent = snapshot?.conversation.id === state.conversationId ? snapshot.conversation.title || t("conversations.untitled") : t("conversations.loading");
    picker.disabled = state.loading || !snapshot; create.disabled = state.loading;
    actions.disabled = state.loading || !snapshot;
    older.hidden = !snapshot?.next_messages_cursor; older.disabled = state.loading;
    const unsaved = snapshot?.storage_status === "unsaved" || state.dirty && !!state.error;
    storageText.textContent = snapshot?.storage_status === "unsaved" && snapshot.storage_error ? localizeBackendText(snapshot.storage_error)
      : t(!snapshot ? "conversations.loading" : unsaved ? "conversations.unsaved" : state.dirty ? "conversations.saving" : "conversations.saved");
    retry.hidden = !unsaved && !state.error;
    storage.hidden = !!snapshot && !state.dirty && !unsaved && !state.error;
    error.textContent = state.error === "conversation_context_changed" ? t("conversations.context_changed") : localizeBackendText(state.error ?? ""); error.hidden = !state.error;
    const oldHeight = history.scrollHeight, oldScroll = history.scrollTop;
    const messages = snapshot?.conversation.id === state.conversationId ? snapshot.messages : [];
    const keep = new Set(messages.map(message => message.id));
    for (const [id, row] of rows) if (!keep.has(id)) { row.element.remove(); rows.delete(id); }
    history.querySelector(".chat-placeholder")?.remove();
    if (!messages.length) history.append(h("div", { class: "chat-placeholder" }, h("h2", {}, l(state.loading ? "conversations.loading" : host.scope.kind === "library" ? "meetings.ask_all" : "conversations.empty")), h("p", { class: "help" }, l("conversations.empty_help"))));
    let before = history.firstElementChild;
    for (const message of messages) {
      let row = rows.get(message.id);
      if (!row) { row = { element: h("article", { class: `chat-exchange conversation-${message.role}` }), signature: "" }; rows.set(message.id, row); }
      const signature = JSON.stringify({ ...message, evidence: message.evidence ? { ...message.evidence, sources: [] } : null });
      if (row.signature !== signature) { drawMessage(message, row.element); row.signature = signature; }
      for (const citation of row.element.querySelectorAll<HTMLElement>("[data-citation-number]")) {
        const label = citationLabel(message, Number(citation.dataset.citationNumber));
        if (citation.textContent !== label) citation.textContent = label;
      }
      if (before !== row.element) history.insertBefore(row.element, before);
      before = row.element.nextElementSibling;
    }
    if (switched) {
      if (!focusMessage) followLatest = !currentConversation || !positions.has(currentConversation);
      history.scrollTop = currentConversation ? positions.get(currentConversation) ?? history.scrollHeight : 0;
    }
    else if (followLatest && oldHeight - oldScroll - history.clientHeight < 40) history.scrollTop = history.scrollHeight;
    else history.scrollTop = oldScroll;
    intendedScroll = history.scrollTop;
    if (focusMessage && root.isConnected) { const row = rows.get(focusMessage); if (row) { row.element.tabIndex = -1; row.element.focus({ preventScroll: true }); row.element.scrollIntoView({ block: "center" }); intendedScroll = history.scrollTop; followLatest = false; requestedFocus.delete(model); focusMessage = null; } }
    readiness();
  }
  async function focus(messageId: string) { focusMessage = messageId; followLatest = false; await model.focus(messageId); render(); }
  async function flush() { await model.flush().catch(() => undefined); }
  function bind(next: Context) {
    ctx = next; const mine = ++binding;
    unsubscribe?.(); unsubscribe = model.subscribe(render);
    poll?.dispose(); poll = new VisibilityPoll(750, async () => { if (root.isConnected && root.offsetParent !== null) await model.refresh(); }, ctx.report);
    poll.setVisible(ctx.visible);
    next.onVisibilityChange(visible => { if (mine === binding) { if (!visible) void flush(); poll?.setVisible(visible); } });
    next.onInfoChange(() => { if (mine === binding) readiness(); });
    next.onFocusChange(focused => { if (focused && mine === binding) void model.refresh(); });
    next.onFlush(flush); next.beforeLeave(flush);
    next.onDispose(() => { if (mine === binding) { ++sourceNavigation; void flush(); unsubscribe?.(); unsubscribe = null; poll?.dispose(); poll = null; } });
    if (!model.snapshot) void model.load(); else void model.refresh();
    render();
    requestAnimationFrame(() => { if (mine === binding) render(); });
  }
  bindLocalization(root, () => { for (const row of rows.values()) row.signature = ""; render(); });
  bind(ctx);
  return { root, model, history, input, bind, flush, focus, readiness };
}
