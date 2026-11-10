export type SummaryConsentAction = "create" | "not_now" | "quiet_create";
export type SummaryPolicy = "manual" | "automatic";
export interface SummaryConsentEffects { savePolicy(policy: SummaryPolicy): Promise<void>; generate(): Promise<void>; dismiss(): Promise<void> }
export function summaryConsentDecision(action: SummaryConsentAction, automatic: boolean): { policy: SummaryPolicy | null; generate: boolean; dismiss: boolean } {
  if (action === "not_now") return { policy: null, generate: false, dismiss: true };
  return { policy: action === "quiet_create" ? null : automatic ? "automatic" : "manual", generate: true, dismiss: false };
}
export async function performSummaryConsent(action: SummaryConsentAction, automatic: boolean, effects: SummaryConsentEffects): Promise<void> {
  const decision = summaryConsentDecision(action, automatic);
  if (decision.policy) await effects.savePolicy(decision.policy);
  if (decision.dismiss) await effects.dismiss();
  if (decision.generate) await effects.generate();
}
