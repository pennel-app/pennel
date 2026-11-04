import React, { useCallback, useEffect, useRef, useState } from 'react';
import { FlatList, View } from 'react-native';
import { useFocusEffect, useRouter, usePreventRemove } from 'expo-router';
import { useRuntime } from '../runtime';
import { clock } from '../format';
import { Actions, Body, Button, Field, NativeChoice, Notice, Page, Section, confirm } from '../components/ui';
import { AskPanel } from '../components/AskPanel';
import { SummaryAutomation } from '../components/SummaryAutomation';
import { useNoteDraft } from '../use-note-draft';
import { completedRecording, nativeOwnsRecordingDraft, RecordingIdentity, type RecordingStopIntent } from '../recording-finish';
import type { MeetingDetail, RuntimeSnapshot } from '../domain';
import type { NoteHandoff } from '../note-session';
import { ownedFailureText } from '../errors';
import { recordingPresentation } from '../workflow-state';
import { workflowText } from '../workflow-copy';
import { useHapticFeedback } from '../haptics';

export default function Recording() {
  const { snapshot, busy, run, t, theme, language } = useRuntime();
  const identity = useRef(new RecordingIdentity());
  const stopIntent = useRef<(RecordingStopIntent & { resolving: boolean; handoff: NoteHandoff }) | null>(null);
  const focused = useRef(true);
  useFocusEffect(useCallback(() => { focused.current = true; return () => { focused.current = false; }; }, []));
  const meeting = identity.current.observe(snapshot?.meeting);
  const router = useRouter();
  const haptic = useHapticFeedback();
  const [pane, setPane] = useState<'notes' | 'transcript' | 'questions'>('notes');
  const [reviewDestination, setReviewDestination] = useState<string | null>(null);
  const restoredPending = useRef<string | null>(null);
  const copy = (key: Parameters<typeof workflowText>[1]) => workflowText(language, key);
  const draft = useNoteDraft(meeting?.id, meeting?.notes ?? '', identity.current.key, meeting?.phase !== 'finishing' && !snapshot?.meeting.retained_id && !stopIntent.current);
  const notes = draft.draft;
  const save = draft.save;
  const presentation = recordingPresentation(meeting?.phase, Boolean(snapshot?.meeting.retained_id === meeting?.id));
  useEffect(() => {
    const retained = snapshot?.meeting.retained_id ?? (snapshot?.meeting.phase === 'finishing' ? snapshot.meeting.id : null);
    const pending = snapshot?.meeting.pending_notes;
    if (!retained || typeof pending !== 'string' || restoredPending.current === retained) return;
    restoredPending.current = retained;
    if (!draft.getCurrent().dirty) draft.setDraft(pending);
  }, [snapshot?.meeting.retained_id, snapshot?.meeting.pending_notes, snapshot?.meeting.phase, snapshot?.meeting.id, draft]);
  useEffect(() => {
    if (meeting?.phase !== 'finishing' || !meeting.id || stopIntent.current) return;
    stopIntent.current = { id: meeting.id, previousFinishedId: snapshot?.meeting.last_finished_id ?? null, resolving: false,
      handoff: { ...draft.captureHandoff(), notes: snapshot?.meeting.pending_notes ?? snapshot?.meeting.notes ?? '' } };
  }, [meeting?.phase, meeting?.id, snapshot?.meeting.pending_notes, snapshot?.meeting.notes, snapshot?.meeting.last_finished_id, draft]);
  const nativeOwnsDraft = nativeOwnsRecordingDraft(snapshot?.meeting, meeting?.id, notes, stopIntent.current?.handoff.notes);
  usePreventRemove(draft.dirty && !nativeOwnsDraft, ({ repeat }) => confirm(t('mobile.notes.leave_title'), t('mobile.notes.leave_help'), t('common.delete'), t('common.cancel'), () => { draft.discard(); repeat(); }, true));
  useEffect(() => {
    const intent = stopIntent.current;
    if (intent && snapshot?.meeting.id && snapshot.meeting.id !== intent.id) { stopIntent.current = null; return; }
    const destination = completedRecording(snapshot?.meeting, intent);
    if (!intent || intent.resolving || !destination) return;
    intent.resolving = true;
    if (identity.current.retarget(intent.id, destination)) draft.retarget(destination);
    void (async () => {
      await draft.settle();
      const persisted = await run<MeetingDetail>('recording-final-notes', 'meetings.read', { id: destination });
      if (!persisted) { setReviewDestination(destination); return; }
      draft.acknowledgeHandoff(intent.handoff, persisted.notes);
      if (!await save() || draft.getCurrent().dirty) { setReviewDestination(destination); return; }
      stopIntent.current = null;
      setReviewDestination(destination);
      if (snapshot?.settings.onboarding.status === 'pending' && snapshot.settings.onboarding.goal === 'meeting') {
        await run('onboarding-settings', 'settings.update', { patch: { onboarding: { ...snapshot.settings.onboarding, status: 'completed', resume_step: 'practice' } } });
      }
      haptic('meetingSaved');
      if (focused.current) router.replace({ pathname: '/meeting/[id]', params: { id: destination } });
    })();
  }, [snapshot, draft, save, run, haptic, router]);
  const finish = async () => {
    const id = meeting?.id;
    if (!id) return;
    const handoff = draft.captureHandoff();
    stopIntent.current = { id, previousFinishedId: snapshot?.meeting.last_finished_id ?? null, resolving: false, handoff };
    const stopped = await run<RuntimeSnapshot>('meeting-stop', 'meetings.stop', { id, ...handoff });
    if (!stopped) stopIntent.current = null;
    haptic(stopped ? 'recordingStopped' : 'error');
  };
  const retrySave = async () => {
    const id = snapshot?.meeting.retained_id;
    if (!id) return;
    const handoff = { ...draft.captureHandoff(), notes: snapshot.meeting.pending_notes ?? snapshot.meeting.notes };
    stopIntent.current = { id, previousFinishedId: snapshot.meeting.last_finished_id ?? null, resolving: false, handoff };
    const result = await run<RuntimeSnapshot>('meeting-retry-save', 'meetings.retrySave', { id });
    if (!result) { stopIntent.current = null; haptic('error'); }
  };
  const leave = () => { if (router.canGoBack()) router.back(); else router.replace('/(tabs)'); };
  const minimize = () => { if (nativeOwnsDraft) leave(); else void save().then(saved => { if (saved) leave(); }); };
  const header = <>
    <Section description={t('mobile.simple.notes_first')}>
      {meeting?.phase === 'interrupted' && <Notice>{copy('pausedHelp')}</Notice>}
      {meeting?.error && <Notice error>{ownedFailureText(language, meeting.error, meeting.error_code)}</Notice>}
      {meeting?.phase === 'finishing' && <Notice>{t('meetings.finishing_pending_many', { count: meeting.pending })}</Notice>}
      <NativeChoice label={copy('content')} segmented value={pane} options={(['notes', 'transcript', 'questions'] as const).map(value => ({ value, label: t(`meetings.${value}`) }))} onChange={setPane} />
    </Section>
    {pane === 'notes' && <><Section description={t('mobile.simple.private_notes_help')}>
      <Field label={t('mobile.simple.personal_notes')} editable={Boolean(meeting?.id)} multiline value={notes} onChangeText={draft.setDraft} placeholder={t('meetings.live_notes_placeholder')} style={{ minHeight: 240 }} />
      {draft.conflict && <Notice error>{t('mobile.notes.remote_change')}</Notice>}
      {draft.dirty && meeting?.phase === 'finishing' && !nativeOwnsDraft && <Notice>{t('mobile.simple.notes_finishing')}</Notice>}
      {draft.dirty && !nativeOwnsDraft && <Button title={t('common.save')} subtle busy={draft.saving} disabled={meeting?.phase === 'finishing'} onPress={() => void save()} />}
      {draft.dirty && (draft.failed || draft.conflict) && <Button title={t('common.copy')} subtle onPress={() => void run('copy-recording-notes', 'dictation.copy', { text: notes })} />}
    </Section><SummaryAutomation /></>}
    {pane === 'questions' && (meeting?.id && snapshot?.meeting.meeting_uid ? <AskPanel meetingId={meeting.id} meetingUid={snapshot.meeting.meeting_uid} beforeAsk={save} /> : <Notice>{t('meetings.workspace_saving_question')}</Notice>)}
  </>;
  const footer = <View accessibilityLiveRegion="polite" style={{ gap: theme.space[3] }}>
      <Body muted>{copy(presentation.status)} · {clock(meeting?.elapsed_secs ?? 0)}</Body>
      <Actions>
        {presentation.canStop && <Button title={t('common.stop_recording')} icon="stop" busy={busy.has('meeting-stop')} onPress={() => void finish()} />}
        {snapshot?.meeting.retained_id === meeting?.id && meeting?.id && <Button title={t('meetings.retry_save')} busy={busy.has('meeting-retry-save')} onPress={() => void retrySave()} />}
        {reviewDestination && <Button title={t(draft.dirty ? 'common.save' : 'mobile.simple.review_meeting')} busy={draft.saving} onPress={() => void save().then(saved => { if (saved) router.replace({ pathname: '/meeting/[id]', params: { id: reviewDestination } }); })} />}
        {meeting?.phase === 'idle' && <Button title={t('common.back')} subtle onPress={minimize} />}
      </Actions>
    </View>;
  return <Page title={meeting?.title || t('meetings.default_title')} scroll={false} bottomInset recordingStatus={false} footer={footer} action={<Button title={copy('minimize')} subtle onPress={minimize} />}>
    <FlatList data={pane === 'transcript' ? meeting?.transcript ?? [] : []} keyExtractor={(segment, index) => segment.utterance_id ?? `${segment.start_ms}-${index}`} ListHeaderComponent={header} keyboardShouldPersistTaps="handled" contentContainerStyle={{ paddingBottom: theme.space[6] }} initialNumToRender={12} maxToRenderPerBatch={12} windowSize={7} renderItem={({ item }) => <View style={{ marginBottom: theme.space[3] }}><Body muted style={{ fontSize: theme.size.label }}>{clock(item.start_ms / 1000)}</Body><Body>{item.text}</Body></View>} ListEmptyComponent={pane === 'transcript' ? <Notice>{t('meetings.empty_transcript')}</Notice> : undefined} />
  </Page>;
}
