export interface SourceRange { start: number; end: number }
export interface SourceBlocks { first: number; last: number }

function normalized(text: string) {
  const positions: SourceRange[] = [];
  let value = "";
  for (const item of new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(text)) {
    const part = item.segment.normalize("NFKC");
    const position = { start: item.index, end: item.index + item.segment.length };
    for (const character of part) {
      if (/\s/u.test(character)) {
        if (!value.length) continue;
        if (value.endsWith(" ")) { positions[positions.length - 1].end = position.end; continue; }
        value += " "; positions.push({ ...position });
      } else {
        value += character;
        for (let index = 0; index < character.length; index++) positions.push({ ...position });
      }
    }
  }
  if (value.endsWith(" ")) { value = value.slice(0, -1); positions.pop(); }
  return { value, positions };
}

/** Match the entire cited passage, preserving UTF-16 offsets for editable source text. */
export function sourceRange(text: string, passage: string): SourceRange | null {
  const haystack = normalized(text);
  const needle = normalized(passage).value;
  if (!needle) return null;
  const word = (character: string | undefined) => !!character && /[\p{L}\p{N}_]/u.test(character);
  const firstWord = word([...needle][0]);
  const lastWord = word([...needle].at(-1));
  const matches: number[] = [];
  for (let offset = haystack.value.indexOf(needle); offset >= 0; offset = haystack.value.indexOf(needle, offset + 1)) {
    const previous = haystack.value.charCodeAt(offset - 1);
    const before = haystack.value.slice(Math.max(0, offset - (previous >= 0xDC00 && previous <= 0xDFFF ? 2 : 1)), offset);
    const next = haystack.value.codePointAt(offset + needle.length);
    const after = next === undefined ? undefined : String.fromCodePoint(next);
    if (firstWord && word(before) || lastWord && word(after)) continue;
    matches.push(offset);
    if (matches.length > 1) return null;
  }
  if (!matches.length) return null;
  const start = matches[0];
  return { start: haystack.positions[start].start, end: haystack.positions[start + needle.length - 1].end };
}

export function sourceBlocks(blocks: string[], passage: string): SourceBlocks | null {
  const range = sourceRange(blocks.join("\n"), passage);
  if (!range) return null;
  let offset = 0;
  let first = -1;
  let last = -1;
  blocks.forEach((block, index) => {
    if (offset < range.end && offset + block.length > range.start) {
      if (first < 0) first = index;
      last = index;
    }
    offset += block.length + 1;
  });
  return first < 0 ? null : { first, last };
}

export function transcriptSourceBlocks(
  segments: { start_ms: number; end_ms: number; text: string; utterance_id?: string | null; source_fingerprint?: string | null }[],
  source: { start_ms: number | null; text: string; utterance_ids?: string[]; utterance_fingerprints?: string[] },
): SourceBlocks | null {
  if (source.utterance_ids != null) {
    const ids = source.utterance_ids;
    if (!ids.length || source.utterance_fingerprints?.length !== ids.length || new Set(ids).size !== ids.length) return null;
    const indices: number[] = [];
    for (const [offset, id] of ids.entries()) {
      const matches = segments.flatMap((segment, index) => segment.utterance_id === id ? [index] : []);
      if (matches.length !== 1 || !segments[matches[0]].source_fingerprint || segments[matches[0]].source_fingerprint !== source.utterance_fingerprints[offset]
        || offset > 0 && matches[0] !== indices[offset - 1] + 1) return null;
      indices.push(matches[0]);
    }
    return { first: indices[0], last: indices[indices.length - 1] };
  }
  const first = source.start_ms === null ? 0
    : segments.findIndex((segment) => segment.start_ms <= source.start_ms! && segment.end_ms > source.start_ms!);
  if (first < 0) return null;
  const found = sourceBlocks(segments.slice(first).map((segment) => segment.text), source.text);
  if (!found || source.start_ms !== null && found.first !== 0) return null;
  return { first: first + found.first, last: first + found.last };
}
