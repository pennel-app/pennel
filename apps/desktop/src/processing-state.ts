import type { CleanupVerificationAvailability, ModelChoice, ProviderAccount, Settings } from "./api";
import { t } from "./i18n.ts";

export type ProcessingTask = "dictation" | "meeting";
export interface ProcessingStage { label: string; destination: string }

const cliKinds = new Set(["claude_code", "codex", "open_code", "copilot", "cursor_agent", "gemini_cli"]);

export function providerDestination(account: ProviderAccount | undefined): string {
  if (!account) return t("dictation.processing.not_configured");
  if (account.kind === "local") return t("dictation.processing.local");
  if (account.kind === "apple_speech") return t("dictation.processing.apple_speech");
  if (account.kind === "apple_intelligence") return t("dictation.processing.apple_intelligence");
  if (cliKinds.has(account.kind)) return t("dictation.processing.cli", { provider: account.name });
  try {
    const url = new URL(account.base_url);
    if (["http:", "https:"].includes(url.protocol)) return `${account.name} (${url.host})`;
  } catch { /* An unfinished address remains editable without making an offline claim. */ }
  return t("dictation.processing.server", { provider: account.name });
}

export function choiceDestination(settings: Settings, choice: ModelChoice): string {
  return providerDestination(settings.providers.find((account) => account.id === choice.provider_id));
}

export function processingSummary(settings: Settings, task: ProcessingTask, _verification?: CleanupVerificationAvailability | null, _frozen = false): ProcessingStage[] {
  return [
    { label: t("dictation.processing.audio"), destination: choiceDestination(settings, task === "meeting" ? settings.meeting_stt : settings.dictation_stt) },
    { label: task === "meeting" ? t("dictation.processing.summary_questions") : t("dictation.processing.text"), destination: task === "dictation" && settings.mode === "fast"
      ? t("dictation.processing.fast") : choiceDestination(settings, settings.llm) },
  ];
}
