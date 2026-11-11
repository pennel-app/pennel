import { api } from "./api";
import type { Context } from "./main";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h, select } from "./dom";
import { providerDestination } from "./processing-state";
import type { SuggestionPolicy } from "./suggestion-consent-state";

export function suggestionAutomationChoice(ctx: Context) {
  let pending = false, active = true, failure = "";
  const description = h("p", { class: "help" });
  const reason = h("p", { class: "help suggestion-settings-reason" });
  const error = h("p", { class: "error", role: "status", hidden: true });
  const automatic = controlButton({ class: "btn compact", "data-suggestion-automation": "automatic", onclick: () => { void choose("automatic"); } }, l("settings.suggestions_automatic"));
  const manual = controlButton({ class: "btn compact", "data-suggestion-automation": "manual", onclick: () => { void choose("manual"); } }, l("settings.suggestions_manual"));
  const language = select([["", l("settings.suggestions_ask_language")], ["en", l("insights.language_en")], ["nl", l("insights.language_nl")], ["mixed", l("insights.language_mixed")]], "", value => { void choose(undefined, value === "" ? null : value as "en" | "nl" | "mixed"); }, "menu");
  const setup = h("button", { class: "text-button suggestion-settings-setup", onclick: () => ctx.navigate("providers") }, l("insights.connection_settings"));
  const root = h("section", { class: "meeting-suggestion-settings", "aria-label": l("settings.suggestions_title") }, h("h3", {}, l("settings.suggestions_title")), description,
    h("div", { class: "row" }, automatic, manual), h("label", { class: "suggestion-settings-language" }, l("settings.suggestions_fallback"), language), reason, setup, error);
  function refresh() {
    const policy = ctx.info.settings.suggestion_automation ?? "unanswered", fallback = ctx.info.settings.suggestion_language;
    root.dataset.suggestionAutomation = policy; automatic.setAttribute("aria-pressed", String(policy === "automatic")); manual.setAttribute("aria-pressed", String(policy === "manual"));
    automatic.disabled = manual.disabled = language.disabled = pending;
    language.value = ["en", "nl", "mixed"].includes(fallback ?? "") ? fallback! : "";
    const choice = ctx.info.settings.decision, provider = ctx.info.settings.providers.find(provider => provider.id === choice?.provider_id);
    const destination = `${provider ? providerDestination(provider) : t("insights.none")} · ${choice?.model ?? t("insights.none")}`;
    description.textContent = t("settings.suggestions_help", { destination });
    reason.textContent = !choice || !provider ? t("insights.reason_unconfigured") : t("settings.suggestions_qualification");
    error.textContent = failure; error.hidden = !failure;
  }
  async function choose(policy?: Exclude<SuggestionPolicy, "unanswered">, fallback?: "en" | "nl" | "mixed" | null) {
    if (pending) return;
    pending = true; failure = ""; refresh();
    try { await ctx.mutate(async () => { const settings = structuredClone((await api.info()).settings); if (policy) settings.suggestion_automation = policy; if (fallback !== undefined) settings.suggestion_language = fallback; return api.saveSettings(settings); }); }
    catch (error) { failure = errorText(error); }
    finally { pending = false; refresh(); }
  }
  ctx.onInfoChange(() => { if (active) refresh(); }); ctx.onDispose(() => { active = false; });
  bindLocalization(root, refresh); refresh();
  return { root };
}
