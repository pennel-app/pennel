import { InsightDraft, appendInsightMarkdown, insightReceipt, insightText, insightsWritable, insertionStillCurrent,
  type InsightAvailability, type InsightClaimRequest, type InsightInsertionPreparation, type InsightInsertionRequest,
  type InsightItem, type InsightJob, type InsightLanguagePolicy, type InsightNoteLatch, type InsightReassociationRequest,
  type InsightReviewRequest, type InsightReviewState, type InsightSnapshot, type InsightInsertionResolution } from "./meeting-insights.ts";

export interface InsightReviewService {
  read(id: string): Promise<InsightSnapshot>;
  availability(id: string, policy: InsightLanguagePolicy): Promise<InsightAvailability>;
  start(id: string, policy: InsightLanguagePolicy): Promise<InsightJob>;
  job(jobId: string): Promise<InsightJob>;
  cancel(jobId: string): Promise<InsightJob>;
  claim(id: string, request: InsightClaimRequest): Promise<InsightSnapshot>;
  review(id: string, request: InsightReviewRequest): Promise<InsightSnapshot>;
  prepare(id: string, request: InsightInsertionRequest): Promise<InsightInsertionPreparation>;
  reconcile(id: string, storeRevision: number): Promise<InsightSnapshot>;
  resolve(id: string, request: InsightInsertionResolution): Promise<InsightSnapshot>;
  reassociate(id: string, request: InsightReassociationRequest): Promise<InsightSnapshot>;
  export(id: string): Promise<string>;
  itemMarkdown(id: string, itemId: string, revision: number): Promise<string>;
}
export interface InsightNoteHost {
  flush(): Promise<boolean>;
  current(): InsightNoteLatch;
  stage(text: string, attemptId: string, before: InsightNoteLatch): Promise<boolean>;
}
export class InsightUiError extends Error {
  readonly code: string;
  constructor(code: string) { super(code); this.code = code; }
}
export interface InsightReviewViewState {
  version: number; snapshot: InsightSnapshot | null; availability: InsightAvailability | null;
  policy: InsightLanguagePolicy; job: InsightJob | null; busy: ReadonlySet<string>; error: unknown;
  edits: ReadonlyMap<string, InsightDraft>;
}

