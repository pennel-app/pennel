import type { InferenceCapability, InferenceConfig, InferenceProtocol, InferenceProtocolInfo, InferenceVerification, NativeFailure, ProviderAccount, RuntimeClient, RuntimeSnapshot } from './domain.ts';
import { nativeFailure } from './errors.ts';

export interface InferenceDraft {
  provider_id: string;
  protocol: InferenceProtocol;
  model: string;
  expected_model: string;
  upstream: string;
  max_input_bytes: string;
  timeout_secs: string;
}

export function inferenceDraft(capability: InferenceCapability, saved?: InferenceConfig | null): InferenceDraft {
  return { provider_id: saved?.provider_id ?? '', protocol: saved?.protocol ?? (capability === 'decision' ? 'open_router_decisions' : 'tei'),
    model: saved?.model ?? '', expected_model: saved?.expected_model ?? '', upstream: saved?.upstream ?? '',
    max_input_bytes: String(saved?.max_input_bytes ?? 16000), timeout_secs: String(saved?.timeout_secs ?? 30) };
}

export function inferenceConfiguration(draft: InferenceDraft, capability: InferenceCapability, protocols: readonly InferenceProtocolInfo[] = []): InferenceConfig | null {
  const metadata = protocols?.find((entry) => entry.protocol === draft.protocol);
  const supported = metadata?.[capability] === true;
  if (!draft.provider_id || !draft.model.trim() || !supported) return null;
  if (draft.protocol === 'pennel_cloud' && (!metadata?.managed || draft.model !== 'pennel-auto' || draft.expected_model.trim() || draft.upstream.trim())) return null;
  if (!/^\d+$/.test(draft.max_input_bytes) || !/^\d+$/.test(draft.timeout_secs)) return null;
  const bytes = Number(draft.max_input_bytes), seconds = Number(draft.timeout_secs);
  if (!Number.isSafeInteger(bytes) || bytes < 1 || bytes > 65536 || !Number.isSafeInteger(seconds) || seconds < 1 || seconds > 90) return null;
  if (draft.protocol.startsWith('open_router_') && !draft.upstream.trim()) return null;
  return { provider_id: draft.provider_id, protocol: draft.protocol, model: draft.model.trim(), expected_model: draft.expected_model.trim() || null,
    upstream: draft.upstream.trim() || null, max_input_bytes: bytes, timeout_secs: seconds };
}

export function inferenceConfigIdentity(config?: InferenceConfig | null): string {
  return config ? JSON.stringify([config.provider_id, config.protocol, config.model, config.expected_model ?? null,
    config.upstream ?? null, config.max_input_bytes, config.timeout_secs]) : '';
}

export function usesOpenRouter(provider: ProviderAccount): boolean {
  if (provider.kind === 'open_router') return true;
  try {
    const host = new URL(provider.base_url).hostname.toLowerCase().replace(/\.+$/, '');
    return host === 'openrouter.ai' || host.endsWith('.openrouter.ai');
  } catch { return false; }
}

export function inferenceProviderEligible(provider: ProviderAccount, capability: InferenceCapability, protocols: readonly InferenceProtocolInfo[] = []): boolean {
  if (provider.kind === 'pennel_cloud') return Boolean(protocols?.some((entry) => entry.protocol === 'pennel_cloud' && entry.managed && entry[capability] && entry.allowed_provider_kinds?.includes('pennel_cloud')));
  if (!['custom', 'open_router'].includes(provider.kind) || (capability === 'reranking' && usesOpenRouter(provider))) return false;
  return protocols.some((entry) => entry[capability] && entry.allowed_provider_kinds?.includes(provider.kind) && entry.protocol !== 'pennel_cloud');
}

export function inferenceProtocolAllowed(provider: ProviderAccount | undefined, capability: InferenceCapability, protocol: InferenceProtocol, protocols: readonly InferenceProtocolInfo[] = []): boolean {
  if (!provider || !inferenceProviderEligible(provider, capability, protocols)) return false;
  const metadata = protocols?.find((entry) => entry.protocol === protocol);
  if (protocol === 'pennel_cloud') return Boolean(provider.kind === 'pennel_cloud' && metadata?.managed && metadata[capability] && metadata.allowed_provider_kinds?.includes(provider.kind));
  if (provider.kind === 'pennel_cloud') return false;
  return Boolean((metadata?.[capability] && metadata.allowed_provider_kinds?.includes(provider.kind))
    && (!usesOpenRouter(provider) || protocol.startsWith('open_router_')));
}

