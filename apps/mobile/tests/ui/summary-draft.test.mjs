import assert from 'node:assert/strict';
import test from 'node:test';
import { SummaryDraft } from '../../src/summary-draft.ts';

const document = (patch = {}) => ({ meeting_uid: 'owned-meeting', markdown: 'Saved summary', revision: 'summary-1', source_revision: 'source-1', edited: false, outdated: false, ...patch });

test('editing preserves exact text and passes the captured summary and source versions to storage', async () => {
  const writes = [];
  const model = new SummaryDraft(async (base, markdown) => { writes.push({ base, markdown }); return document({ markdown, revision: 'summary-2', edited: true }); }, async () => { throw new Error('Ordinary edits cannot accept generated content'); });
  model.adopt(document());
  model.edit('My summary\n\n- Keep this line\n');
  assert.equal(await model.save(), true);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].base.revision, 'summary-1'); assert.equal(writes[0].base.source_revision, 'source-1');
  assert.equal(writes[0].markdown, 'My summary\n\n- Keep this line\n');
  assert.equal(model.getSnapshot().dirty, false);
});

test('a source-only change cannot silently advance a dirty summary editor', async () => {
  let writes = 0;
  const model = new SummaryDraft(async () => { writes++; return document(); }, async () => document());
  model.adopt(document()); model.edit('My retained draft');
  model.adopt(document({ source_revision: 'changed-transcript', outdated: true }));
  assert.equal(model.getSnapshot().conflict, true);
  assert.equal(model.getSnapshot().document.source_revision, 'source-1');
  assert.equal(await model.save(), false); assert.equal(writes, 0);
  assert.equal(model.getSnapshot().draft, 'My retained draft');
  model.discard();
  assert.equal(model.getSnapshot().document.source_revision, 'changed-transcript');
  assert.equal(model.getSnapshot().dirty, false);
});

test('edits typed during a save queue against the newly saved version', async () => {
  const writes = []; let finish;
  const first = new Promise(resolve => { finish = resolve; });
  const model = new SummaryDraft(async (base, markdown) => { writes.push({ revision: base.revision, source: base.source_revision, markdown }); return writes.length === 1 ? first : document({ markdown, revision: 'summary-3' }); }, async () => document());
  model.adopt(document()); model.edit('First edit'); const saving = model.save();
  model.edit('Latest edit'); finish(document({ markdown: 'First edit', revision: 'summary-2' }));
  assert.equal(await saving, true);
  assert.deepEqual(writes, [
    { revision: 'summary-1', source: 'source-1', markdown: 'First edit' },
    { revision: 'summary-2', source: 'source-1', markdown: 'Latest edit' },
  ]);
  assert.equal(model.getSnapshot().draft, 'Latest edit'); assert.equal(model.getSnapshot().dirty, false);
});

test('a failed save preserves the draft and never installs a remote version over it', async () => {
  const model = new SummaryDraft(async () => undefined, async () => document());
  model.adopt(document()); model.edit('Keep this private edit');
  assert.equal(await model.save(), false);
  assert.equal(model.getSnapshot().failed, true);
  model.adopt(document({ revision: 'another-summary', markdown: 'Other saved content' }));
  assert.equal(model.getSnapshot().draft, 'Keep this private edit');
  assert.equal(model.getSnapshot().conflict, true);
});

test('a reviewed replacement uses explicit acceptance and stale candidates leave saved content untouched', async () => {
  const accepted = [];
  const model = new SummaryDraft(async () => { throw new Error('Generation acceptance must not masquerade as an edit'); }, async (base, markdown) => { accepted.push({ base, markdown }); return document({ markdown, revision: 'accepted', edited: false }); });
  model.adopt(document());
  const candidate = { meeting_uid: 'owned-meeting', markdown: 'Proposed replacement', revision: 'summary-1', source_revision: 'source-1' };
  for (const patch of [{ revision: 'older' }, { source_revision: 'older-source' }, { meeting_uid: 'another-meeting' }]) assert.equal(await model.accept({ ...candidate, ...patch }), false);
  assert.equal(accepted.length, 0); assert.equal(model.getSnapshot().draft, 'Saved summary');
  assert.equal(await model.accept(candidate), true);
  assert.equal(accepted.length, 1); assert.equal(model.getSnapshot().draft, candidate.markdown);
  assert.equal(model.getSnapshot().dirty, false);
});

