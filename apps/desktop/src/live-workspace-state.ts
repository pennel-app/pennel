export type WritingPane = "notes" | "summary";
export type CompanionPane = "transcript" | "questions" | "suggestions" | "tools";
export type LivePane = WritingPane | CompanionPane;

export class LivePaneState {
  selected: LivePane = "notes";
  primary: WritingPane = "notes";
  companion: CompanionPane = "transcript";
  companionOpen = false;
  wide = false;
  ratio = 60;
  private finished = false;
  private companionChosen = false;
  setFinished(finished: boolean) {
    this.finished = finished;
    if (!this.companionChosen) this.companion = finished ? "suggestions" : "transcript";
  }
  select(pane: LivePane) {
    this.selected = pane;
    if (pane === "notes" || pane === "summary") {
      this.primary = pane;
      if (!this.wide) this.companionOpen = false;
    } else { this.companionChosen = true; this.companion = pane; this.companionOpen = true; }
  }
  closeCompanion() { this.companionChosen = true; this.companionOpen = false; this.selected = this.primary; }
  toggleCompanion(pane: CompanionPane) {
    if (this.companionOpen && this.companion === pane) this.closeCompanion();
    else this.select(pane);
  }
  resize(width: number) {
    this.wide = width >= 1100;
    if (!this.companionChosen) { this.companionOpen = this.wide; this.companion = this.finished ? "suggestions" : "transcript"; this.selected = this.primary; }
  }
  shown(pane: LivePane) {
    if (pane === "notes" || pane === "summary") return this.finished || pane === this.primary;
    return this.companionOpen && pane === this.companion;
  }
}

export function formatSelection(value: string, start: number, end: number, format: "heading" | "bold" | "list" | "task") {
  if (format === "bold") return { value: value.slice(0, start) + "**" + value.slice(start, end) + "**" + value.slice(end), start: start + 2, end: end + 2 };
  const from = value.lastIndexOf("\n", Math.max(0, start - 1)) + 1;
  const next = value.indexOf("\n", end);
  const until = next < 0 ? value.length : next;
  const prefix = format === "heading" ? "## " : format === "task" ? "- [ ] " : "- ";
  const selected = value.slice(from, until);
  const changed = selected.split("\n").map(line => prefix + line).join("\n");
  const insertedBeforeEnd = prefix.length * (value.slice(from, end).split("\n").length);
  return { value: value.slice(0, from) + changed + value.slice(until), start: start + prefix.length, end: end + insertedBeforeEnd };
}

export class LiveStatusEpoch {
  private revision = 0;
  capture() { return this.revision; }
  invalidate() { ++this.revision; }
  accepts(revision: number) { return revision === this.revision; }
}

export function segmentEvidenceValue(segment: { start_ms: number; end_ms?: number; speaker: string; text: string; failed: boolean; voice: number | null; utterance_id?: string | null; source_id?: string | null }) {
  const text = segment.failed ? "" : segment.text.split(/\p{White_Space}+/u).filter(Boolean).join(" ");
  if (segment.utterance_id) return [segment.utterance_id, segment.source_id ?? null, segment.start_ms, segment.end_ms, text, segment.failed];
  return [segment.start_ms, segment.speaker, text, segment.failed, segment.speaker === "others" ? segment.voice ?? null : null];
}