export class InsightReviewModel {
  id: string;
  readonly meetingUid: string;
  readonly service: InsightReviewService;
  private state: InsightReviewViewState = { version: 0, snapshot: null, availability: null, policy: "unknown", job: null, busy: new Set(), error: null, edits: new Map() };
  private listeners = new Set<() => void>();
  private readEpoch = 0;
  private readAgain = false;
  private availabilityEpoch = 0;
  private pendingWrite: Promise<unknown> = Promise.resolve();
  private itemBackups = new Map<string, InsightItem>();
  constructor(id: string, meetingUid: string, service: InsightReviewService) { this.id = id; this.meetingUid = meetingUid; this.service = service; }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private changed(patch: Partial<InsightReviewViewState> = {}) {
    this.state = { ...this.state, ...patch, version: this.state.version + 1 };
    for (const id of this.itemBackups.keys()) if (!this.state.edits.has(id)) this.itemBackups.delete(id);
    for (const listener of this.listeners) listener();
  }
  private async action<T>(key: string, work: () => Promise<T>): Promise<T | undefined> {
    if (this.state.busy.has(key)) return undefined;
    this.changed({ busy: new Set([...this.state.busy, key]), error: null });
    try { return await work(); }
    catch (error) { this.changed({ error }); return undefined; }
    finally { const busy = new Set(this.state.busy); busy.delete(key); this.changed({ busy }); }
  }
  private write<T>(work: () => Promise<T>): Promise<T | undefined> {
    const pending = this.action("write", work); this.pendingWrite = pending; return pending;
  }
  apply(snapshot: InsightSnapshot, ownedRead = false) {
    if (snapshot.meeting_uid !== this.meetingUid) throw new InsightUiError("wrong_meeting");
    if (this.state.snapshot && !insightsWritable(this.state.snapshot) && insightsWritable(snapshot) && !ownedRead) return;
    if (this.state.snapshot && snapshot.revision < this.state.snapshot.revision && insightsWritable(snapshot)) return;
    for (const item of snapshot.items) if (this.state.edits.has(item.id)) this.itemBackups.set(item.id, item);
    const incoming = snapshot.analysis_job, current = this.state.job;
    const job = incoming && incoming.meeting_uid === this.meetingUid && (!current || current.job_id !== incoming.job_id || incoming.revision >= current.revision) ? incoming : current;
    this.changed({ snapshot, job });
  }
  item(id: string) { return this.state.snapshot?.items.find(item => item.id === id); }
  get dirty() { return [...this.state.edits.values()].some(edit => edit.dirty); }
  get analysisBusy() { return this.state.job?.physical_running || this.state.job?.state === "queued" || this.state.job?.state === "running" || this.state.busy.has("analysis"); }
  get canAnalyze() { return this.state.availability?.available === true && this.state.policy !== "unknown" && insightsWritable(this.state.snapshot)
    && !this.analysisBusy && !this.state.edits.size && !this.state.busy.has("write"); }
  items() {
    const items = [...(this.state.snapshot?.items ?? [])];
    for (const id of this.state.edits.keys()) if (!items.some(item => item.id === id) && this.itemBackups.has(id)) items.push(this.itemBackups.get(id)!);
    return items;
  }
  async load() {
    if (this.state.busy.has("load")) { this.readAgain = true; return; }
    const epoch = ++this.readEpoch;
    await this.action("load", async () => { const snapshot = await this.service.read(this.id); if (epoch === this.readEpoch) this.apply(snapshot, true); });
    if (this.readAgain) { this.readAgain = false; await this.load(); }
  }
  async choosePolicy(policy: InsightLanguagePolicy) {
    this.changed({ policy }); const epoch = ++this.availabilityEpoch;
    await this.action(`availability:${epoch}`, async () => {
      const availability = await this.service.availability(this.id, policy);
      if (epoch === this.availabilityEpoch && policy === this.state.policy) this.changed({ availability });
    });
  }
  async analyze() {
    if (!this.canAnalyze) return false;
    return !!await this.action("analysis", async () => {
      const job = await this.service.start(this.id, this.state.policy);
      if (job.meeting_uid !== this.meetingUid) throw new InsightUiError("wrong_meeting");
      this.changed({ job }); return true;
    });
  }
  async poll() {
    const current = this.state.job;
    if (!current || !this.analysisBusy) return;
    await this.action("job", async () => {
      const job = await this.service.job(current.job_id);
      if (job.meeting_uid !== this.meetingUid || this.state.job?.job_id !== job.job_id || job.revision < this.state.job.revision) return;
      this.changed({ job });
      if (!job.physical_running && ["completed", "cancelled", "failed"].includes(job.state)) await this.load();
    });
  }
  async cancel() {
    const current = this.state.job; if (!current) return;
    await this.action("cancel", async () => { const job = await this.service.cancel(current.job_id);
      if (job.meeting_uid === this.meetingUid && this.state.job?.job_id === job.job_id && job.revision >= this.state.job.revision) this.changed({ job }); });
  }
  async review(id: string, reviewState: InsightReviewState) {
    if (!insightsWritable(this.state.snapshot) || this.state.edits.has(id)) return false;
    return !!await this.write(async () => {
      const item = this.item(id); if (!item || !this.state.snapshot) return false;
      this.apply(await this.service.review(this.id, { store_revision: this.state.snapshot.revision, item_id: id, item_revision: item.revision, update: { review_state: reviewState } })); return true;
    });
  }
  async beginEdit(id: string) {
    if (!insightsWritable(this.state.snapshot) || this.state.edits.has(id)) return;
    await this.write(async () => {
      const item = this.item(id); if (!item || !this.state.snapshot) return;
      const snapshot = await this.service.claim(this.id, { store_revision: this.state.snapshot.revision, item_id: id, item_revision: item.revision, claim_id: item.edit_claim });
      this.apply(snapshot); const claimed = snapshot.items.find(item => item.id === id);
      if (!claimed?.edit_claim) throw new InsightUiError("edit_conflict");
      this.itemBackups.set(id, claimed);
      const edits = new Map(this.state.edits); edits.set(id, new InsightDraft(id, claimed.edit_claim, insightText(claimed), claimed.revision)); this.changed({ edits });
    });
  }
  edit(id: string, text: string) { const edit = this.state.edits.get(id); if (edit) { edit.edit(text); this.changed(); } }
  compose(id: string, value: boolean) { const edit = this.state.edits.get(id); if (edit) { edit.composing = value; this.changed(); } }
  async saveEdit(id: string, finish = true) {
    const initial = this.state.edits.get(id); if (!initial || initial.composing || !initial.text.trim() || !insightsWritable(this.state.snapshot)) return false;
    if (!initial.dirty) { if (finish) await this.cancelEdit(id); return true; }
    return !!await this.write(async () => {
      let item = this.item(id); const edit = this.state.edits.get(id);
      if (!item || !edit || !this.state.snapshot) { if (edit) edit.conflict = true; throw new InsightUiError("edit_conflict"); }
      if (item.revision !== edit.item_revision) { edit.conflict = true; this.changed(); throw new InsightUiError("edit_conflict"); }
      if (!edit.claim_id) {
        this.apply(await this.service.claim(this.id, { store_revision: this.state.snapshot.revision, item_id: id, item_revision: item.revision }));
        item = this.item(id)!; edit.claim_id = item.edit_claim; edit.item_revision = item.revision;
      }
      const text = edit.text, localRevision = edit.local_revision;
      try {
        const snapshot = await this.service.review(this.id, { store_revision: this.state.snapshot!.revision, item_id: id, item_revision: edit.item_revision,
          update: { edited_text: text, edit_claim: edit.claim_id, release_edit_claim: finish } });
        this.apply(snapshot); if (!insightsWritable(this.state.snapshot)) throw new InsightUiError("edit_conflict");
        const saved = snapshot.items.find(item => item.id === id);
        if (!saved) throw new InsightUiError("edit_conflict");
        const unchanged = edit.acknowledge(text, saved.revision, localRevision, finish);
        if (finish && unchanged) { const edits = new Map(this.state.edits); edits.delete(id); this.changed({ edits }); }
        else this.changed();
        return true;
      } catch (error) { edit.conflict = true; this.changed(); throw error; }
    });
  }
  async cancelEdit(id: string) {
    const edit = this.state.edits.get(id); if (!edit) return;
    const localRevision = edit.local_revision;
    await this.write(async () => {
      const item = this.item(id);
      if (item && this.state.snapshot && edit.claim_id) this.apply(await this.service.review(this.id, { store_revision: this.state.snapshot.revision,
        item_id: id, item_revision: item.revision, update: { edit_claim: edit.claim_id, release_edit_claim: true } }));
      edit.claim_id = null;
      if (edit.local_revision === localRevision) { const edits = new Map(this.state.edits); edits.delete(id); this.changed({ edits }); }
    });
  }
  async flush() {
    await this.pendingWrite;
    for (const [id, edit] of this.state.edits) if (edit.dirty && !await this.saveEdit(id, false)) return false;
    return !this.dirty;
  }
  discardLocalEdits() {
    const edits = [...this.state.edits.values()]; this.changed({ edits: new Map() });
    void this.write(async () => {
      for (const edit of edits) {
        const item = this.item(edit.item_id); if (!item || !edit.claim_id || !this.state.snapshot) continue;
        this.apply(await this.service.review(this.id, { store_revision: this.state.snapshot.revision, item_id: item.id, item_revision: item.revision,
          update: { edit_claim: edit.claim_id, release_edit_claim: true } }));
      }
    });
  }
  async insert(id: string, host: InsightNoteHost) {
    if (!insightsWritable(this.state.snapshot) || this.state.edits.size || this.item(id)?.review_state !== "accepted") return false;
    const receipt = insightReceipt(this.state.snapshot, id); if (receipt && receipt.state !== "not_inserted") return false;
    return !!await this.write(async () => {
      if (!await host.flush()) throw new InsightUiError("notes_not_saved");
      const before = { ...host.current() }; if (before.composing) throw new InsightUiError("notes_changed");
      this.apply(await this.service.read(this.id), true);
      if (!insertionStillCurrent(before, host.current())) throw new InsightUiError("notes_changed");
      const prior = insightReceipt(this.state.snapshot, id);
      if (prior && prior.state !== "not_inserted" || !insightsWritable(this.state.snapshot) || this.item(id)?.review_state !== "accepted") return false;
      const item = this.item(id)!; const markdown = await this.service.itemMarkdown(this.id, id, item.revision);
      if (!insertionStillCurrent(before, host.current())) throw new InsightUiError("notes_changed");
      const preparation = await this.service.prepare(this.id, { store_revision: this.state.snapshot!.revision, item_id: id,
        item_revision: item.revision, expected_notes: before.text, intended_notes: appendInsightMarkdown(before.text, markdown), markdown });
      this.apply(preparation.snapshot);
      if (!preparation.stage || preparation.intended_notes === null) return false;
      if (!insertionStillCurrent(before, host.current())) throw new InsightUiError("notes_changed");
      if (!await host.stage(preparation.intended_notes, preparation.receipt.attempt_id, before)) throw new InsightUiError("notes_not_saved");
      this.apply(await this.service.read(this.id), true); return true;
    });
  }
  async reconcile() { await this.write(async () => { if (this.state.snapshot) this.apply(await this.service.reconcile(this.id, this.state.snapshot.revision)); }); }
  async resolveInsertion(attemptId: string, decision: "acknowledged" | "not_inserted", host: InsightNoteHost, inspected: InsightNoteLatch) {
    await this.write(async () => {
      const current = host.current();
      if (!this.state.snapshot || !insertionStillCurrent({ ...inspected, interaction_revision: current.interaction_revision }, current)) throw new InsightUiError("notes_changed");
      this.apply(await this.service.resolve(this.id, { store_revision: this.state.snapshot.revision, attempt_id: attemptId, decision, expected_notes: inspected.text }));
    });
  }
  async reassociate(request: Omit<InsightReassociationRequest, "store_revision">) {
    await this.write(async () => { const item = this.item(request.item_id); if (!item || !this.state.snapshot) return;
      if (item.revision !== request.item_revision) throw new InsightUiError("edit_conflict");
      this.apply(await this.service.reassociate(this.id, { ...request, store_revision: this.state.snapshot.revision })); });
  }
  async export() { return this.action("export", () => this.service.export(this.id)); }
}