export function selectInferenceProvider(draft: InferenceDraft, provider?: ProviderAccount, capability: InferenceCapability = 'decision', protocols: readonly InferenceProtocolInfo[] = []): InferenceDraft {
  if (provider?.kind === 'pennel_cloud') return { ...draft, provider_id: provider.id, protocol: 'pennel_cloud', model: 'pennel-auto', expected_model: '', upstream: '' };
  if (draft.protocol === 'pennel_cloud') {
    const protocol = protocols?.find((entry) => inferenceProtocolAllowed(provider, capability, entry.protocol, protocols))?.protocol
      ?? (capability === 'reranking' ? 'tei' : provider && usesOpenRouter(provider) ? 'open_router_decisions' : 'type_safe');
    return { ...draft, provider_id: provider?.id ?? '', protocol, model: '', expected_model: '', upstream: '' };
  }
  return { ...draft, provider_id: provider?.id ?? '',
    protocol: provider && usesOpenRouter(provider) && !draft.protocol.startsWith('open_router_') ? 'open_router_decisions' : draft.protocol };
}

export interface InferenceSelection { capability: InferenceCapability; fingerprint: string }

export function inferenceSelection(snapshot: RuntimeSnapshot | null, capability: InferenceCapability, protocols: readonly InferenceProtocolInfo[]): InferenceSelection | null {
  if (!snapshot || snapshot.platform !== 'ios') return null;
  const config = snapshot.settings[capability];
  if (!config) return null;
  const provider = snapshot.settings.providers.find((account) => account.id === config.provider_id);
  const protocol = protocols.find((entry) => entry.protocol === config.protocol);
  if (!provider || !inferenceProtocolAllowed(provider, capability, config.protocol, protocols) || !protocol?.[capability]
    || !inferenceConfiguration(inferenceDraft(capability, config), capability, protocols)) return null;
  if (protocol.requires_key && !snapshot.keys[provider.id]) return null;
  if (protocol.managed && (!snapshot.cloud.signed_in || !snapshot.cloud.account_epoch)) return null;
  return { capability, fingerprint: JSON.stringify([capability, inferenceConfigIdentity(config), provider.id, provider.kind, provider.base_url,
    snapshot.keys[provider.id] === true, snapshot.inference_key_revisions?.[provider.id] ?? 0,
    protocol.managed ? snapshot.cloud.account_epoch : null]) };
}

export interface InferenceVerificationState { running: boolean; changed: boolean; result: InferenceVerification | null; error: NativeFailure | null }
let sequence = 0;

export class InferenceVerificationController {
  private state: InferenceVerificationState = { running: false, changed: false, result: null, error: null };
  private selection: InferenceSelection | null = null;
  private generation = 0;
  private active: { id: string; pending: Promise<void> } | null = null;
  private listeners = new Set<() => void>();
  private readonly client: Pick<RuntimeClient, 'invoke'>;
  private readonly requestId: () => string;

  constructor(client: Pick<RuntimeClient, 'invoke'>, requestId = () => `mobile-inference-${Date.now().toString(36)}-${++sequence}-${Math.random().toString(36).slice(2)}`) {
    this.client = client; this.requestId = requestId;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<InferenceVerificationState>) { this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener(); }

  configure(selection: InferenceSelection | null): void {
    if (selection?.fingerprint === this.selection?.fingerprint) return;
    this.selection = selection;
    this.invalidate();
  }
  invalidate = (): void => {
    this.generation += 1;
    this.update({ result: null, error: null, changed: this.state.running });
    if (this.active) void this.client.invoke('inference.cancel', { request_id: this.active.id }).catch(() => {});
  };
  cancel = (): void => { this.invalidate(); };
  verify = (): Promise<void> => {
    if (this.active) return this.active.pending;
    if (!this.selection) return Promise.resolve();
    const selected = this.selection, generation = this.generation, id = this.requestId();
    this.update({ running: true, changed: false, result: null, error: null });
    const pending = this.perform(selected, generation, id).finally(() => { if (this.active?.id === id) this.active = null; });
    this.active = { id, pending };
    return pending;
  };
  private async perform(selection: InferenceSelection, generation: number, id: string): Promise<void> {
    try {
      const result = await this.client.invoke<InferenceVerification>('inference.verify', { capability: selection.capability, request_id: id });
      if (generation === this.generation && selection.fingerprint === this.selection?.fingerprint) this.update({ result: { ...result, verified: result?.verified === true } });
    } catch (error) {
      if (generation === this.generation && selection.fingerprint === this.selection?.fingerprint) this.update({ error: nativeFailure(error) });
    } finally { this.update({ running: false, changed: false }); }
  }
}

