import { api, type MeetingExport } from "./api";
import type { Context } from "./main";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h, menuButton } from "./dom";
import { languageModelConfigured } from "./presentation";
import { summaryDestination } from "./meeting-summary-automation";

export function meetingExportPreview(initial: Context, host: { id(): string; template(): string; flush(): Promise<void> }) {
  let ctx = initial, scope: "shareable" | "personal" = "shareable", prepared: MeetingExport | null = null, busy = false, epoch = 0;
  const explanation = h("p", { class: "help" });
  const status = h("p", { class: "help", role: "status" });
  const body = h("textarea", { class: "meeting-export-copy", readonly: true, "aria-label": l("meetings.export_preview_title"), spellcheck: "false" });
  const transcript = h("input", { type: "checkbox", checked: false });
  const shareable = controlButton({ class: "btn compact", "data-export-scope": "shareable", onclick: () => { scope = "shareable"; void prepare(); } }, l("meetings.export_shareable"));
  const personal = controlButton({ class: "btn compact", "data-export-scope": "personal", onclick: () => { scope = "personal"; void prepare(); } }, l("meetings.export_personal"));
  const generate = controlButton({ class: "btn meeting-export-generate", onclick: () => { void generateShareable(); } }, l("meetings.export_generate_shareable"));
  const setup = h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_summaries"));
  const retry = controlButton({ class: "btn compact", onclick: () => { void prepare(); } }, l("common.retry"));
  const copyPlain = controlButton({ class: "btn meeting-export-copy-markdown", onclick: () => { void use(value => api.copyText(value.markdown), "meetings.export_copy_done"); } }, l("meetings.export_copy_markdown"));
  const copyRich = controlButton({ class: "btn meeting-export-copy-formatted", onclick: () => { void use(value => api.copyText(value.markdown, value.html), "meetings.export_copy_done"); } }, l("meetings.export_copy_formatted"));
  const save = controlButton({ class: "btn meeting-export-save", onclick: () => { void use(value => api.exportPreparedMeeting(value), "meetings.export_saved"); } }, l("meetings.export_save_markdown"));
  const systemShare = menuButton(l("meetings.workspace_more"), [
    { label: l("meetings.email"), run: () => { void use(value => api.sharePreparedMeeting(value, "mail")); } },
    { label: l("meetings.share_action"), disabled: ctx.info.platform !== "macos", title: ctx.info.platform === "macos" ? null : l("meetings.share_unavailable"),
      run: () => { void use(value => api.sharePreparedMeeting(value, "share")); } },
  ]);
  const root = h("section", { class: "meeting-export-preview", hidden: true }, h("h2", {}, l("meetings.export_preview_title")),
    h("div", { class: "row" }, shareable, personal), explanation,
    h("label", { class: "checkbox" }, transcript, l("meetings.export_include_transcript")), status,
    h("div", { class: "row" }, generate, setup, retry), body, h("div", { class: "row" }, copyPlain, copyRich, save, systemShare));
  transcript.addEventListener("change", () => { void prepare(); });
  function update() {
    root.dataset.exportScope = scope;
    shareable.setAttribute("aria-pressed", String(scope === "shareable")); personal.setAttribute("aria-pressed", String(scope === "personal"));
    shareable.disabled = busy; personal.disabled = busy; transcript.disabled = busy;
    explanation.textContent = t(scope === "shareable" ? "meetings.export_shareable_help" : "meetings.export_personal_help");
    body.hidden = !prepared;
    for (const control of [copyPlain, copyRich, save, systemShare]) control.disabled = busy || !prepared;
    copyRich.disabled ||= ctx.info.capabilities.rich_clipboard.status !== "available";
    copyRich.title = ctx.info.capabilities.rich_clipboard.reason ?? "";
    generate.hidden = scope !== "shareable" || !!prepared;
    generate.disabled = busy || !languageModelConfigured(ctx.info);
    generate.title = summaryDestination(ctx);
    setup.hidden = scope !== "shareable" || languageModelConfigured(ctx.info) || !!prepared;
    retry.hidden = busy || !!prepared;
  }
  async function prepare() {
    const request = ++epoch; busy = true; prepared = null; body.value = "";
    status.textContent = t("meetings.export_preparing"); update();
    try {
      await host.flush();
      const output = await api.previewMeetingExport(host.id(), scope, transcript.checked);
      if (request !== epoch) return;
      prepared = Object.freeze({ ...output }); body.value = output.markdown; status.textContent = t("meetings.export_ready");
    } catch (failure) {
      if (request !== epoch) return;
      status.textContent = scope === "shareable" ? `${t("meetings.export_missing_shareable")} ${errorText(failure)}` : errorText(failure);
    } finally { if (request === epoch) { busy = false; update(); } }
  }
  async function generateShareable() {
    if (busy || generate.disabled) return;
    busy = true; status.textContent = t("meetings.export_preparing"); update();
    try { await host.flush(); await api.generateShareableMeetingSummary(host.id(), host.template()); busy = false; await prepare(); }
    catch (failure) { busy = false; status.textContent = errorText(failure); update(); }
  }
  async function use(action: (output: MeetingExport) => Promise<unknown>, acknowledgement?: "meetings.export_copy_done" | "meetings.export_saved") {
    if (!prepared || busy) return;
    const exact = prepared; busy = true; update();
    try { const result = await action(exact); if (acknowledgement && result !== false) status.textContent = t(acknowledgement); }
    catch (failure) { status.textContent = errorText(failure); }
    finally { busy = false; update(); }
  }
  bindLocalization(root, update); update();
  return { root, bind(next: Context) { ctx = next; next.onInfoChange(update); update(); }, open(nextScope: "shareable" | "personal" = "shareable") {
    scope = nextScope; root.hidden = false; void prepare();
  } };
}
