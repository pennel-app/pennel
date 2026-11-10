import { selectMeetingPane } from './meeting-workspace-test-controls.mjs';
const assert = (value, message) => { if (!value) throw Error(message); };
const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
// The pinned view clears a composition view after 50ms; settle it before the next held-measure scenario.
async function settle() { await new Promise(resolve => setTimeout(resolve, 60)); await frame(); await frame(); }
async function until(check, message) { for (let i = 0; i < 200; i++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); } throw Error(message); }

/** Hold real RAF scheduling, including CodeMirror measurement, without accessing editor internals. */
async function holdMeasurement(action) {
  const raf = window.requestAnimationFrame, cancel = window.cancelAnimationFrame, held = new Map();
  let next = 1_000_000_000, scheduled = 0;
  window.requestAnimationFrame = callback => { const id = next++; scheduled++; held.set(id, callback); return id; };
  window.cancelAnimationFrame = id => { if (!held.delete(id)) cancel.call(window, id); };
  try { await action(); assert(scheduled, 'The regression did not queue any real pending measurement.'); }
  finally { window.requestAnimationFrame = raf; window.cancelAnimationFrame = cancel; for (const callback of held.values()) raf.call(window, callback); }
  await settle();
}
export async function checkRevealOwnership() {
  const { inlineNotesEditor } = await import('/src/inline-notes-editor.ts');
  const { l } = await import('/src/i18n.ts');
  const text = Array.from({ length: 180 }, (_, i) => `Synthetic retained line ${i}: exact source.`).join('\n');
  const shell = document.createElement('section'); shell.style.cssText = 'position:fixed;inset:20px;z-index:10000;padding:20px;background:var(--surface);overflow:auto';
  const editor = inlineNotesEditor({ value: text, label: l('meetings.your_notes') }); editor.style.cssText = 'height:260px;width:500px;max-width:100%';
  shell.append(editor); document.body.append(shell); editor.focus({ preventScroll: true });
  const content = editor.contentDOM, bookmark = text.indexOf('Synthetic retained line 35:'), deep = text.indexOf('Synthetic retained line 150:');
  let inputs = 0; editor.addEventListener('input', () => inputs++);
  try {
    editor.setSelectionRange(bookmark, bookmark + 12, 'backward'); editor.revealSelection(); await settle();
    const scroll = editor.scrollTop; assert(scroll > 400, 'The bookmark must retain a genuinely nonzero long scroll.');
    content.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    await holdMeasurement(async () => {
      editor.setSelectionRange(deep, deep + 12); editor.revealSelection();
      editor.setSelectionRange(bookmark, bookmark + 12, 'backward'); editor.scrollTop = scroll;
    });
    assert(Math.abs(editor.scrollTop - scroll) < 2, `Older explicit reveal replaced the restored bookmark: ${scroll}→${editor.scrollTop}.`);
    assert(editor.contentDOM === content && document.activeElement === content && editor.selectionStart === bookmark && editor.selectionEnd === bookmark + 12 && editor.selectionDirection === 'backward' && editor.composing && editor.value === text && inputs === 0, 'Reveal supersession changed canonical text, selection, real focus or composition.');
    content.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true })); await settle();
    await holdMeasurement(async () => {
      editor.setSelectionRange(deep, deep + 12); editor.revealSelection(); editor.scrollTop = scroll + 180;
    });
    assert(Math.abs(editor.scrollTop - (scroll + 180)) < 2, 'An older reveal overrode a newer facade scroll without a selection change.');
    const scrollEvents = []; editor.scrollDOM.addEventListener('scroll', () => scrollEvents.push(editor.scrollTop), true);
    await holdMeasurement(async () => {
      editor.revealSelection(); editor.scrollDOM.scrollTop = scroll + 240;
      await new Promise(resolve => setTimeout(resolve, 30));
    });
    assert(Math.abs(editor.scrollTop - (scroll + 240)) < 2, `An older reveal overrode a newer actual scroller event: expected=${scroll + 240}, actual=${editor.scrollTop}, events=${scrollEvents}.`);
    editor.revealSelection(); await settle();
    const line = [...editor.querySelectorAll('.cm-line')].find(node => node.textContent.startsWith('Synthetic retained line 150:')), bounds = line?.getBoundingClientRect(), viewport = editor.scrollDOM.getBoundingClientRect();
    assert(editor.scrollTop > scroll + 1000 && bounds && bounds.bottom > viewport.top && bounds.top < viewport.bottom, 'An ordinary current explicit source reveal stopped working.');
    editor.setSelectionRange(bookmark, bookmark); editor.contentDOM.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', code: 'ArrowRight', bubbles: true, cancelable: true })); await settle();
    assert(editor.selectionStart === bookmark + 1 && editor.scrollTop < scroll + 500, 'Normal user caret scrolling was suppressed.');
    return { bookmarkWins: true, newerFacadeScrollWins: true, actualScrollWins: true, normalReveal: true, caretScroll: true, actualFocusAndComposition: true };
  } finally { editor.destroy(); shell.remove(); }
}
export async function checkImmediateSourceReturn() {
  const fixture = window.__PENNEL_FIXTURE__; assert(fixture?.workspace, 'Use the owned synthetic meeting fixture.');
  await until(() => document.querySelector('.live-notes .cm-content'), 'The actual workspace editor did not open.');
  const root = document.querySelector('.live-workspace'), editor = root.querySelector('.live-notes'), doc = root.querySelector('.meeting-document');
  const text = Array.from({ length: 180 }, (_, i) => `Owned source line ${i}: exact note evidence.`).join('\n');
  editor.applyUserChange(text); await import('/src/views/meetings.ts').then(module => module.flushMeetingDrafts());
  selectMeetingPane(root, 'questions');
  const question = root.querySelector('.live-question-input'); question.value = 'Which deep note supports this answer?'; question.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' })); root.querySelector('.live-question-submit').click();
  await until(() => fixture.workspace.requests().length === 1, 'The actual question source snapshot did not start.');
  const snapshot = fixture.workspace.requests()[0], index = snapshot.anchors.findIndex(anchor => anchor.passage.kind === 'notes' && anchor.source_text.startsWith('Owned source line 150:'));
  assert(index >= 0, 'The actual frozen note snapshot has no deep source.');
  await fixture.conversations.complete(snapshot.request_id, { text: `The deep note [${index + 1}]`, citations: [{ number: index + 1, passage: snapshot.anchors[index].passage }] });
  await until(() => root.querySelector('.citation'), 'The answer has no actual citation.');
  const bookmark = text.indexOf('Owned source line 35:'), deep = text.indexOf('Owned source line 150:');
  editor.setSelectionRange(bookmark, bookmark + 12, 'backward'); editor.revealSelection(); await settle();
  const scrollSurface = root.classList.contains('meeting-finished-document') ? doc : editor;
  const scroll = scrollSurface.scrollTop, content = editor.contentDOM; assert(scroll > 400, 'Source-return bookmark must retain a genuinely nonzero long scroll.');
  await holdMeasurement(async () => {
    const citation = root.querySelector('.citation'); citation.focus({ preventScroll: true }); citation.click();
    await until(() => editor.selectionStart === deep, 'The real citation did not queue the deep note reveal.');
    root.querySelector('.live-source-back').click();
    assert(editor.selectionStart === bookmark && editor.selectionDirection === 'backward', 'Answer return did not restore the canonical selection immediately.');
  });
  assert(Math.abs(scrollSurface.scrollTop - scroll) < 2, `Older source reveal recentered the returned reading position: ${scroll}→${scrollSurface.scrollTop}.`);
  assert(editor.contentDOM === content && editor.value === text && editor.selectionStart === bookmark && editor.selectionEnd === bookmark + 12 && editor.selectionDirection === 'backward' && !root.querySelector('.live-questions-pane').hidden, 'Returning before source measurement changed the retained editor or answer context.');
  return { actualCitation: true, immediateAnswerReturn: true, readingBookmarkWins: true, retainedSourceSelection: true };
}
