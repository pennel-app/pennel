import { t, type TranslationKey } from "./i18n.ts";
import type { SettingsCategory } from "./workspace-state.ts";

export interface SettingsSearchEntry {
  id: string;
  category: SettingsCategory;
  tab?: string;
  anchor?: string;
  titleKey: TranslationKey;
  categoryKey: TranslationKey;
  keywords: string[];
}

const entries: SettingsSearchEntry[] = [
  {
    id: "ai-quick-setup",
    category: "ai-models",
    anchor: "ai-quick",
    titleKey: "settings.ai.quick_title",
    categoryKey: "navigation.ai_models",
    keywords: ["preset", "quick", "local", "lokaal", "cloud", "pennel cloud", "offline", "eigen provider", "own provider", "setup", "instellen"],
  },
  {
    id: "ai-speech",
    category: "ai-models",
    anchor: "ai-tasks",
    titleKey: "settings.ai.speech_title",
    categoryKey: "navigation.ai_models",
    keywords: ["stt", "speech", "spraak", "spraakherkenning", "whisper", "parakeet", "apple speech", "dictation", "dicteren", "transcript", "transcriberen", "model", "modellen"],
  },
  {
    id: "ai-text",
    category: "ai-models",
    anchor: "ai-tasks",
    titleKey: "settings.ai.text_title",
    categoryKey: "navigation.ai_models",
    keywords: ["llm", "text", "tekst", "ai", "cleanup", "opkuisen", "opschonen", "summary", "samenvatting", "samenvatten", "action items", "actiepunten", "decisions", "beslissingen", "review", "beoordelen", "chat", "vragen", "gpt", "claude", "apple intelligence"],
  },
  {
    id: "ai-speakers",
    category: "ai-models",
    anchor: "ai-speakers",
    titleKey: "settings.ai.speakers_title",
    categoryKey: "navigation.ai_models",
    keywords: ["speakers", "sprekers", "diarization", "diarisatie", "sortformer", "voice", "stem", "teams"],
  },
  {
    id: "ai-local-models",
    category: "ai-models",
    anchor: "ai-local",
    titleKey: "settings.ai.local_models_title",
    categoryKey: "navigation.ai_models",
    keywords: ["local", "lokaal", "offline", "download", "whisper", "parakeet", "turbo", "unload", "geheugen", "memory"],
  },
  {
    id: "ai-cloud-providers",
    category: "ai-models",
    anchor: "ai-providers",
    titleKey: "settings.ai.providers_title",
    categoryKey: "navigation.ai_models",
    keywords: ["api", "key", "sleutel", "api key", "api-sleutel", "openai", "groq", "mistral", "openrouter", "pennel cloud", "provider", "verbinding", "connection"],
  },
  {
    id: "ai-advanced-providers",
    category: "ai-models",
    anchor: "ai-advanced",
    titleKey: "settings.ai.advanced_providers_title",
    categoryKey: "navigation.ai_models",
    keywords: ["ollama", "lm studio", "lmstudio", "custom", "server", "cli", "claude code", "codex", "opencode", "copilot", "cursor", "gemini", "agent", "localhost"],
  },
  {
    id: "dictation-result",
    category: "dictation",
    anchor: "during",
    titleKey: "dictation.settings.result",
    categoryKey: "navigation.dictation",
    keywords: ["clean", "fast", "opgeschoond", "gesproken", "cleanup", "mode", "modus", "dicteren", "dictation"],
  },
  {
    id: "dictation-language",
    category: "dictation",
    anchor: "during",
    titleKey: "dictation.language.title",
    categoryKey: "navigation.dictation",
    keywords: ["language", "taal", "spreektaal", "dutch", "nederlands", "english", "engels", "french", "frans", "german", "duits"],
  },
  {
    id: "dictation-pill",
    category: "dictation",
    anchor: "pill",
    titleKey: "dictation.pill.settings_title",
    categoryKey: "navigation.dictation",
    keywords: ["pill", "pil", "floating", "zwevend", "live text", "live tekst", "overlay", "colour", "kleur", "size", "grootte", "position", "positie"],
  },
  {
    id: "dictation-dictionary",
    category: "dictation",
    tab: "dictionary",
    titleKey: "dictation.dictionary.title",
    categoryKey: "navigation.dictation",
    keywords: ["dictionary", "woordenlijst", "woordenboek", "terms", "termen", "jargon", "names", "namen", "spelling"],
  },
  {
    id: "dictation-styles",
    category: "dictation",
    tab: "styles",
    titleKey: "dictation.style.title",
    categoryKey: "navigation.dictation",
    keywords: ["style", "stijl", "stijlen", "tone", "toon", "formal", "formeel", "casual", "informeel", "literal", "letterlijk", "app"],
  },
  {
    id: "dictation-history",
    category: "dictation",
    anchor: "words",
    titleKey: "dictation.history.retention",
    categoryKey: "navigation.dictation",
    keywords: ["history", "geschiedenis", "retention", "bewaren", "bewaartijd", "days", "dagen"],
  },
  {
    id: "dictation-permissions",
    category: "dictation",
    anchor: "permissions",
    titleKey: "dictation.access.title",
    categoryKey: "navigation.dictation",
    keywords: ["permission", "permissions", "toegang", "machtiging", "microphone", "microfoon", "accessibility", "toegankelijkheid"],
  },
  {
    id: "meetings-after",
    category: "meeting-settings",
    anchor: "after-meeting",
    titleKey: "settings.after_meeting",
    categoryKey: "navigation.meetings",
    keywords: ["summary", "samenvatting", "samenvatten", "automatic", "automatisch", "suggestions", "suggesties", "action items", "actiepunten", "decisions", "beslissingen", "titles", "titels"],
  },
  {
    id: "meetings-recording",
    category: "meeting-settings",
    anchor: "recording-detect",
    titleKey: "settings.recording.detect_title",
    categoryKey: "navigation.meetings",
    keywords: ["call", "calls", "opnemen", "recording", "detect", "herkennen", "echo", "consent", "toestemming", "zoom", "teams", "meet", "slack"],
  },
  {
    id: "meetings-calendar",
    category: "meeting-settings",
    anchor: "recording-calendar",
    titleKey: "settings.calendar.title",
    categoryKey: "navigation.meetings",
    keywords: ["calendar", "agenda", "outlook", "google calendar", "apple calendar", "event", "afspraak", "per meeting", "per app"],
  },
  {
    id: "meetings-templates",
    category: "meeting-settings",
    anchor: "meeting-templates",
    titleKey: "settings.template.title",
    categoryKey: "navigation.meetings",
    keywords: ["template", "templates", "sjabloon", "sjablonen", "standup", "1-op-1", "one-to-one", "klantgesprek", "client", "markdown"],
  },
  {
    id: "meetings-voices",
    category: "meeting-settings",
    anchor: "meeting-voices",
    titleKey: "settings.voice.title",
    categoryKey: "navigation.meetings",
    keywords: ["voice", "voices", "stem", "stemmen", "known voices", "bekende stemmen", "enroll", "inleren", "mijn stem"],
  },
  {
    id: "meetings-folder",
    category: "meeting-settings",
    anchor: "meeting-folder",
    titleKey: "settings.folder.title",
    categoryKey: "navigation.meetings",
    keywords: ["folder", "map", "obsidian", "markdown", "audio", "retention", "bewaren", "storage", "opslag"],
  },
  {
    id: "shortcuts",
    category: "shortcuts",
    titleKey: "shortcuts.title",
    categoryKey: "navigation.shortcuts",
    keywords: ["shortcut", "shortcuts", "sneltoets", "sneltoetsen", "hotkey", "fn", "command", "keybinding", "toets"],
  },
  {
    id: "account-sync",
    category: "account",
    anchor: "cloud-sync",
    titleKey: "navigation.cloud_integrations",
    categoryKey: "navigation.cloud_integrations",
    keywords: ["account", "cloud", "sync", "synchronisatie", "recovery", "herstelcode", "devices", "apparaten", "sign in", "aanmelden", "usage", "verbruik"],
  },
  {
    id: "account-claude",
    category: "account",
    anchor: "cloud-integrations",
    titleKey: "account.claude.title",
    categoryKey: "navigation.cloud_integrations",
    keywords: ["claude", "mcp", "connector", "chatgpt", "assistant", "assistent", "integration", "integratie"],
  },
  {
    id: "account-writing-profile",
    category: "account",
    anchor: "writing-profile",
    titleKey: "settings.writing.title",
    categoryKey: "navigation.cloud_integrations",
    keywords: ["writing profile", "schrijfprofiel", "schrijfstijl", "style", "voice", "chatgpt", "claude", "tone"],
  },
  {
    id: "general-language",
    category: "appearance",
    anchor: "interface-language",
    titleKey: "interface.language",
    categoryKey: "navigation.general",
    keywords: ["interface", "language", "taal", "english", "dutch", "nederlands", "ui"],
  },
  {
    id: "general-appearance",
    category: "appearance",
    anchor: "appearance",
    titleKey: "appearance.title",
    categoryKey: "navigation.general",
    keywords: ["appearance", "weergave", "theme", "thema", "dark", "donker", "light", "licht", "system", "systeem"],
  },
  {
    id: "general-intro",
    category: "appearance",
    anchor: "intro",
    titleKey: "intro.title",
    categoryKey: "navigation.general",
    keywords: ["onboarding", "intro", "introductie", "welcome", "welkom", "tutorial"],
  },
];

