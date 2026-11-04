import { useEffect, useMemo, useSyncExternalStore } from 'react';
import type { MeetingSummaryDocument } from './domain';
import { useQuery, useRuntime } from './runtime';
import { SummaryDraft } from './summary-draft';

export function useMeetingSummary(id: string) {
  const { run, snapshot } = useRuntime();
  const query = useQuery<MeetingSummaryDocument>('meetings.summary', { id }, Boolean(id && snapshot?.ready));
  const model = useMemo(() => new SummaryDraft(
    (base, markdown) => run<MeetingSummaryDocument>(`summary-save-${id}`, 'meetings.summary.save', { id, markdown, expected_revision: base.revision, expected_source_revision: base.source_revision }),
    (base, markdown) => run<MeetingSummaryDocument>(`summary-accept-${id}`, 'meetings.summary.accept', { id, markdown, expected_revision: base.revision, expected_source_revision: base.source_revision }),
  ), [id, run]);
  const state = useSyncExternalStore(model.subscribe, model.getSnapshot, model.getSnapshot);
  useEffect(() => { if (query.value) model.adopt(query.value); }, [model, query.value]);
  return { ...state, model, query };
}
