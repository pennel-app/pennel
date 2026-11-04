import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { FlatList, Modal, Pressable, View } from 'react-native';
import { Stack, useFocusEffect, useLocalSearchParams, useRouter, usePreventRemove } from 'expo-router';
import type { MeetingDetail, SpeakerJob } from '../domain';
import { useQuery, useReducedMotion, useRuntime } from '../runtime';
import { meetingDetail, speakerJob } from '../normalize';
import { clock, dateLabel, recognitionActions } from '../format';
import { conversationOrigin, conversationReturnRoute, rememberSource, resolveSource } from '../source-navigation';
import { useNoteDraft } from '../use-note-draft';
import { saveThenRename } from '../meeting-rename';
import { localizedFailure, ownedFailureText } from '../errors';
import { ActionMenu, Actions, Body, Button, Empty, Field, Loading, NativeChoice, Notice, Page, Row, Section, confirm } from '../components/ui';
import { TextDialog } from '../components/TextDialog';
import { AskPanel } from '../components/AskPanel';
import { conversationRequests, type ConversationResolvedSource, type ConversationSnapshot } from '../../../../assets/contracts/meeting-conversations';
import { workflowText } from '../workflow-copy';
import { useMeetingSummary } from '../use-meeting-summary';
import { MeetingSummary } from '../components/MeetingSummary';
import { MeetingExport } from '../components/MeetingExport';
import { SummaryAutomation } from '../components/SummaryAutomation';
import { useHapticFeedback } from '../haptics';
import { attributionEditable, transcriptKeys, transcriptSpeakerLabel, transcriptPlayback, transcriptAttributionProvisional } from '../transcript-state';
import { InsightReview } from '../components/InsightReview';
import { mobileInsightService } from '../insight-service';
import { InsightReviewModel, type InsightNoteHost } from '../../../../assets/contracts/insight-review-model.ts';
import { insertionStillCurrent, type InsightNoteLatch } from '../../../../assets/contracts/meeting-insights.ts';

