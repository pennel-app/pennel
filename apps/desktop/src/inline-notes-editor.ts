import { playMotion } from "./motion";
import { Compartment, EditorState, Prec, StateEffect, StateField, Transaction, type Range, type SelectionRange } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, keymap, placeholder, type DecorationSet } from "@codemirror/view";
import { historyKeymap, isolateHistory, redo, standardKeymap, undo } from "@codemirror/commands";
import { bindLocalization, type UiText } from "./i18n.ts";
import { icon } from "./icons";
import { noteMarkdown } from "./inline-notes-markdown";
import { formatNote, inlineNoteState, noteSelection, quietNoteChange, quietNoteValue, userNoteValue, type NoteFormat } from "./inline-notes-state";
import "./inline-notes.css";

type Direction = "forward" | "backward" | "none";
export interface InlineNotesSurface extends HTMLDivElement {
  value: string;
  selectionStart: number;
  selectionEnd: number;
  selectionDirection: Direction;
  disabled: boolean;
  readOnly: boolean;
  readonly composing: boolean;
  readonly contentDOM: HTMLElement;
  readonly scrollDOM: HTMLElement;
  setSelectionRange(start: number, end: number, direction?: Direction): void;
  applyUserChange(value: string, selection?: { start: number; end: number; direction?: Direction }): boolean;
  applyFormat(format: NoteFormat): boolean;
  revealSelection(): void;
  setDocumentMode(value: boolean): void;
  undo(): boolean;
  redo(): boolean;
  destroy(): void;
}

class Bullet extends WidgetType {
  eq(): boolean { return true; }
  toDOM(): HTMLElement { const node = document.createElement("span"); node.className = "cm-note-bullet"; node.textContent = "•"; node.setAttribute("aria-hidden", "true"); return node; }
  ignoreEvent(): boolean { return false; }
}
class TaskMarker extends WidgetType {
  constructor(readonly at: number, readonly checked: boolean, readonly disabled: boolean, readonly label: string) { super(); }
  eq(other: TaskMarker): boolean { return this.at === other.at && this.checked === other.checked && this.disabled === other.disabled && this.label === other.label; }
  toDOM(view: EditorView): HTMLElement {
    const box = document.createElement("span"); box.className = "cm-note-task-marker"; box.contentEditable = "false";
    const input = document.createElement("input"); input.type = "checkbox";
    const mark = document.createElement("span"); mark.setAttribute("aria-hidden", "true"); mark.append(icon("check"));
    box.append(input, mark); this.updateDOM(box);
    input.addEventListener("pointerdown", event => event.preventDefault());
    input.addEventListener("input", event => event.stopPropagation());
    input.addEventListener("change", event => {
      event.stopPropagation();
      const at = Number(input.dataset.sourceOffset);
      if (!view.dom.contains(input) || !Number.isSafeInteger(at) || view.state.readOnly || !/^\[[ xX]\]$/.test(view.state.sliceDoc(at - 1, at + 2))) return;
      view.dispatch({ changes: { from: at, to: at + 1, insert: input.checked ? "x" : " " }, userEvent: "input.task", annotations: isolateHistory.of("full") });
    });
    return box;
  }
  updateDOM(node: HTMLElement): boolean {
    const input = node.querySelector("input")!;
    input.checked = this.checked; input.disabled = this.disabled; input.dataset.sourceOffset = String(this.at);
    input.setAttribute("aria-label", this.label);
    return true;
  }
}
const noteFocus = StateEffect.define<boolean>();
function buildDecorations(state: EditorView["state"], focused: boolean): DecorationSet {
  const active: number[] = [];
  if (focused) {
    const range = state.selection.main;
    for (let line = state.doc.lineAt(range.from).number, end = state.doc.lineAt(range.to).number; line <= end; line++) active.push(line);
  }
  const ranges: Range<Decoration>[] = [];
  for (const line of noteMarkdown(state.doc.toString(), active)) {
    if (line.heading) ranges.push(Decoration.line({ class: `cm-note-heading cm-note-h${line.heading}` }).range(line.from));
    if (line.code) ranges.push(Decoration.line({ class: "cm-note-code" }).range(line.from));
    if (line.active) continue;
    if (line.heading) ranges.push(Decoration.replace({}).range(line.from, line.prefixTo));
    const marker = line.marker;
    if (marker) {
      ranges.push(Decoration.replace({ widget: marker.kind === "task" ? new TaskMarker(marker.checkAt!, marker.checked, state.readOnly, state.sliceDoc(marker.to, line.to)) : new Bullet() }).range(marker.from, marker.to));
      if (marker.checked && marker.to < line.to) ranges.push(Decoration.mark({ class: "cm-note-task-done" }).range(marker.to, line.to));
    }
    for (const mark of line.strong) {
      ranges.push(Decoration.replace({}).range(mark.open, mark.open + 2), Decoration.mark({ class: "cm-note-strong" }).range(mark.from, mark.to), Decoration.replace({}).range(mark.close, mark.close + 2));
    }
  }
  return Decoration.set(ranges, true);
}
const noteDecorations = StateField.define<{ focused: boolean; decorations: DecorationSet }>({
  create: state => ({ focused: false, decorations: buildDecorations(state, false) }),
  update(value, transaction) {
    let focused = value.focused;
    for (const effect of transaction.effects) if (effect.is(noteFocus)) focused = effect.value;
    return transaction.docChanged || transaction.selection || transaction.reconfigured || focused !== value.focused
      ? { focused, decorations: buildDecorations(transaction.state, focused) } : value;
  },
  provide: field => EditorView.decorations.from(field, value => value.decorations),
});