export const inferenceCopy = {
  en: {
    decision: 'Decision processing', reranking: 'Passage ranking',
    help: 'Optional connection for future smart features. Saving this choice does not change dictation or meetings.',
    provider: 'Provider', protocol: 'API format', model: 'Model', expected: 'Expected returned model', upstream: 'Upstream provider',
    bytes: 'Max input bytes', timeout: 'Timeout (seconds)', advanced: 'Advanced', less: 'Hide advanced',
    save: 'Save configuration', disable: 'Disable', test: 'Test with sample', cancel: 'Cancel',
    configured: 'Configured. Run a sample test to verify this connection.', disabled: 'Not configured',
    changed: 'The configuration changed. Run the test again with the current choice.',
    sample: 'The test sends a built-in synthetic example to the selected endpoint.',
    saveFirst: 'Save this configuration before testing it.', missingKey: 'Save this provider’s API key in Providers first.',
    providerHelp: 'Choose Pennel Cloud or add a compatible connection in Providers.',
    managed: 'Managed by Pennel Cloud', cloudSignIn: 'Sign in to Pennel Cloud in Account to test this connection.', cloudAccount: 'Open Account',
    cloudReady: 'Signed in. Run a sample test to check that this Cloud service is available.',
    destination: 'Processing destination', actualModel: 'Returned model', actualProvider: 'Returned provider', revision: 'Model revision', request: 'Request ID', unknown: 'Not reported',
    failed: 'The connection could not be verified. Check the provider, API format, model and access.', passed: 'The selected connection processed the synthetic example.',
    invalid: 'Choose a provider and model. OpenRouter requires an upstream provider; use 1–65,536 input bytes and a 1–90 second timeout.',
    unavailable: 'This connection is unavailable. Check its provider, endpoint and model configuration.',
    loopback: 'Loopback addresses refer to this iPhone. The Mac Decider app’s localhost server is not reachable at this address.',
    android: 'Decision processing and passage ranking are not available on Android yet.',
    optionalKey: 'API key (optional)', customKeyHelp: 'Decision services can require an API key. Local servers can remain keyless.',
  },
  nl: {
    decision: 'Beslissingen verwerken', reranking: 'Passages rangschikken',
    help: 'Optionele verbinding voor toekomstige slimme functies. Deze keuze verandert dicteren of vergaderingen niet.',
    provider: 'Provider', protocol: 'API-indeling', model: 'Model', expected: 'Verwacht antwoordmodel', upstream: 'Uitvoerende provider',
    bytes: 'Maximale invoerbytes', timeout: 'Time-out (seconden)', advanced: 'Geavanceerd', less: 'Geavanceerd verbergen',
    save: 'Configuratie bewaren', disable: 'Uitschakelen', test: 'Test met voorbeeld', cancel: 'Annuleren',
    configured: 'Ingesteld. Voer een voorbeeldtest uit om deze verbinding te controleren.', disabled: 'Niet ingesteld',
    changed: 'De configuratie is veranderd. Test opnieuw met de huidige keuze.',
    sample: 'De test stuurt een ingebouwd synthetisch voorbeeld naar het gekozen eindpunt.',
    saveFirst: 'Bewaar deze configuratie voordat je ze test.', missingKey: 'Bewaar eerst de API-sleutel van deze provider bij Providers.',
    providerHelp: 'Kies Pennel Cloud of voeg een geschikte verbinding toe bij Providers.',
    managed: 'Beheerd door Pennel Cloud', cloudSignIn: 'Meld je bij Account aan bij Pennel Cloud om deze verbinding te testen.', cloudAccount: 'Account openen',
    cloudReady: 'Aangemeld. Voer een voorbeeldtest uit om te controleren of deze Clouddienst beschikbaar is.',
    destination: 'Verwerkingsbestemming', actualModel: 'Antwoordmodel', actualProvider: 'Antwoordprovider', revision: 'Modelversie', request: 'Aanvraag-ID', unknown: 'Niet gemeld',
    failed: 'De verbinding kon niet worden gecontroleerd. Controleer de provider, API-indeling, het model en de toegang.', passed: 'De gekozen verbinding heeft het synthetische voorbeeld verwerkt.',
    invalid: 'Kies een provider en model. OpenRouter vereist een uitvoerende provider; gebruik 1–65.536 invoerbytes en een time-out van 1–90 seconden.',
    unavailable: 'Deze verbinding is niet beschikbaar. Controleer de provider, het eindpunt en de modelconfiguratie.',
    loopback: 'Loopbackadressen verwijzen naar deze iPhone. De localhostserver van de Mac-app Decider is op dit adres niet bereikbaar.',
    android: 'Beslissingen verwerken en passages rangschikken zijn nog niet beschikbaar op Android.',
    optionalKey: 'API-sleutel (optioneel)', customKeyHelp: 'Beslissingsdiensten kunnen een API-sleutel vereisen. Lokale servers kunnen zonder sleutel werken.',
  },
} as const;
