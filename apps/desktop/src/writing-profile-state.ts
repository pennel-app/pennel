import type { WritingProfileConnection, WritingProfileLanguage, WritingProfileSource, WritingProfileStatus } from "./api.ts";
import { l, liveText, t, uiLocale, type TranslationKey, type UiText } from "./i18n.ts";

export const writingTraitValues = {
  directness: ["direct", "balanced", "indirect"],
  detail: ["concise", "balanced", "detailed"],
  register: ["conversational", "plain", "technical", "formal"],
  sentence_rhythm: ["short", "mixed", "long"],
  clause_structure: ["simple", "linked", "layered"],
  hedging: ["rare", "occasional", "frequent"],
} as const;

export type WritingTraitKey = keyof typeof writingTraitValues;

const traitLabels: Record<WritingTraitKey, TranslationKey> = {
  directness: "settings.writing.trait.directness",
  detail: "settings.writing.trait.detail",
  register: "settings.writing.trait.register",
  sentence_rhythm: "settings.writing.trait.sentence_rhythm",
  clause_structure: "settings.writing.trait.clause_structure",
  hedging: "settings.writing.trait.hedging",
};

const valueLabels: Record<string, TranslationKey> = {
  direct: "settings.writing.value.direct", balanced: "settings.writing.value.balanced", indirect: "settings.writing.value.indirect",
  concise: "settings.writing.value.concise", detailed: "settings.writing.value.detailed",
  conversational: "settings.writing.value.conversational", plain: "settings.writing.value.plain", technical: "settings.writing.value.technical", formal: "settings.writing.value.formal",
  short: "settings.writing.value.short", mixed: "settings.writing.value.mixed", long: "settings.writing.value.long",
  simple: "settings.writing.value.simple", linked: "settings.writing.value.linked", layered: "settings.writing.value.layered",
  rare: "settings.writing.value.rare", occasional: "settings.writing.value.occasional", frequent: "settings.writing.value.frequent",
};

export function isWritingTraitKey(value: string): value is WritingTraitKey {
  return Object.hasOwn(writingTraitValues, value);
}

export function writingTraitLabel(key: WritingTraitKey): UiText { return l(traitLabels[key]); }

export function writingTraitValue(value: string): UiText {
  return l(valueLabels[value] ?? "settings.writing.automatic");
}

export function writingTraitChoices(key: WritingTraitKey, offered: readonly string[], learned: string | null): [string, UiText][] {
  const permitted: readonly string[] = writingTraitValues[key];
  return [["", learned && permitted.includes(learned)
    ? l("settings.writing.automatic_value", () => ({ value: String(writingTraitValue(learned)) }))
    : l("settings.writing.automatic")],
  ...offered.filter((value) => permitted.includes(value)).map((value): [string, UiText] => [value, writingTraitValue(value)])];
}

export function writingLanguageName(language: string): UiText {
  return liveText(() => {
    try { return new Intl.DisplayNames([uiLocale()], { type: "language" }).of(language) ?? language; }
    catch { return language; }
  });
}

export function writingProgress(language: WritingProfileLanguage, status: Pick<WritingProfileStatus, "min_words" | "min_sessions">): number {
  if (status.min_words <= 0 || status.min_sessions <= 0) return 0;
  return Math.min(1, Math.max(0, language.word_count / status.min_words), Math.max(0, language.session_count / status.min_sessions));
}

export function writingSourceSessions(sources: WritingProfileSource[]): WritingProfileSource[] {
  const sessions = new Map<string, WritingProfileSource[]>();
  for (const source of sources) {
    const key = `${source.kind}:${source.session_id}`;
    const group = sessions.get(key) ?? [];
    group.push(source); sessions.set(key, group);
  }
  return [...sessions.values()].map((group) => {
    const languages = new Set(group.map((source) => source.language));
    return { ...group[0], word_count: group.reduce((sum, source) => sum + source.word_count, 0),
      language: languages.size === 1 ? group[0].language : null,
      excluded: group.some((source) => source.excluded), analyzed: group.every((source) => source.analyzed) };
  });
}

/** Error channels must never reproduce a model response or source text. */
export function writingErrorText(code: string | null | undefined): string {
  if (!code) return "";
  if (code === "provider_unavailable" || code === "model_not_configured") return t("settings.writing.error.provider");
  if (code === "connection_unavailable" || code === "connection_failed") return t("settings.writing.sharing.connection_error");
  if (code === "preview_only") return t("settings.writing.error.preview");
  if (code === "busy") return t("settings.writing.error.busy");
  if (code === "signed_out" || code === "unauthorized") return t("settings.writing.sharing.sign_in");
  return t("settings.writing.error.generic");
}

export function writingConnectionText(connection: WritingProfileConnection): string {
  if (!connection.signed_in) return t("settings.writing.sharing.sign_in");
  if (!connection.enabled) return t("settings.writing.sharing.off");
  if (connection.error) return writingErrorText(connection.error);
  return t(connection.designated ? "settings.writing.sharing.active" : "settings.writing.sharing.other_source");
}
