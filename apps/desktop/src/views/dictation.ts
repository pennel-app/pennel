import { bindLocalization, l, localizeBackendText, t, type UiText } from "../i18n.ts";
import { api, type ModelChoice, type PermissionInfo, type PermissionKind, type Style } from "../api";
import { controlButton, textField, errorText, h, section, select, showPopupMenu, toggle } from "../dom";
import type { Context } from "../main";
import { icon } from "../icons";
import { appBadge } from "../app-icon";
import { settingsTaskRows } from "./setup";
import { applicationLabel } from "../presentation";
import { shortcutRecorder } from "../shortcut-recorder";
import { cleanupVerificationControl, cleanupVerificationDestination, cleanupVerificationReason } from "../cleanup-verification-state.ts";


function styleOptions(): [Style, UiText][] { return [
  ["neutral", l("dictation.style.neutral")],
  ["casual", l("dictation.style.casual")],
  ["formal", l("dictation.style.formal")],
  ["literal", l("dictation.style.literal")],
]; }

function preference(title: UiText, help: UiText | null, control: HTMLElement, name: string): HTMLElement {
  return h("div", { class: "dictation-preference", "data-preference": name },
    h("div", { class: "dictation-preference-copy" }, h("h3", {}, title), help ? h("p", { class: "help" }, help) : null), control);
}

