import { api, type InsightAvailability, type InsightLanguagePolicy, type InsightSnapshot } from "./api";
import type { Context } from "./main";
import { LatestRequest } from "./async";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h, select } from "./dom";
import { safeInsightDestination } from "../../../assets/contracts/meeting-insights.ts";
import { meetingPostprocess } from "./meeting-postprocess-state";
import { performSuggestionConsent, suggestionConsentPresentation, type SuggestionConsentAction, type SuggestionPolicy } from "./suggestion-consent-state";

export function meetingSuggestionConsent(initial: Context, host: {
  id(): string; uid: string; snapshot(): InsightSnapshot | null; language(): InsightLanguagePolicy;
  analysisBusy(): boolean; canAnalyze(): boolean; chooseLanguage(language: InsightLanguagePolicy): Promise<void>;
  start(): Promise<unknown>; dismiss(): Promise<void>; canDismiss(): boolean; blockedReason(): string;
  reason(code: string): string; changed(): void; focusTarget: HTMLElement; languageControlVisible(): boolean;
  manualAvailability?(): InsightAvailability | null;
}) {
  let ctx = initial, bound = false, binding = 0, generation = 0, loaded = false, loading = false, pending = false;
  let dismissed = false, readFailure = false, lastRead = -Infinity, failure = "", queryKey = "", policy: SuggestionPolicy = ctx.info.settings.suggestion_automation ?? "unanswered";
  let resolvedLanguage: Exclude<InsightLanguagePolicy, "unknown"> | null = null;
  let availability: InsightAvailability | null = null, stopProgress: (() => void) | null = null;
  const reads = new LatestRequest();
  const automatic = h("input", { type: "checkbox", "data-suggestion-consent-automatic": true });
  const language = select([["unknown", l("insights.language_unknown")], ["en", l("insights.language_en")], ["nl", l("insights.language_nl")], ["mixed", l("insights.language_mixed")]], "unknown", value => { void host.chooseLanguage(value as InsightLanguagePolicy); }, "menu");
  const languageRow = h("label", { class: "insight-language suggestion-consent-language" }, l("insights.language"), language);
  const destination = h("p", { class: "help" });
  const find = controlButton({ class: "btn primary", "data-suggestion-consent": "find", onclick: () => { void choose("consent"); } }, l("insights.find"));
  const later = controlButton({ class: "btn", "data-suggestion-consent": "not_now", onclick: () => { void choose("not_now"); } }, l("meetings.summary_consent_not_now"));
  const block = h("p", { class: "help suggestion-consent-block", hidden: true });
  const question = h("div", { class: "summary-consent-card" }, h("h3", {}, l("insights.consent_question")), destination, languageRow,
    h("label", { class: "checkbox" }, automatic, l("meetings.summary_consent_automatic")), h("div", { class: "row" }, find, later), block);
  const quietFind = h("button", { class: "text-button", "data-suggestion-consent": "one_off", onclick: () => { void choose("one_off"); } }, l("insights.find"));
  const quiet = h("div", { class: "summary-consent-quiet" }, h("p", { class: "help" }, l("insights.consent_quiet")), quietFind);
  const status = h("p", { class: "help suggestion-consent-status", role: "status" });
  const error = h("p", { class: "error suggestion-consent-error", hidden: true, role: "status" });
  const retry = controlButton({ class: "btn compact", hidden: true, onclick: () => { void load(true); } }, l("common.retry"));
  const root = h("section", { class: "summary-consent suggestion-consent", tabindex: -1, "aria-label": l("insights.consent_question") }, question, quiet, status, error, retry);
  function invalidate() { ++generation; reads.cancel(); loading = false; }
  function preparing() { return (["postprocess", "summary"] as const).some(stage => meetingPostprocess.get(host.uid, stage)?.state === "started"); }
  function blocksAnalysis() { return pending || state().mode === "loading" || host.analysisBusy() || preparing() || meetingPostprocess.get(host.uid, "suggestions")?.state === "started"; }
  function state() {
    const current = loaded && queryKey === key() ? availability : null, manual = host.manualAvailability?.();
    // Refresh can observe a connection edit before the cached automatic-status read catches up.
    return suggestionConsentPresentation({ policy, snapshot: host.snapshot(), availability: current && manual?.available === false ? manual : current, dismissed,
      working: host.analysisBusy() || meetingPostprocess.get(host.uid, "suggestions")?.state === "started",
      summaryWorking: preparing() });
  }
  function needsLanguage() { return !resolvedLanguage; }
  function key() { return `${host.id()}:${host.language()}`; }
  function refresh() {
    const current = state(); root.dataset.mode = current.mode; root.setAttribute("aria-busy", String(pending || loading));
    question.hidden = !current.showQuestion; quiet.hidden = current.mode !== "quiet";
    languageRow.hidden = !needsLanguage() || host.languageControlVisible(); language.value = host.language(); language.disabled = pending;
    const target = safeInsightDestination(availability?.endpoint);
    const where = target.host ? target.on_device ? t("insights.this_device", { host: target.host }) : target.host : t("insights.none");
    destination.textContent = t("insights.consent_destination", { destination: `${where} · ${availability?.model ?? t("insights.none")}` });
    automatic.disabled = pending; find.disabled = quietFind.disabled = blocksAnalysis() || !host.canAnalyze();
    later.disabled = pending || !host.canDismiss() || preparing(); later.title = host.blockedReason();
    block.textContent = preparing() ? t("insights.waiting_processing") : host.canDismiss() ? "" : host.blockedReason(); block.hidden = !block.textContent;
    error.hidden = !failure; error.textContent = failure; retry.hidden = !failure || !readFailure; retry.disabled = pending || loading;
    status.textContent = current.mode === "running" ? t("insights.automatic_running") : current.mode === "waiting_summary" ? t("insights.waiting_processing")
      : current.mode === "done_empty" ? t("insights.none_found") : current.mode === "stale" ? t("insights.automatic_stale")
      : current.mode === "failed" ? `${host.reason(current.reason ?? "failed")} ${t("insights.automatic_no_retry")}`
      : current.mode === "not_run" || current.mode === "unavailable" ? host.reason(current.reason ?? "not_run")
      : current.mode === "loading" ? t("insights.checking_options") : "";
    status.hidden = !status.textContent; root.hidden = question.hidden && quiet.hidden && status.hidden && error.hidden;
    if (bound && !pending && !loading && !failure && (queryKey !== key() || performance.now() - lastRead > 3000)) void load();
  }
  async function load(force = false) {
    if (!bound || loading && !force) return;
    const mine = binding, request = ++generation, id = host.id(), chosen = host.language(), expectedKey = key();
    loading = true; failure = ""; readFailure = false; refresh();
    const checkAvailability = async () => {
      const actual = await api.automaticInsightsAvailability(id);
      const next = !actual.resolved_language && chosen !== "unknown" ? await api.automaticInsightsAvailability(id, chosen) : actual;
      return { next, resolved: actual.resolved_language ?? null };
    };
    await reads.run(() => Promise.all([checkAvailability(), api.meetingSuggestionPromptDismissed(id)]), ([{ next, resolved }, value]) => {
      if (!bound || mine !== binding || id !== host.id() || expectedKey !== key()) return;
      resolvedLanguage = resolved; availability = next; dismissed = value; loaded = true; queryKey = expectedKey; lastRead = performance.now();
      if (chosen === "unknown" && resolved) void host.chooseLanguage(resolved);
    }, reason => { if (bound && mine === binding && id === host.id() && expectedKey === key()) { failure = errorText(reason); readFailure = true; } });
    if (bound && mine === binding && request === generation) { loading = false; refresh(); host.changed(); }
  }
  async function choose(action: SuggestionConsentAction) {
    if (pending || action !== "not_now" && (blocksAnalysis() || !host.canAnalyze()) || action === "not_now" && (!host.canDismiss() || preparing())) return;
    const focused = root.contains(document.activeElement), mine = binding;
    invalidate(); pending = true; failure = ""; readFailure = false; refresh(); host.changed();
    if (focused) host.focusTarget.focus({ preventScroll: true });
    try {
      if (action === "consent") {
        const current = await api.automaticInsightsAvailability(host.id(), host.language() === "unknown" ? undefined : host.language());
        if (!current.available) throw new Error(host.reason(current.reason_code));
      }
      await performSuggestionConsent(action, automatic.checked, host.language(), needsLanguage(), {
        savePolicy: (next, fallback) => ctx.mutate(async () => {
          const settings = structuredClone((await api.info()).settings); settings.suggestion_automation = next;
          if (fallback) settings.suggestion_language = fallback;
          return api.saveSettings(settings);
        }),
        start: host.start,
        dismiss: async () => { await host.dismiss(); invalidate(); dismissed = true; loaded = true; queryKey = key(); lastRead = performance.now(); },
      });
      invalidate();
    } catch (reason) { if (mine === binding) failure = errorText(reason); }
    finally {
      const restore = focused && document.activeElement === host.focusTarget;
      pending = false; refresh(); host.changed();
      if (restore && !root.hidden) (state().mode === "quiet" ? quietFind : !question.hidden ? action === "not_now" ? later : find : root).focus({ preventScroll: true });
    }
  }
  bindLocalization(root, refresh); refresh();
  return { root, refresh, get busy() { return pending; }, get blocksAnalysis() { return blocksAnalysis(); }, get ownsFind() { return ["consent", "quiet"].includes(state().mode); }, get hasStatus() { return !status.hidden; }, get mode() { return state().mode; }, get reason() { return state().reason; }, findOnce: () => choose("one_off"), bind(next: Context) {
    ctx = next; policy = ctx.info.settings.suggestion_automation ?? "unanswered"; const mine = ++binding; bound = true; invalidate(); queryKey = "";
    next.onInfoChange(() => { if (mine !== binding) return; const nextPolicy = ctx.info.settings.suggestion_automation ?? "unanswered"; if (nextPolicy !== policy && nextPolicy === "unanswered") automatic.checked = false; policy = nextPolicy; invalidate(); queryKey = ""; refresh(); });
    stopProgress?.(); const stop = meetingPostprocess.subscribe(() => { refresh(); host.changed(); }); stopProgress = stop;
    next.onDispose(() => { stop(); if (mine === binding) { bound = false; invalidate(); if (stopProgress === stop) stopProgress = null; } });
    refresh();
  } };
}
