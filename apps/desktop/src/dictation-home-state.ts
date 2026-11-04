import type { HistoryEntry, LocalModel } from "./api";

/** Only actual successful output belongs in Recent; failures retain their recovery route in History. */
export function recentDictations(entries: HistoryEntry[], limit = 5): HistoryEntry[] {
  return entries.filter((entry) => entry.status !== "failed" && entry.text.trim())
    .sort((left, right) => right.created_at_ms - left.created_at_ms).slice(0, Math.max(0, limit));
}

export function matchingDictations(entries: HistoryEntry[], query: string): HistoryEntry[] {
  const text = query.trim().toLocaleLowerCase();
  return text ? entries.filter((entry) => `${entry.text} ${entry.raw}`.toLocaleLowerCase().includes(text)) : [];
}

/** The backend's language-filtered recommendation is shared by onboarding and task recovery. */
export function recommendedSpeechModel(models: LocalModel[]): LocalModel | undefined {
  return models.find((model) => model.recommended && !model.legacy) ?? models.find((model) => !model.legacy);
}
