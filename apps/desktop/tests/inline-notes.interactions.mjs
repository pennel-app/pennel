const assert = (value, message) => { if (!value) throw Error(message); };
const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
export async function prepareInlineNotes() {
  const { inlineNotesEditor } = await import('/src/inline-notes-editor.ts');
  const { l } = await import('/src/i18n.ts');
  const shell = document.createElement('section');
  shell.style.cssText = 'position:fixed;inset:20px;z-index:10000;padding:20px;background:var(--surface);overflow:auto';
  const editor = inlineNotesEditor({ value: '# Heading\nA **bold** word 😀\n- bullet\n- [ ] task\n- [x] done\n`**literal**`', label: l('meetings.your_notes'), placeholder: l('meetings.workspace_placeholder') });
  editor.style.cssText = 'height:260px;width:500px;max-width:100%';
  shell.append(editor); document.body.append(shell);
  const input = [], select = [];
  editor.addEventListener('input', event => input.push({ value: editor.value, type: event.inputType, composing: event.isComposing }));
  editor.addEventListener('select', () => select.push([editor.selectionStart, editor.selectionEnd]));
  window.__T4_NOTES__ = { editor, input, select, shell };
  await frame();
  assert(!editor.querySelector('textarea') && editor.contentDOM.contentEditable === 'true', 'Notes must use the actual editable CodeMirror contentDOM.');
  assert(editor.querySelector('.cm-note-strong')?.textContent === 'bold', 'Inactive bold did not resolve inline.');
  editor.setSelectionRange(0, 0); editor.focus({ preventScroll: true });
  await frame();
  assert(document.activeElement === editor.contentDOM && editor.contentDOM.querySelector('.cm-line').textContent === '# Heading', 'Focused caret line must reveal raw Markdown markers.');
  editor.value = 'Before 😀 words after'; editor.setSelectionRange(7, 15, 'backward');
  assert(input.length === 0 && editor.selectionStart === 7 && editor.selectionEnd === 15 && editor.selectionDirection === 'backward', 'Quiet hydration emitted input or lost the canonical UTF-16 selection.');
  editor.applyFormat('bold');
  assert(editor.value === 'Before **😀 words** after' && input.length === 1, 'Toolbar formatting did not make exactly one user edit.');
  assert(editor.selectionDirection === 'backward', 'Formatting lost the backward selection.');
  const beforeUndo = input.length;
  editor.contentDOM.dispatchEvent(new InputEvent('beforeinput', { inputType: 'historyUndo', bubbles: true, cancelable: true }));
  assert(editor.value === 'Before 😀 words after' && input.length === beforeUndo + 1 && input.at(-1).type === 'historyUndo', 'Native beforeinput historyUndo did not undo the toolbar edit exactly once.');
  editor.contentDOM.dispatchEvent(new InputEvent('beforeinput', { inputType: 'historyRedo', bubbles: true, cancelable: true }));
  assert(editor.value === 'Before **😀 words** after' && input.at(-1).type === 'historyRedo', 'Native beforeinput historyRedo did not restore the toolbar edit.');
  editor.value = 'Type here'; editor.setSelectionRange(9, 9); editor.focus({ preventScroll: true });
  window.__T4_NOTES__.nativeStart = input.length;
}
export async function checkInlineNotesTyping() {
  const { editor, input, nativeStart } = window.__T4_NOTES__;
  assert(editor.value === 'Type here 🦉' && input.length === nativeStart + 1, 'Actual CDP text input did not produce one canonical user transaction.');
  const count = input.length;
  editor.value = Array.from({ length: 100 }, (_, i) => `Line ${i} **bold**`).join('\n');
  editor.setSelectionRange(20, 25, 'backward'); editor.scrollTop = 180;
  await frame(); const scroll = editor.scrollTop;
  assert(scroll > 100, 'A deferred programmatic selection snapshot overrode the newer actual scroll.');
  editor.value += '\nquiet remote tail';
  await frame();
  assert(input.length === count && editor.selectionStart === 20 && editor.selectionEnd === 25 && editor.selectionDirection === 'backward' && Math.abs(editor.scrollTop - scroll) < 2, 'Quiet assignment changed input count, selection or actual scroll position.');
  assert(document.activeElement === editor.contentDOM, 'Quiet assignment stole actual content focus.');
  editor.value = '- [ ] task\n- [x] done'; editor.blur(); await frame();
  const task = editor.querySelector('input[type=checkbox]');
  assert(task.getAttribute('aria-label') === 'task', 'Inline task control is missing the actual task text as its accessible name.');
  task.focus(); task.click();
  assert(editor.value === '- [x] task\n- [x] done' && input.length === count + 1, 'Task checkbox did not update canonical Markdown exactly once.');
  assert(task.isConnected && document.activeElement === task, 'Task toggle replaced its focused native control.');
  assert(editor.undo() && editor.value === '- [ ] task\n- [x] done', 'Task checkbox edit was not undoable.');
  editor.readOnly = true;
  assert(editor.contentDOM.contentEditable === 'false' && !editor.applyFormat('bold') && !editor.applyUserChange('forbidden'), 'Readonly surface accepted a user mutation.');
  assert([...editor.querySelectorAll('input')].every(node => node.disabled), 'Readonly tasks remain writable.');
  editor.readOnly = false; editor.disabled = true; editor.blur(); editor.focus();
  assert(document.activeElement !== editor.contentDOM && editor.contentDOM.tabIndex === -1, 'Disabled surface still claims editable keyboard focus.');
  editor.disabled = false; editor.value = 'composition'; editor.setSelectionRange(11, 11); editor.focus({ preventScroll: true });
  const composition = [];
  editor.addEventListener('compositionstart', () => composition.push('start'));
  editor.addEventListener('compositionend', () => composition.push('end'));
  editor.contentDOM.dispatchEvent(new CompositionEvent('compositionstart', { data: '', bubbles: true }));
  assert(editor.composing && !editor.applyFormat('heading'), 'Composition did not block toolbar edits.');
  editor.contentDOM.dispatchEvent(new CompositionEvent('compositionend', { data: 'é', bubbles: true }));
  await new Promise(resolve => setTimeout(resolve, 30));
  assert(!editor.composing && composition.join(',') === 'start,end', 'Composition lifecycle was duplicated or not released.');
  const { setInterfaceLanguage, t } = await import('/src/i18n.ts');
  const content = editor.contentDOM;
  const language = document.documentElement.lang;
  setInterfaceLanguage(language === 'nl' ? 'en' : 'nl');
  assert(editor.contentDOM === content && content.getAttribute('aria-label') === t('meetings.your_notes') && editor.value === 'composition', 'Localization replaced the real editor or retained its stale accessible name.');
  setInterfaceLanguage(language);
  editor.setDocumentMode(true);
  assert(getComputedStyle(editor.scrollDOM).overflowY === 'visible', 'Finished document mode still owns a competing inner scroller.');
  window.__T4_NOTES__.result = { canonical: editor.value, transactions: input.length, actualContentFocus: document.activeElement === content, quietSelectionAndScroll: true, undoRedo: true, task: true, compositionLifecycle: true, localization: true };
  return window.__T4_NOTES__.result;
}
async function until(check, message) { for (let i = 0; i < 200; i++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); } throw Error(message); }
export async function checkInlineNotesWorkspace() {
  const fixture = window.__PENNEL_FIXTURE__;
  assert(fixture, 'Use the owned synthetic fixture.');
  await until(() => document.querySelector('.live-notes'), 'Workspace notes did not open.');
  const editor = document.querySelector('.live-notes'), root = editor.closest('.live-workspace');
  assert(editor.contentDOM && editor.querySelector('.cm-content') === editor.contentDOM && !editor.querySelector('textarea'), 'Workspace still uses a textarea instead of the actual inline Markdown editor.');
  assert(!root.querySelector('.live-preview-toggle') && root.querySelectorAll('.live-save-controls [data-live-format]').length === 4, 'Notes footer did not replace the Markdown/Preview mode controls.');
  const client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client), writes = [], events = [];
  client.invoke = async (command, args = {}) => { if (['set_meeting_notes', 'save_meeting_notes'].includes(command)) writes.push({ command, args }); return original(command, args); };
  editor.addEventListener('input', event => events.push(event.inputType));
  const flush = async () => import('/src/views/meetings.ts').then(module => module.flushMeetingDrafts());
  try {
    const source = '# Planning\nBefore 😀 words after\n- [ ] Check the source\n- [x] Confirmed\n`**literal**`';
    editor.applyUserChange(source); await flush();
    assert(fixture.workspace.notes() === source, 'Stored notes differ from the canonical Markdown source.');
    editor.focus({ preventScroll: true }); editor.setSelectionRange(source.indexOf('😀'), source.indexOf(' after'), 'backward');
    const before = writes.length, inputs = events.length;
    root.querySelector('[data-live-format="bold"]').click();
    await until(() => fixture.workspace.notes() === source.replace('😀 words', '**😀 words**'), 'Formatting did not autosave canonical Markdown.');
    await new Promise(resolve => setTimeout(resolve, 80));
    assert(writes.length === before + 1 && events.length === inputs + 1 && document.activeElement === editor.contentDOM, 'Formatting duplicated autosave/input or lost real editor focus.');
    editor.contentDOM.dispatchEvent(new InputEvent('beforeinput', { inputType: 'historyUndo', bubbles: true, cancelable: true }));
    await flush(); assert(fixture.workspace.notes() === source, 'Toolbar edit did not undo through the real autosave path.');
    editor.contentDOM.dispatchEvent(new InputEvent('beforeinput', { inputType: 'historyRedo', bubbles: true, cancelable: true }));
    await flush(); assert(fixture.workspace.notes().includes('**😀 words**'), 'Redo did not persist the formatter transaction.');
    editor.setSelectionRange(0, 0); await frame();
    const task = editor.querySelector('input[type=checkbox]'); task.click(); await flush();
    assert(fixture.workspace.notes().includes('- [x] Check the source'), 'Inline task edit did not autosave its exact Markdown marker.');
    const content = editor.contentDOM, text = editor.value;
    editor.setSelectionRange(3, 8, 'backward'); editor.focus({ preventScroll: true });
    fixture.emit('settings-changed', null); await frame();
    assert(document.querySelector('.live-notes') === editor && editor.contentDOM === content && document.activeElement === content && editor.value === text && editor.selectionDirection === 'backward', 'Info refresh remounted the editor or lost editing context.');
    await original('stop_meeting');
    await until(() => root.classList.contains('meeting-finished-document'), 'Recording did not finalize in the same workspace.');
    assert(root.querySelector('.live-notes') === editor && editor.contentDOM === content && document.activeElement === content && editor.value === text, 'Finalization replaced the actual editor or changed its source/focus.');
    assert(editor.dataset.documentMode === 'true', 'Finished notes retained a competing live scroller.');
    const { api } = await import('/src/api.ts');
    await api.setShortcut('find', { mods: ['meta', 'shift'], code: 'KeyZ', key: 'z' });
    try {
      await frame();
      editor.contentDOM.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', code: 'KeyZ', metaKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await until(() => document.activeElement === document.querySelector('.library-search input'), 'The real application shortcut dispatcher did not receive a shortcut shared with editor redo.');
      assert(editor.value === text, 'Editor redo stole an effective app shortcut.');
    } finally { await api.resetShortcut('find'); }
    return { actualEditor: true, canonicalStored: true, oneFormattingAutosave: true, undoRedo: true, taskAutosave: true, retainedFinalization: true };
  } finally { client.invoke = original; }
}
export async function showInlineNotesReference() {
  await until(() => document.querySelector('.live-notes'), 'Notes did not open.');
  const editor = document.querySelector('.live-notes');
  const value = document.documentElement.lang === 'nl'
    ? '# Projectnotities\n\n## Besluiten\nWe bespreken **de planning** en spreken een volgende stap af.\n- Eerst de bron controleren\n- [ ] Voorstel delen met Sam\n- [x] Scope bevestigd\n\n### Volgende stap\nDe notities blijven privé en worden als Markdown bewaard.'
    : '# Project notes\n\n## Decisions\nWe discuss **the schedule** and agree on a next step.\n- Check the source first\n- [ ] Share the proposal with Sam\n- [x] Scope confirmed\n\n### Next step\nThe notes stay private and are stored as Markdown.';
  if (editor.applyUserChange) editor.applyUserChange(value); else { editor.value = value; editor.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' })); }
  await import('/src/views/meetings.ts').then(module => module.flushMeetingDrafts());
  editor.blur();
  const doc = editor.closest('.meeting-document');
  if (doc && editor.closest('.meeting-finished-document')) doc.scrollTop += editor.closest('.live-notes-pane').getBoundingClientRect().top - doc.getBoundingClientRect().top;
  await frame();
}
export async function checkDeepNotesFinalization() {
  await until(() => document.querySelector('.live-notes .cm-content'), 'Actual notes editor did not open.');
  const editor = document.querySelector('.live-notes'), root = editor.closest('.live-workspace'), doc = root.querySelector('.meeting-document');
  const text = Array.from({ length: 120 }, (_, i) => `Synthetic line ${i}: retained source and composition.`).join('\n'), start = text.indexOf('Synthetic line 70:');
  editor.applyUserChange(text); editor.focus({ preventScroll: true }); editor.setSelectionRange(start, start + 17, 'backward'); editor.revealSelection();
  await frame(); await new Promise(resolve => setTimeout(resolve, 60));
  const selectedLine = () => [...editor.querySelectorAll('.cm-line')].find(node => node.textContent.startsWith('Synthetic line 70:'));
  assert(editor.scrollTop > 500 && selectedLine(), 'Deep live source did not use the actual editor scroller.');
  const value = editor.value, content = editor.contentDOM;
  content.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
  await window.__PENNEL_PLATFORM__.invoke('stop_meeting');
  await until(() => root.classList.contains('meeting-finished-document'), 'Deep notes did not finalize.');
  await frame(); await new Promise(resolve => setTimeout(resolve, 60));
  const bounds = selectedLine()?.getBoundingClientRect(), viewport = doc.getBoundingClientRect();
  assert(editor.contentDOM === content && document.activeElement === content && editor.value === value && editor.selectionStart === start && editor.selectionEnd === start + 17 && editor.selectionDirection === 'backward', 'Finalization changed deep note text, actual focus or canonical source selection.');
  assert(bounds && bounds.bottom > viewport.top && bounds.top < viewport.bottom, `Finalization lost the deep reading position: ${JSON.stringify({ bounds: bounds?.toJSON(), viewport: viewport.toJSON(), documentScroll: doc.scrollTop, innerScroll: editor.scrollTop })}`);
  content.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true }));
  return { sameActualEditor: true, deepReadingRetained: true, compositionSelectionRetained: true };
}
