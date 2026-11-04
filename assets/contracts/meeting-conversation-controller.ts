import { conversationRequests, type ConversationBeginRequest, type ConversationCancelRequest, type ConversationResolvedSource,
  type ConversationScope, type ConversationSnapshot, type ConversationSnapshotRequest, type ConversationSourceRequest,
  type ConversationSummary, type ConversationUpdateRequest } from "./meeting-conversations.ts";

export interface ConversationTransport {
  snapshot(request: ConversationSnapshotRequest): Promise<ConversationSnapshot>;
  update(request: ConversationUpdateRequest): Promise<ConversationSnapshot>;
  begin(request: ConversationBeginRequest): Promise<ConversationSnapshot>;
  cancel(request: ConversationCancelRequest): Promise<ConversationSnapshot>;
  source(request: ConversationSourceRequest): Promise<ConversationResolvedSource>;
}
interface LocalDraft { text: string; version: number }
type Page = "replace" | "refresh" | "messages" | "conversations";
export type ConversationPreparation = { expected_notes_revision?: string; session_id?: string };
export interface ConversationViewState {
  snapshot: ConversationSnapshot | null;
  loading: boolean;
  error: string | null;
  draft: string;
  busy: boolean;
  dirty: boolean;
  conversationId: string | null;
}

/** Coordinates views and draft writes; the Rust core owns history, selection and request lifetime. */
export class ConversationController {
  scope: ConversationScope;
  snapshot: ConversationSnapshot | null = null;
  loading = false;
  error: string | null = null;
  private readonly transport: ConversationTransport;
  private readonly listeners = new Set<() => void>();
  private readonly drafts = new Map<string, LocalDraft>();
  private readonly revisions = new Map<string, number>();
  private readonly acknowledgedDrafts = new Map<string, { revision: number; text: string }>();
  private readonly scheduled = new Set<string>();
  private readonly draftErrors = new Map<string, string>();
  private writes: Promise<void> = Promise.resolve();
  private initialLoad: Promise<void> | null = null;
  private epoch = 0;
  private stateVersion = 0;
  private selected: string | null = null;
  private olderMessages = false;
  private olderConversations = false;
  private readonly submitting = new Map<string, string>();
  private readonly dispatching = new Set<string>();
  private readonly cancelledAdmissions = new Set<string>();
  private readonly cancelling = new Map<string, string>();
  private readonly pendingQuestions = new Map<string, { text: string; draft: string; editClock: number }>();
  private draftClock = 0;
  private view: ConversationViewState = { snapshot: null, loading: false, error: null, draft: "", busy: false, dirty: false, conversationId: null };

  constructor(scope: ConversationScope, transport: ConversationTransport) { this.scope = scope; this.transport = transport; }
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  getSnapshot = (): ConversationViewState => this.view;
  private changed(): void {
    this.view = { snapshot: this.snapshot, loading: this.loading, error: this.error, draft: this.draft, busy: this.busy, dirty: this.dirty, conversationId: this.selected };
    for (const listener of this.listeners) listener();
  }
  get conversationId(): string | null { return this.selected; }
  get draft(): string {
    if (!this.selected) return "";
    return this.drafts.get(this.selected)?.text ?? (this.snapshot?.conversation.id === this.selected ? this.snapshot.conversation.draft
      : this.snapshot?.conversations.find(conversation => conversation.id === this.selected)?.draft) ?? "";
  }
  get dirty(): boolean { return this.drafts.size > 0; }
  get activeRequestId(): string | null { return this.snapshot?.conversation.id === this.selected ? this.snapshot.conversation.active_request_id : null; }
  get busy(): boolean { return !!this.activeRequestId || !!this.selected && (this.submitting.has(this.selected) || this.cancelling.has(this.selected)); }