export function dictationView(ctx: Context): HTMLElement {
  let disposed = false;
  const group = (name: string, title: UiText, ...children: HTMLElement[]) => h("section", { class: "dictation-settings-group", "data-dictation-group": name }, h("h2", {}, title), h("div", { class: "dictation-settings-rows" }, ...children));
  const save = (change: Parameters<Context["save"]>[0]) => { void ctx.save(change).catch(ctx.report); };
  const mode = select([["clean", l("dictation.settings.cleaned")], ["fast", l("dictation.settings.spoken")]], ctx.info.settings.mode, value => save(settings => { settings.mode = value; }), "segmented");
  mode.setAttribute("aria-label", t("dictation.settings.result"));
  const presetChoices: [string, UiText][] = ctx.info.platform === "macos" ? [
    ["fn", "fn"], ["right_alt", l("dictation.shortcut.option.right_option")], ["right_meta", l("dictation.shortcut.option.right_command")], ["right_ctrl", l("dictation.shortcut.option.right_control")],
  ] : [["right_ctrl", l("dictation.shortcut.option.right_control")], ["right_alt", l("dictation.shortcut.option.right_alt")], ["ctrl,meta", ctx.info.platform === "windows" ? "Ctrl + Windows" : "Ctrl + Super"]];
  const presets = presetChoices.map(([mods, label]) => ({ label, binding: { mods: mods.split(","), code: null, key: null } }));
  const shortcut = shortcutRecorder(ctx, "dictate", { presets, recordLabel: true });
  const language = select([["", l("common.automatic")], ["nl", l("dictation.language.dutch")], ["en", l("dictation.language.english")], ["fr", l("dictation.language.french")], ["de", l("dictation.language.german")]], ctx.info.settings.language ?? "", value => save(settings => { settings.language = value || null; }), "menu");
  language.setAttribute("aria-label", t("dictation.language.title"));
  const live = toggle(l("settings.live_text.enabled"), ctx.info.settings.live_text_enabled !== false, value => save(settings => { settings.live_text_enabled = value; }));
  const verify = toggle(l("dictation.verification.title"), !!ctx.info.settings.cleanup_verification_enabled, value => save(settings => { settings.cleanup_verification_enabled = value; }));
  const verificationState = h("p", { class: "help", role: "status" });
  const verificationDestination = h("p", { class: "help" });
  const paste = toggle(l("dictation.pad.paste_externally"), ctx.info.settings.dictation_pad_auto_paste, value => save(settings => { settings.dictation_pad_auto_paste = value; }));
  const pasteReason = h("p", { class: "help", role: "status", hidden: true });
  const retention = select([["7", l("dictation.history.days", { days: 7 })], ["30", l("dictation.history.days", { days: 30 })], ["90", l("dictation.history.days", { days: 90 })], ["0", l("dictation.history.always")]], String(ctx.info.settings.history_retention_days), value => save(settings => { settings.history_retention_days = Number(value); }), "menu");
  retention.setAttribute("aria-label", t("dictation.history.retention"));
  const destination = (target: "dictionary" | "styles", title: UiText, help: UiText) => h("button", { class: "dictation-settings-link", "data-open-dictation": target, onclick: () => ctx.navigate(target) }, h("span", { class: "grow" }, h("strong", {}, title), h("span", { class: "help" }, help)), icon("chevronRight"));
  const permissions = h("div", { class: "dictation-permissions" });
  const permissionError = h("p", { class: "error", role: "status", hidden: true });
  const permissionRows = new Map<PermissionKind, { root: HTMLElement; status: HTMLElement; action: HTMLButtonElement; help: HTMLElement }>();
  let permissionValues: PermissionInfo[] = [];
  let permissionRevision = 0;
  for (const [kind, title, description] of [["microphone", l("onboarding.microphone"), l("onboarding.microphone_description")], ["accessibility", l("onboarding.accessibility"), l("onboarding.accessibility_description")], ["speech", l("onboarding.speech_recognition"), l("onboarding.apple_permission_description")]] as const) {
    const status = h("span", { class: "permission-status", role: "status" });
    const help = h("p", { class: "help", hidden: true });
    const action = controlButton({ class: "btn", onclick: async () => {
      action.disabled = true;
      try { const values = await api.requestPermission(kind); if (!disposed) { permissionValues = values; permissionError.hidden = true; refreshPermissions(); } }
      catch (error) { if (!disposed) { permissionError.textContent = errorText(error); permissionError.hidden = false; } }
      finally { if (!disposed) updatePermissions(); }
    } }, l("onboarding.grant_access"));
    const root = h("div", { class: "dictation-permission-row", "data-permission-kind": kind },
      h("div", { class: "dictation-preference-copy grow" }, h("h3", {}, title), h("p", { class: "help" }, description), help), status, action);
    permissions.append(root); permissionRows.set(kind, { root, status, action, help });
  }
  const updatePermissions = () => {
    const account = ctx.info.settings.providers.find(provider => provider.id === ctx.info.settings.dictation_stt.provider_id);
    for (const [kind, row] of permissionRows) {
      row.root.hidden = kind === "speech" && account?.kind !== "apple_speech";
      const state = permissionValues.find(permission => permission.kind === kind)?.status ?? "unknown";
      const granted = state === "granted" || state === "not_required";
      const unsupported = state === "unsupported";
      row.status.replaceChildren(icon(granted ? "check" : "warning"), h("span", {}, l(granted ? "onboarding.permission_ready" : unsupported ? "dictation.settings.permission_unavailable" : "dictation.settings.permission_needed")));
      row.status.classList.toggle("success", granted);
      row.action.hidden = granted || unsupported;
      row.action.disabled = unsupported || ctx.info.platform === "linux" && kind === "microphone";
      row.action.textContent = t(state === "denied" || kind === "accessibility" || ctx.info.platform === "windows" ? "onboarding.open_settings" : "onboarding.grant_access");
      row.help.textContent = unsupported ? t("onboarding.permission_unsupported") : state === "unknown" && ["linux", "windows"].includes(ctx.info.platform) && kind === "microphone" ? t("onboarding.microphone_checked_later") : "";
      row.help.hidden = !row.help.textContent;
    }
  };
  const refreshPermissions = () => {
    const revision = ++permissionRevision;
    void api.permissions().then(values => { if (!disposed && revision === permissionRevision) { permissionValues = values; permissionError.hidden = true; updatePermissions(); } }).catch(error => { if (!disposed && revision === permissionRevision) { permissionError.textContent = errorText(error); permissionError.hidden = false; } });
  };
  let speechModels: Awaited<ReturnType<typeof api.configuredDictationModels>> = [];
  let speechRevision = 0;
  const sameChoice = (choice: ModelChoice) => JSON.stringify([choice.provider_id, choice.model]);
  const speechPickerLabel = h("span", { class: "menu-button-label" }, l("dictation.pill.models"));
  const speechPicker = controlButton({ class: "menu-button speech-model-picker", "data-speech-model-picker": true, "aria-haspopup": "menu", disabled: true, onclick: () => showPopupMenu(speechPicker, [
    ...speechModels.map(model => ({ value: sameChoice(model.choice), label: model.label, run: () => ctx.mutate(() => api.selectDictationModel(model.choice)) })),
    { value: "setup", label: l("dictation.pill.model_setup"), separatorBefore: true, run: () => ctx.openSetup("dictation_stt") },
  ], { current: sameChoice(ctx.info.settings.dictation_stt), report: ctx.report }) }, speechPickerLabel, icon("chevronRight"));
  const modelSwitch = toggle(l("dictation.pill.switch_enabled"), ctx.info.settings.pill_model_switch_enabled !== false, value => save(settings => { settings.pill_model_switch_enabled = value; }));
  const modelSwitchLabel = modelSwitch.querySelector("label")!;
  modelSwitchLabel.replaceChildren(modelSwitch.querySelector("input")!, h("span", { class: "visually-hidden" }, l("dictation.pill.switch_enabled")));
  const modelSwitchReason = h("p", { class: "help", role: "status" });
  const refreshModels = () => {
    const revision = ++speechRevision;
    void api.configuredDictationModels().then(models => {
      if (disposed || revision !== speechRevision) return;
      speechModels = models;
      const selected = models.find(model => sameChoice(model.choice) === sameChoice(ctx.info.settings.dictation_stt));
      speechPickerLabel.textContent = selected?.label ?? t("dictation.pill.models");
      speechPicker.disabled = models.length === 0;
      speechPicker.hidden = models.length === 0;
      if (speechSetupAction) speechSetupAction.hidden = models.length > 0;
      speechPicker.title = models.length === 0 ? t("dictation.pill.model_unavailable") : "";
      const key = ctx.info.shortcuts?.find(row => row.id === "next_model" && row.binding && row.capability.available)?.label.text;
      modelSwitchReason.textContent = models.length < 2 ? t("dictation.pill.switch_needs_models") : key ? t("dictation.pill.switch_key", { shortcut: key }) : t("dictation.pill.switch_none");
    }).catch(error => { if (!disposed && revision === speechRevision) { speechModels = []; speechPicker.disabled = true; modelSwitchReason.textContent = errorText(error); } });
  };
  const taskRows = settingsTaskRows(ctx, ["dictation_stt", "llm"]);
  const speechSetupAction = taskRows.querySelector<HTMLElement>('[data-task-row="dictation_stt"] [data-setup-task]');
  taskRows.querySelector('[data-task-row="dictation_stt"]')?.append(speechPicker);
  taskRows.querySelector('[data-task-row="llm"] h3')?.replaceChildren(h("span", {}, l("dictation.processing.text")));
  const root = h("div", { class: "dictation-settings" },
    group("processing", l("dictation.settings.processing"), taskRows),
    group("during", l("dictation.settings.during"),
      preference(l("dictation.settings.result"), l("dictation.settings.result_help"), mode, "result"),
      preference(l("dictation.shortcut.title"), l("dictation.shortcut.help"), h("div", { class: "dictation-shortcut-controls" }, shortcut, h("button", { class: "text-button", onclick: () => ctx.navigate("shortcuts") }, l("shortcuts.all"))), "shortcut"),
      preference(l("dictation.language.title"), null, language, "language"),
      preference(l("settings.live_text.title"), l("settings.live_text.help"), live, "live-text"),
      preference(l("dictation.verification.title"), l("dictation.verification.help"), h("div", { "data-cleanup-verification": true }, verify, verificationState, verificationDestination, h("button", { class: "text-button", onclick: () => ctx.navigate("models") }, l("navigation.models"))), "verification"),
      preference(l("dictation.pad.title"), l("dictation.pad.preference_help"), h("div", {}, paste, h("p", { class: "help" }, l("dictation.settings.pad_target")), pasteReason), "pad")),
    group("pill", l("dictation.pill.settings_title"), preference(l("dictation.pill.switch_enabled"), l("dictation.pill.switch_help"), h("div", {}, modelSwitch, modelSwitchReason), "model-switch")),
    group("words", l("dictation.settings.words"), destination("dictionary", l("dictation.dictionary.title"), l("dictation.dictionary.help")), destination("styles", l("dictation.style.title"), l("dictation.settings.style_help")), preference(l("dictation.history.retention"), null, retention, "history")),
    group("permissions", l("dictation.access.title"), permissions, permissionError, h("button", { class: "text-button dictation-permission-repair", onclick: () => api.showWindow("onboarding", "dictation-permissions") }, l("dictation.access.check_ellipsis"))));
  const refresh = () => {
    const settings = ctx.info.settings, verification = ctx.info.cleanup_verification;
    mode.setAttribute("aria-label", t("dictation.settings.result"));
    language.setAttribute("aria-label", t("dictation.language.title"));
    retention.setAttribute("aria-label", t("dictation.history.retention"));
    mode.value = settings.mode; language.value = settings.language ?? ""; retention.value = String(settings.history_retention_days);
    live.querySelector<HTMLInputElement>("input")!.checked = settings.live_text_enabled !== false;
    const control = cleanupVerificationControl(settings, verification);
    const input = verify.querySelector<HTMLInputElement>("input")!; input.checked = control.enabled; input.disabled = control.disabled;
    verificationState.textContent = control.enabled && verification?.available && settings.mode === "clean" ? t("dictation.verification.active") : cleanupVerificationReason(verification);
    verificationDestination.textContent = t("dictation.verification.destination", { destination: cleanupVerificationDestination(verification) });
    const external = ctx.info.capabilities.pad_external_paste, pasteInput = paste.querySelector<HTMLInputElement>("input")!;
    pasteInput.checked = settings.dictation_pad_auto_paste; pasteInput.disabled = ["unsupported", "unavailable"].includes(external.status);
    pasteReason.textContent = external.reason ? localizeBackendText(external.reason) : ""; pasteReason.hidden = !pasteReason.textContent;
    modelSwitch.querySelector<HTMLInputElement>("input")!.checked = settings.pill_model_switch_enabled !== false;
    refreshModels();
    updatePermissions(); refreshPermissions();
  };
  ctx.onInfoChange(refresh); ctx.onFocusChange(focused => { if (focused) refresh(); }); ctx.onDispose(() => { disposed = true; permissionRevision++; speechRevision++; });
  bindLocalization(root, refresh); refresh();
  return root;
}

