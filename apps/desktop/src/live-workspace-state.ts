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
  select(pane: LivePane) {
    this.selected = pane;
    if (pane === "notes" || pane === "summary") this.primary = pane;
    else { this.companion = pane; this.companionOpen = true; }
  }
  closeCompanion() { this.companionOpen = false; this.selected = this.primary; }
  toggleCompanion(pane: CompanionPane) {
    if (this.companionOpen && this.companion === pane && (this.wide || this.selected === pane)) this.closeCompanion();
    else this.select(pane);
  }
  resize(width: number) { this.wide = width >= 800; }
  shown(pane: LivePane) { return this.wide ? pane === this.primary || this.companionOpen && pane === this.companion : pane === this.selected; }
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
