import { liveText, bindLocalization, l, t, formatNumber } from "../i18n.ts";
import { templateDisplayName, recordActionOptions } from "../settings-presentation.ts";
import { api, type Capability, type KnownVoice, type MeetingTemplate, type SpeakerRecognitionMode, type SpeakerSetup } from "../api";
import { controlButton, textField, askText, confirmDialog, errorText, h, section, select, toggle } from "../dom";
import { trainedLabel } from "../speaker-state";
import { SpeakerModels, enrollmentBlocker, enrollmentDialog, speakerDownload } from "./voices";
import type { Context } from "../main";
import { summaryAutomationChoice } from "../meeting-summary-automation";
import { icon } from "../icons";
import { appBadge } from "../app-icon";
import { writingProfileSection } from "./writing-profile";

const apps: [string, string][] = [
  ["com.microsoft.teams2", "Microsoft Teams"], ["us.zoom.xos", "Zoom"],
  ["com.tinyspeck.slackmacgap", "Slack"], ["com.apple.FaceTime", "FaceTime"],
  ["net.whatsapp.WhatsApp", "WhatsApp"], ["Cisco-Systems.Spark", "Webex"],
  ["com.hnc.Discord", "Discord"], ["com.google.Chrome", "Google Chrome"],
  ["com.apple.Safari", "Safari"], ["company.thebrowser.Browser", "Arc"], ["com.microsoft.edgemac", "Microsoft Edge"],
  ["org.mozilla.firefox", "Firefox"], ["com.brave.Browser", "Brave"],
];

function locked(capability: Capability): boolean {
  return capability.status === "unsupported" || capability.status === "unavailable";
}

function templateEditor(ctx: Context, target: { id: string | null; name: string; body: string }) {
  const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const titleId = `template-editor-${crypto.randomUUID()}`;
  const dialog = h("dialog", { class: "editor-dialog template-editor", "aria-labelledby": titleId });
  const name = textField({ type: "text", value: target.name, placeholder: l("common.name"), "aria-label": l("common.name") });
  const body = h("textarea", { rows: 12, placeholder: l("settings.template.body_label"), "aria-label": l("settings.template.body_label") });
  body.value = target.body;
  const status = h("p", { class: "error", role: "status" });
  let invalidName = false;
  let saveError: unknown = null;
  const updateStatus = () => { status.textContent = invalidName ? t("settings.template.name_required") : saveError ? errorText(saveError) : ""; };
  bindLocalization(status, updateStatus);
  const sections = h("p", { class: "help" });
  const update = () => {
    const found = body.value.split("\n").map((line) => line.trim().match(/^##\s+(.+)/)?.[1]).filter(Boolean);
    sections.textContent = t("settings.template.sections", { sections: (found.length ? found : [t("settings.template.summary"), t("settings.template.decisions"), t("settings.template.actions")]).join(" · ") });
  };
  body.addEventListener("input", update);
  update();
  bindLocalization(sections, update);
  const close = () => { dialog.close(); dialog.remove(); if (previousFocus?.isConnected) previousFocus.focus(); };
  const save = controlButton({ class: "btn primary", type: "submit" }, l("common.save"));
  const form = h("form", {}, h("h2", { id: titleId }, target.id ? l("settings.template.edit_title") : l("settings.template.new")), name, body,
    h("p", { class: "help" }, l("settings.template.editor_help")),
    sections, status, h("div", { class: "row dialog-actions" }, h("span", { class: "grow" }),
      controlButton({ class: "btn", type: "button", onclick: close }, l("common.cancel")), save));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    invalidName = !name.value.trim(); saveError = null; updateStatus();
    if (invalidName) return;
    save.disabled = true;
    try { await api.saveTemplate(target.id, name.value.trim(), body.value); close(); ctx.rerender(); }
    catch (error) { saveError = error; updateStatus(); save.disabled = false; }
  });
  dialog.append(form);
  dialog.addEventListener("cancel", (event) => { event.preventDefault(); close(); });
  document.body.append(dialog);
  dialog.showModal();
  name.focus();
  ctx.onDispose(close);
}

