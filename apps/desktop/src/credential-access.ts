import { api } from "./api";
import { controlButton, h, section } from "./dom";
import { bindLocalization, l, t } from "./i18n";
import type { Context } from "./main";

// A settings-category change must not admit another migration while the first is pending.
let updating = false;
let failed = false;
const listeners = new Set<() => void>();

export function credentialAccess(ctx: Context): HTMLElement | null {
  const state = ctx.info.credential_store;
  if (state.status === "ready") { failed = false; return null; }
  const status = state.status;
  let disposed = false;
  const message = h("p", { class: "help", role: "status", "aria-live": "polite", "data-credential-message": true });
  const pending = state.pending > 0
    ? h("p", { class: "help" }, l("credentials.pending", () => ({ count: state.pending }))) : null;
  const action = controlButton({ "data-credential-update": true, onclick: async () => {
    if (updating) return;
    updating = true;
    failed = false;
    for (const update of listeners) update();
    // Keychain approval can wait for the user; it must not hold the settings/navigation queue.
    try { await api.migrateSavedCredentials(); await ctx.refresh(); }
    catch { failed = true; await ctx.refresh().catch(ctx.report); }
    finally { updating = false; for (const update of listeners) update(); }
  } });
  const body = section(l("credentials.title"), null, message, pending, action);
  body.dataset.credentialAccess = state.status;
  const update = () => {
    if (!updating && !disposed && (ctx.info.credential_store.status !== status || ctx.info.credential_store.pending !== state.pending)) {
      ctx.rerender();
      return;
    }
    body.setAttribute("aria-busy", String(updating));
    message.textContent = updating ? t("credentials.updating") : failed && status === "migration_required" ? t("credentials.failed") : t(`credentials.${status}`);
    message.className = failed && !updating ? "error" : "help";
    action.disabled = updating;
    action.textContent = updating ? t("credentials.updating") : state.status === "migration_required" ? t("credentials.update") : t("credentials.retry");
  };
  listeners.add(update);
  bindLocalization(body, update);
  ctx.onDispose(() => { disposed = true; listeners.delete(update); });
  update();
  return body;
}
