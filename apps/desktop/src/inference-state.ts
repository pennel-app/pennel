import type { AppInfo, InferenceCapability, InferenceConfig, InferenceProtocolInfo, InferenceVerification, ProviderAccount } from "./api";
import { providerFingerprint } from "./provider-verification.ts";

export function inferenceConfig(info: Pick<AppInfo, "settings">, capability: InferenceCapability): InferenceConfig | null {
  return info.settings[capability] ?? null;
}

export function inferenceProtocolsForProvider(protocols: InferenceProtocolInfo[], account: ProviderAccount, capability: InferenceCapability): InferenceProtocolInfo[] {
  return protocols.filter((item) => item[capability] && item.allowed_provider_kinds?.includes(account.kind)
    && (account.kind === "pennel_cloud" ? item.managed === true && item.protocol === "pennel_cloud" : item.managed === false && item.protocol !== "pennel_cloud"));
}

export function inferenceFingerprint(info: Pick<AppInfo, "settings" | "keys" | "provider_revisions" | "cloud_account_epoch" | "cloud_signed_in">, capability: InferenceCapability): string {
  const config = inferenceConfig(info, capability);
  const account = info.settings.providers.find((provider) => provider.id === config?.provider_id);
  return JSON.stringify([config, account ? providerFingerprint(info, account) : null]);
}

export class InferenceVerificationState {
  private active: { id: string; fingerprint: string } | null = null;
  private saved: { fingerprint: string; result: InferenceVerification } | null = null;

  get running(): boolean { return this.active !== null; }

  begin(id: string, fingerprint: string): boolean {
    if (this.active) return false;
    this.active = { id, fingerprint };
    this.saved = null;
    return true;
  }

  finish(id: string, currentFingerprint: string, result: InferenceVerification): boolean {
    if (!this.active || this.active.id !== id) return false;
    const accepted = this.active.fingerprint === currentFingerprint;
    this.active = null;
    if (accepted) this.saved = { fingerprint: currentFingerprint, result };
    return accepted;
  }

  get(fingerprint: string): InferenceVerification | null {
    if (this.saved?.fingerprint !== fingerprint) this.saved = null;
    return this.saved?.result ?? null;
  }

  invalidate(): string | null {
    const id = this.active?.id ?? null;
    this.active = null;
    this.saved = null;
    return id;
  }
}
