import type { FinishedSuggestionPreset, InferenceConfig, ManagedMeetingReviewProfile, ProviderAccount } from "./api";
import { testedSuggestionDraft } from "./inference-state.ts";

const cloudEndpoints = new Set(["https://api.pennel.app/v1/decisions", "https://api-staging.pennel.app/v1/decisions"]);

export type ReviewProcessingSource = "cloud" | "local" | "advanced";

export function reviewProcessingSource(config: InferenceConfig | null | undefined, providers: ProviderAccount[]): ReviewProcessingSource | null {
  if (!config) return null;
  const account = providers.find(item => item.id === config.provider_id);
  if (account?.kind === "pennel_cloud" && config.protocol === "pennel_cloud" && config.model === "pennel-review-v3") return "cloud";
  if (account?.kind === "local" && config.protocol === "local_decider") return "local";
  return "advanced";
}

export function validManagedReviewProfile(value: unknown): value is ManagedMeetingReviewProfile {
  if (!value || typeof value !== "object") return false;
  const profile = value as ManagedMeetingReviewProfile;
  return profile.available === true && profile.profile === "supported_passages_v3" && profile.model === "pennel-review-v3"
    && profile.configured_model === "perplexity/pplx-decider-v1.2" && profile.upstream === "perplexity"
    && /^perplexity\/pplx-decider-v1\.2-\d{8}$/.test(profile.expected_model ?? "")
    && cloudEndpoints.has(profile.endpoint ?? "") && profile.limits?.max_input_bytes === 65536 && profile.limits?.max_questions === 64
    && profile.limits?.max_batch_windows === 8 && profile.limits?.request_timeout_secs === 35;
}

export function cloudReviewDraft(profile: unknown, presets: FinishedSuggestionPreset[], account: ProviderAccount | undefined): InferenceConfig | null {
  if (account && account.kind !== "pennel_cloud" || !validManagedReviewProfile(profile)) return null;
  const preset = presets.find(item => item.provider_kind === "pennel_cloud" && item.configured?.endpoint === profile.endpoint
    && item.configured?.expected_model === profile.expected_model && item.configured?.model === profile.model);
  // An empty draft id requests atomic account creation on explicit native Save.
  return preset ? testedSuggestionDraft(preset, account ?? { id: "", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "" }) : null;
}