async function templateSection(ctx: Context, templates: MeetingTemplate[]) {
  const edit = async (template: MeetingTemplate) => templateEditor(ctx, {
    id: template.id, name: template.name, body: await api.templateSource(template.id) ?? "",
  });
  const remove = async (template: MeetingTemplate) => {
    if (!await confirmDialog(template.builtin ? l("settings.template.restore_title") : l("settings.template.delete_title", { name: String(templateDisplayName(template)) }),
      template.builtin ? l("settings.template.restore_help") : l("settings.template.delete_help"),
      template.builtin ? l("settings.template.restore") : l("settings.template.delete"))) return;
    await api.deleteTemplate(template.id);
    const remaining = await api.meetingTemplates();
    if (!remaining.some((item) => item.id === ctx.info.settings.default_template)) {
      await ctx.save((settings) => { settings.default_template = remaining[0]?.id ?? "algemeen"; });
    } else ctx.rerender();
  };
  return section(l("settings.template.title"), l("settings.template.help"),
    templates.map((template) => h("div", { class: "row template-row" },
      h("span", { class: "grow" }, templateDisplayName(template)),
      !template.builtin || template.customized ? h("span", { class: "badge" }, template.builtin ? l("settings.template.modified") : l("settings.template.custom")) : null,
      controlButton({ class: "btn", onclick: () => edit(template) }, l("settings.template.edit")),
      !template.builtin || template.customized ? controlButton({ class: "btn", onclick: () => remove(template) }, template.builtin ? l("settings.template.restore") : l("settings.template.delete")) : null)),
    controlButton({ class: "btn", onclick: () => templateEditor(ctx, { id: null, name: "", body: "" }) }, l("settings.template.new")));
}

async function speakerSections(ctx: Context, setup: SpeakerSetup): Promise<HTMLElement[]> {
  const settings = ctx.info.settings;
  const mode: SpeakerRecognitionMode = settings.speaker_recognition === "cloud" ? "cloud" : "local";
  const modelStatus = h("p", { class: "help", role: "status" });
  let download: ReturnType<typeof speakerDownload> | null = null;
  const updateModels = () => {
    modelStatus.textContent = models.ready ? t("settings.speakers.models_ready")
      : models.downloading ? t("settings.speakers.models_downloading")
      : setup.legacy_model ? t("settings.speakers.legacy_help")
      : t("settings.speakers.models_missing");
    download?.update();
    learnReason.textContent = enrollmentBlocker(setup, mode, models) ?? "";
    learnReason.hidden = !learnReason.textContent;
    for (const button of [learn, again]) button.disabled = !!enrollmentBlocker(setup, mode, models);
  };
  const models = new SpeakerModels(setup.model_ids, updateModels);
  download = speakerDownload(models);
  ctx.onDispose(() => models.dispose());

  const choice = select<SpeakerRecognitionMode>([["local", l("settings.speakers.route_local")], ["cloud", l("settings.speakers.route_cloud")]], mode,
    (value) => ctx.save((next) => { next.speaker_recognition = value; }), "segmented");
  choice.setAttribute("aria-label", t("settings.speakers.route_label"));
  bindLocalization(choice, () => choice.setAttribute("aria-label", t("settings.speakers.route_label")));
  choice.options[0].disabled = !setup.local_available;
  const route = [
    setup.local_available ? null : h("p", { class: "help" }, l("settings.speakers.unavailable")),
    ...mode === "cloud"
      ? [h("p", { class: "help" }, l("settings.speakers.cloud_help")),
        setup.signed_in ? null : h("button", { class: "text-button speaker-sign-in", onclick: () => ctx.navigate("account") }, l("settings.speakers.sign_in"))]
      : setup.local_available ? [modelStatus, download.element, h("p", { class: "help" }, l("settings.speakers.local_help"))] : [],
  ];

  const learnVoice = async () => {
    if (await enrollmentDialog(setup.enrollment_text)) ctx.rerender();
  };
  const learn = controlButton({ class: "btn", onclick: learnVoice }, l("settings.voice.learn"));
  const again = controlButton({ class: "btn", onclick: learnVoice }, l("settings.voice.again"));
  const forget = controlButton({ class: "btn danger", onclick: async () => {
    if (!await confirmDialog(l("settings.voice.forget_title"), l("settings.voice.forget_help"), l("settings.voice.forget"))) return;
    try { await api.forgetVoiceProfile(); ctx.rerender(); } catch (error) { ctx.report(error); }
  } }, l("settings.voice.forget_mine"));
  const learnReason = h("p", { class: "help", hidden: true });

  const knownRow = (voice: KnownVoice) => h("div", { class: "row template-row" },
    h("span", { class: "grow" }, voice.name),
    h("span", { class: "help" }, voice.meetings === 1 ? l("settings.voice.meeting") : l("settings.voice.meetings", () => ({ count: formatNumber(voice.meetings) }))),
    controlButton({ class: "btn", onclick: async () => {
      const name = await askText(l("settings.voice.name_prompt"), voice.name);
      if (!name || name === voice.name) return;
      try { await api.renameKnownVoice(voice.id, name); ctx.rerender(); } catch (error) { ctx.report(error); }
    } }, l("settings.voice.rename")),
    controlButton({ class: "btn", onclick: async () => {
      if (!await confirmDialog(l("settings.voice.forget_known_title", { name: voice.name }), l("settings.voice.forget_known_help"), l("settings.voice.forget"))) return;
      try { await api.forgetKnownVoice(voice.id); ctx.rerender(); } catch (error) { ctx.report(error); }
    } }, l("settings.voice.forget")));
  const forgetAll = controlButton({ class: "btn danger", disabled: !setup.known.length && !setup.profile, onclick: async () => {
    if (!await confirmDialog(l("settings.voice.forget_all_title"), l("settings.voice.forget_all_help"), l("settings.voice.forget_all_action"))) return;
    try { await api.forgetAllVoices(); ctx.rerender(); } catch (error) { ctx.report(error); }
  } }, l("settings.voice.forget_all"));

  const trained = h("span", { class: "grow" }, liveText(() => trainedLabel(setup.profile)));
  bindLocalization(modelStatus, updateModels);
  const sections = [
    section(l("settings.speakers.title"), l("settings.speakers.identity_help"),
      toggle(l("settings.speakers.automatic"), settings.diarization,
        (value) => ctx.save((next) => { next.diarization = value; }), ctx.info.capabilities.diarization),
      toggle(l("settings.speakers.meeting_app"), settings.meeting_app_speakers,
        (value) => ctx.save((next) => { next.meeting_app_speakers = value; }), ctx.info.capabilities.meeting_app_speakers),
      h("p", { class: "help" }, l("settings.speakers.meeting_app_help")),
      h("div", { class: "row speaker-route" }, choice), ...route),
    section(l("settings.voice.title"), l("settings.voice.help"),
      h("div", { class: "row" }, trained,
        ...(setup.profile ? [again, forget] : [learn])), learnReason),
    section(l("settings.voice.known_title"), l("settings.voice.known_help"),
      setup.known.length ? setup.known.map(knownRow)
        : h("p", { class: "help" }, l("settings.voice.known_empty")),
      h("div", { class: "row speaker-forget-all" }, forgetAll)),
  ];
  await models.start(ctx.info.active_model_downloads ?? []);
  return sections;
}

