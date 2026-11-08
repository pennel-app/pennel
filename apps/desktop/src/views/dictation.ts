import { bindLocalization, l, liveText, localizeBackendText, t, type UiText } from "../i18n.ts";
import { api, type HotkeyTrigger, type Style } from "../api";
import { controlButton, textField, errorText, h, section, select, toggle } from "../dom";
import type { Context } from "../main";
import { icon } from "../icons";
import { appBadge } from "../app-icon";
import { applicationLabel } from "../presentation";
import { cleanupVerificationControl, cleanupVerificationDestination, cleanupVerificationReason } from "../cleanup-verification-state.ts";

function hotkeys(platform: string): [HotkeyTrigger, UiText][] {
  return platform === "macos"
    ? [["fn", "fn"], ["right_option", l("dictation.shortcut.option.right_option")], ["right_command", l("dictation.shortcut.option.right_command")], ["right_control", l("dictation.shortcut.option.right_control")]]
    : [["right_control", l("dictation.shortcut.option.right_control")], ["right_option", l("dictation.shortcut.option.right_alt")], ["control_super", platform === "windows" ? "Ctrl + Windows" : "Ctrl + Super"]];
}

function styleOptions(): [Style, UiText][] { return [
  ["neutral", l("dictation.style.neutral")],
  ["casual", l("dictation.style.casual")],
  ["formal", l("dictation.style.formal")],
  ["literal", l("dictation.style.literal")],
]; }

function backendMessage(source: string, className: string): HTMLElement {
  const node = h("p", { class: className }, localizeBackendText(source));
  bindLocalization(node, () => { node.textContent = localizeBackendText(source); });
  return node;
}

function radio(name: string, value: string, current: string, label: UiText, onPick: () => void) {
  return h(
    "label",
    { class: "radio" },
    h("input", { type: "radio", name, value, checked: value === current, onchange: onPick }),
    label,
  );
}

export function dictationView(ctx: Context): HTMLElement {
  const s = ctx.info.settings;
  const linux = ctx.info.platform === "linux";
  const verification = ctx.info.cleanup_verification;
  const control = cleanupVerificationControl(s, verification);
  const verificationSettings = section(l("dictation.verification.title"), l("dictation.verification.help"),
    toggle(l("dictation.verification.title"), control.enabled, (enabled) => {
      void ctx.save((settings) => { settings.cleanup_verification_enabled = enabled; }).catch((error) => {
        ctx.report(error);
        ctx.rerender();
      });
    }, { status: control.disabled ? "unavailable" : "available", reason: null }),
    h("p", { class: "help" }, l(control.enabled ? "dictation.verification.saved_enabled" : "dictation.verification.saved_disabled")),
    h("p", { class: "help", role: "status" }, liveText(() => control.enabled && verification?.available && s.mode === "clean"
      ? t("dictation.verification.active") : cleanupVerificationReason(verification))),
    h("p", { class: "help" }, liveText(() => t("dictation.verification.destination", { destination: cleanupVerificationDestination(verification) }))),
    h("button", { class: "text-button", onclick: () => ctx.navigate("models") }, l("navigation.models")));
  verificationSettings.dataset.cleanupVerification = "true";
  return h(
    "div",
    { class: "dictation-settings" },
    section(
      l("dictation.mode.title"),
      null,
      radio("mode", "clean", s.mode, l("dictation.mode.clean"), () =>
        ctx.save((n) => (n.mode = "clean")),
      ),
      radio("mode", "fast", s.mode, l("dictation.mode.fast"), () => ctx.save((n) => (n.mode = "fast"))),
    ),
    section(l("settings.live_text.title"), l("settings.live_text.help"),
      toggle(l("settings.live_text.enabled"), s.live_text_enabled !== false,
        (value) => ctx.save((settings) => { settings.live_text_enabled = value; }))),
    verificationSettings,
    section(
      l("dictation.shortcut.title"),
      l("dictation.shortcut.help"),
      h(
        "div",
        { class: "segmented" },
        hotkeys(ctx.info.platform).map(([value, label]) =>
          h(
            "button",
            { "aria-pressed": String(value === s.hotkey), onclick: () => ctx.save((n) => (n.hotkey = value)) },
            label,
          ),
        ),
      ),
      linux && ctx.info.capabilities.global_hotkey.reason
        ? backendMessage(ctx.info.capabilities.global_hotkey.reason, "subtitle")
        : null,
      ctx.info.hotkey_error ? backendMessage(ctx.info.hotkey_error, "error") : null,
    ),
    section(
      l("dictation.language.title"),
      null,
      select<string>(
        [
          ["", l("common.automatic")],
          ["nl", l("dictation.language.dutch")],
          ["en", l("dictation.language.english")],
          ["fr", l("dictation.language.french")],
          ["de", l("dictation.language.german")],
        ],
        s.language ?? "",
        (v) => ctx.save((n) => (n.language = v || null)),
      ),
    ),
    section(
      l("dictation.history.retention"),
      null,
      select<string>(
        [
          ["7", l("dictation.history.days", { days: 7 })],
          ["30", l("dictation.history.days", { days: 30 })],
          ["90", l("dictation.history.days", { days: 90 })],
          ["0", l("dictation.history.always")],
        ],
        String(s.history_retention_days),
        (v) => ctx.save((n) => (n.history_retention_days = Number(v))),
      ),
    ),
    section(l("dictation.pad.title"), l("dictation.pad.preference_help"),
      toggle(l("dictation.pad.paste_externally"), s.dictation_pad_auto_paste, (value) => { void ctx.save((settings) => { settings.dictation_pad_auto_paste = value; }); }, ctx.info.capabilities.pad_external_paste),
      h("p", { class: "help" }, l("dictation.pad.paste_help"))),
    section(l("dictation.access.title"), l("dictation.access.help"),
      controlButton({ class: "btn", onclick: () => api.showWindow("onboarding", "dictation-setup") }, l("dictation.access.check_ellipsis"))),
  );
}

