import type { InterfaceLanguage, NativeFailure } from './domain.ts';
import { translate } from './localization.ts';
import { ownedMetadata } from './owned-labels.ts';

function envelope(message: string): NativeFailure | null {
  const text = message.slice(0, 32_768);
  let candidates = 0;
  for (let start = text.indexOf('{'); start >= 0 && candidates++ < 32; start = text.indexOf('{', start + 1)) {
    let depth = 0; let quoted = false; let escaped = false;
    for (let end = start; end < text.length; end += 1) {
      const character = text[end];
      if (quoted) { if (escaped) escaped = false; else if (character === '\\') escaped = true; else if (character === '"') quoted = false; continue; }
      if (character === '"') { quoted = true; continue; }
      if (character === '{') depth += 1;
      if (character !== '}' || --depth !== 0) continue;
      try {
        const value: unknown = JSON.parse(text.slice(start, end + 1));
        if (!value || typeof value !== 'object' || !('code' in value) || !('message' in value)) break;
        const data = value as { code: unknown; message: unknown; recoverable?: unknown };
        if (typeof data.code !== 'string' || !/^[a-z][a-z0-9_]*$/.test(data.code) || typeof data.message !== 'string') break;
        return { code: data.code, message: data.message, recoverable: data.recoverable === true };
      } catch { break; }
    }
  }
  return null;
}

export function nativeFailure(error: unknown): NativeFailure {
  if (error && typeof error === 'object' && 'message' in error) {
    const value = error as { code?: unknown; message: unknown; recoverable?: unknown };
    const message = String(value.message);
    return envelope(message) ?? { code: typeof value.code === 'string' ? value.code : 'operation_failed', message, recoverable: value.recoverable === true };
  }
  if (typeof error === 'string') return envelope(error) ?? { code: 'operation_failed', message: error };
  return { code: 'operation_failed', message: 'The operation could not be completed.' };
}

const aliases: Record<string, string> = {
  speaker_stale: 'speaker_stale', speaker_checkpoint_version: 'speaker_checkpoint_version', speaker_checkpoint_invalid: 'speaker_checkpoint_invalid',
  speaker_retry_confirmation: 'speaker_retry_confirmation', speaker_cleanup: 'speaker_cleanup', speaker_model: 'speaker_model',
  stale_notes: 'stale_notes', notes_stale: 'stale_notes', quota: 'quota', quota_exceeded: 'quota', audio_quota: 'quota',
  auth: 'auth', unauthorized: 'auth', unauthenticated: 'auth', signed_out: 'auth', not_signed_in: 'auth', token_expired: 'auth', invalid_token: 'auth',
  provider_auth: 'provider_auth', http_401: 'provider_auth', http_403: 'provider_auth',
  forbidden: 'access', vault_locked: 'vault_locked', needs_recovery_code: 'vault_locked', vault_reset: 'vault_locked', storage: 'storage', storage_full: 'storage', speaker_storage: 'storage',
  network: 'network', offline: 'network', timeout: 'network', http_408: 'network', http_504: 'network', cancelled: 'cancelled', canceled: 'cancelled',
  interrupted: 'interrupted', recognition: 'recognition', consent_required: 'consent_required', microphone_denied: 'microphone_denied',
  unsupported: 'unsupported', unsupported_operation: 'unsupported', meeting_busy: 'meeting_busy', credential_store: 'credential_store',
  not_found: 'not_found', invalid_request: 'request', request: 'request', http_400: 'request', invalid_conflict: 'not_found',
  too_large: 'audio', invalid_audio: 'audio', audio_too_long: 'audio', http_413: 'audio',
  overloaded: 'overloaded', rate_limited: 'overloaded', http_429: 'overloaded', upstream: 'overloaded', speakers_unavailable: 'overloaded', speaker_unavailable: 'overloaded', http_500: 'overloaded', http_502: 'overloaded', http_503: 'overloaded',
  session_unavailable: 'session_unavailable', last_provider: 'last_provider', audio_unavailable: 'audio_unavailable', operation_busy: 'operation_busy',
  provider_changed: 'model_changed', provider_not_selected: 'model_changed', meeting_changed: 'meeting_busy',
  no_speech: 'no_speech', missing_api_key: 'missing_api_key', model_not_downloaded: 'model_not_downloaded',
  recording_gaps: 'recording_gaps', recording_terminated: 'recording_terminated', recording_resume_failed: 'recording_resume_failed',
  recording_microphone_lost: 'recording_microphone_lost', recording_paused: 'recording_paused',
  camera_denied: 'camera_denied', scanner_unavailable: 'scanner_unavailable', recovery_unconfirmed: 'recovery_unconfirmed',
  pairing_code: 'pairing_code', pairing_invalid: 'pairing_invalid', pairing_expired: 'pairing_expired', pairing_restart: 'pairing_restart',
  pairing_unavailable: 'pairing_unavailable', pairing_incompatible: 'pairing_incompatible', pairing_service_access: 'pairing_service_access', pairing_service_retry: 'pairing_service_retry', pairing_storage: 'pairing_storage', pairing_busy: 'overloaded',
  pairing_unknown: 'pairing_used', pairing_claimed: 'pairing_used', pairing_used: 'pairing_used',
  pairing_account: 'pairing_account', pairing_existing: 'pairing_existing', pairing_changed: 'pairing_invalid',
  pairing_transcript: 'pairing_invalid', pairing_message: 'pairing_invalid', pairing_phase: 'pairing_invalid', pairing_approval: 'pairing_invalid',
  pairing_retry: 'pairing_retry',
  pairing_not_found: 'pairing_expired', device_limit: 'device_limit', generation: 'pairing_library_changed',
  recovery_code: 'recovery_code', recovery_stale: 'recovery_unconfirmed', recovery_changed: 'recovery_unconfirmed', pairing_pending: 'pairing_pending',
  dictation_session_changed: 'practice_changed', dictation_owned: 'practice_owned',
};

