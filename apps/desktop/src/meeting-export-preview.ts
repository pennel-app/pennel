import { api, type MeetingExport } from "./api";
import type { Context } from "./main";
import { bindLocalization, l, t, refreshLocalization } from "./i18n";
import { controlButton, errorText, h, markdown, menuButton, namedDialog } from "./dom";
import { languageModelConfigured } from "./presentation";
import { summaryDestination } from "./meeting-summary-automation";
import "./meeting-export-preview.css";

export function meetingExportPreview(initial: Context, host: { id(): string; template(): string; flush(): Promise<void> }) {
  let ctx = initial, scope: "shareable" | "personal" = "shareable", prepared: MeetingExport | null = null, busy = false, epoch = 0;
  let view: "formatted" | "markdown" = "formatted";
  let modal: ReturnType<typeof namedDialog> | null = null, binding = 0;
  const explanation = h("p", { class: "help" });
  const status = h("p", { class: "help", role: "status" });
  const body = h("textarea", { class: "meeting-export-copy", readonly: true, "aria-label": l("meetings.export_preview_title"), spellcheck: "false" });
  const rendered = h("div", { class: "meeting-export-rendered", tabindex: 0, "aria-label": l("meetings.export_preview_title") });
  const formattedView = controlButton({ class: "btn compact", "data-export-view": "formatted", onclick: () => { view = "formatted"; update(); } }, l("meetings.export_view_formatted"));
  const markdownView = controlButton({ class: "btn compact", "data-export-view": "markdown", onclick: () => { view = "markdown"; update(); } }, l("meetings.export_view_markdown"));
  const formats = h("div", { class: "meeting-export-formats row", role: "group", "aria-label": l("meetings.export_preview_format") }, formattedView, markdownView);
  const transcript = h("input", { type: "checkbox", checked: false });
  const shareable = controlButton({ class: "btn compact", "data-export-scope": "shareable", onclick: () => { scope = "shareable"; void prepare(); } }, l("meetings.export_shareable"));
  const personal = controlButton({ class: "btn compact", "data-export-scope": "personal", onclick: () => { scope = "personal"; void prepare(); } }, l("meetings.export_personal"));
  const generate = controlButton({ class: "btn meeting-export-generate", onclick: () => { void generateShareable(); } }, l("meetings.export_generate_shareable"));
  const setup = h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_summaries"));
  const retry = controlButton({ class: "btn compact", onclick: () => { void prepare(); } }, l("common.retry"));
  const copyPlain = controlButton({ class: "btn meeting-export-copy-markdown", onclick: () => { void use(value => api.copyText(value.markdown), "meetings.export_copy_done"); } }, l("meetings.export_copy_markdown"));
  const copyRich = controlButton({ class: "btn primary meeting-export-copy-formatted", onclick: () => {
    void use(value => api.copyText(value.markdown, ctx.info.capabilities.rich_clipboard.status === "available" ? value.html : undefined), "meetings.export_copy_done");
  } }, l("common.copy"));
  const save = controlButton({ class: "btn meeting-export-save", onclick: () => { void use(value => api.exportPreparedMeeting(value), "meetings.export_saved"); } }, l("meetings.export_save_markdown"));
  const richCopyAction = { label: l("meetings.export_copy_formatted"), disabled: false, title: ctx.info.capabilities.rich_clipboard.reason,
    run: () => { void use(value => api.copyText(value.markdown, value.html), "meetings.export_copy_done"); } };
  const systemShare = menuButton(l("meetings.workspace_more"), [
    richCopyAction,
    { label: l("meetings.email"), run: () => { void use(value => api.sharePreparedMeeting(value, "mail")); } },
    { label: l("meetings.share_action"), disabled: ctx.info.platform !== "macos", title: ctx.info.platform === "macos" ? null : l("meetings.share_unavailable"),
      run: () => { void use(value => api.sharePreparedMeeting(value, "share")); } },
  ]);
  const preparation = h("div", { class: "row meeting-export-preparation" }, generate, setup, retry);
  const root = h("section", { class: "meeting-export-preview", hidden: true },
    h("div", { class: "meeting-export-review" },
      h("div", { class: "meeting-export-heading segmented", role: "group", "aria-label": l("meetings.export_preview_title") }, shareable, personal), explanation,
      h("div", { class: "meeting-export-options row" }, h("label", { class: "checkbox" }, transcript, l("meetings.export_include_transcript")), h("span", { class: "grow" }), formats, copyPlain),
      status, preparation, h("div", { class: "meeting-export-document" }, rendered, body)),
    h("div", { class: "row meeting-export-actions" }, controlButton({ class: "btn meeting-export-cancel", onclick: () => close() }, l("common.cancel")), save, copyRich, systemShare));
  transcript.addEventListener("change", () => { void prepare(); });
  function showStatus(text: string, quiet = false) {
    status.textContent = text; status.classList.toggle("visually-hidden", quiet);
  }
  function update() {
    root.dataset.exportScope = scope;
    root.dataset.exportView = view;
    shareable.setAttribute("aria-pressed", String(scope === "shareable")); personal.setAttribute("aria-pressed", String(scope === "personal"));
    shareable.disabled = busy; personal.disabled = busy; transcript.disabled = busy;
    explanation.textContent = t(scope === "shareable" ? "meetings.export_shareable_help" : "meetings.export_personal_help");
    explanation.classList.toggle("danger", scope === "personal");
    body.hidden = !prepared || view !== "markdown";
    rendered.hidden = !prepared || view !== "formatted";
    formats.hidden = !prepared;
    formattedView.setAttribute("aria-pressed", String(view === "formatted"));
    markdownView.setAttribute("aria-pressed", String(view === "markdown"));
    for (const control of [copyPlain, copyRich, save, systemShare]) control.disabled = busy || !prepared;
    copyRich.title = ctx.info.capabilities.rich_clipboard.reason ?? "";
    richCopyAction.disabled = ctx.info.capabilities.rich_clipboard.status !== "available";
    richCopyAction.title = ctx.info.capabilities.rich_clipboard.reason;
    generate.hidden = scope !== "shareable" || !!prepared;
    generate.disabled = busy || !languageModelConfigured(ctx.info);
    generate.title = summaryDestination(ctx);
    setup.hidden = scope !== "shareable" || languageModelConfigured(ctx.info) || !!prepared;
    retry.hidden = busy || !!prepared;
    preparation.hidden = generate.hidden && setup.hidden && retry.hidden;
  }
  async function prepare() {
    const request = ++epoch, requestScope = scope, includeTranscript = transcript.checked;
    busy = true; prepared = null; body.value = ""; rendered.replaceChildren();
    showStatus(t("meetings.export_preparing")); update();
    try {
      await host.flush();
      const output = await api.previewMeetingExport(host.id(), requestScope, includeTranscript);
      if (request !== epoch) return;
      prepared = Object.freeze({ ...output }); body.value = prepared.markdown;
      rendered.replaceChildren(markdown(prepared.markdown)); showStatus(t("meetings.export_ready"), true);
    } catch (failure) {
      if (request !== epoch) return;
      showStatus(requestScope === "shareable" ? `${t("meetings.export_missing_shareable")} ${errorText(failure)}` : errorText(failure));
    } finally { if (request === epoch) { busy = false; update(); } }
  }
  async function generateShareable() {
    if (busy || generate.disabled) return;
    const request = epoch;
    busy = true; showStatus(t("meetings.export_preparing")); update();
    try {
      await host.flush();
      if (request !== epoch) return;
      await api.generateShareableMeetingSummary(host.id(), host.template());
      if (request !== epoch) return;
      busy = false; await prepare();
    }
    catch (failure) { if (request === epoch) { busy = false; showStatus(errorText(failure)); update(); } }
  }
  async function use(action: (output: MeetingExport) => Promise<unknown>, acknowledgement?: "meetings.export_copy_done" | "meetings.export_saved") {
    if (!prepared || busy) return;
    const exact = prepared, request = epoch; busy = true; update();
    try { const result = await action(exact); if (request === epoch && acknowledgement && result !== false) showStatus(t(acknowledgement)); }
    catch (failure) { if (request === epoch) showStatus(errorText(failure)); }
    finally { if (request === epoch) { busy = false; update(); } }
  }
  function close() {
    if (!modal) return;
    const previous = modal; modal = null; ++epoch;
    root.hidden = true; root.remove(); prepared = null; busy = false; body.value = ""; rendered.replaceChildren();
    previous.close();
  }
  function bind(next: Context) {
    ctx = next; const current = ++binding;
    next.onInfoChange(update); next.onDispose(() => { if (current === binding) close(); }); update();
  }
  bindLocalization(root, update); update();
  bind(initial);
  return { root, bind, close, open(nextScope: "shareable" | "personal" = "shareable") {
    scope = nextScope; view = "formatted"; root.hidden = false; void prepare();
    if (!modal) {
      modal = namedDialog(l("meetings.export_preview_title"));
      modal.dialog.classList.add("meeting-export-dialog");
      modal.dialog.append(modal.heading, root);
      modal.dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
      document.body.append(modal.dialog); refreshLocalization(modal.dialog); modal.dialog.showModal();
      shareable.focus({ preventScroll: true });
    }
  } };
}
