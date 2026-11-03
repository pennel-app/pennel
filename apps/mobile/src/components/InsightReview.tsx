import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { AppState, Modal, View } from 'react-native';
import { useRuntime, useReducedMotion } from '../runtime';
import { Body, Button, Actions, Field, NativeChoice, Notice, Page, Section } from './ui';
import type { MeetingDetail } from '../domain';
import { clock } from '../format';
import { insightReceipt, insightSourceMatches, insightText, insightsWritable, safeInsightDestination, sourceCandidates,
  type InsightItem, type InsightSource } from '../../../../assets/contracts/meeting-insights.ts';
import { InsightReviewModel, InsightUiError, type InsightNoteHost } from '../../../../assets/contracts/insight-review-model.ts';

const reasonCodes = new Set(['unknown_language', 'unconfigured', 'unqualified', 'configuration_changed', 'identity_changed', 'mutation_in_progress', 'meeting_active', 'meeting_busy', 'source_unavailable', 'no_transcript', 'context_too_large', 'cancelled', 'deadline_exceeded', 'failed']);
export function InsightReview({ model, notes, onDirtyChange }: { model: InsightReviewModel; notes: InsightNoteHost; onDirtyChange?: (dirty: boolean) => void }) {
  const { store, t, theme } = useRuntime();
  const reducedMotion = useReducedMotion();
  const state = useSyncExternalStore(model.subscribe, model.getSnapshot, model.getSnapshot);
  const [showDismissed, setShowDismissed] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [sourceView, setSourceView] = useState<{ source: InsightSource; matched: boolean; text: string } | null>(null);
  const [choices, setChoices] = useState<{ item: InsightItem; sourceIndex: number; sources: InsightSource[]; chosen: string } | null>(null);
  useEffect(() => { void Promise.all([model.load(), model.choosePolicy(model.getSnapshot().policy)]); }, [model]);
  useEffect(() => { onDirtyChange?.(model.dirty); }, [model, state.version, onDirtyChange]);
  useEffect(() => {
    const timer = setInterval(() => { if (AppState.currentState === 'active') void model.poll(); }, 750);
    const listener = AppState.addEventListener('change', value => { if (value === 'active') void model.load(); });
    return () => { clearInterval(timer); listener.remove(); };
  }, [model]);
  const openSource = async (source: InsightSource) => {
    try {
      const detail = await store.client.invoke<MeetingDetail>('meetings.read', { id: model.id });
      const row = detail.transcript.find(row => row.utterance_id === source.utterance_id);
      const matched = insightSourceMatches(source, row, detail.meeting_uid ?? model.meetingUid);
      setSourceView({ source, matched, text: matched ? row!.text : source.quote });
    } catch { setSourceView({ source, matched: false, text: source.quote }); }
  };
  const reassociate = async (item: InsightItem, sourceIndex: number) => {
    try {
      const detail = await store.client.invoke<MeetingDetail>('meetings.read', { id: model.id });
      setChoices({ item, sourceIndex, sources: sourceCandidates(item.sources[sourceIndex], detail.transcript, detail.meeting_uid ?? model.meetingUid), chosen: '' });
    } catch { setNotice(t('insights.operation_failed')); }
  };
  const readonly = !insightsWritable(state.snapshot);
  const writing = state.busy.has('write');
  const destination = safeInsightDestination(state.availability?.endpoint);
  const error = state.error instanceof InsightUiError && ['notes_changed', 'notes_not_saved', 'edit_conflict', 'wrong_meeting'].includes(state.error.code)
    ? `insights.error_${state.error.code}` : 'insights.operation_failed';
  const reason = state.edits.size ? 'insights.finish_edit' : state.availability?.available ? 'insights.qualified'
    : `insights.reason_${reasonCodes.has(state.availability?.reason_code ?? '') ? state.availability!.reason_code : 'unavailable'}`;
  return <>
    <Section title={t('insights.title')} description={t('insights.description')}>
      <NativeChoice label={t('insights.language')} value={state.policy} options={(['unknown', 'en', 'nl', 'mixed'] as const).map(value => ({ value, label: t(`insights.language_${value}`) }))} onChange={value => void model.choosePolicy(value)} />
      <Body muted>{t('insights.destination', { destination: destination.host ? destination.on_device ? t('insights.this_device', { host: destination.host }) : destination.host : t('insights.none'), model: state.availability?.model ?? t('insights.none') })}</Body>
      <Notice>{readonly && state.snapshot ? t(`insights.store_${state.snapshot.status}`) : t(reason)}</Notice>
      <Actions><Button title={t('insights.find')} disabled={!model.canAnalyze} busy={state.busy.has('analysis')} onPress={() => void model.analyze()} />{model.analysisBusy && <Button title={t('insights.cancel_analysis')} subtle busy={state.busy.has('cancel')} onPress={() => void model.cancel()} />}</Actions>
      <Actions><Button title={t('insights.refresh')} subtle onPress={() => { void Promise.all([model.load(), model.choosePolicy(state.policy)]); }} /><Button title={t('insights.copy_markdown')} subtle disabled={readonly || model.dirty || writing || !state.snapshot?.items.some(item => item.review_state === 'accepted')} onPress={() => { void model.export().then(async markdown => { if (markdown !== undefined) { await store.client.invoke('dictation.copy', { text: markdown }); setNotice(t('insights.copied')); } }); }} /></Actions>
      {state.job && <Notice>{t(`insights.job_${state.job.state}`)}{state.job.physical_running && state.job.state === 'cancelled' ? ` ${t('insights.worker_finishing')}` : ''}</Notice>}
      {state.error != null && <Notice error>{t(error)}</Notice>}{notice && <Notice>{notice}</Notice>}
      <Button title={t(showDismissed ? 'insights.hide_dismissed' : 'insights.show_dismissed')} subtle onPress={() => setShowDismissed(value => !value)} />
    </Section>
    {model.items().filter(item => showDismissed || item.review_state !== 'dismissed' || state.edits.has(item.id)).map(item => {
      const edit = state.edits.get(item.id), receipt = insightReceipt(state.snapshot, item.id);
      return <Section key={item.id} title={t(`insights.kind_${item.kind}`)}>
        <Body muted>{t(`insights.review_${item.review_state}`)} · {t(`insights.commitment_${item.commitment}`)}{item.edited_text !== null ? ` · ${t('insights.edited')}` : ''}</Body>
        {edit ? <Field label={t('insights.edit_label')} multiline value={edit.text} onChangeText={text => model.edit(item.id, text)} onEndEditing={() => void model.saveEdit(item.id, false)} style={{ minHeight: 120 }} /> : <Body selectable>{insightText(item)}</Body>}
        <Body muted>{t('insights.owner')}: {item.owner?.text ?? t('insights.unknown')} · {t('insights.due')}: {item.due?.text ?? t('insights.unknown')}</Body>
        <Body muted>{t('insights.cutoff', { time: clock(item.evidence_cutoff_ms / 1000) })}</Body>
        {item.sources.map((source, index) => <View key={`${source.utterance_id}:${index}`} style={{ gap: theme.space[2] }}><Button title={`${t('insights.open_source')} · ${clock(source.start_ms / 1000)}`} subtle onPress={() => void openSource(source)} /><Body selectable>{source.quote}</Body>{item.source_state !== 'current' && <Button title={t('insights.reassociate')} subtle disabled={readonly || writing || !!edit} onPress={() => void reassociate(item, index)} />}</View>)}
        {item.source_state !== 'current' && <Notice>{t('insights.source_changed')}</Notice>}
        {edit?.conflict && <Notice error>{t('insights.error_edit_conflict')}</Notice>}
        {item.source_history?.length > 0 && <Section title={t('insights.source_history')}>{item.source_history.map((source, index) => <Body selectable key={`${source.utterance_id}:${index}`}>{source.quote}</Body>)}</Section>}
        {receipt && <Notice>{t(`insights.insertion_${receipt.state}`)}</Notice>}
        <Actions>{edit ? <><Button title={t('insights.save_edit')} disabled={readonly || writing || !edit.text.trim()} onPress={() => void model.saveEdit(item.id)} /><Button title={t('common.cancel')} subtle disabled={readonly || writing} onPress={() => void model.cancelEdit(item.id)} /></> : <>
          {item.review_state === 'proposed' && <Button title={t('insights.accept')} disabled={readonly || writing} onPress={() => void model.review(item.id, 'accepted')} />}
          <Button title={t('insights.edit')} subtle disabled={readonly || writing} onPress={() => void model.beginEdit(item.id)} />
          <Button title={t(item.review_state === 'dismissed' ? 'insights.restore' : 'insights.dismiss')} subtle disabled={readonly || writing} onPress={() => void model.review(item.id, item.review_state === 'dismissed' ? 'proposed' : 'dismissed')} />
          {item.review_state === 'accepted' && <Button title={t('insights.add_notes')} disabled={readonly || writing || item.source_state !== 'current' || !!receipt && receipt.state !== 'not_inserted'} onPress={() => void model.insert(item.id, notes)} />}
        </>}{receipt && ['pending', 'needs_review'].includes(receipt.state) && <Button title={t('insights.check_insertion')} subtle disabled={readonly || writing} onPress={() => void model.reconcile()} />}</Actions>
      </Section>;
    })}
    {model.items().length === 0 && <Body muted>{t('insights.empty')}</Body>}
    <Modal visible={sourceView !== null} presentationStyle="pageSheet" animationType={reducedMotion ? 'none' : 'slide'} onRequestClose={() => setSourceView(null)}>
      <Page title={sourceView ? `${t('insights.open_source')} · ${clock(sourceView.source.start_ms / 1000)}` : undefined} bottomInset>
        {sourceView && <>{!sourceView.matched && <Notice>{t('insights.source_changed')}</Notice>}<Body selectable>{sourceView.text}</Body></>}
        <Button title={t('insights.back')} subtle onPress={() => setSourceView(null)} />
      </Page>
    </Modal>
    <Modal visible={choices !== null} presentationStyle="pageSheet" animationType={reducedMotion ? 'none' : 'slide'} onRequestClose={() => setChoices(null)}>
      <Page title={t('insights.reassociate')} bottomInset>
        <Body>{t('insights.reassociate_help')}</Body>
        {choices?.sources.length ? <NativeChoice label={t('insights.choose_source')} value={choices.chosen} options={[{ value: '', label: t('insights.choose_source') }, ...choices.sources.map((source, index) => ({ value: String(index), label: `${clock(source.start_ms / 1000)} · ${source.quote.slice(0, 80)} (${index + 1})` }))]} onChange={value => setChoices(current => current && ({ ...current, chosen: value }))} /> : <Notice>{t('insights.no_matching_source')}</Notice>}
        <Actions><Button title={t('insights.use_source')} disabled={!choices || choices.chosen === ''} onPress={() => { if (choices) { const source = choices.sources[Number(choices.chosen)]; void model.reassociate({ item_id: choices.item.id, item_revision: choices.item.revision, source_index: choices.sourceIndex, source, evidence_cutoff_ms: Math.max(choices.item.evidence_cutoff_ms, source.end_ms) }).then(() => setChoices(null)); } }} /><Button title={t('common.cancel')} subtle onPress={() => setChoices(null)} /></Actions>
      </Page>
    </Modal>
  </>;
}