export function localizedFailure(language: InterfaceLanguage, error: NativeFailure): string {
  return translate(language, `mobile.errors.${aliases[error.code.toLowerCase()] ?? 'operation_failed'}`);
}

const legacyOwnedCodes = new Map<string, string>([
  ...['Geen spraak herkend. Probeer opnieuw.', 'Geen spraak gehoord.', 'No speech recognized. Try again.', 'no speech detected'].map((text) => [text, 'no_speech'] as [string, string]),
  ...['Geen API-sleutel ingesteld', 'No API key configured'].map((text) => [text, 'missing_api_key'] as [string, string]),
  ...['API-sleutel geweigerd', 'API key rejected'].map((text) => [text, 'provider_auth'] as [string, string]),
  ...['Geen verbinding', 'No connection'].map((text) => [text, 'network'] as [string, string]),
  ...['Model nog niet gedownload', 'Model not downloaded'].map((text) => [text, 'model_not_downloaded'] as [string, string]),
  ...['Geen toegang tot de microfoon. Sta de microfoon toe in Instellingen.', 'No microphone access. Allow microphone access in Settings.'].map((text) => [text, 'microphone_denied'] as [string, string]),
  ...['De opname is gestopt omdat iOS Pennel heeft afgesloten.', 'The recording stopped because iOS closed Pennel.'].map((text) => [text, 'recording_terminated'] as [string, string]),
  ...['De opname is na een onderbreking gestopt omdat de microfoon niet terugkwam.', 'The recording stopped after an interruption because the microphone did not return.'].map((text) => [text, 'recording_resume_failed'] as [string, string]),
  ...['Pennel hoort de microfoon niet meer. De opname gaat verder zodra hij terug is.', 'Pennel cannot hear the microphone. Recording resumes when it returns.'].map((text) => [text, 'recording_microphone_lost'] as [string, string]),
  ...['Opname gepauzeerd door een oproep. Pennel neemt verder op zodra de oproep voorbij is.', 'Recording paused by a call. Pennel resumes after the call ends.'].map((text) => [text, 'recording_paused'] as [string, string]),
  ...['Deze opname bevat onderbrekingen. De tijdstempels houden rekening met de gaten.', 'This recording contains interruptions. Timestamps preserve the gaps.'].map((text) => [text, 'recording_gaps'] as [string, string]),
]);

/** Only app-owned failure channels enter here; notes, transcripts and provider content stay literal. */
export function ownedFailureText(language: InterfaceLanguage, message: string, code?: string | null): string {
  const selected = code && aliases[code.toLowerCase()] ? code : legacyOwnedCodes.get(message);
  return selected ? localizedFailure(language, { code: selected, message }) : ownedMetadata(language, message, 'mobile.errors.operation_failed');
}