export function dictionaryView(ctx: Context): HTMLElement {
  const terms = ctx.info.settings.dictionary;
  const input = textField({ type: "text", placeholder: l("dictation.dictionary.example"), class: "grow" });
  const add = async () => {
    const term = input.value.trim();
    if (!term || terms.includes(term)) return;
    try { await ctx.save((n) => n.dictionary.push(term)); } catch { return; }
    document.querySelector<HTMLInputElement>(".dictionary-settings input")?.focus({ preventScroll: true });
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") add();
  });
  return h(
    "div",
    { class: "dictionary-settings" },
    section(
      l("dictation.dictionary.title"),
      l("dictation.dictionary.help"),
      h("div", { class: "row" }, input, controlButton({ class: "btn", onclick: add }, l("dictation.add"))),
      h(
        "div",
        { class: "dictionary-list" },
        terms.map((term) =>
          h(
            "div",
            { class: "dictionary-row" },
            h("span", { class: "grow" }, term),
            h(
              "button",
              { class: "button-icon", "aria-label": l("dictation.dictionary.remove", { term }), onclick: () => ctx.save((n) => (n.dictionary = n.dictionary.filter((t) => t !== term))) },
              icon("close"),
            ),
          ),
        ),
      ),
    ),
  );
}

export function stylesView(ctx: Context): HTMLElement {
  const rules = ctx.info.settings.style_rules;
  const input = textField({ type: "text", placeholder: ctx.info.platform === "macos" ? l("dictation.style.app_id") : l("dictation.style.program_name"), class: "grow" });
  const add = () => {
    const value = input.value.trim();
    const id = ctx.info.platform === "macos" || (value.includes(".") && !/\.exe$/i.test(value)) ? value : value.toLowerCase().replace(/\.exe$/, "");
    if (!id || rules.some((r) => r.bundle_id === id)) return;
    void ctx.save((n) => n.style_rules.push({ bundle_id: id, style: "neutral" }));
  };
  input.addEventListener("keydown", (event) => { if (event.key === "Enter") add(); });
  const pickerStatus = h("p", { class: "help", role: "status" });
  const unavailable = ctx.info.platform === "linux" && ctx.info.capabilities.selection.status === "unsupported";
  const pick = controlButton({ class: "btn", disabled: unavailable, onclick: async () => {
    pick.disabled = true;
    try {
      const application = await api.pickStyleApplication();
      if (application && !ctx.info.settings.style_rules.some((rule) => rule.bundle_id === application.id)) {
        await ctx.save((settings) => { settings.style_rules.push({ bundle_id: application.id, style: "neutral" }); });
      }
    } catch (error) { pickerStatus.textContent = errorText(error); }
    finally {
      pick.disabled = unavailable;
      if (pick.isConnected && !pick.disabled) pick.focus({ preventScroll: true });
    }
  } }, l("dictation.style.add_app"));
  return h(
    "div",
    { class: "style-settings" },
    section(
      l("dictation.style.title"),
      l("dictation.style.help"),
      rules.map((rule) => {
        const app = appBadge(rule.bundle_id);
        app.name.classList.add("grow");
        return h(
          "div",
          { class: "row style-rule" },
          app.icon,
          app.name,
          select(styleOptions(), rule.style, (style) =>
            ctx.save((n) => { const item = n.style_rules.find((r) => r.bundle_id === rule.bundle_id); if (item) item.style = style; }),
          ),
          h(
            "button",
            {
              class: "button-icon",
              "aria-label": l("dictation.style.remove", { app: applicationLabel(rule.bundle_id) }),
              onclick: () => ctx.save((n) => (n.style_rules = n.style_rules.filter((r) => r.bundle_id !== rule.bundle_id))),
            },
            icon("close"),
          ),
        );
      }),
      h("div", { class: "row style-add" }, pick), pickerStatus,
      h("details", { class: "advanced-style" }, h("summary", {}, l("dictation.style.manual")),
        h("div", { class: "row" }, input, controlButton({ class: "btn", disabled: unavailable, onclick: add }, l("dictation.add")))),
      unavailable ? h("p", { class: "help" }, l("dictation.style.wayland")) : null,
    ),
  );
}
