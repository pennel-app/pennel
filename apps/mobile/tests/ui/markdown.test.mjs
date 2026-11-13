import assert from 'node:assert/strict';
import test from 'node:test';
import { contentHeadingLevels, markdownBlocks, markdownSpans } from '../../src/markdown.ts';

test('inline bold and code become styled spans and keep the surrounding text', () => {
  assert.deepEqual(markdownSpans('Ship **Friday** via `make release` now'), [
    { kind: 'text', text: 'Ship ' }, { kind: 'bold', text: 'Friday' }, { kind: 'text', text: ' via ' },
    { kind: 'code', text: 'make release' }, { kind: 'text', text: ' now' },
  ]);
  assert.deepEqual(markdownSpans('Unclosed **marker'), [{ kind: 'text', text: 'Unclosed **marker' }]);
});

test('headings directly followed by text are recognised line by line', () => {
  assert.deepEqual(markdownBlocks('## Summary\nWe agreed.\n\n### Decisions\n- Ship it'), [
    { kind: 'heading', level: 2, spans: [{ kind: 'text', text: 'Summary' }] },
    { kind: 'paragraph', spans: [{ kind: 'text', text: 'We agreed.' }] },
    { kind: 'gap' },
    { kind: 'heading', level: 3, spans: [{ kind: 'text', text: 'Decisions' }] },
    { kind: 'bullet', spans: [{ kind: 'text', text: 'Ship it' }] },
  ]);
});

test('task items keep their state and every bullet marker is accepted', () => {
  assert.deepEqual(markdownBlocks('- [x] Done\n* [ ] Open\n• Plain').map(block => [block.kind, block.done]), [['task', true], ['task', false], ['bullet', undefined]]);
});

test('heading depths match the desktop mapping below the meeting title', () => {
  assert.deepEqual(contentHeadingLevels([1, 2, 2, 1, 3, 6]), [2, 3, 3, 2, 3, 4]);
  assert.deepEqual(contentHeadingLevels([3, 4, 5, 6, 6]), [2, 3, 4, 5, 5]);
});

test('blank runs collapse to one gap and leading or trailing blanks are dropped', () => {
  assert.deepEqual(markdownBlocks('\n\nOne\n\n\n\nTwo\n\n').map(block => block.kind), ['paragraph', 'gap', 'paragraph']);
  assert.deepEqual(markdownBlocks(''), []);
});