export function dictionaryView(ctx: Context): HTMLElement {
  const input = textField({ type: "text", placeholder: l("dictation.dictionary.example"), class: "grow" });
  const list = h("div", { class: "dictionary-list" });
  const rows = new Map<string, HTMLElement>();
  const refresh = () => {
    const terms = ctx.info.settings.dictionary;
    for (const [term, row] of rows) if (!terms.includes(term)) { row.remove(); rows.delete(term); }
    terms.forEach((term, index) => {
      let row = rows.get(term);
      if (!row) {
        row = h("div", { class: "dictionary-row" }, h("span", { class: "grow" }, term), h("button", { class: "button-icon", "aria-label": l("dictation.dictionary.remove", { term }), onclick: async () => {
          try { await ctx.save(settings => { settings.dictionary = settings.dictionary.filter(value => value !== term); }); input.focus({ preventScroll: true }); } catch { /* Save errors stay visible in the workspace. */ }
        } }, icon("close")));
        rows.set(term, row);
      }
      if (list.children[index] !== row) list.insertBefore(row, list.children[index] ?? null);
    });
  };
  const add = async () => {
    const term = input.value.trim();
    if (!term || ctx.info.settings.dictionary.includes(term)) return;
    try { await ctx.save(settings => { settings.dictionary.push(term); }); if (input.value.trim() === term) input.value = ""; input.focus({ preventScroll: true }); } catch { /* Save errors stay visible in the workspace. */ }
  };
  input.addEventListener("keydown", event => { if (event.key === "Enter") void add(); });
  const root = h("div", { class: "dictionary-settings" }, section(l("dictation.dictionary.title"), l("dictation.dictionary.help"), h("div", { class: "row" }, input, controlButton({ class: "btn", onclick: add }, l("dictation.add"))), list));
  ctx.onInfoChange(refresh); refresh(); return root;
}

