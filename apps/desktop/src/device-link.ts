import type { CloudLinkPhase, CloudLinkRole, CloudLinkView, CloudSyncSetup } from "./api.ts";

/** Failed phases and their owned error describe one outcome, even after native state is restored. */
export function connectionFeedback(view: CloudLinkView | null, failure: unknown | null): { status: CloudLinkPhase | null; error: unknown | null } {
  const error = failure ?? (view && (view.error_code || view.error_message || view.phase === "failed") ? { code: view.error_code ?? "error" } : null);
  return { status: view && view.phase !== "failed" && view.phase !== "complete" ? view.phase : null, error };
}

export function linkIsPending(phase: CloudLinkPhase): boolean {
  return ["waiting_peer", "confirming", "awaiting_approval", "waiting_approval", "provisioning"].includes(phase);
}

export function connectionRole(setup: CloudSyncSetup): CloudLinkRole {
  return setup.can_approve_devices ? "trusted_device" : "new_device";
}

export function syncSetupAction(setup: CloudSyncSetup): "sign_in" | "create" | "unlock" | "resume" | "active" {
  switch (setup.phase) {
    case "signed_out": return "sign_in";
    case "new_library": return "create";
    case "locked": return "unlock";
    case "ready":
    case "paused": return "resume";
    case "active": return "active";
  }
}

export function syncRequiresBackup(setup: CloudSyncSetup, confirmed: boolean | null): boolean {
  return setup.phase === "ready" || (!setup.sync_enabled && setup.phase === "paused" && confirmed === false);
}

/** The QR is a native-generated image, never active markup from the relay. */
export function qrImageSource(svg: string | null): string | null {
  if (!svg || svg.length > 262_144 || !/^<svg(?:\s|>)/.test(svg.trim())) return null;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