export function inlineNotesEditor(options: { value?: string; label: UiText; placeholder?: UiText; language?: string; className?: string; deferKey?(event: KeyboardEvent): boolean }): InlineNotesSurface {
  const element = document.createElement("div") as InlineNotesSurface;
  element.className = `inline-notes-editor ${options.className ?? ""}`.trim();
  let disabled = false, readOnly = false, composing = false, destroyed = false, direction: Direction | null = null, language = options.language ?? "";
  type ReaderPosition = { node: HTMLElement; top: number; left: number };
  let revealGeneration = 0;
  let pendingReveal: { generation: number; range: SelectionRange; readers: ReaderPosition[] } | null = null;
  const readerMoved = (readers: ReaderPosition[]) => readers.some(({ node, top, left }) => Math.abs(node.scrollTop - top) > 1 || Math.abs(node.scrollLeft - left) > 1);
  function revealScroll(event: Event) {
    const reveal = pendingReveal;
    if (reveal && (event.target === element.ownerDocument || reveal.readers.some(({ node }) => node === event.target)) && readerMoved(reveal.readers)) ++revealGeneration;
  }
  const behavior = new Compartment();
  const configuration = () => [EditorState.readOnly.of(disabled || readOnly), EditorView.editable.of(!disabled && !readOnly),
    EditorView.contentAttributes.of({ "aria-label": String(options.label), "aria-disabled": String(disabled), tabindex: disabled ? "-1" : "0", lang: language, spellcheck: "true" }),
    placeholder(String(options.placeholder ?? ""))];
  const view = new EditorView({ parent: element,
    state: inlineNoteState(options.value ?? "", [noteDecorations, EditorView.lineWrapping, behavior.of(configuration()),
      EditorView.focusChangeEffect.of((_state, focused) => noteFocus.of(focused)),
      EditorView.scrollHandler.of((_view, range, options) => {
        const reveal = pendingReveal; pendingReveal = null;
        element.ownerDocument.removeEventListener("scroll", revealScroll, true);
        // A source reveal may outlive its selection until CodeMirror's next measure pass.
        // Include outer readers: finished Notes expands inside the scrolling document.
        return !!reveal && options.y === "center" && range.eq(reveal.range)
          && (reveal.generation !== revealGeneration || readerMoved(reveal.readers));
      }),
      Prec.highest(EditorView.domEventHandlers({
        keydown: event => options.deferKey?.(event) === true,
        input: event => { event.stopPropagation(); return false; },
        compositionstart: event => { event.stopPropagation(); composing = true; element.dispatchEvent(new CompositionEvent("compositionstart", { data: event.data })); return false; },
        compositionend: event => { event.stopPropagation(); queueMicrotask(() => { if (!destroyed) { composing = false; element.dispatchEvent(new CompositionEvent("compositionend", { data: event.data })); } }); return false; },
        blur: event => { element.dispatchEvent(new FocusEvent("blur", { relatedTarget: event.relatedTarget })); return false; },
      })), keymap.of([...historyKeymap, ...standardKeymap])]),
    dispatchTransactions(transactions, target) {
      const before = target.state.selection;
      const oldState = target.state;
      const resolving = !composing && !target.composing && !transactions.some(transaction => transaction.docChanged)
        && oldState.field(noteDecorations).focused
        ? noteMarkdown(oldState.doc.toString()).filter(line => line.number >= oldState.doc.lineAt(before.main.from).number
          && line.number <= oldState.doc.lineAt(before.main.to).number && (line.heading || line.marker || line.strong.length)).map(line => line.from) : [];
      for (const transaction of transactions) {
        if (transaction.docChanged && pendingReveal) pendingReveal.range = pendingReveal.range.map(transaction.changes);
        if (transaction.docChanged || transaction.selection) ++revealGeneration;
      }
      target.update(transactions);
      for (const from of resolving) {
        const line = target.state.doc.lineAt(from), range = target.state.selection.main;
        if (target.state.field(noteDecorations).focused && line.number >= target.state.doc.lineAt(range.from).number && line.number <= target.state.doc.lineAt(range.to).number) continue;
        const node = target.domAtPos(from).node;
        const element = (node instanceof HTMLElement ? node : node.parentElement)?.closest<HTMLElement>(".cm-line");
        if (element) playMotion(element, "motion-resolve");
      }
      if (!before.eq(target.state.selection)) { direction = null; element.dispatchEvent(new Event("select")); }
      const changes = transactions.filter(transaction => transaction.docChanged && !transaction.annotation(quietNoteChange));
      if (changes.length) element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: changes.some(transaction => transaction.isUserEvent("undo")) ? "historyUndo" : changes.some(transaction => transaction.isUserEvent("redo")) ? "historyRedo" : "insertText", isComposing: composing || target.composing }));
    },
  });
  function reconfigure() { if (!destroyed) view.dispatch({ effects: behavior.reconfigure(configuration()), annotations: quietNoteChange.of(true) }); }
  function setRange(start: number, end: number, next: Direction = "none") {
    view.dispatch({ selection: noteSelection(start, end, next, view.state.doc.length), annotations: Transaction.addToHistory.of(false) });
    direction = next;
  }
  function assign(value: string) {
    const change = quietNoteValue(view.state, String(value));
    if (!change) return;
    const previous = direction;
    view.dispatch(change); direction = previous;
  }
  function revealSelection() {
    const readers: ReaderPosition[] = [];
    for (let node: HTMLElement | null = view.scrollDOM; node; node = node.parentElement) readers.push({ node, top: node.scrollTop, left: node.scrollLeft });
    pendingReveal = { generation: ++revealGeneration, range: view.state.selection.main, readers };
    element.ownerDocument.addEventListener("scroll", revealScroll, true);
    view.dispatch({ effects: EditorView.scrollIntoView(pendingReveal.range, { y: "center" }), annotations: Transaction.addToHistory.of(false) });
  }
  Object.defineProperties(element, {
    value: { get: () => view.state.doc.toString(), set: assign },
    contentDOM: { value: view.contentDOM }, scrollDOM: { value: view.scrollDOM },
    composing: { get: () => composing || view.composing },
    selectionStart: { get: () => view.state.selection.main.from, set: (value: number) => setRange(value, Math.max(value, element.selectionEnd), element.selectionDirection) },
    selectionEnd: { get: () => view.state.selection.main.to, set: (value: number) => setRange(Math.min(value, element.selectionStart), value, element.selectionDirection) },
    selectionDirection: { get: () => view.state.selection.main.empty ? "none" : direction ?? (view.state.selection.main.anchor > view.state.selection.main.head ? "backward" : "forward"), set: (value: Direction) => setRange(element.selectionStart, element.selectionEnd, value) },
    disabled: { get: () => disabled, set: (value: boolean) => { if (disabled !== Boolean(value)) { disabled = Boolean(value); reconfigure(); } } },
    readOnly: { get: () => readOnly, set: (value: boolean) => { if (readOnly !== Boolean(value)) { readOnly = Boolean(value); reconfigure(); } } },
    lang: { get: () => language, set: (value: string) => { if (language !== value) { language = value; reconfigure(); } } },
    focus: { value: () => { if (!disabled) view.focus(); } },
    blur: { value: () => view.contentDOM.blur() },
    scrollTop: { get: () => view.scrollDOM.scrollTop, set: (value: number) => { ++revealGeneration; view.scrollDOM.scrollTop = value; } },
    scrollLeft: { get: () => view.scrollDOM.scrollLeft, set: (value: number) => { ++revealGeneration; view.scrollDOM.scrollLeft = value; } },
    scrollHeight: { get: () => view.scrollDOM.scrollHeight },
    setSelectionRange: { value: setRange }, revealSelection: { value: revealSelection },
    applyUserChange: { value: (value: string, selection?: { start: number; end: number; direction?: Direction }) => {
      const range = selection ? noteSelection(selection.start, selection.end, selection.direction ?? "forward", value.length).main : null;
      const change = userNoteValue(view.state, value, range ? { anchor: range.anchor, head: range.head } : undefined);
      if (!change) return false; view.dispatch(change); return true;
    } },
    applyFormat: { value: (format: NoteFormat) => { if (composing || view.composing) return false; const change = formatNote(view.state, format); if (!change) return false; view.dispatch(change); return true; } },
    undo: { value: () => undo(view) }, redo: { value: () => redo(view) },
    setDocumentMode: { value: (value: boolean) => { if (element.dataset.documentMode !== String(value)) { element.dataset.documentMode = String(value); view.requestMeasure(); } } },
    destroy: { value: () => { destroyed = true; element.ownerDocument.removeEventListener("scroll", revealScroll, true); unbind(); view.destroy(); } },
  });
  const unbind = bindLocalization(element, reconfigure);
  return element;
}
