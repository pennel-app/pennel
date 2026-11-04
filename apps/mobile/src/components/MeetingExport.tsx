import React, { useState } from 'react';
import { Modal } from 'react-native';
import type { MeetingExport as ExportDocument } from '../domain';
import { useQuery, useReducedMotion, useRuntime } from '../runtime';
import { taskReadiness } from '../task-readiness';
import { SummaryDestination } from './SummaryAutomation';
import { Actions, Body, Button, Loading, NativeChoice, Notice, Page, Section, Toggle, useRecordingSheetReturn } from './ui';

export function MeetingExport({ id, visible, onClose }: { id: string; visible: boolean; onClose: () => void }) {
  const { t, run, busy, snapshot } = useRuntime();
  const reduced = useReducedMotion();
  const [scope, setScope] = useState<'shareable' | 'personal'>('shareable');
  const [includeTranscript, setIncludeTranscript] = useState(false);
  const [notice, setNotice] = useState(false);
  const recordingReturn = useRecordingSheetReturn(onClose);
  const preview = useQuery<ExportDocument>('meetings.export.preview', { id, scope, include_transcript: includeTranscript }, visible);
  const generate = async () => {
    const result = await run('shareable-summary', 'meetings.shareableSummary.generate', { id });
    if (result !== undefined) { setNotice(false); await preview.refresh(); }
  };
  const prepared = async (action: 'copy' | 'formatted' | 'share') => {
    const output = preview.value;
    if (!output || preview.loading || preview.error) return;
    const result = await run('meeting-export', 'meetings.export.prepared', { output, copy: action !== 'share', share: action === 'share', formatted: action === 'formatted' });
    if (result !== undefined && action !== 'share') setNotice(true);
  };
  return <Modal visible={visible && !recordingReturn.returning} presentationStyle="pageSheet" animationType={reduced ? 'none' : 'slide'} allowSwipeDismissal onRequestClose={onClose} onDismiss={recordingReturn.didDismiss}><Page title={t('mobile.simple.preview_title')} bottomInset onReturnToRecording={recordingReturn.requestReturn} action={<Button title={t('common.close')} subtle onPress={onClose} />}>
    <Section description={t(scope === 'shareable' ? 'mobile.simple.preview_shareable_help' : 'mobile.simple.preview_personal_help')}>
      <NativeChoice label={t('mobile.simple.preview_scope')} value={scope} options={[{ value: 'shareable', label: t('mobile.simple.preview_shareable') }, { value: 'personal', label: t('mobile.simple.preview_personal') }]} onChange={value => { setScope(value); setNotice(false); }} />
      <Toggle title={t('mobile.simple.preview_transcript')} value={includeTranscript} onChange={value => { setIncludeTranscript(value); setNotice(false); }} />
    </Section>
    {scope === 'shareable' && <Section description={t('mobile.simple.preview_generate_help')}><SummaryDestination inputs={false} /><Button title={t('mobile.simple.preview_generate')} subtle busy={busy.has('shareable-summary')} disabled={!taskReadiness(snapshot, 'llm').ready} onPress={() => void generate()} /></Section>}
    {preview.loading && <Loading />}
    {preview.error && <Section><Notice error>{preview.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void preview.refresh()} /></Section>}
    {preview.value && !preview.loading && !preview.error && <Section description={t('mobile.simple.preview_exact')}><Body selectable>{preview.value.markdown}</Body></Section>}
    <Actions><Button title={t('common.copy')} disabled={!preview.value || preview.loading || Boolean(preview.error) || busy.has('shareable-summary')} busy={busy.has('meeting-export')} onPress={() => void prepared('copy')} /><Button title={t('meetings.copy_formatted')} subtle disabled={!preview.value || preview.loading || Boolean(preview.error) || busy.has('shareable-summary')} onPress={() => void prepared('formatted')} /><Button title={t('meetings.share_action')} subtle disabled={!preview.value || preview.loading || Boolean(preview.error) || busy.has('shareable-summary')} onPress={() => void prepared('share')} /></Actions>
    {notice && <Notice>{t('meetings.copied')}</Notice>}
  </Page></Modal>;
}
