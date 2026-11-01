import type { AppInfo, ModelChoice, ProviderAccount } from "./api";

export type ProviderSlot = "dictation_stt" | "meeting_stt" | "llm";
export interface TaskVerification { verified: boolean; message: string }
type VerificationInfo = Pick<AppInfo, "settings" | "keys" | "cloud_account_epoch" | "cloud_signed_in"> & { provider_revisions?: Record<string, number> };

export function providerFingerprint(info: VerificationInfo, account: ProviderAccount): string {
  if (account.kind === "pennel_cloud") return JSON.stringify([account.id, account.kind, account.name, account.base_url, info.cloud_account_epoch ?? null, info.cloud_signed_in === true]);
  return JSON.stringify([account.id, account.kind, account.name, account.base_url, !!info.keys[account.id], info.provider_revisions?.[account.id] ?? 0]);
}

export function taskFingerprint(info: VerificationInfo, slot: ProviderSlot): string {
  return choiceFingerprint(info, info.settings[slot]);
}

export function choiceFingerprint(info: VerificationInfo, choice: ModelChoice): string {
  const account = info.settings.providers.find((provider) => provider.id === choice.provider_id);
  return JSON.stringify([account ? providerFingerprint(info, account) : null, choice.model, info.settings.language]);
}

export class ProviderVerification {
  private readonly results = new Map<ProviderSlot, { fingerprint: string; result: TaskVerification }>();

  get(slot: ProviderSlot, fingerprint: string): TaskVerification | null {
    const saved = this.results.get(slot);
    if (saved?.fingerprint === fingerprint) return saved.result;
    this.results.delete(slot);
    return null;
  }

  set(slot: ProviderSlot, fingerprint: string, result: TaskVerification): void {
    this.results.set(slot, { fingerprint, result });
  }

  invalidate(): void { this.results.clear(); }
}

export const providerVerification = new ProviderVerification();

export class ModelCatalogCache<T> {
  private readonly entries = new Map<string, { fingerprint: string; pending: Promise<T[]> }>();

  get(providerId: string, fingerprint: string, load: () => Promise<T[]>): Promise<T[]> {
    const saved = this.entries.get(providerId);
    if (saved?.fingerprint === fingerprint) return saved.pending;
    const pending = Promise.resolve().then(load);
    this.entries.set(providerId, { fingerprint, pending });
    void pending.catch(() => {
      if (this.entries.get(providerId)?.pending === pending) this.entries.delete(providerId);
    });
    return pending;
  }

  invalidate(providerId: string): void { this.entries.delete(providerId); }
}
