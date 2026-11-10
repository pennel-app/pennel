import { t } from "./i18n.ts";
import type { LocalModel, OnboardingGoal, OnboardingProgress, OnboardingStep, PermissionInfo, Settings, SetupReadiness } from "./api";
import { recommendedSpeechModel } from "./dictation-home-state.ts";

export type Engine = "local" | "apple" | "cloud" | "custom";

/** The saved recognizer, else local; a processor too old for local models starts on a provider instead. */
export function initialEngine(accountKind: string | undefined, localAvailable: boolean): Engine {
  if (accountKind === "apple_speech") return "apple";
  if (accountKind === "pennel_cloud") return "cloud";
  if (accountKind === "local" || !accountKind) return localAvailable ? "local" : "custom";
  return "custom";
}

export const onboardingSteps: OnboardingStep[] = ["language", "welcome", "engine", "permissions", "practice", "voice"];

export function onboardingGoal(progress: OnboardingProgress): OnboardingGoal {
  return progress.goal ?? "dictation";
}

export interface OnboardingRouteIntent {
  goal: OnboardingGoal;
  preserveProgress: boolean;
  step: OnboardingStep | "resume";
}

/** Task repair must not inherit the job selected during first use. */
export function onboardingRouteIntent(route: unknown, progress: OnboardingProgress): OnboardingRouteIntent {
  const dictation = route === "practice" || route === "dictation-setup" || route === "dictation-permissions";
  return {
    goal: dictation ? "dictation" : onboardingGoal(progress),
    preserveProgress: dictation,
    step: route === "dictation-permissions" ? "permissions"
      : route === "dictation-setup" || route === "practice" || route === "resume" ? "resume"
      : onboardingSteps.includes(route as OnboardingStep) ? route as OnboardingStep : progress.resume_step ?? "welcome",
  };
}

export function onboardingResumeStep(readiness: SetupReadiness, saved: OnboardingStep | undefined, intent: OnboardingRouteIntent): OnboardingStep {
  if (intent.preserveProgress && intent.step === "permissions" && readiness.resume_step !== "engine") return "permissions";
  return resumeStep(readiness.resume_step, intent.preserveProgress ? undefined : saved);
}

/** Named legacy steps remain resumable without adding them to new first use. */
export function onboardingFlow(saved?: OnboardingStep): OnboardingStep[] {
  return [...(saved === "language" ? ["language" as const] : []), "welcome", "engine", "permissions", "practice", ...(saved === "voice" ? ["voice" as const] : [])];
}

/** Older profiles may have saved optional voice enrollment after their practice. */
export function resumeStep(ready: OnboardingStep, saved: OnboardingStep | undefined): OnboardingStep {
  return ready === "practice" && saved === "voice" ? "voice" : ready;
}

export function stepIndex(step: unknown): number {
  const index = onboardingSteps.indexOf(step as OnboardingStep);
  return index < 0 ? 0 : index;
}

export function onboardingProgressChange(current: OnboardingProgress, step: OnboardingStep, status?: "skipped" | "completed", goal?: OnboardingGoal): OnboardingProgress | null {
  const next: OnboardingProgress = { version: 1, resume_step: step,
    status: status === "completed" ? "completed" : current.status === "completed" ? "completed" : status ?? current.status,
    ...(goal || current.goal ? { goal: goal ?? current.goal } : {}) };
  return current.version === next.version && current.resume_step === next.resume_step && current.status === next.status && current.goal === next.goal ? null : next;
}

export function permissionGranted(permissions: PermissionInfo[], kind: string): boolean {
  const status = permissions.find((permission) => permission.kind === kind)?.status;
  return status === "granted" || status === "not_required";
}

export function manualPracticeAllowed(readiness: SetupReadiness | null, permissions: PermissionInfo[], settings: Settings, goal: OnboardingGoal = "dictation"): boolean {
  if (readiness) return readiness.configured && readiness.can_record;
  const microphone = permissions.find((permission) => permission.kind === "microphone")?.status;
  const speech = permissions.find((permission) => permission.kind === "speech")?.status;
  const choice = goal === "meeting" ? settings.meeting_stt : settings.dictation_stt;
  const apple = settings.providers.find((provider) => provider.id === choice?.provider_id)?.kind === "apple_speech";
  return !["denied", "restricted", "unsupported"].includes(microphone ?? "unknown")
    && (!apple || speech === "granted") && !!choice?.model;
}

export function defaultModelId(models: LocalModel[], current: string, compatible: LocalModel[] = models): string {
  if (models.some((model) => model.id === current)) return current;
  return recommendedSpeechModel(compatible)?.id ?? current;
}

export function modelOptionLabel(model: LocalModel, formatSize: (bytes: number) => string): string {
  const size = formatSize(model.size_bytes);
  return model.recommended ? t("onboarding.model_option_recommended", { name: model.name, size }) : [model.name, size].join(" · ");
}
