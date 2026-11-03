import type { RuntimeClient } from './domain.ts';
import type { InsightReviewService } from '../../../assets/contracts/insight-review-model.ts';

export function mobileInsightService(client: Pick<RuntimeClient, 'invoke'>): InsightReviewService {
  return {
    read: id => client.invoke('insights.read', { id }),
    availability: (id, language_policy) => client.invoke('insights.availability', { id, language_policy }),
    start: (id, language_policy) => client.invoke('insights.start', { id, language_policy }),
    job: job_id => client.invoke('insights.job', { job_id }), cancel: job_id => client.invoke('insights.cancel', { job_id }),
    claim: (id, request) => client.invoke('insights.claimEdit', { id, request }), review: (id, request) => client.invoke('insights.review', { id, request }),
    prepare: (id, request) => client.invoke('insights.prepareInsertion', { id, request }), reconcile: (id, store_revision) => client.invoke('insights.reconcile', { id, store_revision }),
    reassociate: (id, request) => client.invoke('insights.reassociate', { id, request }), export: id => client.invoke('insights.export', { id }),
    itemMarkdown: (id, item_id, item_revision) => client.invoke('insights.itemMarkdown', { id, item_id, item_revision }),
  };
}
