import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useFocusEffect, useLocalSearchParams, useRouter } from 'expo-router';
import { Pressable, SectionList, View } from 'react-native';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useQuery, useRuntime } from '../runtime';
import type { MeetingDetail, MeetingInfo, RuntimeSnapshot } from '../domain';
import { clock, dateLabel, meetingGroups } from '../format';
import { Body, Button, Empty, Field, Loading, NativeChoice, Notice, Page, Row, Section } from '../components/ui';
import { AskPanel } from '../components/AskPanel';
import { useHapticFeedback } from '../haptics';
import { recordingPresentation } from '../workflow-state';
import { workflowText } from '../workflow-copy';

export default function Meetings() {
  const { snapshot, connecting, run, busy, t, theme, language, store } = useRuntime();
  const [query, setQuery] = useState('');
  const [title, setTitle] = useState('');
  const [search, setSearch] = useState('');
  const router = useRouter();
  const params = useLocalSearchParams<{ pane?: string; conversation_id?: string; message_id?: string }>();
  const [workspace, setWorkspace] = useState<'library' | 'questions'>(params.pane === 'questions' ? 'questions' : 'library');
  const [questionMeetingId, setQuestionMeetingId] = useState('');
  useEffect(() => { if (params.pane === 'questions') { setWorkspace('questions'); setQuestionMeetingId(''); } }, [params.pane, params.conversation_id, params.message_id]);
  const haptic = useHapticFeedback();
  const meetings = useQuery<MeetingInfo[]>('meetings.list', { query: search }, Boolean(snapshot?.ready));
  const questionMeeting = useQuery<MeetingDetail>('meetings.read', { id: questionMeetingId }, Boolean(snapshot?.ready && questionMeetingId));
  useEffect(() => { const timer = setTimeout(() => setSearch(query.trim()), 180); return () => clearTimeout(timer); }, [query]);
  useFocusEffect(useCallback(() => { void meetings.refresh(); void store.refresh(); }, [meetings.refresh, store]));
  const phase = snapshot?.meeting.phase;
  useEffect(() => { if (phase === 'idle') void meetings.refresh(); }, [meetings.refresh, phase]);
  const active = Boolean(snapshot?.meeting.id && phase !== 'idle');
  const retainedId = snapshot?.meeting.retained_id;
  const sections = useMemo(() => meetingGroups((meetings.value ?? []).filter((meeting) => meeting.id !== snapshot?.meeting.id || !active)).map((group) => ({ key: group.key, title: t(`meetings.group_${group.key}`), data: group.meetings })), [active, meetings.value, snapshot?.meeting.id, t]);
  const start = async () => {
    if (!snapshot?.ready || busy.has('meeting-start')) return;
    if (active) { router.push('/recording'); return; }
    if (retainedId) return;
    const value = await run<RuntimeSnapshot>('meeting-start', 'meetings.start', { title: title.trim() });
    if (value !== undefined) { setTitle(''); haptic('recordingStarted'); router.push('/recording'); }
    else haptic('error');
  };
  const workspaceControl = <View style={{ marginBottom: theme.space[5] }}><NativeChoice label={t('navigation.meetings')} segmented value={workspace} options={[{ value: 'library', label: t('mobile.simple.library') }, { value: 'questions', label: t('mobile.simple.questions') }]} onChange={value => { setWorkspace(value); router.setParams({ pane: value }); }} /></View>;
  if (workspace === 'questions') return <Page title={t('navigation.meetings')}>
    {workspaceControl}
    <Section description={t('mobile.simple.scope_help')}><NativeChoice label={t('mobile.simple.scope')} value={questionMeetingId} options={[{ value: '', label: t('mobile.simple.scope_all') }, ...(meetings.value ?? []).map(meeting => ({ value: meeting.id, label: meeting.title }))]} onChange={setQuestionMeetingId} /></Section>
    {questionMeetingId && questionMeeting.loading && <Loading />}
    {questionMeeting.error && <><Notice error>{questionMeeting.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void questionMeeting.refresh()} /></>}
    {snapshot?.ready && (!questionMeetingId || questionMeeting.value?.meeting_uid) && <AskPanel meetingId={questionMeetingId || undefined} meetingUid={questionMeeting.value?.meeting_uid} targetConversationId={!questionMeetingId ? params.conversation_id : undefined} targetMessageId={!questionMeetingId ? params.message_id : undefined} />}
  </Page>;
  const header = <>
    {workspaceControl}
    {connecting && !snapshot && <Loading />}
    {!snapshot && !connecting && <Section><Notice error>{t('mobile.unavailable')}</Notice><Button title={t('common.retry')} subtle onPress={() => void store.connect()} /></Section>}
    <Section title={t('meetings.new_meeting')} description={t('mobile.recording.help')}>
      {!active && <Field label={t('meetings.meeting_title_label')} placeholder={t('meetings.meeting_title_placeholder')} value={title} onChangeText={setTitle} returnKeyType="go" onSubmitEditing={() => void start()} />}
      <Button title={active ? t('meetings.return_recording') : t('common.start_recording')} icon={active ? 'pulse' : 'add'} busy={busy.has('meeting-start')} disabled={!snapshot?.ready || Boolean(retainedId)} onPress={() => void start()} />
      {active && <Row title={snapshot?.meeting.title || t('meetings.default_title')} subtitle={`${workflowText(language, recordingPresentation(phase).status)} · ${clock(snapshot?.meeting.elapsed_secs ?? 0)}`} onPress={() => router.push('/recording')} trailing={phase === 'recording' ? <View style={{ width: theme.space[2], height: theme.space[2], borderRadius: theme.radius.pill, backgroundColor: theme.colors.accent }} /> : undefined} />}
    </Section>
    {retainedId && <Section title={t('meetings.retry_save')} description={t('mobile.recording.unsaved')}><Button title={t('meetings.retry_save')} busy={busy.has('meeting-retry-save')} onPress={() => void run<RuntimeSnapshot>('meeting-retry-save', 'meetings.retrySave', { id: retainedId }).then((result) => { if (result) { haptic('meetingSaved'); return meetings.refresh(); } })} /></Section>}
    <Field label={t('meetings.search_label')} value={query} onChangeText={setQuery} clearButtonMode="while-editing" autoCorrect={false} returnKeyType="search" />
    <View style={{ height: theme.space[4] }} />
    {meetings.loading && !meetings.value && <Loading />}
    {meetings.error && <Section><Notice error>{meetings.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void meetings.refresh()} /></Section>}
  </>;
  return <Page title={t('navigation.meetings')} scroll={false} action={<Pressable accessibilityRole="button" accessibilityLabel={t('mobile.refresh')} onPress={() => void meetings.refresh()} style={{ padding: theme.space[2], minHeight: 44, minWidth: 44, alignItems: 'center', justifyContent: 'center' }}><Ionicons accessible={false} name="refresh-outline" color={theme.colors.foreground} size={22} /></Pressable>}>
    <SectionList sections={sections} keyExtractor={(meeting) => meeting.id} ListHeaderComponent={header} stickySectionHeadersEnabled={false} keyboardShouldPersistTaps="handled" contentInsetAdjustmentBehavior="automatic" automaticallyAdjustKeyboardInsets contentContainerStyle={{ paddingBottom: theme.space[10] }} refreshing={meetings.loading && Boolean(meetings.value)} onRefresh={() => void meetings.refresh()} initialNumToRender={12} maxToRenderPerBatch={12} windowSize={7} renderSectionHeader={({ section }) => <Body accessibilityRole="header" style={{ fontFamily: theme.font.semibold, paddingTop: theme.space[4], paddingBottom: theme.space[2] }}>{section.title}</Body>} renderItem={({ item }) => <Row title={item.title} subtitle={`${dateLabel(item.start_iso, language)} · ${t('meetings.duration_minutes', { count: item.duration_min })}${item.sync_state ? ` · ${t(`meetings.sync.${item.sync_state}`)}` : ''}`} onPress={() => router.push({ pathname: '/meeting/[id]', params: { id: item.id } })} />} ListEmptyComponent={!meetings.loading && !meetings.error ? <Empty title={t(search ? 'meetings.none_found' : 'meetings.empty_meetings')} description={!search ? t('meetings.empty_meetings_help') : undefined} /> : undefined} />
  </Page>;
}
