import React, { useRef } from 'react';
import { FlatList, View } from 'react-native';
import { useRouter, usePreventRemove } from 'expo-router';
import { useRuntime } from '../runtime';
import { clock } from '../format';
import { Actions, Body, Button, Field, Notice, Page, Section, confirm } from '../components/ui';
import { useNoteDraft } from '../use-note-draft';
import { RecordingIdentity, stopThenSave } from '../recording-finish';
import type { RuntimeSnapshot } from '../domain';
import { ownedFailureText } from '../errors';
import { recordingPresentation } from '../workflow-state';
import { workflowText } from '../workflow-copy';
import { useHapticFeedback } from '../haptics';

export default function Recording() {
  const { snapshot, busy, run, t, theme, language } = useRuntime();
  const identity = useRef(new RecordingIdentity());
  const meeting = identity.current.observe(snapshot?.meeting);
  const router = useRouter();
  const haptic = useHapticFeedback();
  const copy = (key: Parameters<typeof workflowText>[1]) => workflowText(language, key);
  const draft = useNoteDraft(meeting?.id, meeting?.notes ?? '', identity.current.key);
  const notes = draft.draft;
  const save = draft.save;
  const presentation = recordingPresentation(meeting?.phase, Boolean(snapshot?.meeting.retained_id === meeting?.id));
  usePreventRemove(draft.dirty, ({ repeat }) => confirm(t('mobile.notes.leave_title'), t('mobile.notes.leave_help'), t('common.delete'), t('common.cancel'), () => { draft.discard(); repeat(); }, true));
  const finish = async () => {
    const id = meeting?.id;
    let destination = id;
    let finalized = false;
    const result = await stopThenSave(async () => {
      const stopped = await run<RuntimeSnapshot>('meeting-stop', 'meetings.stop', { id });
      const finishedId = stopped?.meeting.last_finished_id;
      if (id && finishedId && identity.current.retarget(id, finishedId)) { destination = finishedId; draft.retarget(finishedId); }
      finalized = Boolean(stopped && !stopped.meeting.retained_id && !stopped.meeting.error);
      return stopped !== undefined;
    }, save);
    if (result.stopped && result.saved && finalized && !draft.getCurrent().dirty && destination) {
      if (snapshot?.settings.onboarding.status === 'pending' && snapshot.settings.onboarding.goal === 'meeting') {
        await run('onboarding-settings', 'settings.update', { patch: { onboarding: { ...snapshot.settings.onboarding, status: 'completed', resume_step: 'practice' } } });
      }
      haptic('meetingSaved');
      router.replace({ pathname: '/meeting/[id]', params: { id: destination } });
    } else haptic(result.stopped ? 'recordingStopped' : 'error');
  };
  const minimize = () => { void save().then((saved) => { if (saved) { if (router.canGoBack()) router.back(); else router.replace('/(tabs)'); } }); };
  const header = <>
    <Section>
      <Body style={{ fontFamily: theme.font.semibold }}>{meeting?.title || t('meetings.default_title')}</Body>
      {presentation.canStop && <Notice>{copy('minimizeHelp')}</Notice>}
      {meeting?.phase === 'interrupted' && <Notice>{copy('pausedHelp')}</Notice>}
      {meeting?.error && <Notice error>{ownedFailureText(language, meeting.error, meeting.error_code)}</Notice>}
      {meeting?.phase === 'finishing' && <Notice>{t('meetings.finishing_pending_many', { count: meeting.pending })}</Notice>}
    </Section>
    <Section>
      <Field label={t('meetings.your_notes')} editable={Boolean(meeting?.id)} multiline value={notes} onChangeText={draft.setDraft} placeholder={t('meetings.live_notes_placeholder')} style={{ minHeight: 120, maxHeight: 220 }} />
      {draft.conflict && <Notice error>{t('mobile.notes.remote_change')}</Notice>}
      {draft.dirty && <Button title={t('common.save')} subtle busy={draft.saving} onPress={() => void save()} />}
      {draft.dirty && (draft.failed || draft.conflict) && <Button title={t('common.copy')} subtle onPress={() => void run('copy-recording-notes', 'dictation.copy', { text: notes })} />}
    </Section>
    <Body accessibilityRole="header" style={{ fontFamily: theme.font.semibold, marginBottom: theme.space[3] }}>{t('meetings.live_transcript')}</Body>
  </>;
  return <Page title={copy('recordingTitle')} subtitle={`${copy(presentation.status)} · ${clock(meeting?.elapsed_secs ?? 0)}`} scroll={false} bottomInset action={<Button title={copy('minimize')} subtle onPress={minimize} />}>
    <View accessibilityLiveRegion="polite" style={{ gap: theme.space[3], marginBottom: theme.space[4] }}>
      <Actions>
        {presentation.canStop && <Button title={t('common.stop_recording')} icon="stop" busy={busy.has('meeting-stop')} onPress={() => void finish()} />}
        {snapshot?.meeting.retained_id === meeting?.id && meeting?.id && <Button title={t('meetings.retry_save')} busy={busy.has('meeting-retry-save')} onPress={() => void run<RuntimeSnapshot>('meeting-retry-save', 'meetings.retrySave', { id: meeting.id }).then(async (result) => { if (result && await save()) haptic('meetingSaved'); })} />}
        {meeting?.phase === 'idle' && <Button title={t('common.back')} subtle onPress={minimize} />}
      </Actions>
    </View>
    <FlatList data={meeting?.transcript ?? []} keyExtractor={(segment, index) => `${segment.start_ms}-${index}`} ListHeaderComponent={header} keyboardShouldPersistTaps="handled" contentContainerStyle={{ paddingBottom: theme.space[6] }} initialNumToRender={12} maxToRenderPerBatch={12} windowSize={7} renderItem={({ item }) => <View style={{ marginBottom: theme.space[3] }}><Body muted style={{ fontSize: theme.size.label }}>{clock(item.start_ms / 1000)}</Body><Body>{item.text}</Body></View>} ListEmptyComponent={<Notice>{t('meetings.empty_transcript')}</Notice>} />
  </Page>;
}
