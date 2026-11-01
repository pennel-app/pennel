import type { AppInfo, ModelChoice, PermissionInfo, ProviderKind } from "./api";
import type { ProviderSlot } from "./provider-verification";

export type SetupTask = ProviderSlot;
export type TaskConfigurationState = "ready" | "provider" | "unsupported" | "permission" | "key" | "model" | "download" | "address" | "account";
export interface TaskConfiguration { state: TaskConfigurationState; reason: string | null }

export type ProcessingLocation = "device" | "cloud" | "own";
const deviceProviders = new Set<ProviderKind>(["local", "apple_speech", "apple_intelligence"]);

export function processingLocation(kind: ProviderKind, baseUrl?: string): ProcessingLocation {
  if (kind === "ollama" || kind === "lm_studio") {
    if (baseUrl === undefined) return "device";
    try {
      const host = new URL(baseUrl).hostname.toLowerCase();
      if (host === "localhost" || host === "[::1]" || /^127(?:\.\d{1,3}){3}$/.test(host)) return "device";
    } catch { /* An unfinished server address must not imply local processing. */ }
    return "own";
  }
  return kind === "pennel_cloud" ? "cloud" : deviceProviders.has(kind) ? "device" : "own";
}

export function missingTaskAccess(task: SetupTask, kind: ProviderKind | undefined, permissions: readonly PermissionInfo[]): PermissionInfo | undefined {
  if (task === "llm") return;
  return permissions.find((permission) => permission.kind === "microphone"
    ? !["granted", "not_required", "unknown"].includes(permission.status)
    : kind === "apple_speech" && permission.kind === "speech" && !["granted", "not_required"].includes(permission.status));
}

export class TaskModelDraft {
  private current: ModelChoice;
  private readonly models = new Map<string, string>();
  constructor(choice: ModelChoice) {
    this.current = { ...choice };
    this.models.set(choice.provider_id, choice.model);
  }
  get choice(): ModelChoice { return { ...this.current }; }
  setModel(model: string): void {
    this.current.model = model;
    this.models.set(this.current.provider_id, model);
  }
  chooseProvider(providerId: string, defaultModel: string): void {
    this.current = { provider_id: providerId, model: this.models.get(providerId) ?? defaultModel };
    this.models.set(providerId, this.current.model);
  }
}

/** Presentation readiness is configuration, not a successful provider or microphone test. */
export function taskConfiguration(info: Pick<AppInfo, "settings" | "kinds" | "keys">, task: SetupTask, downloaded: readonly string[], choice: ModelChoice = info.settings[task]): TaskConfiguration {
  const account = info.settings.providers.find((provider) => provider.id === choice.provider_id);
  const kind = info.kinds.find((candidate) => candidate.kind === account?.kind);
  const result = (state: TaskConfigurationState, reason: string | null = null): TaskConfiguration => ({ state, reason });
  if (!account || !kind) return result("provider");
  if (!(task === "llm" ? kind.llm : kind.stt)) return result("unsupported", kind.availability.reason);
  if (kind.availability.status !== "available") return result(kind.availability.status === "permission_required" ? "permission" : "unsupported", kind.availability.reason);
  if (kind.requires_key && !info.keys[account.id]) return result("key");
  if (kind.is_cli_agent && !account.base_url.trim()) return result("address");
  if (!kind.is_cli_agent && !["local", "apple_speech", "apple_intelligence", "pennel_cloud"].includes(account.kind)) {
    try { if (!["http:", "https:"].includes(new URL(account.base_url).protocol)) return result("address"); }
    catch { return result("address"); }
  }
  if (!kind.is_cli_agent && !choice.model.trim()) return result("model");
  if (account.kind === "local" && !downloaded.includes(choice.model)) return result("download");
  return result("ready");
}

/** A dialog owns its async work; a late load cannot replace a newer choice or a closed view. */
export class TaskSetupScope {
  private generation = 0;
  private disposed = false;
  private readonly pending = new Set<Promise<unknown>>();
  private readonly failures = new Map<"settings" | "mutation", unknown>();
  get failure(): unknown { return this.failures.get("mutation") ?? this.failures.get("settings") ?? null; }
  get busy(): boolean { return this.pending.size > 0; }

  next(): number { return ++this.generation; }
  current(generation: number): boolean { return !this.disposed && generation === this.generation; }
  dispose(): void { this.disposed = true; this.generation++; }

  track<T>(work: Promise<T>, kind: "settings" | "mutation" = "mutation"): Promise<T> {
    const pending = work.then((value) => { this.failures.delete(kind); return value; }, (error: unknown) => { this.failures.set(kind, error); throw error; });
    this.pending.add(pending);
    void pending.then(() => this.pending.delete(pending), () => this.pending.delete(pending));
    return pending;
  }

  async settle(): Promise<void> {
    await Promise.all([...this.pending]);
  }

  settingsSaved(): void { this.failures.delete("settings"); }
}
