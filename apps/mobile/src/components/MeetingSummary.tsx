import React, { useState } from 'react';
import { Modal } from 'react-native';
import { useRouter } from 'expo-router';
import type { MeetingSummaryCandidate, MeetingSummaryDocument, MeetingSummaryRevision, MeetingTemplate } from '../domain';
import { useQuery, useReducedMotion, useRuntime } from '../runtime';
import { taskReadiness } from '../task-readiness';
import { useMeetingSummary } from '../use-meeting-summary';
import { dateLabel } from '../format';
import { templateLabel } from '../owned-labels';
import { SummaryDestination, summaryDestination } from './SummaryAutomation';
import { ActionMenu, Actions, Body, Button, Field, Loading, NativeChoice, Notice, Page, Row, Section, useRecordingSheetReturn } from './ui';
import { Markdown } from './Markdown';

export function MeetingSummary({ id, summary, active, sourceBusy, beforeGenerate }: { id: string; summary: ReturnType<typeof useMeetingSummary>; active: boolean; sourceBusy: boolean; beforeGenerate: () => Promise<boolean> }) {
  const { run, busy, t, snapshot, language, store } = useRuntime();
  const router = useRouter();
  const reduced = useReducedMotion();
  const [editing, setEditing] = useState(false);
  const [historyShown, setHistoryShown] = useState(false);
  const [selectedVersion, setSelectedVersion] = useState<MeetingSummaryRevision | null>(null);
  const [candidate, setCandidate] = useState<MeetingSummaryCandidate | null>(null);
  const candidateReturn = useRecordingSheetReturn(() => setCandidate(null));
  const historyReturn = useRecordingSheetReturn(() => setHistoryShown(false));
  const [candidateDestination, setCandidateDestination] = useState('');
  const [candidateChanged, setCandidateChanged] = useState(false);
  const [chooseTemplate, setChooseTemplate] = useState(false);
  const [template, setTemplate] = useState(snapshot?.settings.default_template ?? 'algemeen');
  const templates = useQuery<MeetingTemplate[]>('meetings.templates', {}, chooseTemplate);
  const history = useQuery<MeetingSummaryRevision[]>('meetings.summary.history', { id }, historyShown);
  const ready = taskReadiness(snapshot, 'llm').ready;
  const reviewBlocked = active || sourceBusy || editing || summary.query.loading || Boolean(summary.query.error) || Boolean(candidate) || busy.has(`summary-generate-${id}`) || busy.has(`summary-restore-${id}`) || busy.has(`summary-load-${id}`);
  const markReviewed = async () => {
    if (reviewBlocked || !summary.model.canMarkReviewed()) return;
    await summary.model.markReviewed();
    await summary.query.refresh();
  };
  const generate = async () => {
    if (!await beforeGenerate() || summary.dirty || summary.saving) return;
    const requestedDestination = summaryDestination(store.getSnapshot().snapshot, t);
    const result = await run<MeetingSummaryCandidate>(`summary-generate-${id}`, 'meetings.summary.generate', { id, template });
    if (result) { setCandidateChanged(false); setCandidateDestination(requestedDestination); setCandidate(result); await summary.query.refresh(); }
  };
  const restore = async () => {
    if (!summary.document || !selectedVersion || summary.dirty) return;
    const result = await run<MeetingSummaryDocument>(`summary-restore-${id}`, 'meetings.summary.restore', { id, revision: selectedVersion.revision, expected_revision: summary.document.revision, expected_source_revision: summary.document.source_revision });
    if (result) { summary.model.adopt(result); setSelectedVersion(null); setHistoryShown(false); await summary.query.refresh(); }
  };
  const loadCurrent = async () => {
    const current = await run<MeetingSummaryDocument>(`summary-load-${id}`, 'meetings.summary', { id });
    if (current) { summary.model.adopt(current); summary.model.discard(); }
  };
  return <>
    <Section title={t('mobile.simple.summary')} description={t('mobile.simple.summary_help')}>
      {summary.query.loading && !summary.document && <Loading />}
      {summary.query.error && <><Notice error>{summary.query.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void summary.query.refresh()} /></>}
      {summary.document && <>
        {summary.document.outdated && <Notice>{t('meetings.summary_outdated')}</Notice>}
        {editing || summary.dirty ? <Field label={t('mobile.simple.summary')} multiline value={summary.draft} onChangeText={summary.model.edit} style={{ minHeight: 180 }} /> : summary.draft ? <Markdown text={summary.draft} /> : <Body muted>{t('meetings.empty_summary')}</Body>}
        {summary.conflict && <Notice error>{t('mobile.simple.summary_conflict')}</Notice>}
        {summary.failed && summary.dirty && <Notice error>{t('mobile.simple.summary_unsaved')}</Notice>}
        <Actions>
          {summary.document.outdated && summary.document.markdown.trim() && <Button title={t('meetings.summary_mark_reviewed')} subtle busy={summary.saving && !summary.dirty} disabled={reviewBlocked || !summary.model.canMarkReviewed()} onPress={() => void markReviewed()} />}
          {summary.dirty && <Button title={t('common.save')} busy={summary.saving} disabled={summary.conflict} onPress={() => void summary.model.save().then(saved => { if (saved) setEditing(false); return summary.query.refresh(); })} />}
          <Button title={t('mobile.simple.summary_edit')} subtle disabled={summary.saving} onPress={() => setEditing(value => !value)} />
          <ActionMenu label={t('mobile.simple.tools')} actions={[
            { label: t('mobile.simple.summary_history'), disabled: summary.dirty || summary.saving, onPress: () => { setSelectedVersion(null); setHistoryShown(true); } },
            { label: t('meetings.template'), onPress: () => setChooseTemplate(value => !value) },
            { label: t('common.copy'), onPress: () => void run('copy-summary-draft', 'dictation.copy', { text: summary.draft }) },
            { label: t('mobile.simple.summary_load'), disabled: summary.saving || !summary.conflict && !summary.failed, onPress: () => void loadCurrent() },
          ]} />
        </Actions>
        {!summary.dirty && <>
          {chooseTemplate && <NativeChoice label={t('meetings.template')} value={template} options={(templates.value ?? []).map(value => ({ value: value.id, label: templateLabel(value, language) }))} onChange={setTemplate} />}
          <SummaryDestination />
          <Button title={t(summary.draft ? 'mobile.simple.summary_regenerate' : 'mobile.simple.summary_generate')} subtle busy={busy.has(`summary-generate-${id}`)} disabled={active || !ready || summary.saving} onPress={() => void generate()} />
        </>}
      </>}
      {!ready && <><Notice>{t('meetings.questions_setup_help')}</Notice><Button title={t('meetings.set_up_summaries')} subtle onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'models', task: 'llm' } })} /></>}
    </Section>
    <Modal visible={candidate !== null && !candidateReturn.returning} presentationStyle="pageSheet" animationType={reduced ? 'none' : 'slide'} allowSwipeDismissal onRequestClose={() => setCandidate(null)} onDismiss={candidateReturn.didDismiss}><Page title={t('mobile.simple.summary_preview')} bottomInset onReturnToRecording={candidateReturn.requestReturn} action={<Button title={t('common.close')} subtle onPress={() => setCandidate(null)} />}>
      <Section description={t('mobile.simple.summary_candidate_help')}><SummaryDestination destination={candidateDestination} requested />{candidate && <Markdown text={candidate.markdown} />}</Section>
      {candidateChanged && <Notice error>{t('mobile.simple.summary_conflict')}</Notice>}
      <Actions><Button title={t('mobile.simple.summary_accept')} busy={summary.saving} onPress={() => { if (candidate) void summary.model.accept(candidate).then(async saved => { if (saved) { setCandidate(null); await summary.query.refresh(); } else { setCandidateChanged(true); await summary.query.refresh(); } }); }} /><Button title={t('mobile.simple.summary_keep')} subtle onPress={() => setCandidate(null)} /></Actions>
    </Page></Modal>
    <Modal visible={historyShown && !historyReturn.returning} presentationStyle="pageSheet" animationType={reduced ? 'none' : 'slide'} allowSwipeDismissal onRequestClose={() => setHistoryShown(false)} onDismiss={historyReturn.didDismiss}><Page title={t('mobile.simple.summary_history')} bottomInset onReturnToRecording={historyReturn.requestReturn} action={<Button title={t('common.close')} subtle onPress={() => setHistoryShown(false)} />}>
      {history.loading && <Loading />}{history.error && <><Notice error>{history.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void history.refresh()} /></>}
      {selectedVersion ? <Section description={t('mobile.simple.summary_restore_help')}><Markdown text={selectedVersion.markdown} /><Button title={t('mobile.simple.summary_restore')} busy={busy.has(`summary-restore-${id}`)} disabled={summary.dirty} onPress={() => void restore()} /><Button title={t('common.back')} subtle onPress={() => setSelectedVersion(null)} /></Section> : <Section>{history.value?.map(version => <Row key={version.revision} title={t('mobile.simple.summary_version', { date: dateLabel(version.created_at, language) })} onPress={() => setSelectedVersion(version)} />)}{!history.loading && history.value?.length === 0 && <Notice>{t('mobile.simple.summary_history_empty')}</Notice>}</Section>}
    </Page></Modal>
  </>;
}
