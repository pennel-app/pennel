import assert from 'node:assert/strict';
import test from 'node:test';
import { ConversationController } from '../../../assets/contracts/meeting-conversation-controller.ts';
import { assertConversationRequest, conversationRequests } from '../../../assets/contracts/meeting-conversations.ts';

const scope = { kind: 'meeting', meeting_uid: 'uid' };
const summary = (id, draft = '', revision = 0) => ({ id, title: '', draft, draft_revision: revision, created_at_ms: 1, updated_at_ms: 1, active_request_id: null });
const snapshot = (id = 'one', draft = '', revision = 0, messages = []) => ({ schema_version: 1, scope, storage_status: 'saved', storage_error: null,
  selected_conversation_id: id, current_meeting_id: 'folder', conversations: [summary(id, draft, revision)], conversation: summary(id, draft, revision),
  messages, next_messages_cursor: null, next_conversations_cursor: null });
function deferred() { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }
function service(changes = {}) { return { snapshot: async () => snapshot(), update: async request => snapshot(request.conversation_id ?? 'created', request.draft ?? '', (request.expected_draft_revision ?? 0) + 1),
  begin: async () => snapshot(), cancel: async () => snapshot(), source: async () => { throw Error('no source'); }, ...changes }; }

test('draft writes capture their conversation and use the acknowledged core revision', async () => {
  const requests = [];
  const model = new ConversationController(scope, service({ update: async request => { assertConversationRequest('update', request); requests.push(request); return snapshot(request.conversation_id, request.draft, request.expected_draft_revision + 1); } }));
  await model.load(); model.editDraft('First question'); await model.flush();
  model.editDraft('Latest question'); await model.flush();
  assert.deepEqual(requests.map(request => [request.conversation_id, request.draft, request.expected_draft_revision]), [['one', 'First question', 0], ['one', 'Latest question', 1]]);
  assert.equal(model.draft, 'Latest question');
});

test('a slow old-conversation read cannot replace the deliberately selected conversation', async () => {
  const waiting = deferred(); let reads = 0;
  const model = new ConversationController(scope, service({ snapshot: async () => ++reads === 1 ? snapshot() : waiting.promise,
    update: async request => snapshot(request.conversation_id) }));
  await model.load(); const oldRead = model.refresh(); await model.select('two'); waiting.resolve(snapshot('one')); await oldRead;
  assert.equal(model.conversationId, 'two'); assert.equal(model.snapshot.conversation.id, 'two');
});

test('question admission stays with its original conversation when selection changes', async () => {
  const waiting = deferred(); let admitted;
  const model = new ConversationController(scope, service({ begin: async request => { admitted = request; return waiting.promise; }, update: async request => snapshot(request.conversation_id) }));
  await model.load(); const asking = model.ask('Original question', 'request-one');
  await new Promise(resolve => setImmediate(resolve));
  const selecting = model.select('two');
  waiting.resolve({ ...snapshot('one'), conversation: { ...summary('one'), active_request_id: 'request-one' } });
  assert.equal(await asking, true); await selecting;
  assert.equal(admitted.conversation_id, 'one'); assert.equal(model.snapshot.conversation.id, 'two');
});

test('typing during a slow draft save retains the new text and advances the draft CAS revision', async () => {
  const waiting = deferred(); const requests = [];
  const model = new ConversationController(scope, service({ update: async request => { requests.push(request); if (requests.length === 1) return waiting.promise; return snapshot('one', request.draft, 2); } }));
  await model.load(); model.editDraft('First'); await new Promise(resolve => setImmediate(resolve)); model.editDraft('Typed later');
  waiting.resolve(snapshot('one', 'First', 1)); await model.flush();
  assert.equal(model.draft, 'Typed later'); assert.equal(requests[1].expected_draft_revision, 1); assert.equal(model.dirty, false);
});

test('a read started before a draft acknowledgement cannot erase the saved composer text', async () => {
  const waiting = deferred(); let reads = 0;
  const model = new ConversationController(scope, service({ snapshot: async () => ++reads === 1 ? snapshot() : waiting.promise }));
  await model.load(); const old = model.refresh(); model.editDraft('Keep this draft'); await model.flush();
  waiting.resolve(snapshot('one', '', 0)); await old;
  assert.equal(model.draft, 'Keep this draft');
});

