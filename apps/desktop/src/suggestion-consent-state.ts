import type { InsightAvailability, InsightLanguagePolicy, InsightSnapshot } from "../../../assets/contracts/meeting-insights.ts";

export type SuggestionPolicy = "unanswered" | "manual" | "automatic";
export type SuggestionConsentAction = "consent" | "one_off" | "not_now";
export type SuggestionPresentationMode = "loading" | "consent" | "quiet" | "manual" | "not_run" | "waiting_summary" | "running" | "items" | "done_empty" | "failed" | "stale" | "unavailable";
export function suggestionConsentPresentation(input: {
  policy: SuggestionPolicy; snapshot: InsightSnapshot | null; availability: InsightAvailability | null;
  manualAvailability?: InsightAvailability | null;
  dismissed: boolean; working: boolean; summaryWorking: boolean;
}): { mode: SuggestionPresentationMode; reason: string | null; showQuestion: boolean } {
  const { snapshot, availability, policy } = input;
  const manual = input.manualAvailability ?? availability;
  let mode: SuggestionPresentationMode, reason: string | null = null;
  if (input.working) mode = "running";
  else if (!snapshot || !availability) mode = "loading";
  else if (input.summaryWorking && policy === "automatic" && !snapshot.automatic_attempt) mode = "waiting_summary";
  else if (!manual?.available) { mode = "unavailable"; reason = manual?.reason_code ?? availability.reason_code; }
  else if (snapshot.automatic_attempt_reason_code) { mode = "failed"; reason = snapshot.automatic_attempt_reason_code; }
  else if (snapshot.automatic_attempt_stale) { mode = "stale"; reason = "source_changed"; }
  else if (snapshot.automatic_attempt && snapshot.automatic_attempt.outcome !== "completed") {
    mode = "failed"; reason = snapshot.automatic_attempt.error_code ?? "interrupted";
  } else if (snapshot.items.length) mode = "items";
  else if (snapshot.automatic_attempt?.outcome === "completed") mode = "done_empty";
  else if (policy === "unanswered") {
    mode = input.dismissed ? "quiet" : "consent";
  } else if (policy === "manual") mode = "manual";
  else { mode = "not_run"; reason = availability.available ? "not_run" : availability.reason_code; }
  return { mode, reason, showQuestion: mode === "consent" };
}
export class SuggestionAvailabilityError extends Error {
  readonly code: string;
  constructor(code: string) { super(code); this.name = "SuggestionAvailabilityError"; this.code = code; }
}
export async function requireSuggestionAvailability(automatic: boolean, checks: {
  manual(): Promise<InsightAvailability>; automatic(): Promise<InsightAvailability>; unchanged(): Promise<boolean>;
}): Promise<void> {
  if (!await checks.unchanged()) throw new SuggestionAvailabilityError("configuration_changed");
  const manual = await checks.manual();
  if (!await checks.unchanged()) throw new SuggestionAvailabilityError("configuration_changed");
  if (!manual.available) throw new SuggestionAvailabilityError(manual.reason_code || "unqualified");
  if (automatic) {
    const available = await checks.automatic();
    if (!await checks.unchanged()) throw new SuggestionAvailabilityError("configuration_changed");
    if (!available.available) throw new SuggestionAvailabilityError(available.reason_code || "foreground_impact_unaccepted");
  }
}
export async function performSuggestionConsent(action: SuggestionConsentAction, automatic: boolean, language: InsightLanguagePolicy, needsLanguage: boolean, effects: {
  validate?(automatic: boolean): Promise<void>;
  savePolicy(policy: Exclude<SuggestionPolicy, "unanswered">, fallback?: Exclude<InsightLanguagePolicy, "unknown">): Promise<void>;
  start(): Promise<unknown>; dismiss(): Promise<void>;
}): Promise<void> {
  if (action === "not_now") { await effects.dismiss(); return; }
  await effects.validate?.(action === "consent" && automatic);
  if (action === "consent") await effects.savePolicy(automatic ? "automatic" : "manual", automatic && needsLanguage && language !== "unknown" ? language : undefined);
  await effects.start();
}
