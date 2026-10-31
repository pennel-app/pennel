import { t, type TranslationKey } from "./i18n.ts";

const messages: Readonly<Record<string, TranslationKey>> = {
  pairing_unavailable: "account.error.unavailable",
  pairing_incompatible: "account.error.incompatible",
  http_403: "account.error.service_access",
  pairing_busy: "account.error.fresh_code",
  pairing_claimed: "account.error.fresh_code",
  pairing_used: "account.error.fresh_code",
  pairing_unknown: "account.error.fresh_code",
  pairing_expired: "account.error.expired",
  pairing_restart: "account.error.fresh_code",
  pairing_invalid: "account.error.verify",
  pairing_changed: "account.error.verify",
  pairing_transcript: "account.error.verify",
  pairing_message: "account.error.verify",
  pairing_phase: "account.error.verify",
  pairing_approval: "account.error.verify",
  pairing_code: "account.error.verify",
  pairing_storage: "account.error.storage",
  storage: "account.error.storage",
  pairing_retry: "account.error.network",
  network: "account.error.network",
  internal: "account.error.network",
  http_408: "account.error.network",
  http_502: "account.error.network",
  http_503: "account.error.network",
  rate_limited: "account.error.rate",
  too_many: "account.error.rate",
  slow_down: "account.error.rate",
  http_429: "account.error.rate",
  device_limit: "account.error.device_limit",
  pairing_account: "account.error.account",
  pairing_existing: "account.error.account",
  generation: "account.error.library_changed",
  pairing_library_changed: "account.error.library_changed",
  vault_reset: "account.error.library_changed",
  unauthorized: "account.error.sign_in",
  session_revoked: "account.error.sign_in",
  not_signed_in: "account.error.sign_in",
  signed_out: "account.error.sign_in",
  vault_locked: "account.error.unlock",
  needs_recovery_code: "account.error.unlock",
  recovery_changed: "account.error.recovery",
  recovery_code: "account.error.recovery",
  pairing_pending: "account.error.pending",
  cancelled: "account.error.cancelled",
};

/** Account failures are owned status, never provider text or user content to echo back. */
export function accountFailureText(error: unknown): string {
  const code = typeof error === "object" && error !== null ? (error as { code?: unknown }).code : null;
  return t(typeof code === "string" ? messages[code] ?? (/^http_5\d\d$/.test(code) ? "account.error.network" : "account.error.generic") : "account.error.generic");
}