  private remember(snapshot: ConversationSnapshot): void {
    for (const summary of [...snapshot.conversations, snapshot.conversation]) {
      const known = this.acknowledgedDrafts.get(summary.id);
      if (!known || summary.draft_revision >= known.revision) this.acknowledgedDrafts.set(summary.id, { revision: summary.draft_revision, text: summary.draft });
      this.revisions.set(summary.id, Math.max(this.revisions.get(summary.id) ?? 0, summary.draft_revision));
    }
  }
  private summaries(previous: ConversationSummary[], next: ConversationSummary[]): ConversationSummary[] {
    const merged = new Map(previous.map(summary => [summary.id, summary]));
    for (const summary of next) merged.set(summary.id, summary);
    return [...merged.values()].sort((a, b) => b.updated_at_ms - a.updated_at_ms || b.id.localeCompare(a.id));
  }
  private apply(snapshot: ConversationSnapshot, page: Page = "refresh"): void {
    this.remember(snapshot);
    const acknowledged = (summary: ConversationSummary): ConversationSummary => {
      const known = this.acknowledgedDrafts.get(summary.id);
      return known && known.revision > summary.draft_revision ? { ...summary, draft: known.text, draft_revision: known.revision } : summary;
    };
    snapshot = { ...snapshot, conversation: acknowledged(snapshot.conversation), conversations: snapshot.conversations.map(acknowledged) };
    if (this.selected && snapshot.conversation.id !== this.selected) {
      if (this.snapshot) this.snapshot = { ...this.snapshot, conversations: this.summaries(this.snapshot.conversations, snapshot.conversations), storage_status: snapshot.storage_status, storage_error: snapshot.storage_error };
      this.changed(); return;
    }
    const previous = this.snapshot;
    const same = previous?.conversation.id === snapshot.conversation.id;
    const mergeMessages = same && page !== "replace";
    const messages = new Map(mergeMessages ? previous.messages.map(message => [message.id, message]) : []);
    const incoming = new Set(snapshot.messages.map(message => message.id));
    const order = mergeMessages ? page === "messages"
      ? [...snapshot.messages.map(message => message.id), ...previous.messages.filter(message => !incoming.has(message.id)).map(message => message.id)]
      : [...previous.messages.filter(message => !incoming.has(message.id)).map(message => message.id), ...snapshot.messages.map(message => message.id)]
      : snapshot.messages.map(message => message.id);
    for (const message of snapshot.messages) {
      const previous = messages.get(message.id);
      if (previous && previous.status !== "pending" && message.status === "pending") continue;
      messages.set(message.id, message);
    }
    this.scope = snapshot.scope;
    this.selected = snapshot.conversation.id;
    this.snapshot = { ...snapshot,
      messages: [...new Set(order)].flatMap(id => messages.get(id) ?? []),
      conversations: page !== "replace" && previous ? this.summaries(previous.conversations, [...snapshot.conversations, snapshot.conversation]) : this.summaries(snapshot.conversations, [snapshot.conversation]),
      next_messages_cursor: same && page !== "replace" && this.olderMessages && page !== "messages" ? previous.next_messages_cursor : snapshot.next_messages_cursor,
      next_conversations_cursor: page !== "replace" && this.olderConversations && page !== "conversations" && previous ? previous.next_conversations_cursor : snapshot.next_conversations_cursor,
    };
    this.error = this.draftErrors.values().next().value ?? null; this.changed();
  }
  private command<T>(work: () => Promise<T>): Promise<T> {
    const operation = this.writes.catch(() => undefined).then(work);
    this.writes = operation.then(() => undefined);
    void this.writes.catch(() => undefined);
    return operation;
  }
  async load(): Promise<void> {
    if (this.initialLoad) return this.initialLoad;
    this.olderMessages = false; this.olderConversations = false;
    if (this.snapshot) { await this.read("replace"); return; }
    const initial = this.read("replace"); this.initialLoad = initial;
    try { await initial; } finally { if (this.initialLoad === initial) this.initialLoad = null; }
  }
  async refresh(): Promise<void> { if (!this.snapshot) { await this.load(); return; } if (!this.loading) await this.read("refresh"); }
  private async read(page: Page, cursors: { messages_before?: string; conversations_before?: string; focus_message_id?: string } = {}): Promise<void> {
    const epoch = ++this.epoch;
    const stateVersion = this.stateVersion;
    this.loading = !this.snapshot; this.changed();
    try {
      const snapshot = await this.transport.snapshot(conversationRequests.snapshot(this.scope, this.selected ?? undefined, cursors));
      if (epoch === this.epoch && stateVersion === this.stateVersion) this.apply(snapshot, page);
    } catch (error) { if (epoch === this.epoch && stateVersion === this.stateVersion) { this.error = error instanceof Error ? error.message : String(error); this.changed(); } }
    finally { if (epoch === this.epoch) { this.loading = false; this.changed(); } }
  }
  editDraft(text: string): void {
    const id = this.selected;
    if (!id || text === this.draft) return;
    const draft = this.drafts.get(id) ?? { text: this.draft, version: 0 };
    draft.text = text; draft.version = ++this.draftClock; this.drafts.set(id, draft); this.changed(); this.scheduleDraft(id);
  }
  private scheduleDraft(id: string): void {
    if (this.scheduled.has(id)) return;
    this.scheduled.add(id);
    void this.command(async () => {
      const draft = this.drafts.get(id);
      if (!draft) { this.scheduled.delete(id); return; }
      const version = draft.version, text = draft.text;
      try {
        const snapshot = await this.transport.update(conversationRequests.draft(this.scope, id, text, this.revisions.get(id) ?? 0));
        ++this.stateVersion;
        if (draft.version === version) this.drafts.delete(id);
        this.draftErrors.delete(id);
        this.apply(snapshot);
      } catch (error) { this.error = error instanceof Error ? error.message : String(error); this.draftErrors.set(id, this.error); this.changed(); throw error; }
      finally {
        this.scheduled.delete(id);
        if (this.drafts.get(id)?.version !== version && this.drafts.has(id)) this.scheduleDraft(id);
      }
    }).catch(() => undefined);
  }
  async flush(): Promise<void> {
    let pending: Promise<void>;
    do { pending = this.writes; await pending; } while (pending !== this.writes);
  }
  private async update(request: ConversationUpdateRequest, select = false): Promise<void> {
    const epoch = ++this.epoch;
    this.loading = select;
    if (request.action === "select") this.selected = request.conversation_id;
    this.changed();
    try {
      await this.flush().catch(() => undefined);
      await this.command(async () => {
        const snapshot = await this.transport.update({ ...request, scope: this.scope });
        ++this.stateVersion;
        this.remember(snapshot);
        if (epoch !== this.epoch) return;
        if (select) { this.selected = snapshot.conversation.id; this.olderMessages = false; this.olderConversations = false; }
        this.apply(snapshot, select ? "replace" : "refresh");
      });
    } catch (error) { if (epoch === this.epoch) { this.error = error instanceof Error ? error.message : String(error); this.changed(); } }
    finally { if (epoch === this.epoch) { this.loading = false; this.changed(); } }
  }
  async create(title?: string): Promise<void> { await this.update(conversationRequests.create(this.scope, title), true); }
  async select(id: string): Promise<void> { await this.update(conversationRequests.select(this.scope, id), true); }
  async rename(title: string): Promise<void> { if (this.selected) await this.update(conversationRequests.rename(this.scope, this.selected, title)); }
  async delete(): Promise<void> {
    const id = this.selected;
    if (!id) return;
    await this.update(conversationRequests.delete(this.scope, id), true);
    if (this.snapshot?.conversation.id !== id) { this.drafts.delete(id); this.revisions.delete(id); this.acknowledgedDrafts.delete(id); this.draftErrors.delete(id); this.error = this.draftErrors.values().next().value ?? null; this.changed(); }
  }
  async ask(question: string, requestId: string, evidence: ConversationPreparation | (() => Promise<ConversationPreparation>) = {}): Promise<boolean> {
    const id = this.selected;
    if (!id || !question.trim() || this.busy) return false;
    const submittedDraft = this.draft, editClock = this.draftClock;
    this.pendingQuestions.set(requestId, { text: question, draft: submittedDraft, editClock });
    this.submitting.set(id, requestId); this.changed();
    try {
      return await this.command(async () => {
        const prepared = typeof evidence === "function" ? await evidence() : evidence;
        if (this.cancelledAdmissions.has(requestId)) return false;
        this.dispatching.add(requestId);
        const snapshot = await this.transport.begin(conversationRequests.begin(this.scope, id, requestId, question, prepared));
        if (this.cancelledAdmissions.has(requestId)) { this.remember(snapshot); return false; }
        ++this.stateVersion;
        this.apply(snapshot);
        const local = this.drafts.get(id);
        if ((!local || local.version <= editClock) && snapshot.conversation.draft === submittedDraft && submittedDraft) {
          this.drafts.set(id, { text: "", version: ++this.draftClock }); this.scheduleDraft(id); this.changed();
        }
        return true;
      });
    } catch (error) { if (!this.cancelledAdmissions.has(requestId)) this.error = error instanceof Error ? error.message : String(error); this.changed(); return false; }
    finally { this.dispatching.delete(requestId); this.pendingQuestions.delete(requestId); this.cancelledAdmissions.delete(requestId); if (this.submitting.get(id) === requestId) this.submitting.delete(id); this.changed(); }
  }
  async cancel(): Promise<void> {
    const id = this.selected, request = this.activeRequestId ?? (id ? this.submitting.get(id) : undefined);
    if (!id || !request) return;
    if (this.cancelling.get(id) === request) return;
    if (this.submitting.get(id) === request && !this.dispatching.has(request) && !this.activeRequestId) {
      this.cancelledAdmissions.add(request); this.changed(); return;
    }
    const pending = this.pendingQuestions.get(request);
    const original = this.snapshot?.messages.find(message => message.request_id === request && message.role === "user")?.text ?? pending?.text;
    const existingDraft = this.draft, editClock = this.draftClock;
    this.cancelling.set(id, request); this.changed();
    if (this.submitting.get(id) === request) this.cancelledAdmissions.add(request);
    try {
      const snapshot = await this.transport.cancel(conversationRequests.cancel(this.scope, id, request));
      ++this.stateVersion; this.apply(snapshot);
      const local = this.drafts.get(id);
      const submittedUnedited = pending && existingDraft === pending.draft && (!local || local.version <= pending.editClock);
      if (original && (!existingDraft || submittedUnedited) && (!local || local.version <= editClock) && !this.acknowledgedDrafts.get(id)?.text) {
        this.drafts.set(id, { text: original, version: ++this.draftClock }); this.scheduleDraft(id); this.changed();
      }
      if (this.submitting.get(id) === request) this.submitting.delete(id);
      this.changed();
    }
    catch (error) { this.error = error instanceof Error ? error.message : String(error); this.changed(); }
    finally { if (this.cancelling.get(id) === request) this.cancelling.delete(id); this.changed(); }
  }
  async retrySave(): Promise<void> {
    await this.update(conversationRequests.retry(this.scope, this.selected ?? undefined));
    for (const id of this.drafts.keys()) this.scheduleDraft(id);
    await this.flush().catch(() => undefined);
  }
  async loadOlder(): Promise<void> {
    const cursor = this.snapshot?.next_messages_cursor;
    if (!cursor || this.loading) return;
    this.olderMessages = true;
    await this.read("messages", { messages_before: cursor });
  }
  async loadMore(): Promise<void> {
    const cursor = this.snapshot?.next_conversations_cursor;
    if (!cursor || this.loading) return;
    this.olderConversations = true;
    await this.read("conversations", { conversations_before: cursor });
  }
  async focus(messageId: string): Promise<void> { this.olderMessages = false; await this.read("replace", { focus_message_id: messageId }); this.olderMessages = true; }
  async source(messageId: string, citationNumber: number, conversationId = this.selected): Promise<ConversationResolvedSource> {
    if (!conversationId) throw new Error("No conversation is selected.");
    return this.transport.source(conversationRequests.source(this.scope, conversationId, messageId, citationNumber));
  }
}