export function stylesView(ctx: Context): HTMLElement {
  const input = textField({ type: "text", placeholder: ctx.info.platform === "macos" ? l("dictation.style.app_id") : l("dictation.style.program_name"), class: "grow" });
  const list = h("div", { class: "style-rules" });
  const rows = new Map<string, { row: HTMLElement; choice: ReturnType<typeof select<Style>> }>();
  const pickerStatus = h("p", { class: "help", role: "status" });
  const unavailable = () => ctx.info.platform === "linux" && ctx.info.capabilities.selection.status === "unsupported";
  const add = async () => {
    const value = input.value.trim();
    const id = ctx.info.platform === "macos" || value.includes(".") && !/\.exe$/i.test(value) ? value : value.toLowerCase().replace(/\.exe$/, "");
    if (!id || ctx.info.settings.style_rules.some(rule => rule.bundle_id === id)) return;
    try { await ctx.save(settings => { settings.style_rules.push({ bundle_id: id, style: "neutral" }); }); input.focus({ preventScroll: true }); } catch { /* Save errors stay visible in the workspace. */ }
  };
  input.addEventListener("keydown", event => { if (event.key === "Enter") void add(); });
  const pick = controlButton({ class: "btn", onclick: async () => {
    pick.disabled = true;
    try {
      const application = await api.pickStyleApplication();
      if (application && !ctx.info.settings.style_rules.some(rule => rule.bundle_id === application.id)) await ctx.save(settings => { settings.style_rules.push({ bundle_id: application.id, style: "neutral" }); });
    } catch (error) { pickerStatus.textContent = errorText(error); }
    finally { pick.disabled = unavailable(); if (pick.isConnected && !pick.disabled) pick.focus({ preventScroll: true }); }
  } }, l("dictation.style.add_app"));
  const addManual = controlButton({ class: "btn", onclick: add }, l("dictation.add"));
  const reason = h("p", { class: "help", hidden: true }, l("dictation.style.wayland"));
  const refresh = () => {
    const rules = ctx.info.settings.style_rules;
    for (const [id, item] of rows) if (!rules.some(rule => rule.bundle_id === id)) { item.row.remove(); rows.delete(id); }
    rules.forEach((rule, index) => {
      let item = rows.get(rule.bundle_id);
      if (!item) {
        const app = appBadge(rule.bundle_id); app.name.classList.add("grow");
        const choice = select(styleOptions(), rule.style, style => { void ctx.save(settings => { const current = settings.style_rules.find(value => value.bundle_id === rule.bundle_id); if (current) current.style = style; }).catch(ctx.report); });
        choice.setAttribute("aria-label", applicationLabel(rule.bundle_id));
        const row = h("div", { class: "row style-rule" }, app.icon, app.name, choice, h("button", { class: "button-icon", "aria-label": l("dictation.style.remove", { app: applicationLabel(rule.bundle_id) }), onclick: async () => {
          try { await ctx.save(settings => { settings.style_rules = settings.style_rules.filter(value => value.bundle_id !== rule.bundle_id); }); pick.focus({ preventScroll: true }); } catch { /* Save errors stay visible in the workspace. */ }
        } }, icon("close")));
        item = { row, choice }; rows.set(rule.bundle_id, item);
      }
      item.choice.value = rule.style;
      if (list.children[index] !== item.row) list.insertBefore(item.row, list.children[index] ?? null);
    });
    pick.disabled = unavailable(); addManual.disabled = unavailable(); reason.hidden = !unavailable();
  };
  const root = h("div", { class: "style-settings" }, section(l("dictation.style.title"), l("dictation.style.help"), list,
    h("div", { class: "row style-add" }, pick), pickerStatus,
    h("details", { class: "advanced-style" }, h("summary", {}, l("dictation.style.manual")), h("div", { class: "row" }, input, addManual)), reason));
  ctx.onInfoChange(refresh); refresh(); return root;
}
