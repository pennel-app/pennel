import type { MeetingSummaryRecord } from "./api";

export interface SummaryDocument {
  meeting_uid: string;
  markdown: string;
  revision: string;
  source_revision: string;
  edited?: boolean;
  outdated?: boolean;
  record?: MeetingSummaryRecord | null;
}

/** The acknowledged document advances independently of text currently being typed. */
export class SummaryDraft {
  document: SummaryDocument | null = null;
  value = "";
  editRevision = 0;
  dirty = false;
  pending = false;
  error: string | null = null;
  candidate: (SummaryDocument & { editRevision: number }) | null = null;
  remote: SummaryDocument | null = null;
  deferredPresentation: SummaryDocument | null = null;

  receive(document: SummaryDocument, protectedInput = false): boolean {
    if (this.document?.revision === document.revision) {
      if (this.document.source_revision !== document.source_revision && (this.dirty || this.pending || protectedInput)) {
        this.remote = document; return false;
      }
      const changedRecord = JSON.stringify(this.document.record ?? null) !== JSON.stringify(document.record ?? null);
      if (changedRecord && (this.dirty || this.pending || protectedInput)) {
        this.deferredPresentation = document;
        this.document = { ...this.document, source_revision: document.source_revision, outdated: document.outdated };
        return false;
      }
      this.deferredPresentation = null;
      this.document = { ...this.document, source_revision: document.source_revision, outdated: document.outdated, record: document.record ?? null };
      return false;
    }
    if (this.document && (this.dirty || this.pending || protectedInput)) { this.remote = document; return false; }
    this.document = document; this.value = document.markdown; this.dirty = false; this.error = null; this.remote = null; this.deferredPresentation = null;
    return true;
  }
  edit(value: string) { this.value = value; ++this.editRevision; this.dirty = value !== this.document?.markdown; }
  applyPresentation(protectedInput = false): boolean {
    if (!this.deferredPresentation || protectedInput || this.dirty || this.pending || this.remote) return false;
    const next = this.deferredPresentation;
    this.deferredPresentation = null;
    if (this.document?.meeting_uid !== next.meeting_uid || this.document.revision !== next.revision || this.document.source_revision !== next.source_revision) return false;
    this.document = { ...this.document, record: next.record ?? null };
    return true;
  }
  stage(document: SummaryDocument, editRevision: number) { this.candidate = { ...document, editRevision }; }
  get canAccept() {
    return !!this.candidate && !this.pending && !this.remote && this.candidate.editRevision === this.editRevision
      && this.candidate.revision === this.document?.revision && this.candidate.source_revision === this.document?.source_revision;
  }
  get canMarkReviewed() {
    return !!this.document?.outdated && !!this.document.markdown.trim() && !this.dirty && !this.pending && !this.remote
      && this.value === this.document.markdown;
  }
  acknowledgeMetadata(document: SummaryDocument): boolean {
    const before = this.document;
    if (!before || this.remote || document.meeting_uid !== before.meeting_uid || document.markdown !== before.markdown || document.source_revision !== before.source_revision) return false;
    this.document = document;
    this.deferredPresentation = null;
    this.dirty = this.value !== document.markdown;
    if (this.candidate?.meeting_uid === before.meeting_uid && this.candidate.revision === before.revision && this.candidate.source_revision === before.source_revision) {
      this.candidate = { ...this.candidate, revision: document.revision };
    }
    return true;
  }
  acknowledge(document: SummaryDocument, target: string) {
    this.document = document; this.error = null; this.remote = null; this.deferredPresentation = null;
    this.dirty = this.value !== target;
  }
  useRemote() {
    if (!this.remote) return false;
    this.document = this.remote; this.value = this.remote.markdown; this.remote = null; this.dirty = false;
    this.candidate = null; this.error = null; this.deferredPresentation = null; ++this.editRevision; return true;
  }
  keepEditAgainstRemote() {
    if (!this.remote) return false;
    this.document = this.remote; this.remote = null; this.dirty = this.value !== this.document.markdown;
    this.candidate = null; this.error = null; this.deferredPresentation = null; return true;
  }
}
