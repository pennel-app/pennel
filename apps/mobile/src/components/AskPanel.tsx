import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useFocusEffect, useRouter } from 'expo-router';
import { AppState, Modal, View } from 'react-native';
import { conversationScopeKey, type ConversationMessage, type ConversationResolvedSource, type ConversationScope } from '../../../../assets/contracts/meeting-conversations';
import { captureConversationIntent, conversationIntentCurrent, mobileConversationModel } from '../conversation-service';
import { conversationSourceRoute } from '../source-navigation';
import { useRuntime } from '../runtime';
import { clock } from '../format';
import { ownedFailureText, ownedNativeFailureText } from '../errors';
import { ActionMenu, Actions, Body, Button, Empty, Field, Loading, NativeChoice, Notice, Page, Row, Section, confirm } from './ui';
import { TextDialog } from './TextDialog';

let requestSequence = 0;
export function AskPanel({ meetingId, meetingUid, targetConversationId, targetMessageId, beforeAsk, onInteraction }: {
  meetingId?: string; meetingUid?: string; targetConversationId?: string; targetMessageId?: string;
  beforeAsk?: () => Promise<boolean>; onInteraction?: () => void;
}) {
  const { store, snapshot: runtime, t, theme, language, run } = useRuntime();
  const router = useRouter();
  const scope: ConversationScope = meetingId ? { kind: 'meeting', meeting_uid: meetingUid ?? '', meeting_id: meetingId } : { kind: 'library' };
  const library = typeof runtime?.settings.meetings_dir === 'string' ? runtime.settings.meetings_dir : '';
  const scopeKey = conversationScopeKey(scope);
  const model = useMemo(() => mobileConversationModel(store.client, library, scope, () => {
    const value = store.getSnapshot().snapshot?.settings.meetings_dir;
    return typeof value === 'string' ? value : '';
  }), [store, library, scopeKey, meetingId]);
  const state = useSyncExternalStore(model.subscribe, model.getSnapshot, model.getSnapshot);
  const current = state.snapshot?.conversation.id === state.conversationId ? state.snapshot : null;
  const conversations = state.snapshot ? [...new Map([...state.snapshot.conversations, state.snapshot.conversation].map(value => [value.id, value])).values()] : [];
  const [renaming, setRenaming] = useState<{ initial: string; id: string } | null>(null);
  const [sourcePreview, setSourcePreview] = useState<ConversationResolvedSource | null>(null);
  const [savingNotes, setSavingNotes] = useState(false);
  const [notesFailed, setNotesFailed] = useState(false);
  const [sendChanged, setSendChanged] = useState(false);
  const [contextChanged, setContextChanged] = useState(false);
  const [focusedMessageId, setFocusedMessageId] = useState(targetMessageId);
  const opened = useRef('');
  const sourceEpoch = useRef(0);
  const focusedView = useRef(false);
  const pendingReturn = useRef<{ conversationId: string; messageId: string } | null>(null);
  useFocusEffect(useCallback(() => {
    focusedView.current = true;
    const target = pendingReturn.current;
    if (target) {
      pendingReturn.current = null;
      void (async () => { await model.select(target.conversationId); await model.focus(target.messageId); setFocusedMessageId(target.messageId); })();
    } else if (model.getSnapshot().snapshot && !model.getSnapshot().loading) void model.refresh();
    return () => { focusedView.current = false; ++sourceEpoch.current; void model.flush().catch(() => undefined); };
  }, [model]));
  useEffect(() => {
    const target = `${targetConversationId ?? ''}:${targetMessageId ?? ''}`;
    void (async () => {
      if (targetConversationId && opened.current !== target) {
        opened.current = target;
        await model.select(targetConversationId);
        if (targetMessageId) await model.focus(targetMessageId);
      } else await model.load();
    })();
    const lifecycle = AppState.addEventListener('change', status => {
      if (status === 'active' && focusedView.current && !model.getSnapshot().loading) void model.refresh();
      else void model.flush().catch(() => undefined);
    });
    return () => { ++sourceEpoch.current; lifecycle.remove(); void model.flush().catch(() => undefined); };
  }, [model, targetConversationId, targetMessageId]);
  useEffect(() => { setFocusedMessageId(targetMessageId); }, [targetMessageId]);
  const running = state.busy || Boolean(state.snapshot?.conversations.some(value => value.active_request_id));
  useEffect(() => {
    if (!running) return;
    let refreshing = false;
    const timer = setInterval(() => {
      if (refreshing || !focusedView.current || AppState.currentState !== 'active') return;
      refreshing = true;
      void model.refresh().finally(() => { refreshing = false; });
    }, 1000);
    return () => clearInterval(timer);
  }, [model, running]);
  const interact = (action: () => void) => { ++sourceEpoch.current; onInteraction?.(); action(); };
  const ask = async () => {
    if (!state.draft.trim() || state.busy || savingNotes) return;
    const epoch = ++sourceEpoch.current; onInteraction?.(); setNotesFailed(false); setSendChanged(false);
    const intent = captureConversationIntent({ scope: model.scope, conversationId: state.conversationId, draft: state.draft }, true);
    if (!conversationIntentCurrent(intent, model)) { setSendChanged(true); return; }
    const requestId = `mobile-${Date.now().toString(36)}-${++requestSequence}-${Math.random().toString(36).slice(2)}`;
    const sent = await model.ask((intent.draft ?? '').trim(), requestId, async () => {
      if (beforeAsk) {
        setSavingNotes(true);
        let saved = false;
        try { saved = await beforeAsk(); } catch { saved = false; } finally { setSavingNotes(false); }
        if (!saved) { setNotesFailed(true); throw new Error('The saved notes changed. Your question was not sent; save your notes and try again.'); }
      }
      return {};
    });
    if (sent && epoch === sourceEpoch.current && conversationIntentCurrent({ ...intent, draft: null }, model)) {
      setFocusedMessageId(undefined); if (focusedMessageId) await model.load();
    }
  };
  const copy = () => {
    const text = [...(current?.messages.map(message => message.text).filter(Boolean) ?? []), state.draft].filter(Boolean).join('\n\n');
    void run('conversation-copy', 'dictation.copy', { text });
  };
  const openSource = async (message: ConversationMessage, citationNumber: number) => {
    const citation = message.answer?.citations.find(value => value.number === citationNumber);
    if (!citation || model.conversationId !== message.conversation_id) return;
    onInteraction?.();
    const epoch = ++sourceEpoch.current;
    const intent = captureConversationIntent(model);
    let source: ConversationResolvedSource;
    try { source = await model.source(message.id, citationNumber, message.conversation_id); }
    catch { source = { status: 'unavailable', passage: citation.passage, anchor: null, current_meeting_id: null }; }
    if (epoch !== sourceEpoch.current || !conversationIntentCurrent(intent, model)) return;
    if (!source.current_meeting_id) { setSourcePreview(source); return; }
    await model.flush().catch(() => undefined);
    if (epoch !== sourceEpoch.current || !conversationIntentCurrent(intent, model)) return;
    pendingReturn.current = { conversationId: message.conversation_id, messageId: message.id };
    setFocusedMessageId(message.id);
    router.push(conversationSourceRoute(source.current_meeting_id, { conversation_id: message.conversation_id, message_id: message.id,
      citation_number: citationNumber, scope: model.scope.kind, meeting_uid: model.scope.kind === 'meeting' ? model.scope.meeting_uid : null }));
  };
  const focused = current?.messages.find(message => message.id === focusedMessageId);
  const seeking = Boolean(focusedMessageId && !focused && !state.error);
  const messages = seeking ? [] : focused ? current?.messages.filter(message => message.id === focused.id || message.id === focused.reply_to) ?? [] : current?.messages ?? [];
  return <>
    <Section>
      {state.snapshot && <NativeChoice label={t('conversations.select')} value={state.conversationId ?? ''}
        options={conversations.map(value => ({ value: value.id, label: value.title || t('conversations.untitled') }))}
        onChange={id => interact(() => { setFocusedMessageId(undefined); void model.select(id); })} />}
      <Actions><Button title={t('conversations.new')} subtle icon="add" onPress={() => interact(() => { setFocusedMessageId(undefined); void model.create(); })} />
        <ActionMenu label={t('mobile.conversations.actions')} actions={[
          { label: t('conversations.rename'), disabled: !current, onPress: () => interact(() => { if (current) setRenaming({ initial: current.conversation.title, id: current.conversation.id }); }) },
          { label: t('conversations.delete'), disabled: !current, destructive: true, onPress: () => interact(() => {
            if (!current || model.conversationId !== current.conversation.id) { setContextChanged(true); return; }
            const intent = captureConversationIntent(model);
            confirm(t('conversations.delete_title'), t('conversations.delete_help'), t('common.delete'), t('common.cancel'), () => {
              if (!conversationIntentCurrent(intent, model)) { setContextChanged(true); return; }
              setFocusedMessageId(undefined); void model.delete();
            }, true);
          }) },
        ]} />
      </Actions>
      {state.snapshot?.next_conversations_cursor && <Button title={t('conversations.more')} subtle onPress={() => void model.loadMore()} />}
      {state.loading && <Loading />}
      {state.error && <><Notice error>{ownedNativeFailureText(language, state.error)}</Notice><Button title={t('common.retry')} subtle onPress={() => void model.retrySave().then(() => model.refresh())} /></>}
      {contextChanged && <Notice>{t('mobile.conversations.context_changed')}</Notice>}
      {(state.snapshot?.storage_status === 'unsaved' || state.dirty && state.error) && <>
        <Notice error>{t('conversations.unsaved')}</Notice>
        {state.snapshot?.storage_error && state.snapshot.storage_error !== state.error && <Notice error>{ownedFailureText(language, state.snapshot.storage_error)}</Notice>}
        <Actions><Button title={t('conversations.retry')} subtle onPress={() => void model.retrySave()} /><Button title={t('conversations.copy')} subtle onPress={copy} /></Actions>
      </>}
    </Section>
    {focusedMessageId && <Section title={t('mobile.conversations.selected_answer')}>{seeking && <Loading />}<Button title={t('mobile.conversations.show_history')} subtle onPress={() => { setFocusedMessageId(undefined); void model.load(); }} /></Section>}
    {!state.loading && !seeking && current && messages.length === 0 && <Empty title={t(meetingId ? 'conversations.empty' : 'meetings.ask_placeholder_all')} description={t('conversations.empty_help')} />}
    {!focused && current?.next_messages_cursor && <Button title={t('conversations.older')} subtle onPress={() => void model.loadOlder()} />}
    {messages.map(message => <Section key={message.id} title={message.role === 'user' ? t('meetings.your_question') : undefined}>
      {message.text ? <Body selectable>{message.text}</Body> : null}
      {message.status !== 'completed' && <Notice error={message.status === 'failed'}>{t(`conversations.status_${message.status}`)}</Notice>}
      {message.error && <Notice error>{ownedFailureText(language, message.error)}</Notice>}
      {message.evidence?.meeting_uid && <Body muted>{t('meetings.workspace_cutoff', { time: clock(message.evidence.included_through_ms / 1000) })}</Body>}
      {message.evidence?.selection.history_reduced && <Body muted>{t('meetings.workspace_history_reduced', { included: message.evidence.selection.included_history_turns, available: message.evidence.selection.available_history_turns })}</Body>}
      {message.answer?.citations.map(citation => {
        const source = message.evidence?.sources.find(value => value.number === citation.number);
        const status = source?.status;
        const statusLabel = status ? t(`conversations.source_${status}`) : t('conversations.source_unavailable');
        const cutoff = !message.evidence?.meeting_uid && source && citation.passage.kind === 'transcript'
          ? ` · ${t('meetings.workspace_cutoff', { time: clock(source.included_through_ms / 1000) })}` : '';
        return <Row key={citation.number} title={`${citation.number}. ${citation.passage.title}`} subtitle={`${t(`meetings.source_${citation.passage.kind}`)} · ${statusLabel}${cutoff}`} onPress={() => void openSource(message, citation.number)} />;
      })}
    </Section>)}
    <View style={{ gap: theme.space[3] }}>
      <Field key={state.conversationId ?? 'loading'} label={t('meetings.your_question')} placeholder={t(meetingId ? 'meetings.ask_placeholder_this' : 'meetings.ask_placeholder_all')}
        value={state.draft} onChangeText={value => { if (model.conversationId === state.conversationId) interact(() => model.editDraft(value)); }} editable={Boolean(current)} multiline style={{ minHeight: 88 }} returnKeyType="default" />
      {savingNotes && <Notice>{t('meetings.workspace_saving_question')}</Notice>}
      {notesFailed && !state.error && <Notice error>{t('meetings.workspace_question_save_error')}</Notice>}
      {sendChanged && <Notice>{t('mobile.conversations.request_changed')}</Notice>}
      <Actions><Button title={t(state.busy ? 'meetings.writing_answer' : 'meetings.ask')} icon="arrow-up" busy={state.busy || savingNotes}
        disabled={!current || !state.draft.trim()} onPress={() => void ask()} />
        {state.busy && <Button title={t('common.cancel')} subtle onPress={() => void model.cancel()} />}
      </Actions>
    </View>
    <TextDialog visible={renaming !== null} title={t('conversations.rename')} label={t('conversations.rename')} initial={renaming?.initial ?? ''}
      onClose={() => setRenaming(null)} onSave={async value => {
        if (!value.trim()) return false;
        if (!renaming || model.conversationId !== renaming.id) { setContextChanged(true); return false; }
        await model.rename(value.trim()); return model.getSnapshot().snapshot?.conversation.title === value.trim();
      }} />
    <Modal visible={sourcePreview !== null} presentationStyle="pageSheet" onRequestClose={() => setSourcePreview(null)}>
      <Page title={t('conversations.source_quote')} bottomInset><Notice>{t(`conversations.source_${sourcePreview?.status ?? 'unavailable'}`)}</Notice>
        <Body selectable>{sourcePreview?.anchor?.source_text ?? sourcePreview?.passage.text}</Body><Button title={t('common.close')} subtle onPress={() => setSourcePreview(null)} />
      </Page>
    </Modal>
  </>;
}
