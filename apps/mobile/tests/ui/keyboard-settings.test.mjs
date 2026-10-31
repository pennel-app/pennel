import assert from 'node:assert/strict';
import test from 'node:test';
import { defaultKeyboardSettings, typingLanguageChoice, typingLanguagePatch, snippetPatch, deleteSnippetPatch, validLearningWord, keyboardCopy } from '../../src/keyboard-settings.ts';

const snippet = (id, keyword = `;${id}`) => ({ id, title: id, keyword, text: `Hello ${id}` });

test('typing language changes make a narrow patch independent of interface and speech language', () => {
  assert.deepEqual(typingLanguagePatch('both'), { keyboard_typing_languages: ['nl', 'en'] });
  assert.deepEqual(typingLanguagePatch('en'), { keyboard_typing_languages: ['en'] });
  assert.equal(typingLanguageChoice(['en', 'nl']), 'both');
  assert.equal(typingLanguageChoice(['nl']), 'nl');
  assert.equal(defaultKeyboardSettings.keyboard_learning, false);
  assert.equal(defaultKeyboardSettings.keyboard_swipe, false);
});

test('saving a snippet merges into the latest snippet list without unrelated settings', () => {
  const before = [snippet('one'), snippet('other')];
  const result = snippetPatch(before, { ...snippet('one'), title: ' Updated ', text: 'Updated\ntext' }, true);
  assert.deepEqual(Object.keys(result.patch), ['keyboard_snippets']);
  assert.equal(result.patch.keyboard_snippets[0].title, 'Updated');
  assert.deepEqual(result.patch.keyboard_snippets[1], before[1]);
  assert.equal(before[0].text, 'Hello one');
  assert.deepEqual(deleteSnippetPatch(before, 'one'), { keyboard_snippets: [before[1]] });
});

test('deleted or duplicate snippet identities cannot silently overwrite or recreate an entry', () => {
  assert.equal(snippetPatch([snippet('one')], snippet('gone'), true).problem, 'snippetChanged');
  assert.equal(snippetPatch([snippet('one')], snippet('one'), false).problem, 'snippetChanged');
  assert.equal(snippetPatch([snippet('one')], snippet('two', ';one'), false).problem, 'shortcutExists');
  assert.equal(snippetPatch([snippet('one', '')], snippet('two', ''), false).problem, undefined);
});

test('snippet validation uses UTF8 byte bounds and preserves accents, emoji and multiline content', () => {
  assert.equal(snippetPatch([], { ...snippet('one'), text: 'café 👨‍👩‍👦\nHallo!\tTot morgen.' }, false).problem, undefined);
  assert.equal(snippetPatch([], { ...snippet('one'), text: '😀'.repeat(512) }, false).problem, undefined);
  assert.equal(snippetPatch([], { ...snippet('one'), text: '😀'.repeat(513) }, false).problem, 'textTooLong');
  assert.equal(snippetPatch([], { ...snippet('one'), keyword: 'has space' }, false).problem, 'shortcutInvalid');
  assert.equal(snippetPatch([], { ...snippet('one'), text: '\u0000secret' }, false).problem, 'textTooLong');
  assert.equal(snippetPatch(Array.from({ length: 64 }, (_, id) => snippet(String(id))), snippet('new'), false).problem, 'snippetLimit');
});

test('forget-word validation accepts natural vocabulary and avoids field-wide deletion input', () => {
  for (const word of ['Pennel', 'café', "'s-morgens", 'coöperatie']) assert.equal(validLearningWord(word), true);
  for (const word of ['', 'a', 'two words', 'name@example.com', '1234', 'x'.repeat(65)]) assert.equal(validLearningWord(word), false);
});

test('every keyboard and validation label is available in both interface languages', () => {
  assert.deepEqual(Object.keys(keyboardCopy.en).sort(), Object.keys(keyboardCopy.nl).sort());
  for (const labels of Object.values(keyboardCopy)) {
    for (const value of Object.values(labels)) assert.equal(typeof value === 'string' && value.length > 0, true);
  }
});