test('cancellation preserves the submitted question without replacing a newer draft', async () => {
  const messages = [{ id: 'q', conversation_id: 'one', request_id: 'r', role: 'user', text: 'Original?', status: 'completed', created_at_ms: 1, reply_to: null },
    { id: 'a', conversation_id: 'one', request_id: 'r', role: 'assistant', text: '', status: 'pending', created_at_ms: 2, reply_to: 'q' }];
  const active = { ...snapshot('one', '', 0, messages), conversation: { ...summary('one'), active_request_id: 'r' } };
  const cancelled = () => ({ ...snapshot('one', '', 0, messages.map(message => message.id === 'a' ? { ...message, status: 'cancelled' } : message)) });
  const model = new ConversationController(scope, service({ snapshot: async () => active, cancel: async () => cancelled() }));
  await model.load(); await model.cancel(); await model.flush(); assert.equal(model.draft, 'Original?');
  await model.load(); model.editDraft('Newer question'); await model.cancel(); await model.flush(); assert.equal(model.draft, 'Newer question');
});

test('a failed draft save retains text through refresh and an explicit retry without duplicate writes', async () => {
  let failed = true, writes = 0;
  const model = new ConversationController(scope, service({ update: async request => {
    if (request.action !== 'draft') return snapshot();
    writes++; if (failed) throw Error('storage unavailable');
    return snapshot('one', request.draft, request.expected_draft_revision + 1);
  } }));
  await model.load(); model.editDraft('Retain this'); await assert.rejects(model.flush()); await model.refresh();
  assert.equal(model.draft, 'Retain this'); assert.match(model.error, /storage unavailable/);
  failed = false; await model.retrySave(); assert.equal(model.draft, 'Retain this'); assert.equal(model.dirty, false); assert.equal(writes, 2);
});

test('old pages merge once and a focused answer uses one bounded request', async () => {
  const calls = [];
  const message = (id, time) => ({ id, conversation_id: 'one', request_id: id, role: 'assistant', status: 'completed', text: id, created_at_ms: time });
  const model = new ConversationController(scope, service({ snapshot: async request => {
    assertConversationRequest('snapshot', request); calls.push(request);
    if (request.focus_message_id) return snapshot('one', '', 0, [message('old-answer', 1)]);
    return { ...snapshot('one', '', 0, request.messages_before ? [message('old-answer', 1), message('recent', 2)] : [message('recent', 2)]), next_messages_cursor: request.messages_before ? null : 'cursor' };
  } }));
  await model.load(); await model.loadOlder(); await model.refresh();
  assert.deepEqual(model.snapshot.messages.map(message => message.id), ['old-answer', 'recent']); assert.equal(model.snapshot.next_messages_cursor, null);
  await model.focus('old-answer'); assert.equal(calls.at(-1).focus_message_id, 'old-answer'); assert.deepEqual(model.snapshot.messages.map(message => message.id), ['old-answer']);
});

test('message ordering follows the core page even when timestamps collide', async () => {
  const messages = [{ id: 'z-user', created_at_ms: 1, status: 'completed', role: 'user' }, { id: 'a-answer', created_at_ms: 1, status: 'completed', role: 'assistant' }];
  const model = new ConversationController(scope, service({ snapshot: async () => snapshot('one', '', 0, messages) }));
  await model.load(); assert.deepEqual(model.snapshot.messages.map(message => message.id), ['z-user', 'a-answer']);
});

test('the shared wire requires draft CAS and target IDs for both platform callers', () => {
  const draft = conversationRequests.draft(scope, 'one', 'Exact draft', 3);
  assert.deepEqual(draft, { scope, action: 'draft', conversation_id: 'one', draft: 'Exact draft', expected_draft_revision: 3 });
  assert.doesNotThrow(() => assertConversationRequest('update', draft));
  assert.throws(() => assertConversationRequest('update', { ...draft, expected_draft_revision: undefined }));
  assert.throws(() => assertConversationRequest('source', { scope, message_id: 'a', citation_number: 1 }));
  assert.doesNotThrow(() => assertConversationRequest('snapshot', conversationRequests.snapshot(scope, 'one', { focus_message_id: 'old-answer' })));
});

