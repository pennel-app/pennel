import type { Context } from "./main";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h } from "./dom";
import { languageModelConfigured } from "./presentation";
import { providerDestination } from "./processing-state";
import "./meeting-workspace.css";

export function summaryDestination(ctx: Context): string {
  const choice = ctx.info.settings.llm;
  const provider = ctx.info.settings.providers.find(provider => provider.id === choice.provider_id);
  if (!provider) return t("meetings.summary_destination_missing");
  const destination = providerDestination(provider);
  return choice.model ? `${destination} · ${choice.model}` : destination;
}

export function summaryAutomationChoice(initial: Context, options: { compact?: boolean } = {}) {
  let ctx = initial, pending = false;
  const text = h("p", { class: "help meeting-automation-explanation" });
  const message = h("p", { class: "error", role: "status", hidden: true });
  const automatic = controlButton({ class: "btn compact", "data-summary-automation": "automatic", onclick: () => choose("automatic") }, l("meetings.summary_automation_automatic"));
  const manual = controlButton({ class: "btn compact", "data-summary-automation": "manual", onclick: () => choose("manual") }, l("meetings.summary_automation_manual"));
  const setup = h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_summaries"));
  const root = h("section", { class: "meeting-summary-automation", "aria-label": l("meetings.summary_automation_title") },
    text, h("div", { class: "row" }, automatic, manual, setup), message);
  function refresh() {
    const policy = ctx.info.settings.summary_automation ?? "unanswered";
    root.hidden = !!options.compact && policy !== "unanswered";
    root.dataset.summaryAutomation = policy;
    text.textContent = t(policy === "unanswered" ? "meetings.summary_automation_question" : "meetings.summary_automation_help", { destination: summaryDestination(ctx) });
    automatic.setAttribute("aria-pressed", String(policy === "automatic"));
    manual.setAttribute("aria-pressed", String(policy === "manual"));
    automatic.disabled = pending; manual.disabled = pending;
    setup.hidden = languageModelConfigured(ctx.info);
  }
  async function choose(policy: "automatic" | "manual") {
    if (pending) return;
    pending = true; message.hidden = true; refresh();
    try { await ctx.save(settings => { settings.summary_automation = policy; }); }
    catch (error) { message.textContent = errorText(error); message.hidden = false; }
    finally { pending = false; refresh(); }
  }
  bindLocalization(root, refresh);
  ctx.onInfoChange(refresh); refresh();
  return { root, refresh, bind(next: Context) { ctx = next; next.onInfoChange(refresh); refresh(); } };
}
