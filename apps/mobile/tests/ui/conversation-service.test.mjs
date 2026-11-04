import assert from 'node:assert/strict';
import test from 'node:test';
import { mobileConversationModel, mobileConversationService } from '../../src/conversation-service.ts';

const scope = { kind: 'meeting', meeting_uid: 'stable-meeting' };
function snapshot() {
  const conversation = { id: 'conversation-one', title: 'Owned conversation', draft: '', draft_revision: 0, created_at_ms: 1, updated_at_ms: 1, active_request_id: null };
  return { schema_version: 1, scope, storage_status: 'saved', storage_error: null, selected_conversation_id: conversation.id,
    conversations: [conversation], conversation, current_meeting_id: 'renamed-folder', messages: [], next_messages_cursor: null, next_conversations_cursor: null };
}

test('mobile remounts retain failed drafts and legacy scope normalization without crossing libraries', async () => {
  const client = { invoke: async (method, payload) => {
    if (method === 'conversations.update' && payload.request.action === 'draft') throw new Error('Storage unavailable');
    return snapshot();
  } };
  const model = mobileConversationModel(client, '/preview/library-one', { kind: 'meeting', meeting_uid: 'legacy-folder', meeting_id: 'old-folder' });
  await model.load();
  const restored = mobileConversationModel(client, '/preview/library-one', scope);
  assert.equal(restored, model);
  restored.editDraft('Exact draft\n\n@Els — still mine.');
  await restored.flush().catch(() => undefined);
  assert.equal(mobileConversationModel(client, '/preview/library-one', scope).getSnapshot().draft, 'Exact draft\n\n@Els — still mine.');
  assert.equal(restored.getSnapshot().dirty, true);
  assert.notEqual(mobileConversationModel(client, '/preview/library-two', scope), restored);
});

test('mobile source requests use only durable IDs and return fresh core validation', async () => {
  const seen = [];
  const result = { status: 'unavailable', current_meeting_id: null, passage: { text: 'Saved quote' }, anchor: null };
  const service = mobileConversationService({ invoke: async (method, payload) => { seen.push({ method, payload }); return result; } });
  assert.equal(await service.source({ scope, conversation_id: 'conversation-one', message_id: 'answer-one', citation_number: 3 }), result);
  assert.deepEqual(seen, [{ method: 'conversations.source', payload: { request: { scope, conversation_id: 'conversation-one', message_id: 'answer-one', citation_number: 3 } } }]);
  assert.equal(JSON.stringify(seen).includes('Saved quote'), false);
});

test('a delayed send or source result cannot target a newly selected conversation or edited question', async () => {
  const { captureConversationIntent, conversationIntentCurrent } = await import('../../src/conversation-service.ts');
  const current = { scope, conversationId: 'conversation-one', draft: 'Original question' };
  const send = captureConversationIntent(current, true);
  assert.equal(conversationIntentCurrent(send, current), true);
  assert.equal(conversationIntentCurrent(send, { ...current, draft: 'Question edited while notes save' }), false);
  assert.equal(conversationIntentCurrent(send, { ...current, conversationId: 'conversation-two' }), false);
  assert.equal(conversationIntentCurrent(send, { ...current, scope: { kind: 'library' } }), false);
  const source = captureConversationIntent(current);
  assert.equal(conversationIntentCurrent(source, { ...current, draft: 'New draft' }), true);
  assert.equal(conversationIntentCurrent(source, { ...current, conversationId: 'conversation-two' }), false);
});

test('a retained view from another library cannot dispatch a new conversation there', async () => {
  const operations = [];
  const client = { invoke: async (method, payload) => { operations.push({ method, payload }); return snapshot(); } };
  const previous = mobileConversationModel(client, '/preview/old-library', scope);
  await previous.load();
  mobileConversationModel(client, '/preview/current-library', scope);
  await previous.create('Old delayed action');
  assert.equal(operations.filter(value => value.method === 'conversations.update').length, 0);
  assert.match(previous.getSnapshot().error, /library changed/i);
});

test('a queued action observes library changes even while its view is unmounted', async () => {
  const operations = [];
  let currentLibrary = '/preview/original';
  const client = { invoke: async (method, payload) => { operations.push({ method, payload }); return snapshot(); } };
  const model = mobileConversationModel(client, currentLibrary, scope, () => currentLibrary);
  await model.load();
  currentLibrary = '/preview/changed-in-settings';
  await model.create('Delayed original-library action');
  assert.equal(operations.filter(value => value.method === 'conversations.update').length, 0);
  assert.match(model.getSnapshot().error, /library changed/i);
});
