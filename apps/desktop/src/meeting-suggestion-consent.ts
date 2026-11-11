import { api, type InsightAvailability, type InsightLanguagePolicy, type InsightSnapshot } from "./api";
import type { Context } from "./main";
import { LatestRequest } from "./async";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h, select } from "./dom";
import { safeInsightDestination } from "../../../assets/contracts/meeting-insights.ts";
import { meetingPostprocess } from "./meeting-postprocess-state";
import { inferenceFingerprint } from "./inference-state";
import { performSuggestionConsent, requireSuggestionAvailability, SuggestionAvailabilityError, suggestionConsentPresentation, type SuggestionConsentAction, type SuggestionPolicy } from "./suggestion-consent-state";

export function meetingSuggestionConsent(initial: Context, host: {
  id(): string; uid: string; snapshot(): InsightSnapshot | null; language(): InsightLanguagePolicy;
  analysisBusy(): boolean; canAnalyze(): boolean; chooseLanguage(language: InsightLanguagePolicy): Promise<void>;
  start(expectedConfigurationFingerprint: string): Promise<unknown>; dismiss(): Promise<void>; canDismiss(): boolean; blockedReason(): string;
  reason(code: string): string; changed(): void; focusTarget: HTMLElement; languageControlVisible(): boolean;
  manualAvailability?(): InsightAvailability | null;
}) {
  let ctx = initial, bound = false, binding = 0, generation = 0, choiceEpoch = 0, loaded = false, loading = false, pending = false;
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
    return suggestionConsentPresentation({ policy, snapshot: host.snapshot(), availability: current, manualAvailability: manual, dismissed,
      working: host.analysisBusy() || meetingPostprocess.get(host.uid, "suggestions")?.state === "started",
      summaryWorking: preparing() });
  }
  function needsLanguage() { return !resolvedLanguage; }
  function key() { return `${host.id()}:${host.language()}`; }
  function automaticEligible() { return loaded && queryKey === key() && availability?.available === true; }
  function manualOnly() { return loaded && queryKey === key() && !availability?.available && host.manualAvailability?.()?.available === true; }
  function refresh() {
    const current = state(); root.dataset.mode = current.mode; root.setAttribute("aria-busy", String(pending || loading));
    question.hidden = !current.showQuestion; quiet.hidden = current.mode !== "quiet";
    languageRow.hidden = !needsLanguage() || host.languageControlVisible(); language.value = host.language(); language.disabled = pending;
    const ready = host.manualAvailability?.()?.available ? host.manualAvailability() : availability;
    const target = safeInsightDestination(ready?.endpoint);
    const where = target.host ? target.on_device ? t("insights.this_device", { host: target.host }) : target.host : t("insights.none");
    destination.textContent = t("insights.consent_destination", { destination: `${where} · ${ready?.model ?? t("insights.none")}` });
    if (!automaticEligible()) automatic.checked = false;
    automatic.disabled = pending || !automaticEligible(); find.disabled = quietFind.disabled = blocksAnalysis() || !host.canAnalyze();
    later.disabled = pending || !host.canDismiss() || preparing(); later.title = host.blockedReason();
    block.textContent = preparing() ? t("insights.waiting_processing") : !host.canDismiss() ? host.blockedReason() : manualOnly() ? t("insights.manual_only") : ""; block.hidden = !block.textContent;
    error.hidden = !failure; error.textContent = failure; retry.hidden = !failure || !readFailure; retry.disabled = pending || loading;
    status.textContent = current.mode === "running" ? t("insights.automatic_running") : current.mode === "waiting_summary" ? t("insights.waiting_processing")
      : current.mode === "done_empty" ? t("insights.none_found") : current.mode === "stale" ? t("insights.automatic_stale")
      : current.mode === "failed" ? `${host.reason(current.reason ?? "failed")} ${t("insights.automatic_no_retry")}`
      : current.mode === "not_run" && manualOnly() ? t("insights.manual_only")
      : current.mode === "not_run" || current.mode === "unavailable" ? host.reason(current.reason ?? "not_run")
      : current.mode === "loading" ? t("insights.checking_options") : "";
    status.hidden = !status.textContent; root.hidden = question.hidden && quiet.hidden && status.hidden && error.hidden;
    const refreshAfterActionFailure = !!failure && !readFailure && queryKey !== key();
    if (bound && !pending && !loading && (!failure || refreshAfterActionFailure) && (queryKey !== key() || performance.now() - lastRead > 3000)) void load(false, refreshAfterActionFailure);
  }
  async function load(force = false, preserveActionFailure = false) {
    if (!bound || loading && !force) return;
    const mine = binding, request = ++generation, id = host.id(), chosen = host.language(), expectedKey = key();
    loading = true; if (!preserveActionFailure) failure = ""; readFailure = false; refresh();
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
    if (!bound || pending || action !== "not_now" && (blocksAnalysis() || !host.canAnalyze()) || action === "not_now" && (!host.canDismiss() || preparing())) return;
    const focused = root.contains(document.activeElement), mine = binding, operation = ++choiceEpoch, ownerContext = ctx, wantedAutomatic = action === "consent" && automatic.checked;
    const expectedConnection = inferenceFingerprint(ctx.info, "decision"), id = host.id(), chosen = host.language();
    const displayedConfiguration = host.manualAvailability?.()?.configuration_fingerprint;
    const sameScope = () => bound && mine === binding && operation === choiceEpoch && ctx === ownerContext && id === host.id() && chosen === host.language();
    const localCurrent = () => sameScope() && (action === "not_now" || displayedConfiguration === host.manualAvailability?.()?.configuration_fingerprint);
    const requireCurrent = () => { if (!localCurrent()) throw new Error(host.reason("configuration_changed")); };
    const unchanged = async () => {
      if (!localCurrent()) return false;
      const latest = await api.info();
      return localCurrent() && expectedConnection === inferenceFingerprint(latest, "decision");
    };
    invalidate(); pending = true; failure = ""; readFailure = false; refresh(); host.changed();
    if (focused) host.focusTarget.focus({ preventScroll: true });
    try {
      await performSuggestionConsent(action, wantedAutomatic, chosen, needsLanguage(), {
        validate: async automatic => {
          try {
            if (typeof displayedConfiguration !== "string" || !/^[a-f0-9]{64}$/.test(displayedConfiguration)) throw new SuggestionAvailabilityError("configuration_changed");
            await requireSuggestionAvailability(automatic, { manual: async () => {
              const current = await api.insightsAvailability(id, chosen);
              requireCurrent();
              if (current.configuration_fingerprint !== displayedConfiguration) throw new SuggestionAvailabilityError("configuration_changed");
              return current;
            },
              automatic: () => api.automaticInsightsAvailability(id, chosen === "unknown" ? undefined : chosen), unchanged });
          } catch (error) { if (error instanceof SuggestionAvailabilityError) throw new Error(host.reason(error.code)); throw error; }
        },
        savePolicy: (next, fallback) => {
          requireCurrent();
          return ownerContext.mutate(async () => {
            requireCurrent();
            const latest = await api.info();
            requireCurrent();
            if (expectedConnection !== inferenceFingerprint(latest, "decision")) throw new Error(host.reason("configuration_changed"));
            requireCurrent();
            return api.setMeetingSuggestionPolicyChecked(chosen, displayedConfiguration!, next, fallback).catch(reason => {
              const code = errorText(reason);
              if (["configuration_changed", "foreground_impact_unaccepted", "unqualified", "unknown_language"].includes(code)) throw new Error(host.reason(code));
              throw reason;
            });
          });
        },
        start: async () => { if (!await unchanged()) throw new Error(host.reason("configuration_changed")); requireCurrent(); return host.start(displayedConfiguration!); },
        dismiss: async () => { requireCurrent(); await host.dismiss(); if (!localCurrent()) return; invalidate(); dismissed = true; loaded = true; queryKey = key(); lastRead = performance.now(); },
      });
      if (localCurrent()) invalidate();
    } catch (reason) { if (sameScope()) failure = errorText(reason); }
    finally {
      if (bound && mine === binding && operation === choiceEpoch) {
        const restore = localCurrent() && focused && document.activeElement === host.focusTarget;
        pending = false; refresh(); host.changed();
        if (restore && localCurrent() && !root.hidden) (state().mode === "quiet" ? quietFind : !question.hidden ? action === "not_now" ? later : find : root).focus({ preventScroll: true });
      }
    }
  }
  bindLocalization(root, refresh); refresh();
  return { root, refresh, get busy() { return pending; }, get blocksAnalysis() { return blocksAnalysis(); }, get ownsFind() { return ["consent", "quiet"].includes(state().mode); }, get hasStatus() { return !status.hidden; }, get mode() { return state().mode; }, get reason() { return state().reason; }, findOnce: () => choose("one_off"), bind(next: Context) {
    ctx = next; policy = ctx.info.settings.suggestion_automation ?? "unanswered"; const mine = ++binding; bound = true; ++choiceEpoch; pending = false; failure = ""; readFailure = false; invalidate(); queryKey = "";
    next.onInfoChange(() => { if (mine !== binding) return; const nextPolicy = ctx.info.settings.suggestion_automation ?? "unanswered"; if (nextPolicy !== policy && nextPolicy === "unanswered") automatic.checked = false; policy = nextPolicy; invalidate(); queryKey = ""; refresh(); });
    stopProgress?.(); const stop = meetingPostprocess.subscribe(() => { refresh(); host.changed(); }); stopProgress = stop;
    next.onDispose(() => { stop(); if (mine === binding) { bound = false; ++choiceEpoch; pending = false; invalidate(); if (stopProgress === stop) stopProgress = null; } });
    refresh();
  } };
}
