export interface NoteStrong { from: number; to: number; open: number; close: number }
export interface NoteMarker { kind: "bullet" | "task"; from: number; to: number; checked: boolean; checkAt: number | null }
export interface NoteMarkdownLine { number: number; from: number; to: number; active: boolean; code: boolean; heading: number; prefixTo: number; marker: NoteMarker | null; strong: NoteStrong[] }
interface Range { from: number; to: number }

function escaped(source: string, at: number): boolean {
  let slashes = 0;
  while (at > 0 && source[--at] === "\\") slashes++;
  return slashes % 2 === 1;
}
function runEnd(source: string, at: number, character: string): number {
  while (source[at] === character) at++;
  return at;
}
function codeSpans(source: string, blocks: readonly Range[]): Range[] {
  const result = [...blocks];
  let blockIndex = 0;
  for (let at = 0; at < source.length;) {
    while (blocks[blockIndex]?.to <= at) blockIndex++;
    const nextBlock = blocks[blockIndex];
    const block = nextBlock && at >= nextBlock.from ? nextBlock : null;
    if (block) { at = block.to; continue; }
    if (source[at] !== "`" || escaped(source, at)) { at++; continue; }
    const end = runEnd(source, at, "`"), size = end - at;
    let close = -1;
    for (let scan = end; scan < source.length;) {
      if (nextBlock && scan >= nextBlock.from) break;
      if (source[scan] === "\n") {
        let next = scan + 1;
        while (source[next] === " " || source[next] === "\t" || source[next] === "\r") next++;
        if (source[next] === "\n") break;
      }
      if (source[scan] !== "`") { scan++; continue; }
      const stop = runEnd(source, scan, "`");
      if (stop - scan === size) { close = scan; break; }
      scan = stop;
    }
    if (close < 0) at = end;
    else { result.push({ from: at, to: close + size }); at = close + size; }
  }
  return result.sort((left, right) => left.from - right.from);
}
function strongSpans(source: string, from: number, to: number, literal: readonly Range[]): NoteStrong[] {
  const result: NoteStrong[] = [];
  let open: number | null = null;
  let low = 0, high = literal.length;
  while (low < high) { const middle = (low + high) >>> 1; if (literal[middle].to <= from) low = middle + 1; else high = middle; }
  let index = low;
  for (let at = from; at < to;) {
    while (literal[index]?.to <= at) index++;
    const range = literal[index];
    const code = range && at >= range.from ? range : null;
    if (code) { at = Math.min(code.to, to); continue; }
    if (source[at] !== "*" || escaped(source, at)) { at++; continue; }
    const end = Math.min(runEnd(source, at, "*"), to), size = end - at;
    const canOpen = end < to && !/\s/.test(source[end]);
    const canClose = at > from && !/\s/.test(source[at - 1]);
    if ((size === 2 || size === 4) && open !== null && canClose) {
      if (at > open + 2) result.push({ from: open + 2, to: at, open, close: at });
      open = size === 4 && canOpen ? at + 2 : null;
    } else if (size === 2 && canOpen) open = at;
    at = end;
  }
  return result;
}

/** Decorations describe canonical UTF-16 source ranges; parsing never edits stored Markdown. */
export function noteMarkdown(source: string, activeLines: readonly number[] = []): NoteMarkdownLine[] {
  const active = new Set(activeLines), lines: NoteMarkdownLine[] = [], blocks: Range[] = [];
  let offset = 0, fence: { marker: string; size: number } | null = null;
  const rawLines = source.split("\n");
  rawLines.forEach((raw, index) => {
    const text = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    const from = offset, to = from + text.length;
    const match = /^( {0,3})(`{3,}|~{3,})(.*)$/.exec(text);
    const marker = match?.[2][0];
    let code = !!fence || /^( {4}|\t)/.test(text);
    if (fence) {
      if (match && marker === fence.marker && match[2].length >= fence.size && !match[3].trim()) fence = null;
    } else if (match && (marker !== "`" || !match[3].includes("`"))) {
      fence = { marker: marker!, size: match[2].length }; code = true;
    }
    const line: NoteMarkdownLine = { number: index + 1, from, to, active: active.has(index + 1), code, heading: 0, prefixTo: from, marker: null, strong: [] };
    if (code) {
      const end = from + raw.length + (index < rawLines.length - 1 ? 1 : 0), previous = blocks.at(-1);
      if (previous?.to === from) previous.to = end;
      else blocks.push({ from, to: end });
    }
    else {
      const heading = /^( {0,3})(#{1,3})([ \t]+)/.exec(text);
      const bullet = /^( {0,3})([-*])([ \t]+)/.exec(text);
      if (heading) { line.heading = heading[2].length; line.prefixTo = from + heading[0].length; }
      else if (bullet) {
        const body = from + bullet[0].length;
        const task = /^\[([ xX])\][ \t]+/.exec(source.slice(body, to));
        line.marker = { kind: task ? "task" : "bullet", from: from + bullet[1].length, to: body + (task?.[0].length ?? 0), checked: !!task && task[1] !== " ", checkAt: task ? body + 1 : null };
      }
    }
    lines.push(line); offset += raw.length + 1;
  });
  const literal = codeSpans(source, blocks);
  let literalIndex = 0;
  for (const line of lines) {
    while (literal[literalIndex]?.to <= line.from) literalIndex++;
    const code = literal[literalIndex];
    if (code && code.from <= line.from && code.to > line.from) { line.heading = 0; line.prefixTo = line.from; line.marker = null; }
    if (!line.active && !line.code) line.strong = strongSpans(source, line.marker?.to ?? line.prefixTo, line.to, literal);
  }
  return lines;
}
