import { Annotation, EditorSelection, EditorState, Text, Transaction, type Extension, type TransactionSpec } from "@codemirror/state";
import { history, isolateHistory } from "@codemirror/commands";
import { formatSelection } from "./live-workspace-state.ts";
export const quietNoteChange = Annotation.define<boolean>();
export type NoteFormat = "heading" | "bold" | "list" | "task";
export function inlineNoteState(value: string, extensions: Extension[] = []): EditorState {
  return EditorState.create({ doc: Text.of(value.split("\n")), extensions: [EditorState.lineSeparator.of("\n"), history(), ...extensions] });
}
function changeBetween(before: string, after: string) {
  if (before === after) return null;
  let from = 0, to = before.length, end = after.length;
  while (from < to && from < end && before[from] === after[from]) from++;
  while (to > from && end > from && before[to - 1] === after[end - 1]) { to--; end--; }
  return { from, to, insert: after.slice(from, end) };
}
export function quietNoteValue(state: EditorState, value: string): TransactionSpec | null {
  const change = changeBetween(state.doc.toString(), value);
  if (!change) return null;
  const range = state.selection.main;
  return { changes: change, selection: EditorSelection.single(Math.min(range.anchor, value.length), Math.min(range.head, value.length)),
    annotations: [quietNoteChange.of(true), Transaction.addToHistory.of(false), Transaction.remote.of(true)], scrollIntoView: false };
}
export function userNoteValue(state: EditorState, value: string, selection?: { anchor: number; head: number }): TransactionSpec | null {
  const changes = changeBetween(state.doc.toString(), value);
  if (changes && state.readOnly) return null;
  if (!changes) return selection ? { selection, scrollIntoView: false } : null;
  return { changes, selection, userEvent: "input.api", annotations: isolateHistory.of("full"), scrollIntoView: false };
}
export function formatNote(state: EditorState, format: NoteFormat): TransactionSpec | null {
  if (state.readOnly) return null;
  const range = state.selection.main, result = formatSelection(state.doc.toString(), range.from, range.to, format);
  const change = userNoteValue(state, result.value, range.anchor > range.head ? { anchor: result.end, head: result.start } : { anchor: result.start, head: result.end });
  return change ? { ...change, userEvent: "input.format" } : null;
}
export function noteSelection(start: number, end: number, direction: "forward" | "backward" | "none", length: number): EditorSelection {
  const to = Math.min(length, Math.max(0, end)), from = Math.min(to, Math.max(0, start));
  return EditorSelection.single(direction === "backward" ? to : from, direction === "backward" ? from : to);
}
