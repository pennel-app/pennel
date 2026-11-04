import React from 'react';
import type { RuntimeSnapshot, Settings, SummaryAutomation as Automation } from '../domain';
import { useRuntime } from '../runtime';
import { processingEndpoint, taskReadiness } from '../task-readiness';
import { NativeChoice, Notice, Section } from './ui';

export function summaryDestination(snapshot: RuntimeSnapshot | null, t: (key: string) => string): string {
  const selected = taskReadiness(snapshot, 'llm');
  const endpoint = processingEndpoint(selected.provider?.base_url);
  return selected.provider ? `${selected.provider.name} · ${selected.model}${endpoint ? ` · ${endpoint}` : ''}` : t('mobile.simple.destination_none');
}

export function SummaryDestination({ inputs = true, destination: frozen, requested = false }: { inputs?: boolean; destination?: string; requested?: boolean }) {
  const { snapshot, t } = useRuntime();
  const destination = frozen ?? summaryDestination(snapshot, t);
  return <>{inputs && <Notice>{t('mobile.simple.summary_inputs')}</Notice>}<Notice>{t(requested ? 'mobile.simple.destination_requested' : 'mobile.simple.destination', { destination })}</Notice></>;
}

export function SummaryAutomation() {
  const { snapshot, run, t, busy } = useRuntime();
  if (!snapshot) return null;
  return <Section title={t('mobile.simple.automation')} description={t('mobile.simple.automation_help')}>
    <NativeChoice<Automation> label={t('mobile.simple.automation')} value={snapshot.settings.summary_automation ?? 'unanswered'} options={(['unanswered', 'manual', 'automatic'] as const).map(value => ({ value, label: t(`mobile.simple.automation_${value}`), disabled: busy.has('summary-automation') }))} onChange={summary_automation => void run<Settings>('summary-automation', 'settings.update', { patch: { summary_automation } })} />
    <SummaryDestination />
  </Section>;
}