test('typing during note preparation saves the newer draft after admission clears the submitted draft', async () => {
  const notes = deferred(); let remoteDraft = 'Submitted', revision = 1, prepared = false, began = false;
  const model = new ConversationController(scope, service({ snapshot: async () => snapshot('one', remoteDraft, revision),
    update: async request => { remoteDraft = request.draft; return snapshot('one', remoteDraft, ++revision); },
    begin: async () => { began = true; remoteDraft = ''; return snapshot('one', remoteDraft, ++revision); } }));
  await model.load(); const asking = model.ask('Submitted', 'request', async () => { prepared = true; await notes.promise; return {}; });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(prepared, true); assert.equal(began, false);
  model.editDraft('Typed while notes save'); notes.resolve();
  assert.equal(await asking, true); await model.flush();
  assert.equal(remoteDraft, 'Typed while notes save'); assert.equal(model.draft, 'Typed while notes save');
});

test('cancelling while notes are being prepared retains the draft without dispatching inference', async () => {
  const notes = deferred(); let begins = 0;
  const model = new ConversationController(scope, service({ snapshot: async () => snapshot('one', 'Retained question'), begin: async () => { begins++; return snapshot(); } }));
  await model.load(); const asking = model.ask('Retained question', 'cancel-before-send', async () => { await notes.promise; return {}; });
  await new Promise(resolve => setImmediate(resolve)); await model.cancel(); notes.resolve();
  assert.equal(await asking, false); assert.equal(begins, 0); assert.equal(model.draft, 'Retained question'); assert.equal(model.busy, false);
});

test('cancellation reaches the core before a delayed begin response and rejects its stale acknowledgement', async () => {
  const begin = deferred(); let cancelled = false;
  const messages = [{ id: 'q', conversation_id: 'one', request_id: 'slow', role: 'user', text: 'Original question', status: 'completed', created_at_ms: 1, reply_to: null },
    { id: 'a', conversation_id: 'one', request_id: 'slow', role: 'assistant', text: '', status: 'cancelled', created_at_ms: 2, reply_to: 'q' }];
  const model = new ConversationController(scope, service({ snapshot: async () => snapshot('one', 'Original question'), begin: async () => begin.promise,
    cancel: async request => { cancelled = true; assert.equal(request.request_id, 'slow'); return snapshot('one', '', 1, messages); } }));
  await model.load(); const asking = model.ask('Original question', 'slow'); await new Promise(resolve => setImmediate(resolve));
  const cancelling = model.cancel(); await new Promise(resolve => setImmediate(resolve)); assert.equal(cancelled, true); await cancelling;
  begin.resolve({ ...snapshot('one', '', 1, messages.map(message => message.id === 'a' ? { ...message, status: 'pending' } : message)), conversation: { ...summary('one', '', 1), active_request_id: 'slow' } });
  await asking; await model.flush(); assert.equal(model.snapshot.messages.find(message => message.id === 'a').status, 'cancelled'); assert.equal(model.draft, 'Original question');
});

test('a conversation stays busy until independent cancellation resolves even if begin responds first', async () => {
  const begin = deferred(), cancel = deferred();
  const model = new ConversationController(scope, service({ snapshot: async () => snapshot('one', 'Submitted'), begin: async () => begin.promise, cancel: async () => cancel.promise }));
  await model.load(); const asking = model.ask('Submitted', 'reverse'); await new Promise(resolve => setImmediate(resolve));
  const cancelling = model.cancel(); model.editDraft('Newer draft');
  begin.resolve(snapshot('one', '', 1)); await asking; await model.flush();
  assert.equal(model.busy, true); assert.equal(model.draft, 'Newer draft');
  cancel.resolve(snapshot('one', 'Newer draft', 2)); await cancelling;
  assert.equal(model.busy, false); assert.equal(model.draft, 'Newer draft');
});

test('concurrent initial loads share legacy identity normalization instead of discarding its canonical result', async () => {
  const first = deferred(); let reads = 0;
  const legacy = { kind: 'meeting', meeting_uid: 'legacy-folder', meeting_id: 'old-folder' };
  const model = new ConversationController(legacy, service({ snapshot: async () => { if (++reads > 1) throw Error('legacy identity no longer matches'); return first.promise; } }));
  const loading = model.load(), remount = model.load(), focus = model.refresh();
  first.resolve({ ...snapshot(), scope: { kind: 'meeting', meeting_uid: 'canonical-uid' } });
  await Promise.all([loading, remount, focus]);
  assert.equal(reads, 1); assert.equal(model.scope.meeting_uid, 'canonical-uid'); assert.equal(model.snapshot.conversation.id, 'one'); assert.equal(model.error, null);
});