export async function meetingSettingsView(ctx: Context): Promise<HTMLElement> {
  const settings = ctx.info.settings;
  const [templates, recording, setup] = await Promise.all([api.meetingTemplates(), api.meetingStatus(), api.speakerSetup()]);
  const pathLocked = ctx.info.preview || recording.busy;
  const path = h("span", { class: "grow meeting-path", "aria-label": l("settings.folder.label") }, ctx.info.preview ? l("settings.folder.preview") : settings.meetings_dir || "~/Pennel");
  return h("div", { class: "settings-pane" },
    summaryAutomationChoice({ ...ctx, get info() { return ctx.info; }, onInfoChange: () => undefined }).root,
    section(l("settings.folder.title"), l("settings.folder.help"),
      h("div", { class: "row" }, path,
        controlButton({ class: "btn", disabled: pathLocked, onclick: async () => {
          const selected = await api.chooseMeetingsDir();
          if (selected !== null) await ctx.save((next) => { next.meetings_dir = selected; });
        } }, l("settings.folder.choose")),
        settings.meetings_dir ? controlButton({ class: "btn", disabled: pathLocked, onclick: () => ctx.save((next) => { next.meetings_dir = ""; }) }, l("settings.folder.default")) : null),
      ctx.info.preview ? h("p", { class: "help" }, l("settings.folder.preview_help")) : null,
      !ctx.info.preview && recording.busy ? h("p", { class: "help" }, l("settings.folder.busy_help")) : null,
      /(?:Mobile Documents|\/Documents(?:\/|$)|\/Desktop(?:\/|$))/.test(settings.meetings_dir)
        ? h("p", { class: "help" }, l("settings.folder.sync_help")) : null),
    section(l("settings.template.default"), null, select(templates.map((template) => [template.id, templateDisplayName(template)]), settings.default_template,
      (id) => ctx.save((next) => { next.default_template = id; }), "menu")),
    await templateSection(ctx, templates),
    ...await speakerSections(ctx, setup),
    writingProfileSection(ctx),
    section(l("settings.audio.title"), l("settings.audio.help"),
      select([["30", l("settings.audio.days30")], ["90", l("settings.audio.days90")], ["365", l("settings.audio.year")], ["0", l("settings.action.always")]], String(settings.meeting_audio_retention_days),
        (value) => ctx.save((next) => { next.meeting_audio_retention_days = Number(value); }))));
}

