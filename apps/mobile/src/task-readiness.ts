import type { RuntimeSnapshot } from './domain.ts';

export function taskReadiness(snapshot: RuntimeSnapshot | null | undefined, task: 'dictation_stt' | 'meeting_stt' | 'llm') {
  const settings = snapshot?.settings;
  const selection = settings?.[task];
  const provider = settings?.providers.find(provider => provider.id === selection?.provider_id);
  const kind = snapshot?.kinds.find(kind => kind.kind === provider?.kind);
  const capability = task === 'llm' ? kind?.llm : kind?.stt;
  const credentials = provider?.kind === 'pennel_cloud' ? snapshot?.cloud.signed_in === true : !kind?.requires_key || Boolean(provider && snapshot?.keys[provider.id]);
  return { ready: Boolean(snapshot?.ready && provider && capability && selection?.model.trim() && kind?.availability.status === 'available' && credentials), provider, model: selection?.model ?? '' };
}

export function recentResults<T>(entries: readonly T[], all: boolean): readonly T[] { return all ? entries : entries.slice(0, 5); }

export function processingEndpoint(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol)) return null;
    url.username = ''; url.password = ''; url.search = ''; url.hash = '';
    return url.toString();
  } catch { return null; }
}

type RouteTranslate = (key: string, values?: Record<string, string | number>) => string;

function selectedProcessingService(snapshot: RuntimeSnapshot | null | undefined, task: 'dictation_stt' | 'llm', t: RouteTranslate): string {
  const selected = taskReadiness(snapshot, task);
  const provider = selected.provider;
  if (!provider) return t('mobile.none_selected');
  let destination: string;
  if (provider.kind === 'apple_speech' || provider.kind === 'apple_intelligence') {
    const service = t(provider.kind === 'apple_speech' ? 'mobile.simple.route_apple_dictation' : 'mobile.simple.route_apple_intelligence');
    destination = t(snapshot?.platform === 'ios' ? 'mobile.simple.route_on_iphone' : 'mobile.simple.route_apple_system', { service });
  } else if (provider.kind === 'pennel_cloud') {
    // The core chooses the authenticated Cloud origin; a saved account URL is not its route.
    destination = t('mobile.onboarding.cloud');
  } else {
    const endpoint = processingEndpoint(provider.base_url);
    destination = `${provider.name}${endpoint ? ` · ${endpoint}` : ''}`;
  }
  return selected.ready ? destination : `${destination} · ${t('mobile.simple.route_needs_setup')}`;
}

export function dictationProcessingRoutes(snapshot: RuntimeSnapshot | null | undefined, t: RouteTranslate): { audio: string; text: string } {
  const audio = selectedProcessingService(snapshot, 'dictation_stt', t);
  const text = snapshot?.settings.mode === 'fast' ? t('mobile.simple.route_no_cleanup') : selectedProcessingService(snapshot, 'llm', t);
  return { audio: t('mobile.simple.audio_route', { destination: audio }), text: t('mobile.simple.text_route', { destination: text }) };
}
