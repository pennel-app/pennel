export type MarkdownSpan = { kind: 'text' | 'bold' | 'code'; text: string };
export type MarkdownBlock =
  | { kind: 'gap' }
  | { kind: 'heading'; level: number; spans: MarkdownSpan[] }
  | { kind: 'paragraph'; spans: MarkdownSpan[] }
  | { kind: 'bullet'; spans: MarkdownSpan[] }
  | { kind: 'task'; done: boolean; spans: MarkdownSpan[] };

export function markdownSpans(text: string): MarkdownSpan[] {
  const spans: MarkdownSpan[] = [];
  let offset = 0;
  for (const match of text.matchAll(/(\*\*[^*]+\*\*|`[^`]+`)/g)) {
    if (match.index > offset) spans.push({ kind: 'text', text: text.slice(offset, match.index) });
    const bold = match[0].startsWith('**');
    spans.push({ kind: bold ? 'bold' : 'code', text: match[0].slice(bold ? 2 : 1, bold ? -2 : -1) });
    offset = match.index + match[0].length;
  }
  if (offset < text.length) spans.push({ kind: 'text', text: text.slice(offset) });
  return spans;
}

/** Mirrors the desktop renderer: the containing meeting owns level 1, so nested heading depths start at 2. */
export function contentHeadingLevels(depths: readonly number[]): number[] {
  const ancestors: number[] = [];
  return depths.map(depth => {
    while (ancestors.length && ancestors[ancestors.length - 1] >= depth) ancestors.pop();
    ancestors.push(depth);
    return Math.min(ancestors.length, 5) + 1;
  });
}

export function markdownBlocks(text: string): MarkdownBlock[] {
  const lines = text.split('\n');
  const levels = contentHeadingLevels(lines.flatMap(line => {
    const marker = line.trim().match(/^(#{1,6})\s+/);
    return marker ? [marker[1].length] : [];
  }));
  let headingIndex = 0;
  const blocks: MarkdownBlock[] = [];
  for (const line of lines) {
    const clean = line.trim();
    if (!clean) {
      if (blocks.length && blocks[blocks.length - 1].kind !== 'gap') blocks.push({ kind: 'gap' });
      continue;
    }
    const heading = clean.match(/^(#{1,6})\s+(.+)/);
    if (heading) { blocks.push({ kind: 'heading', level: levels[headingIndex++], spans: markdownSpans(heading[2]) }); continue; }
    const bullet = clean.match(/^[-*•]\s+(.+)/);
    const task = bullet?.[1].match(/^\[([ xX])\]\s+(.+)/);
    if (task) blocks.push({ kind: 'task', done: Boolean(task[1].trim()), spans: markdownSpans(task[2]) });
    else if (bullet) blocks.push({ kind: 'bullet', spans: markdownSpans(bullet[1]) });
    else blocks.push({ kind: 'paragraph', spans: markdownSpans(clean) });
  }
  if (blocks[blocks.length - 1]?.kind === 'gap') blocks.pop();
  return blocks;
}