export interface SettingsSearchMatch {
  entry: SettingsSearchEntry;
  title: string;
  categoryLabel: string;
  score: number;
}

export function searchSettings(rawQuery: string): SettingsSearchMatch[] {
  const query = rawQuery.trim().toLocaleLowerCase();
  if (!query) return [];
  const tokens = query.split(/\s+/).filter(Boolean);
  const results: SettingsSearchMatch[] = [];
  for (const entry of entries) {
    const title = t(entry.titleKey);
    const categoryLabel = t(entry.categoryKey);
    const titleLower = title.toLocaleLowerCase();
    const categoryLower = categoryLabel.toLocaleLowerCase();
    const keywordText = entry.keywords.join(" ").toLocaleLowerCase();
    const haystack = `${titleLower} ${categoryLower} ${keywordText}`;
    if (!tokens.every((token) => haystack.includes(token))) continue;
    let score = 0;
    if (titleLower === query) score += 100;
    else if (titleLower.startsWith(query)) score += 60;
    else if (titleLower.includes(query)) score += 40;
    if (entry.keywords.some((kw) => kw === query)) score += 50;
    else if (entry.keywords.some((kw) => kw.startsWith(query))) score += 25;
    if (categoryLower.includes(query)) score += 10;
    results.push({ entry, title, categoryLabel, score });
  }
  results.sort((a, b) => b.score - a.score || a.title.localeCompare(b.title));
  return results;
}

export function allSettingsEntries(): SettingsSearchMatch[] {
  return entries.map((entry) => ({
    entry,
    title: t(entry.titleKey),
    categoryLabel: t(entry.categoryKey),
    score: 0,
  }));
}
