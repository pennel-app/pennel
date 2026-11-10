import { api } from "./api";
import type { Context } from "./main";
import { LatestRequest } from "./async";
import { controlButton, errorText, h } from "./dom";
import { bindLocalization, l, t } from "./i18n";
import { languageModelConfigured } from "./presentation";
import { summaryDestination } from "./meeting-summary-automation";
import { performSummaryConsent, type SummaryConsentAction } from "./summary-consent-state";

export function meetingSummaryConsent(initial: Context, host: {
  id(): string; finished(): boolean; working(): boolean; canGenerate(): boolean; canDismiss(): boolean;
  blockedReason(): string; generate(): Promise<void>; dismiss(): Promise<void>; focusTarget: HTMLElement;
}) {
  let ctx = initial, binding = 0, bound = false, pending = false, loading = false, loaded = false, dismissed = false;
  let loadedId = "", lastRead = -Infinity, failure = "", lastPolicy = ctx.info.settings.summary_automation ?? "unanswered";
  const requests = new LatestRequest();
  const automatic = h("input", { type: "checkbox", "data-summary-consent-automatic": true });
  const description = h("p", { class: "help" });
  const reason = h("p", { class: "help summary-consent-reason" });
  const create = controlButton({ class: "btn primary", "data-summary-consent": "create", onclick: () => { void choose("create"); } }, l("meetings.summary_generate"));
  const later = controlButton({ class: "btn", "data-summary-consent": "not_now", onclick: () => { void choose("not_now"); } }, l("meetings.summary_consent_not_now"));
  const quietCreate = h("button", { class: "text-button", "data-summary-consent": "quiet_create", onclick: () => { void choose("quiet_create"); } }, l("meetings.summary_generate"));
  const setup = h("button", { class: "text-button summary-consent-setup", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_summaries"));
  const question = h("div", { class: "summary-consent-card" }, h("h3", {}, l("meetings.summary_consent_question")), description,
    h("label", { class: "checkbox" }, automatic, l("meetings.summary_consent_automatic")), h("div", { class: "row" }, create, later));
  const quiet = h("div", { class: "summary-consent-quiet" }, h("p", { class: "help" }, l("meetings.summary_consent_quiet")), quietCreate);
  const checking = h("p", { class: "help", role: "status" }, l("meetings.summary_consent_reading"));
  const message = h("p", { class: "error", role: "status", hidden: true });
  const retry = controlButton({ class: "btn compact", hidden: true, onclick: () => { void load(); } }, l("common.retry"));
  const root = h("section", { class: "summary-consent", hidden: true, "aria-label": l("meetings.summary_consent_question") }, question, quiet, checking, reason, setup, message, retry);

  function refresh() {
    if (!bound) { root.hidden = true; return; }
    const policy = lastPolicy;
    const visible = host.finished() && policy === "unanswered" && !host.working();
    const heldFocus = root.contains(document.activeElement) ? document.activeElement as HTMLElement : null;
    root.hidden = !visible;
    if (!visible) { if (heldFocus && document.activeElement === heldFocus) host.focusTarget.focus({ preventScroll: true }); return; }
    const mode = !loaded ? failure ? "error" : "loading" : dismissed ? "quiet" : "question";
    root.dataset.mode = mode; root.setAttribute("aria-busy", String(pending || loading));
    question.hidden = mode !== "question"; quiet.hidden = mode !== "quiet"; checking.hidden = mode !== "loading";
    message.hidden = !failure; message.textContent = failure;
    retry.hidden = !failure || loaded; retry.disabled = loading || pending;
    const ready = languageModelConfigured(ctx.info), blocked = host.blockedReason();
    description.textContent = ready ? t("meetings.summary_consent_destination", { destination: summaryDestination(ctx) }) : t("meetings.summary_setup_help");
    reason.textContent = blocked; reason.hidden = !blocked || mode === "loading";
    setup.hidden = ready; automatic.disabled = pending;
    create.disabled = pending || !host.canGenerate(); quietCreate.disabled = create.disabled;
    create.title = blocked; quietCreate.title = blocked;
    later.disabled = pending || !host.canDismiss();
    if (mode === "quiet" && heldFocus && question.contains(heldFocus)) quietCreate.focus({ preventScroll: true });
    if (!loading && !failure && !pending && (loadedId !== host.id() || performance.now() - lastRead > 3000)) void load();
  }
  async function load() {
    if (!bound || !host.finished()) return;
    const mine = binding, id = host.id(), retryFocus = document.activeElement === retry; loading = true; failure = ""; refresh();
    await requests.run(() => api.meetingSummaryPromptDismissed(id), value => {
      if (!bound || mine !== binding || host.id() !== id) return;
      dismissed = value; loaded = true; loadedId = id; lastRead = performance.now();
    }, error => { if (bound && mine === binding && host.id() === id) failure = errorText(error); });
    if (bound && mine === binding) {
      loading = false; refresh();
      if (retryFocus && !root.hidden && document.activeElement === document.body) (failure ? retry : dismissed ? quietCreate : create).focus({ preventScroll: true });
    }
  }
  async function choose(action: SummaryConsentAction) {
    if (pending) return;
    const hadFocus = root.contains(document.activeElement);
    pending = true; failure = ""; refresh();
    if (hadFocus) host.focusTarget.focus({ preventScroll: true });
    try {
      await performSummaryConsent(action, automatic.checked, {
        savePolicy: policy => ctx.mutate(async () => {
          const settings = structuredClone((await api.info()).settings);
          settings.summary_automation = policy;
          return api.saveSettings(settings);
        }),
        generate: host.generate,
        dismiss: async () => { await host.dismiss(); dismissed = true; loaded = true; loadedId = host.id(); lastRead = performance.now(); },
      });
    } catch (error) { failure = errorText(error); }
    finally {
      const restoreFocus = hadFocus && document.activeElement === host.focusTarget;
      pending = false; refresh();
      if (restoreFocus && !root.hidden) (action === "not_now" ? dismissed ? quietCreate : later : action === "quiet_create" ? quietCreate : create).focus({ preventScroll: true });
    }
  }
  bindLocalization(root, refresh);
  return { root, refresh, bind(next: Context) {
    ctx = next; lastPolicy = ctx.info.settings.summary_automation ?? "unanswered"; const mine = ++binding; bound = true; requests.cancel(); loading = false; loadedId = "";
    next.onInfoChange(() => {
      if (mine !== binding) return;
      const policy = ctx.info.settings.summary_automation ?? "unanswered";
      if (policy !== lastPolicy && policy === "unanswered") { automatic.checked = false; loadedId = ""; }
      lastPolicy = policy; refresh();
    });
    next.onDispose(() => { if (mine === binding) { bound = false; requests.cancel(); loading = false; } });
    refresh();
  } };
}
