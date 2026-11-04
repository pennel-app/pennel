import type { MeetingSummaryCandidate, MeetingSummaryDocument } from './domain.ts';

interface SummaryDraftSnapshot {
  document: MeetingSummaryDocument | null;
  remote: MeetingSummaryDocument | null;
  draft: string;
  dirty: boolean;
  conflict: boolean;
  saving: boolean;
  failed: boolean;
}

function sameVersion(left: Pick<MeetingSummaryDocument, 'revision' | 'source_revision'> | null, right: Pick<MeetingSummaryDocument, 'revision' | 'source_revision'> | null) {
  return Boolean(left && right && left.revision === right.revision && left.source_revision === right.source_revision);
}

/** Keeps an editable UI draft while the core owns version checks and storage. */
export class SummaryDraft {
  private state: SummaryDraftSnapshot = { document: null, remote: null, draft: '', dirty: false, conflict: false, saving: false, failed: false };
  private listeners = new Set<() => void>();
  private pending: Promise<boolean> | null = null;
  private readonly persist: (base: MeetingSummaryDocument, markdown: string) => Promise<MeetingSummaryDocument | undefined>;
  private readonly acceptCandidate: (base: MeetingSummaryDocument, markdown: string) => Promise<MeetingSummaryDocument | undefined>;
  constructor(persist: (base: MeetingSummaryDocument, markdown: string) => Promise<MeetingSummaryDocument | undefined>, acceptCandidate: (base: MeetingSummaryDocument, markdown: string) => Promise<MeetingSummaryDocument | undefined>) { this.persist = persist; this.acceptCandidate = acceptCandidate; }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<SummaryDraftSnapshot>) {
    const next = { ...this.state, ...patch };
    next.dirty = Boolean(next.document && next.draft !== next.document.markdown);
    next.conflict = next.dirty && !sameVersion(next.document, next.remote);
    this.state = next;
    for (const listener of this.listeners) listener();
  }
  private finishWrite(before: MeetingSummaryDocument, saved: MeetingSummaryDocument, writtenDraft: string) {
    const remote = sameVersion(this.state.remote, before) ? saved : this.state.remote;
    if (this.state.draft === writtenDraft && remote && !sameVersion(remote, saved)) this.update({ document: remote, remote, draft: remote.markdown, saving: false });
    else this.update({ document: saved, remote, draft: this.state.draft === writtenDraft ? saved.markdown : this.state.draft, saving: false });
  }
  adopt = (remote: MeetingSummaryDocument) => {
    if (!this.state.dirty && !this.state.saving) this.update({ document: remote, remote, draft: remote.markdown, failed: false });
    else this.update({ remote });
  };
  edit = (draft: string) => this.update({ draft, failed: false });
  discard = () => {
    const document = this.state.remote;
    this.update({ document, draft: document?.markdown ?? '', failed: false });
  };
  canMarkReviewed = (): boolean => Boolean(this.state.document?.outdated && this.state.document.markdown.trim() && !this.state.dirty && !this.state.saving && !this.state.conflict && sameVersion(this.state.document, this.state.remote));
  markReviewed = (): Promise<boolean> => {
    const document = this.state.document;
    if (!document || !this.canMarkReviewed()) return Promise.resolve(false);
    return this.accept(document);
  };
  accept = async (candidate: MeetingSummaryCandidate): Promise<boolean> => {
    const document = this.state.document;
    if (!document || this.state.dirty || this.state.saving || !sameVersion(document, candidate) || document.meeting_uid !== candidate.meeting_uid) return false;
    this.update({ saving: true, failed: false });
    let saved: MeetingSummaryDocument | undefined;
    try { saved = await this.acceptCandidate(document, candidate.markdown); } catch { saved = undefined; }
    if (!saved) { this.update({ saving: false, failed: true }); return false; }
    this.finishWrite(document, saved, document.markdown);
    return true;
  };
  save = (): Promise<boolean> => {
    if (this.pending) return this.pending;
    if (this.state.saving) return Promise.resolve(false);
    this.pending = this.flush().finally(() => { this.pending = null; });
    return this.pending;
  };
  private async flush(): Promise<boolean> {
    while (this.state.dirty) {
      const document = this.state.document;
      if (!document || this.state.conflict) return false;
      const written = this.state.draft;
      this.update({ saving: true, failed: false });
      let saved: MeetingSummaryDocument | undefined;
      try { saved = await this.persist(document, written); } catch { saved = undefined; }
      if (!saved) { this.update({ saving: false, failed: true }); return false; }
      this.finishWrite(document, saved, written);
    }
    return true;
  }
}
