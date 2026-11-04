import type { RuntimeClient } from './domain.ts';
import { ConversationController, type ConversationTransport } from '../../../assets/contracts/meeting-conversation-controller.ts';
import { conversationScopeKey, type ConversationScope } from '../../../assets/contracts/meeting-conversations.ts';

interface IntentOwner { scope: ConversationScope; conversationId: string | null; draft: string }
export function captureConversationIntent(owner: IntentOwner, includeDraft = false) {
  return { scope: conversationScopeKey(owner.scope), conversation_id: owner.conversationId, draft: includeDraft ? owner.draft : null };
}
export function conversationIntentCurrent(intent: ReturnType<typeof captureConversationIntent>, owner: IntentOwner): boolean {
  return intent.scope === conversationScopeKey(owner.scope) && intent.conversation_id === owner.conversationId
    && (intent.draft === null || intent.draft === owner.draft);
}

export function mobileConversationService(client: Pick<RuntimeClient, 'invoke'>, isCurrent: () => boolean = () => true): ConversationTransport {
  const invoke = <T>(method: string, request: unknown): Promise<T> => {
    if (!isCurrent()) return Promise.reject(new Error('The library changed. Reopen the conversation to continue.'));
    return client.invoke<T>(method, { request });
  };
  return {
    snapshot: request => invoke('conversations.read', request),
    update: request => invoke('conversations.update', request),
    begin: request => invoke('conversations.begin', request),
    cancel: request => invoke('conversations.cancel', request),
    source: request => invoke('conversations.source', request),
  };
}

const models = new WeakMap<RuntimeClient, Map<string, ConversationController>>();
const currentLibraries = new WeakMap<RuntimeClient, string>();
const libraryReaders = new WeakMap<RuntimeClient, () => string>();
const retainedScopeKey = (scope: ConversationScope) => scope.kind === 'meeting' && !scope.meeting_uid
  ? `meeting-folder:${scope.meeting_id ?? ''}` : conversationScopeKey(scope);

/** Retain failed drafts across remounts; durable history and background work belong to the core. */
export function mobileConversationModel(client: RuntimeClient, library: string, scope: ConversationScope, readLibrary?: () => string): ConversationController {
  currentLibraries.set(client, library);
  if (readLibrary) libraryReaders.set(client, readLibrary);
  let retained = models.get(client);
  if (!retained) { retained = new Map(); models.set(client, retained); }
  const key = `${library}\0${retainedScopeKey(scope)}`;
  let model = retained.get(key);
  if (!model) {
    model = [...retained.entries()].find(([existing, value]) => existing.startsWith(`${library}\0`)
      && retainedScopeKey(value.scope) === retainedScopeKey(scope))?.[1];
    model ??= new ConversationController(scope, mobileConversationService(client, () => (libraryReaders.get(client)?.() ?? currentLibraries.get(client)) === library));
    retained.set(key, model);
  }
  if (retained.size > 20) {
    for (const [oldKey, value] of retained) {
      const state = value.getSnapshot();
      if (value !== model && !state.dirty && !state.busy && state.snapshot?.storage_status === 'saved') retained.delete(oldKey);
      if (retained.size <= 20) break;
    }
  }
  return model;
}
