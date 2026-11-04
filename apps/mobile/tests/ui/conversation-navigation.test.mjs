import assert from 'node:assert/strict';
import test from 'node:test';
import { conversationSourceRoute, conversationOrigin, conversationReturnRoute } from '../../src/source-navigation.ts';

test('durable source navigation carries only source and answer identities and can return after restart', () => {
  const origin = { conversation_id: 'conversation-one', message_id: 'message-one', citation_number: 2, scope: 'meeting', meeting_uid: 'stable-origin' };
  const route = conversationSourceRoute('current-destination-folder', origin);
  assert.equal(route.pathname, '/meeting/[id]');
  assert.equal(route.params.id, 'current-destination-folder');
  assert.deepEqual(conversationOrigin(route.params), origin);
  assert.equal(JSON.stringify(route).includes('excerpt'), false);
  const back = conversationReturnRoute(origin, 'renamed-origin-folder');
  assert.deepEqual(back, { pathname: '/meeting/[id]', params: { id: 'renamed-origin-folder', conversation_id: 'conversation-one', message_id: 'message-one', pane: 'questions' } });
});

test('library answer return stays in the library route and malformed source origins are ignored', () => {
  const origin = { conversation_id: 'library-conversation', message_id: 'answer', citation_number: 1, scope: 'library', meeting_uid: null };
  assert.deepEqual(conversationReturnRoute(origin), { pathname: '/(tabs)/ask', params: { conversation_id: 'library-conversation', message_id: 'answer' } });
  for (const change of [{ citation_number: '0' }, { citation_number: '1.5' }, { conversation_id: '' }, { origin_scope: 'unknown' }, { origin_meeting_uid: '' }]) {
    const meeting = { ...origin, scope: 'meeting', meeting_uid: 'origin' };
    assert.equal(conversationOrigin({ ...conversationSourceRoute('target', meeting).params, ...change }), null);
  }
});
