import { api } from "./api";
import type { InsightReviewService } from "../../../assets/contracts/insight-review-model.ts";

export const insightService: InsightReviewService = {
  read: api.insightsSnapshot, availability: api.insightsAvailability, start: api.beginInsightsAnalysis,
  job: api.insightsJob, cancel: api.cancelInsightsAnalysis, claim: api.claimInsightEdit, review: api.reviewInsight,
  prepare: api.prepareInsightInsertion, reconcile: api.reconcileInsightInsertions, resolve: api.resolveInsightInsertion, reassociate: api.reassociateInsightSource,
  export: api.exportInsights, itemMarkdown: api.insightMarkdown,
};