export async function recordingSettingsView(ctx: Context): Promise<HTMLElement> {
  const { settings, capabilities, platform } = ctx.info;
  const snapshot = await api.calendar();
  const names = new Map(apps);
  const appIds = new Map((ctx.info.recording_app_ids ?? apps.map(([id]) => id)).map((id) => [id, names.get(id) ?? id]));
  settings.app_rules.forEach((rule) => { if (!appIds.has(rule.bundle_id)) appIds.set(rule.bundle_id, rule.bundle_id); });
  const addTitle = textField({ type: "text", placeholder: l("settings.recording.title_placeholder"), class: "grow" });
  const add = async () => {
    const title = addTitle.value.trim();
    if (!title || settings.event_rules.some((rule) => rule.title.toLocaleLowerCase() === title.toLocaleLowerCase())) return;
    await ctx.save((next) => { next.event_rules.push({ title, action: "always" }); });
  };
  addTitle.addEventListener("keydown", (event) => { if (event.key === "Enter") void add(); });
  return h("div", { class: "settings-pane" },
    section(l("settings.recording.detect_title"), l("settings.recording.detect_help"),
      toggle(l("settings.recording.auto_detect"), settings.auto_detect, (value) => ctx.save((next) => { next.auto_detect = value; }), capabilities.call_detection),
      toggle(l("settings.recording.shortcut", () => ({ shortcut: platform === "macos" ? "⌘⇧R" : "Ctrl+Shift+R" })), settings.record_shortcut_enabled,
        (value) => ctx.save((next) => { next.record_shortcut_enabled = value; }), capabilities.record_shortcut),
      toggle(l("settings.recording.echo"), settings.echo_cancellation,
        (value) => ctx.save((next) => { next.echo_cancellation = value; }), capabilities.echo_cancellation),
      toggle(l("settings.recording.consent"), settings.consent_enabled,
        (value) => ctx.save((next) => { next.consent_enabled = value; }))),
    section(l("settings.recording.per_app"), null,
      [...appIds].map(([id, name]) => h("div", { class: "row rule-row" }, appBadge(id).icon, h("span", { class: "grow" }, name),
        select(recordActionOptions(), settings.app_rules.find((rule) => rule.bundle_id === id)?.action ?? "ask", (action) => ctx.save((next) => {
          next.app_rules = next.app_rules.filter((rule) => rule.bundle_id !== id);
          if (action !== "ask") next.app_rules.push({ bundle_id: id, action });
        }))))),
    section(l("settings.calendar.title"), platform === "macos"
      ? l("settings.calendar.mac_help")
      : l("settings.calendar.help"),
      toggle(l("settings.calendar.use"), settings.calendar_enabled, async (value) => {
        if (value && capabilities.calendar.status === "permission_required") await api.requestPermission("calendar");
        await ctx.save((next) => { next.calendar_enabled = value; });
      }, capabilities.calendar),
      settings.calendar_enabled && !locked(capabilities.calendar) ? snapshot.calendars.map((calendar) => h("div", { class: "row rule-row" },
        h("span", { class: "grow" }, calendar.name),
        select(recordActionOptions(), settings.calendar_rules.find((rule) => rule.calendar_id === calendar.id)?.action ?? "ask", (action) => ctx.save((next) => {
          next.calendar_rules = next.calendar_rules.filter((rule) => rule.calendar_id !== calendar.id);
          if (action !== "ask") next.calendar_rules.push({ calendar_id: calendar.id, calendar_name: calendar.name, action });
        })))) : null),
    section(l("settings.recording.per_meeting"), l("settings.recording.rule_help"),
      h("div", { class: "row" }, addTitle, controlButton({ class: "btn", onclick: add }, l("settings.recording.add_rule"))),
      settings.event_rules.map((rule) => h("div", { class: "row rule-row" }, h("span", { class: "grow" }, rule.title),
        select(recordActionOptions(), rule.action, (action) => ctx.save((next) => { const found = next.event_rules.find((item) => item.title === rule.title); if (found) found.action = action; })),
        h("button", { class: "button-icon", "aria-label": l("settings.recording.remove_rule", () => ({ title: rule.title })), onclick: () => ctx.save((next) => { next.event_rules = next.event_rules.filter((item) => item.title !== rule.title); }) }, icon("close"))))));
}
