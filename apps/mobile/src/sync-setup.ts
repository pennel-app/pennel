import type { CloudLinkPhase, CloudLinkRole, CloudLinkView, CloudRecoveryDisplay, CloudState, CloudSyncSetup, NativeFailure } from '../modules/pennel-mobile/src/types.ts';

export function requiresRecovery(error: Pick<NativeFailure, 'code'> | null | undefined): boolean {
  return error?.code === 'vault_locked' || error?.code === 'vault_reset';
}

export function syncSetupStage(cloud: Pick<CloudState, 'signed_in' | 'recovery_confirmed'>, setup: CloudSyncSetup | null): 'sign_in' | 'checking' | 'create' | 'unlock' | 'backup' | 'resume' | 'connected' {
  if (!cloud.signed_in) return 'sign_in';
  if (!setup) return 'checking';
  if (setup.phase === 'locked') return 'unlock';
  if (setup.phase === 'new_library') return 'create';
  if (setup.phase === 'ready' || !cloud.recovery_confirmed) return 'backup';
  if (setup.phase === 'paused') return 'resume';
  if (setup.phase === 'signed_out') return 'sign_in';
  return 'connected';
}

export function syncStatusKey(cloud: Pick<CloudState, 'signed_in' | 'sync_enabled' | 'syncing' | 'status'>, setup?: Pick<CloudSyncSetup, 'phase'> | null): string {
  if (!cloud.signed_in) return 'mobile.sync.not_connected';
  if (!cloud.sync_enabled && setup && ['new_library', 'locked', 'ready'].includes(setup.phase)) return 'mobile.sync.setup_needed';
  if (!cloud.sync_enabled) return 'mobile.sync.paused';
  if (cloud.syncing) return 'account.sync.busy';
  if (cloud.status === 'synced') return 'account.sync.synced';
  if (cloud.status === 'offline') return 'account.sync.offline';
  if (cloud.status === 'needs_recovery_code' || cloud.status === 'vault_reset') return 'mobile.cloud.recovery_required';
  if (cloud.status === 'pending') return 'account.sync.first_pending';
  return 'mobile.sync.needs_attention';
}

export function linkIsPending(phase: CloudLinkPhase): boolean {
  return !['idle', 'complete', 'cancelled', 'expired', 'failed'].includes(phase);
}

export function restorableLink(view: CloudLinkView, role: CloudLinkRole, cloud: Pick<CloudState, 'signed_in' | 'account'> | null | undefined, ownedRequest: string | null = null): CloudLinkView | null {
  if (!cloud || (!cloud.signed_in && view.role === 'trusted_device')
    || (cloud.signed_in && view.account_email && view.account_email !== cloud.account?.email)) return null;
  return view.role === role && (linkIsPending(view.phase) || (view.phase === 'complete' && view.request_id !== null && view.request_id === ownedRequest)) ? view : null;
}

export type RecoveryDisplay = CloudRecoveryDisplay & { recovery_code: string; confirmation_token: string };

export function recoveryDisplayFor(cloud: Pick<CloudState, 'signed_in' | 'account_epoch' | 'recovery_display_token'> | null | undefined, display: CloudRecoveryDisplay | null | undefined): RecoveryDisplay | null {
  if (!cloud?.signed_in || !display?.recovery_code || !display.confirmation_token || !cloud.account_epoch
    || display.account_epoch !== cloud.account_epoch || display.confirmation_token !== cloud.recovery_display_token) return null;
  return display as RecoveryDisplay;
}