type Pane = 'notes' | 'transcript' | 'questions' | 'insights';
type Editor = { type: 'rename' } | { type: 'speaker'; voice: number; initial: string } | null;
export default function Meeting() {
  const params = useLocalSearchParams<{ id: string; source?: string; pane?: string; conversation_id?: string; message_id?: string; citation_number?: string; origin_scope?: string; origin_meeting_uid?: string }>();
  const id = Array.isArray(params.id) ? params.id[0] : params.id;
  const { snapshot, run, busy, t, theme, language, store } = useRuntime();
  const router = useRouter();
  const reducedMotion = useReducedMotion();
  const haptic = useHapticFeedback();
  const copy = (key: Parameters<typeof workflowText>[1]) => workflowText(language, key);
  const [editingNotes, setEditingNotes] = useState(false);
  const [showSpeakers, setShowSpeakers] = useState(false);
  const [playRequested, setPlayRequested] = useState(false);
  const query = useQuery<MeetingDetail>('meetings.read', { id }, Boolean(id && snapshot?.ready));
  const detail = useMemo(() => query.value?.info ? meetingDetail(query.value) : null, [query.value]);
  const [pane, setPane] = useState<Pane>('notes');
  const origin = conversationOrigin(params);
  const originScope = origin?.scope === 'meeting' ? { kind: 'meeting' as const, meeting_uid: origin.meeting_uid! } : { kind: 'library' as const };
  const durableSource = useQuery<ConversationResolvedSource>('conversations.source', { request: origin ? conversationRequests.source(originScope, origin.conversation_id, origin.message_id, origin.citation_number) : null }, Boolean(origin && snapshot?.ready));
  const originConversation = useQuery<ConversationSnapshot>('conversations.read', { request: origin ? conversationRequests.snapshot(originScope, origin.conversation_id) : null }, Boolean(origin && snapshot?.ready));
  const returnRoute = origin ? conversationReturnRoute(origin, originConversation.value?.conversation.id === origin.conversation_id ? originConversation.value.current_meeting_id ?? undefined : undefined) : null;
  const draft = useNoteDraft(id, detail?.notes ?? '');
  const summary = useMeetingSummary(id);
  useFocusEffect(useCallback(() => { void query.refresh(); void summary.query.refresh(); }, [query.refresh, summary.query.refresh]));
  const [exportShown, setExportShown] = useState(false);
  const insightHolder = useRef<InsightReviewModel | null>(null);
  if (!insightHolder.current || detail?.meeting_uid && insightHolder.current.meetingUid !== detail.meeting_uid) {
    insightHolder.current = new InsightReviewModel(id, detail?.meeting_uid ?? id, mobileInsightService(store.client));
  }
  const insightModel = insightHolder.current;
  if (detail?.info.id === id) insightModel.id = id;
  const insightState = useSyncExternalStore(insightModel.subscribe, insightModel.getSnapshot, insightModel.getSnapshot);
  const noteFocused = useRef(false);
  const noteSelection = useRef({ start: 0, end: 0, revision: 0 });
  const interactions = useRef(0);
  useEffect(() => { if (pane !== 'notes') noteFocused.current = false; }, [pane]);
  const currentNotes = (): InsightNoteLatch => {
    const value = draft.getCurrent(); return { text: value.draft, revision: value.revision, selection_start: noteSelection.current.start,
      selection_end: noteSelection.current.end, selection_revision: noteSelection.current.revision, interaction_revision: interactions.current, composing: noteFocused.current };
  };
  const insightNotes: InsightNoteHost = {
    flush: draft.save, current: currentNotes,
    stage: async (text, attempt, before) => {
      if (insightModel.id !== id || !insertionStillCurrent(before, currentNotes()) || !draft.stageInsightInsertion(text, attempt, before.text)) return false;
      setPane('notes'); setEditingNotes(true); return draft.save();
    },
  };
  const notes = draft.draft;
  const [editor, setEditor] = useState<Editor>(null);
  const [viewConflict, setViewConflict] = useState<{ name: string; text: string } | null>(null);
  const [currentJob, setCurrentJob] = useState<SpeakerJob | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [sourceIssue, setSourceIssue] = useState<string | null>(null);
  const transcriptList = useRef<FlatList>(null);
  const transcriptOffset = useRef(0);
  useEffect(() => { if (pane !== 'transcript') return; const frame = requestAnimationFrame(() => transcriptList.current?.scrollToOffset({ offset: transcriptOffset.current, animated: false })); return () => cancelAnimationFrame(frame); }, [pane]);
  const openedSource = useRef<string | null>(null);
  useEffect(() => { if (params.pane === 'questions' && !origin) setPane('questions'); }, [params.pane, params.origin_scope]);
  const dirty = draft.dirty;
  const remoteChanged = draft.conflict;
  useEffect(() => { if (draft.dirty || draft.failed || draft.conflict) setEditingNotes(true); }, [draft.dirty, draft.failed, draft.conflict]);
  const disableRemoval = usePreventRemove(dirty || insightModel.dirty || summary.dirty, ({ repeat }) => confirm(t(summary.dirty ? 'mobile.simple.summary_leave' : insightModel.dirty ? 'insights.edit_unsaved_leave' : 'mobile.notes.leave_title'),
    t(summary.dirty ? 'mobile.simple.summary_leave_help' : insightModel.dirty ? 'insights.edit_unsaved_help' : 'mobile.notes.leave_help'), t('common.delete'), t('common.cancel'), () => { draft.discard(); summary.model.discard(); insightModel.discardLocalEdits(); repeat(); }, true));
  useEffect(() => {
    const revision = snapshot?.insight_revisions?.[insightModel.meetingUid];
    if (revision !== undefined && revision > (insightState.snapshot?.revision ?? 0) && pane === 'insights') void insightModel.load();
  }, [snapshot?.insight_revisions, insightModel, insightState.snapshot?.revision, pane]);
  useEffect(() => {
    const token = typeof params.source === 'string' ? params.source : null;
    if (!token || !detail || token === openedSource.current) return;
    openedSource.current = token;
    const source = resolveSource(token, detail);
    if (source.changed || !source.passage) { setSourceIssue(source.passage?.text ?? ''); return; }
    setSourceIssue(null);
    setPane(source.passage.kind === 'transcript' ? 'transcript' : 'notes');
    if (source.transcriptIndex !== null) {
      const index = source.transcriptIndex;
      setTimeout(() => transcriptList.current?.scrollToIndex({ index, animated: !reducedMotion, viewPosition: 0.2 }), 300);
    }
  }, [detail, params.source, reducedMotion, t]);
  useEffect(() => {
    const source = durableSource.value;
    if (!source || !origin) return;
    const token = `conversation:${origin.conversation_id}:${origin.message_id}:${origin.citation_number}`;
    if (openedSource.current === token) return;
    if (source.status !== 'current' || !detail) { setSourceIssue(source.anchor?.source_text ?? source.passage.text); return; }
    const resolved = resolveSource(rememberSource(source.passage), detail);
    if (resolved.changed) { setSourceIssue(source.anchor?.source_text ?? source.passage.text); return; }
    openedSource.current = token;
    setSourceIssue(null);
    setPane(source.passage.kind === 'transcript' ? 'transcript' : 'notes');
    if (resolved.transcriptIndex !== null) {
      const index = resolved.transcriptIndex;
      setTimeout(() => transcriptList.current?.scrollToIndex({ index, animated: !reducedMotion, viewPosition: 0.2 }), 300);
    }
  }, [durableSource.value, detail, params.conversation_id, params.message_id, params.citation_number, reducedMotion]);
  const jobs = (snapshot?.recognition_jobs ?? []).map(speakerJob);
  const job = currentJob ? jobs.find((job) => job.id === currentJob.id) ?? currentJob : jobs.find((job) => job.meeting_id === id);
  const jobStage = job?.stage;
  useEffect(() => { if (jobStage === 'completed') void query.refresh(); }, [jobStage, query.refresh]);
  useEffect(() => {
    if (!job || !['preparing', 'uploading', 'recognizing', 'applying'].includes(job.stage)) return;
    const timer = setInterval(() => { void run<SpeakerJob>(`job-poll-${job.id}`, 'recognition.get', { job_id: job.id }).then((value) => { if (value) setCurrentJob(speakerJob(value)); }); }, 1500);
    return () => clearInterval(timer);
  }, [job?.id, jobStage, run]);
  const save = async () => {
    if (!detail || remoteChanged) return;
    if (await draft.save()) { setNotice(t('mobile.saved')); setEditingNotes(false); await query.refresh(); }
  };
  const recognize = () => confirm(t('mobile.speakers.consent_title'), t('mobile.speakers.consent'), t('mobile.speakers.allow'), t('common.cancel'), () => { void run<SpeakerJob>('recognition-start', 'recognition.start', { id, consent: true }).then((value) => { if (value) setCurrentJob(speakerJob(value)); }); });
  const resume = () => {
    if (!job) return;
    const perform = () => { void run<SpeakerJob>('recognition-resume', 'recognition.resume', { job_id: job.id, confirm_unknown: job.stage === 'response_unknown' }).then((value) => { if (value) setCurrentJob(speakerJob(value)); }); };
    if (job.stage === 'response_unknown') confirm(t('mobile.speakers.retry_title'), t('mobile.speakers.retry_help'), t('common.retry'), t('common.cancel'), perform);
    else perform();
  };
  const exportMeeting = async () => {
    if (await draft.save() && await summary.model.save()) setExportShown(true);
  };
  if (!detail) return <Page title={t('navigation.meetings')} inset={false}>
    {origin && <Section><Notice>{t('conversations.source_unavailable')}</Notice><Body selectable>{durableSource.value?.anchor?.source_text ?? durableSource.value?.passage.text}</Body>{returnRoute && <Button title={t('conversations.return_to_answer')} subtle onPress={() => router.replace(returnRoute)} />}</Section>}
    {query.loading ? <Loading /> : query.error ? <><Notice error>{query.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void query.refresh()} /></> : <Empty title={t('meetings.not_found')} />}
  </Page>;
  const voices = [...new Set(detail.transcript.flatMap((segment) => segment.voice === null ? [] : [segment.voice]))].sort((a, b) => a - b);
  const names = new Map(detail.speakers.map((speaker) => [speaker.voice, speaker.name]));
  const rowKeys = transcriptKeys(detail.transcript);
  const canPlay = detail.audio_tracks.some((track) => track.available);
  const active = Boolean(snapshot && snapshot.meeting.id === id && snapshot.meeting.phase !== 'idle');
  const jobBusy = job && ['preparing', 'uploading', 'recognizing', 'applying'].includes(job.stage);
  const canEditAttribution = attributionEditable(detail.metadata_status);
  const speakerEnabled = snapshot?.cloud.signed_in && !active && canPlay && !jobBusy && canEditAttribution;
  const deleteMeeting = () => confirm(t('meetings.delete_title', { title: detail.info.title }), t('meetings.delete_help'), t('common.delete'), t('common.cancel'), () => { void draft.save().then((saved) => saved ? run('delete-meeting', 'meetings.delete', { id }) : undefined).then((value) => { if (value !== undefined) { draft.discard(); disableRemoval(); router.replace('/(tabs)'); } }); }, true);
  return <Page subtitle={`${dateLabel(detail.info.start_iso, language)} · ${t('meetings.duration_minutes', { count: detail.info.duration_min })}`} inset={false} scroll={pane !== 'transcript'} scrollKey={`${id}:${pane}`} action={<ActionMenu label={copy('moreActions')} actions={[
    { label: t('mobile.simple.preview_title'), onPress: () => void exportMeeting() },
    { label: t('insights.title'), onPress: () => setPane('insights') },
    { label: t('meetings.rename_action'), onPress: () => setEditor({ type: 'rename' }) },
    { label: t('meetings.delete_action'), onPress: deleteMeeting, destructive: true, disabled: active },
  ]} />}>
    <Stack.Screen options={{ title: detail.info.title }} />
    {notice && <Section><Notice>{notice}</Notice></Section>}
    {origin && <Section>{durableSource.loading && <Loading />}{durableSource.error && <Notice error>{durableSource.error}</Notice>}{returnRoute && <Button title={t('conversations.return_to_answer')} subtle onPress={() => router.replace(returnRoute)} />}</Section>}
    {sourceIssue !== null && <Section><Notice>{t('meetings.source_changed')}</Notice>{sourceIssue ? <Body selectable>{sourceIssue}</Body> : null}</Section>}
    {detail.interruption && <Section><Notice error>{ownedFailureText(language, detail.interruption, detail.interruption_code)}</Notice></Section>}
    {(detail.conflicts ?? []).length > 0 && <Section description={t('meetings.conflict_notice')}>{detail.conflicts?.map((conflict) => <Row key={conflict.name} title={conflict.name} onPress={() => setViewConflict({ name: conflict.name, text: conflict.text ?? '' })} />)}</Section>}
    <View style={{ marginBottom: theme.space[5] }}><NativeChoice label={copy('content')} segmented value={pane} options={(['notes', 'transcript', 'questions'] as const).map((value) => ({ value, label: t(`meetings.${value}`) }))} onChange={value => { ++interactions.current; setPane(value); }} /></View>
    {pane === 'notes' && <>
      <Section title={t('mobile.simple.personal_notes')} description={t('mobile.simple.private_notes_help')}>{editingNotes ? <Field label={copy('notesPlaceholder')} multiline value={notes} onChangeText={value => { ++interactions.current; draft.setDraft(value); }} onFocus={() => { noteFocused.current = true; }} onBlur={() => { noteFocused.current = false; }} onSelectionChange={event => { const value = event.nativeEvent.selection; if (value.start !== noteSelection.current.start || value.end !== noteSelection.current.end) noteSelection.current = { ...value, revision: noteSelection.current.revision + 1 }; }} placeholder={t('meetings.notes_placeholder')} style={{ minHeight: 180 }} autoFocus /> : <Body selectable muted={!notes.trim()}>{notes.trim() ? notes : copy('noNotes')}</Body>}{!editingNotes && <Button title={copy(notes.trim() ? 'editNotes' : 'addNote')} subtle icon="create-outline" onPress={() => setEditingNotes(true)} />}{remoteChanged && <><Notice error>{t('mobile.notes.remote_change')}</Notice><Button title={t('mobile.refresh')} subtle onPress={() => confirm(t('mobile.notes.leave_title'), t('mobile.notes.leave_help'), t('mobile.refresh'), t('common.cancel'), draft.discard)} /></>}{editingNotes && <Button title={copy('finishEditing')} busy={draft.saving} disabled={remoteChanged} onPress={() => void save()} />}</Section>
      <MeetingSummary id={id} summary={summary} active={active} sourceBusy={draft.dirty || draft.saving || draft.conflict || editingNotes} beforeGenerate={draft.save} />
      {snapshot?.settings.summary_automation === 'unanswered' && <SummaryAutomation />}
    </>}
    {pane === 'transcript' && <>
      <Actions><Button title={copy('manageSpeakers')} subtle icon="people-outline" onPress={() => setShowSpeakers(true)} />{canPlay && playRequested && <Button title={t('mobile.playback.stop')} subtle onPress={() => void run('playback', 'meetings.stopPlayback').then((value) => { if (value !== undefined) setPlayRequested(false); })} />}</Actions>
      {detail.metadata_status && detail.metadata_status !== 'valid' && <Notice>{t(`meetings.attribution_metadata_${detail.metadata_status}`)}</Notice>}
      <FlatList ref={transcriptList} data={detail.transcript} onScroll={(event) => { transcriptOffset.current = event.nativeEvent.contentOffset.y; }} scrollEventThrottle={32} initialNumToRender={12} maxToRenderPerBatch={12} windowSize={7} contentContainerStyle={{ paddingTop: theme.space[3], paddingBottom: theme.space[10] }} keyExtractor={(_segment, index) => rowKeys[index]} onScrollToIndexFailed={({ index, averageItemLength }) => { transcriptList.current?.scrollToOffset({ offset: averageItemLength * index, animated: !reducedMotion }); }} renderItem={({ item }) => {
        const label = transcriptSpeakerLabel(item, detail.speakers, t);
        const provisional = transcriptAttributionProvisional(item) ? ` · ${t('meetings.live_speakers_provisional_tag')}` : '';
        const source = item.source_id ? t(`meetings.attribution_source_${item.source_id}`) : '';
        const playback = transcriptPlayback(id, item, detail.audio_tracks, detail.metadata_status);
        return <Pressable accessibilityRole="button" accessibilityLabel={`${clock(item.start_ms / 1000)} ${label}${provisional}${source ? ` · ${source}` : ''}. ${item.text}`} accessibilityState={{ disabled: !playback }} accessibilityHint={playback ? t('meetings.play_from_here') : t('meetings.passage_audio_unavailable')} disabled={!playback} onPress={() => { if (playback) void run('playback', playback.method, playback.payload).then(value => { if (value !== undefined) setPlayRequested(true); }); }} style={{ gap: theme.space[1], minHeight: 44, paddingBottom: theme.space[5] }}><Body muted style={{ fontSize: theme.size.label }}>{clock(item.start_ms / 1000)} · {label}{provisional}{source ? ` · ${source}` : ''}</Body><Body selectable style={item.failed ? { color: theme.colors.accent } : undefined}>{item.failed ? t('meetings.transcription_failed') : item.text}</Body>{item.correction_review_required && <Body muted style={{ fontSize: theme.size.label }}>{t('meetings.attribution_correction_review')}</Body>}</Pressable>;
      }} ListEmptyComponent={<Empty title={t('meetings.empty_transcript')} />} />
    </>}
    {pane === 'questions' && <AskPanel meetingId={id} meetingUid={detail.meeting_uid} targetConversationId={origin ? undefined : params.conversation_id} targetMessageId={origin ? undefined : params.message_id}
      beforeAsk={draft.save} onInteraction={() => { ++interactions.current; }} />}
    {pane === 'insights' && <Section title={t('insights.title')}><Button title={t('meetings.notes')} subtle onPress={() => setPane('notes')} /></Section>}
    {exportShown && <MeetingExport key={id} id={id} visible onClose={() => setExportShown(false)} />}
    {pane === 'insights' && (active ? <Notice>{t('insights.finished_only')}</Notice> : <View onTouchStart={() => { ++interactions.current; }}><InsightReview model={insightModel} notes={insightNotes} /></View>)}
    <Modal visible={showSpeakers} presentationStyle="pageSheet" animationType={reducedMotion ? 'none' : 'slide'} allowSwipeDismissal onRequestClose={() => { setShowSpeakers(false); setEditor(null); }}><Page title={copy('manageSpeakers')} bottomInset recordingStatus={false} action={<Button title={t('common.close')} subtle onPress={() => { setShowSpeakers(false); setEditor(null); }} />}>
      {editor?.type === 'speaker' ? <Section description={t('mobile.speakers.name_help')}><Field label={t('mobile.speakers.name')} value={editor.initial} onChangeText={(initial) => setEditor((current) => current?.type === 'speaker' ? { ...current, initial } : current)} autoFocus /><Actions><Button title={t('common.save')} busy={busy.has('rename-speaker')} disabled={!canEditAttribution} onPress={() => { void run('rename-speaker', 'recognition.renameSpeaker', { id, voice: editor.voice, name: editor.initial.trim(), remember: false }).then((value) => { if (value !== undefined) { setEditor(null); return query.refresh(); } }); }} /><Button title={t('common.cancel')} subtle onPress={() => setEditor(null)} /></Actions></Section> : <>
      <Section title={t('mobile.speakers.choice')}>
        <Button title={t('mobile.speakers.manual')} busy={busy.has('recognition-start')} disabled={!speakerEnabled} onPress={recognize} />
        {!snapshot?.cloud.signed_in && <Button title={t('meetings.speaker_cloud_sign_in')} subtle onPress={() => { setShowSpeakers(false); router.push({ pathname: '/settings/[section]', params: { section: 'account' } }); }} />}
        {!canPlay && <Notice>{t('meetings.no_saved_audio')}</Notice>}
        {job && <><Notice error={job.stage === 'failed' || job.stage === 'response_unknown'}>{t(`mobile.speakers.${job.stage}`, { count: job.confirmed_chunks })}</Notice>{job.error && <Notice error>{localizedFailure(language, job.error)}</Notice>}<Actions>{recognitionActions(job).cancel && <Button title={t(job.error?.code === 'speaker_cleanup' ? 'mobile.speakers.cleanup' : 'common.cancel')} subtle onPress={() => void run<SpeakerJob>('recognition-cancel', 'recognition.cancel', { job_id: job.id }).then((value) => { if (value) setCurrentJob(speakerJob(value)); })} />}{recognitionActions(job).resume && <Button title={t('mobile.speakers.resume')} subtle busy={busy.has('recognition-resume')} onPress={resume} />}</Actions></>}
        {voices.map((voice) => <Row key={voice} disabled={!canEditAttribution} title={names.get(voice) || t('mobile.speakers.unknown', { number: voice })} onPress={() => setEditor({ type: 'speaker', voice, initial: names.get(voice) ?? '' })} trailing={detail.voices?.find((item) => item.voice === voice)?.can_remember && names.get(voice) ? <Button title={t('mobile.speakers.remember')} subtle disabled={!canEditAttribution} onPress={() => confirm(t('mobile.speakers.remember'), t('mobile.speakers.remember_help'), t('common.save'), t('common.cancel'), () => void run('remember-speaker', 'recognition.renameSpeaker', { id, voice, name: names.get(voice), remember: true }).then(() => query.refresh()))} /> : undefined} />)}
        {detail.transcript.some((segment) => segment.failed) && <Button title={t('common.retry')} subtle busy={busy.has('retry-transcription')} onPress={() => void run('retry-transcription', 'meetings.retryTranscription', { id }).then(() => query.refresh())} />}
      </Section>
      </>}
    </Page></Modal>
    <TextDialog visible={editor?.type === 'rename'} title={t(editor?.type === 'rename' ? 'meetings.rename_title' : 'meetings.voice_name')} label={t(editor?.type === 'rename' ? 'meetings.meeting_title_label' : 'mobile.speakers.name')} initial={editor?.type === 'speaker' ? editor.initial : detail.info.title} help={editor?.type === 'speaker' ? t('mobile.speakers.name_help') : undefined} onClose={() => setEditor(null)} onSave={async (value) => {
      if (!editor) return false;
      if (editor.type === 'rename') { if (!value.trim()) return false; const renamed = await saveThenRename(async () => await draft.save() && await summary.model.save(), () => run<string>('rename-meeting', 'meetings.rename', { id, title: value.trim() })); if (renamed !== undefined) { router.setParams({ id: renamed }); return true; } }
      else { const saved = await run('rename-speaker', 'recognition.renameSpeaker', { id, voice: editor.voice, name: value.trim(), remember: false }); if (saved !== undefined) { await query.refresh(); return true; } }
      return false;
    }} />
    <Modal visible={viewConflict !== null} presentationStyle="pageSheet" animationType={reducedMotion ? 'none' : 'slide'} allowSwipeDismissal onRequestClose={() => setViewConflict(null)}><Page title={viewConflict?.name} bottomInset recordingStatus={false}><Body selectable>{viewConflict?.text}</Body><Actions><Button title={t('common.close')} subtle onPress={() => setViewConflict(null)} /><Button title={t('common.delete')} subtle destructive onPress={() => confirm(t('meetings.delete_conflict_title'), t('meetings.file_will_delete', { name: viewConflict?.name ?? '' }), t('common.delete'), t('common.cancel'), () => { void run('conflict-delete', 'meetings.conflictDelete', { id, name: viewConflict?.name }).then(() => { setViewConflict(null); return query.refresh(); }); }, true)} /></Actions></Page></Modal>
  </Page>;
}