test('an edit to an outdated summary retains the core response instead of claiming new coverage', async () => {
  const model = new SummaryDraft(async (_base, markdown) => document({ markdown, revision: 'edited-old', outdated: true, edited: true }), async () => document());
  model.adopt(document({ outdated: true })); model.edit('Corrected wording');
  assert.equal(await model.save(), true);
  assert.equal(model.getSnapshot().document.outdated, true);
});

test('a fresher remote version arriving during a save remains current without losing later local edits', async () => {
  let finish;
  const pending = new Promise(resolve => { finish = resolve; });
  const model = new SummaryDraft(async () => pending, async () => document());
  model.adopt(document()); model.edit('Written edit'); const saving = model.save();
  const current = document({ revision: 'remote-newer', source_revision: 'source-2', markdown: 'Fresh content' });
  model.adopt(current); model.edit('Still typing');
  finish(document({ revision: 'local-save', markdown: 'Written edit' }));
  assert.equal(await saving, false);
  assert.equal(model.getSnapshot().draft, 'Still typing');
  assert.equal(model.getSnapshot().conflict, true);
  model.discard();
  assert.equal(model.getSnapshot().draft, 'Fresh content');
  assert.equal(model.getSnapshot().document.revision, 'remote-newer');
});

test('reviewing an outdated personal summary accepts its unchanged text and exact source without a provider or generation', async () => {
  const accepted = [];
  const model = new SummaryDraft(async () => { throw new Error('Review must not be an ordinary edit'); }, async (base, markdown) => {
    accepted.push({ revision: base.revision, source_revision: base.source_revision, markdown });
    return document({ markdown, revision: 'reviewed-personal', outdated: false });
  });
  const saved = document({ markdown: 'Owner-only summary\n\nPrivate context stays personal.\n', outdated: true });
  model.adopt(saved);
  assert.equal(model.canMarkReviewed(), true);
  assert.equal(await model.markReviewed(), true);
  assert.deepEqual(accepted, [{ revision: saved.revision, source_revision: saved.source_revision, markdown: saved.markdown }]);
  assert.equal(model.getSnapshot().document.outdated, false);
  assert.equal(model.getSnapshot().draft, saved.markdown);
  assert.equal(model.canMarkReviewed(), false);
});

test('review cannot replace dirty, conflicting, blank, or already current summaries', async () => {
  let accepted = 0;
  const model = new SummaryDraft(async () => undefined, async () => { accepted++; return document(); });
  for (const saved of [document(), document({ outdated: true, markdown: ' \n' })]) {
    model.adopt(saved); assert.equal(await model.markReviewed(), false);
  }
  model.adopt(document({ outdated: true })); model.edit('Unsaved wording');
  assert.equal(await model.markReviewed(), false);
  model.adopt(document({ outdated: true, source_revision: 'changed-source' }));
  assert.equal(model.getSnapshot().conflict, true); assert.equal(await model.markReviewed(), false);
  assert.equal(accepted, 0);
});

test('review is single-flight and a rejected source check preserves the personal summary and outdated warning', async () => {
  let finish; let accepted = 0;
  const pending = new Promise(resolve => { finish = resolve; });
  const model = new SummaryDraft(async () => undefined, async () => { accepted++; return pending; });
  model.adopt(document({ outdated: true }));
  const reviewing = model.markReviewed();
  assert.equal(model.canMarkReviewed(), false); assert.equal(await model.markReviewed(), false);
  finish(undefined); assert.equal(await reviewing, false);
  assert.equal(accepted, 1);
  assert.equal(model.getSnapshot().draft, 'Saved summary');
  assert.equal(model.getSnapshot().document.outdated, true);
  assert.equal(model.getSnapshot().dirty, false);
});
