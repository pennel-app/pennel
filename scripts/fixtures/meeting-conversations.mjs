import { assertConversationRequest } from "../../assets/contracts/meeting-conversations.ts";

export function createConversationFixture(environment) {
  const clone = value => structuredClone(value);
  let beginDelay = 0, cancelDelay = 0;
  const pendingBegins = new Set(), pendingCancels = new Set();
  let state;
  try { state = JSON.parse(environment.storageKey ? environment.storage?.getItem(environment.storageKey) || "null" : "null"); } catch {}
  state ??= { schema_version: 1, sequence: 0, clock: 0, scopes: {}, requests: {} };
  const supported = state.schema_version === 1;
  if (!supported) state = { schema_version: 1, sequence: 0, clock: 0, scopes: {}, requests: {} };
  const timestamp = () => environment.now + ++state.clock;
  const id = () => `00000000-0000-7000-8000-${String(++state.sequence + 2000).padStart(12, "0")}`;
  const invalid = message => { throw Error(message); };
  const storageUnavailable = () => !supported || environment.storageUnavailable();
  for (const scope of Object.values(state.scopes)) for (const conversation of scope.conversations) {
    for (const message of conversation.messages) if (message.status === "pending") {
      message.status = "interrupted";
      message.error = "The answer was interrupted before Pennel reopened.";
      if (state.requests[message.request_id]) state.requests[message.request_id].status = "interrupted";
    }
    conversation.summary.active_request_id = null;
  }
  function create(store, title = "") {
    const time = timestamp(), summary = { id: id(), title, draft: "", draft_revision: 0,
      created_at_ms: time, updated_at_ms: time, active_request_id: null };
    const conversation = { summary, messages: [] };
    store.conversations.push(conversation); store.selected = summary.id;
    return conversation;
  }
  function persist(store) {
    store.unsaved = true;
    if (storageUnavailable()) return;
    try {
      store.unsaved = false;
      if (environment.storageKey) environment.storage?.setItem(environment.storageKey, JSON.stringify(state));
    } catch { store.unsaved = true; }
  }
  async function context(scope) {
    const resolved = await environment.resolveScope(scope);
    const key = JSON.stringify([environment.libraryContext(), resolved.scope]);
    let store = state.scopes[key];
    if (!store) {
      store = { scope: resolved.scope, selected: null, conversations: [], unsaved: false };
      state.scopes[key] = store; create(store); persist(store);
    }
    return { ...resolved, key, store };
  }
  function conversation(store, conversationId = store.selected) {
    return store.conversations.find(item => item.summary.id === conversationId) || invalid("This conversation is no longer available.");
  }
  function page(values, cursor, key) {
    const end = cursor ? values.findIndex(value => key(value) === cursor) : -1;
    if (cursor && end < 0) invalid("The conversation page is no longer available.");
    const selected = cursor ? values.slice(end + 1) : values;
    return { values: selected.slice(0, 50), next: selected.length > 50 ? key(selected[49]) : null };
  }
  async function read(request, provided) {
    const current = provided || await context(request.scope), { store } = current;
    const target = conversation(store, request.conversation_id || store.selected);
    const summaries = page([...store.conversations].sort((a, b) => b.summary.updated_at_ms - a.summary.updated_at_ms), request.conversations_before, item => item.summary.id);
    const newest = [...target.messages].reverse();
    let messages = page(newest, request.messages_before, item => item.id);
    if (request.focus_message_id) {
      const focus = newest.findIndex(message => message.id === request.focus_message_id);
      if (focus < 0) invalid("The referenced conversation message is no longer available.");
      const start = Math.max(0, focus - 24), values = newest.slice(start, start + 50);
      messages = { values, next: newest.length > start + 50 ? values.at(-1).id : null };
    }
    const views = clone(messages.values.reverse());
    for (const message of views) if (message.evidence) for (const source of message.evidence.sources) {
      source.status = (await environment.resolveSource(source)).status;
    }
    return { schema_version: 1, scope: clone(current.scope), storage_status: store.unsaved || storageUnavailable() ? "unsaved" : "saved",
      storage_error: store.unsaved || storageUnavailable() ? environment.storageReason() : null,
      selected_conversation_id: store.selected, current_meeting_id: current.current_meeting_id,
      conversations: clone(summaries.values.map(item => item.summary)), conversation: clone(target.summary),
      messages: views, next_messages_cursor: messages.next, next_conversations_cursor: summaries.next };
  }
  async function update(request) {
    const current = await context(request.scope), { store } = current;
    if (request.action === "create") create(store, request.title);
    else if (request.action === "select") store.selected = conversation(store, request.conversation_id).summary.id;
    else if (request.action === "delete") {
      conversation(store, request.conversation_id);
      store.conversations = store.conversations.filter(item => item.summary.id !== request.conversation_id);
      for (const entry of Object.values(state.requests)) if (entry.conversation_id === request.conversation_id) entry.status = "deleted";
      if (store.selected === request.conversation_id) store.selected = store.conversations[0]?.summary.id || create(store).summary.id;
    } else if (request.action === "rename") {
      const item = conversation(store, request.conversation_id);
      item.summary.title = request.title; item.summary.updated_at_ms = timestamp();
    } else if (request.action === "draft") {
      const item = conversation(store, request.conversation_id);
      if (item.summary.draft_revision !== request.expected_draft_revision) invalid("This conversation draft changed. Reload before saving.");
      item.summary.draft = request.draft; item.summary.draft_revision++; item.summary.updated_at_ms = timestamp();
    } else if (request.action === "retry_save" && request.conversation_id) conversation(store, request.conversation_id);
    persist(store);
    return read({ scope: request.scope }, current);
  }
  async function begin(request) {
    const current = await context(request.scope), item = conversation(current.store, request.conversation_id);
    const previous = state.requests[request.request_id];
    if (previous) {
      if (previous.status === "deleted" || previous.key !== current.key || previous.conversation_id !== item.summary.id || previous.question !== request.question.trim()) invalid("This question request was already used for another question or conversation.");
      return read({ scope: current.scope, conversation_id: item.summary.id }, current);
    }
    if (item.summary.active_request_id) invalid("A question is already running in this conversation.");
    const history = item.messages.filter(message => message.role === "assistant" && message.status === "completed").length;
    const evidence = await environment.capture({ ...request, scope: current.scope, history_turns: history });
    const user = { id: id(), conversation_id: item.summary.id, request_id: request.request_id, role: "user", text: request.question,
      status: "completed", created_at_ms: timestamp(), reply_to: null, answer: null, evidence: null, error: null };
    const assistant = { id: id(), conversation_id: item.summary.id, request_id: request.request_id, role: "assistant", text: "",
      status: "pending", created_at_ms: timestamp(), reply_to: user.id, answer: null, evidence: clone(evidence), error: null };
    item.messages.push(user, assistant); item.summary.active_request_id = request.request_id;
    item.summary.updated_at_ms = timestamp();
    if (!item.summary.title) item.summary.title = request.question.slice(0, 80);
    if (item.summary.draft.trim() === request.question.trim()) { item.summary.draft = ""; item.summary.draft_revision++; }
    state.requests[request.request_id] = { key: current.key, library: environment.libraryContext(), conversation_id: item.summary.id, question: request.question.trim(),
      assistant_id: assistant.id, evidence: clone(evidence), status: "pending" };
    persist(current.store);
    if (environment.autoComplete) setTimeout(() => { void complete(request.request_id); }, environment.answerDelay ?? 80);
    const response = await read({ scope: current.scope, conversation_id: item.summary.id }, current);
    if (beginDelay) await new Promise(resolve => {
      const finish = () => { clearTimeout(timer); pendingBegins.delete(finish); resolve(); };
      const timer = setTimeout(finish, beginDelay); pendingBegins.add(finish);
    });
    return response;
  }
  async function complete(requestId, result) {
    const entry = state.requests[requestId];
    if (!entry || entry.status !== "pending") return false;
    const store = state.scopes[entry.key], item = store?.conversations.find(item => item.summary.id === entry.conversation_id);
    const assistant = item?.messages.find(message => message.id === entry.assistant_id);
    if (!assistant || item.summary.active_request_id !== requestId) return false;
    if (entry.library !== environment.libraryContext() || !environment.requestIsCurrent(entry.evidence)) {
      assistant.status = "interrupted"; assistant.error = "The original question session is no longer available.";
      entry.status = "interrupted"; item.summary.active_request_id = null; persist(store); return false;
    }
    entry.status = result?.error ? "failed" : "completed";
    assistant.status = entry.status;
    assistant.error = result?.error || null;
    if (result?.error) assistant.text = result.partial_text || "";
    else {
      const first = entry.evidence.sources.find(source => source.passage.kind === "transcript") || entry.evidence.sources[0];
      assistant.answer = clone(result || { text: "Synthetic frozen answer [1]", citations: first ? [{ number: first.number, passage: first.passage }] : [] });
      assistant.text = assistant.answer.text;
    }
    item.summary.active_request_id = null; item.summary.updated_at_ms = timestamp(); persist(store);
    environment.completed?.(entry.evidence, assistant.answer, assistant.error);
    return true;
  }
  async function cancel(request) {
    const current = await context(request.scope), item = conversation(current.store, request.conversation_id), entry = state.requests[request.request_id];
    if (!entry || entry.key !== current.key || entry.conversation_id !== item.summary.id) invalid("This question request is no longer available.");
    if (entry.status === "pending") {
      const assistant = item.messages.find(message => message.id === entry.assistant_id);
      assistant.status = "cancelled"; assistant.error = null; entry.status = "cancelled";
      item.summary.active_request_id = null; persist(current.store);
    }
    const response = await read({ scope: current.scope, conversation_id: item.summary.id }, current);
    if (cancelDelay) await new Promise(resolve => {
      const finish = () => { clearTimeout(timer); pendingCancels.delete(finish); resolve(); };
      const timer = setTimeout(finish, cancelDelay); pendingCancels.add(finish);
    });
    return response;
  }
  async function source(request) {
    const current = await context(request.scope), item = conversation(current.store, request.conversation_id);
    const message = item.messages.find(message => message.id === request.message_id), saved = message?.evidence?.sources.find(source => source.number === request.citation_number);
    if (!saved) invalid("The cited conversation source is no longer available.");
    return environment.resolveSource(saved);
  }
  return {
    async invoke(operation, request) {
      assertConversationRequest(operation, request);
      if (operation === "snapshot") return read(request);
      if (operation === "update") return update(request);
      if (operation === "begin") return begin(request);
      if (operation === "cancel") return cancel(request);
      return source(request);
    },
    complete,
    beginDelay: value => { beginDelay = value; }, pendingBeginResponses: () => pendingBegins.size,
    releaseBegins: () => { for (const finish of [...pendingBegins]) finish(); },
    cancelDelay: value => { cancelDelay = value; }, pendingCancelResponses: () => pendingCancels.size,
    releaseCancels: () => { for (const finish of [...pendingCancels]) finish(); },
    requests: () => Object.values(state.requests).map(entry => ({ ...clone(entry.evidence), conversation_id: entry.conversation_id })),
    deleteMeeting(meetingUid) {
      for (const [key, store] of Object.entries(state.scopes)) if (store.scope.kind === "meeting" && store.scope.meeting_uid === meetingUid) {
        delete state.scopes[key];
        for (const entry of Object.values(state.requests)) if (entry.key === key) entry.status = "deleted";
      }
      persist(Object.values(state.scopes)[0] || { unsaved: false });
    },
  };
}
