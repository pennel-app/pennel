export interface SummaryDocument {
  meeting_uid: string;
  markdown: string;
  revision: string;
  source_revision: string;
  edited?: boolean;
  outdated?: boolean;
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

  receive(document: SummaryDocument, protectedInput = false): boolean {
    if (this.document?.revision === document.revision) {
      if (this.document.source_revision !== document.source_revision && (this.dirty || this.pending || protectedInput)) {
        this.remote = document; return false;
      }
      this.document = { ...this.document, source_revision: document.source_revision, outdated: document.outdated };
      return false;
    }
    if (this.document && (this.dirty || this.pending || protectedInput)) { this.remote = document; return false; }
    this.document = document; this.value = document.markdown; this.dirty = false; this.error = null; this.remote = null;
    return true;
  }
  edit(value: string) { this.value = value; ++this.editRevision; this.dirty = value !== this.document?.markdown; }
  stage(document: SummaryDocument, editRevision: number) { this.candidate = { ...document, editRevision }; }
  get canAccept() {
    return !!this.candidate && !this.pending && !this.remote && this.candidate.editRevision === this.editRevision
      && this.candidate.revision === this.document?.revision && this.candidate.source_revision === this.document?.source_revision;
  }
  get canMarkReviewed() {
    return !!this.document?.outdated && !!this.document.markdown.trim() && !this.dirty && !this.pending && !this.remote
      && this.value === this.document.markdown;
  }
  acknowledge(document: SummaryDocument, target: string) {
    this.document = document; this.error = null; this.remote = null;
    this.dirty = this.value !== target;
  }
  useRemote() {
    if (!this.remote) return false;
    this.document = this.remote; this.value = this.remote.markdown; this.remote = null; this.dirty = false;
    this.candidate = null; this.error = null; ++this.editRevision; return true;
  }
  keepEditAgainstRemote() {
    if (!this.remote) return false;
    this.document = this.remote; this.remote = null; this.dirty = this.value !== this.document.markdown;
    this.candidate = null; this.error = null; return true;
  }
}
