import { api, type WritingProfileConnection, type WritingProfileLanguage, type WritingProfileStatus } from "../api";
import { confirmDialog, controlButton, errorCode, h, section, select, toggle } from "../dom";
import { bindLocalization, formatDateTime, formatNumber, l, liveText, t } from "../i18n";
import type { Context } from "../main";
import { isWritingTraitKey, writingConnectionText, writingErrorText, writingLanguageName, writingProgress, writingSourceSessions, writingTraitChoices, writingTraitLabel } from "../writing-profile-state";

const STATUS_INTERVAL_MS = 10_000;

export function writingProfileSection(ctx: Context): HTMLElement | null {
  const root = h("div", { class: "writing-profile-settings" });
  let status: WritingProfileStatus | null = null;
  let connection: WritingProfileConnection | null = null;
  let failure: string | null = null;
  let connectionFailure: string | null = null;
  let busy = false;
  let disposed = false;
  let copied: boolean | null = null;
  let copyTimer: number | undefined;
  let loading = false;
  let updateVersion = 0;
  let openSources = false;
  const openLanguages = new Set<string>();
  const changedLanguages = new Set<string>();

  const current = () => !disposed;
  const disabledControls = (node: HTMLElement) => {
    for (const control of node.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input, button")) control.disabled = true;
    for (const choice of node.querySelectorAll<HTMLElement & { disabled?: boolean }>(".choice")) choice.disabled = true;
  };

  async function update(action: () => Promise<WritingProfileStatus | void>) {
    if (busy || disposed) return;
    const focused = document.activeElement instanceof HTMLElement
      ? document.activeElement.closest<HTMLElement>("[data-writing-control]")?.dataset.writingControl : undefined;
    updateVersion++;
    busy = true; failure = null; copied = null; draw();
    try {
      const next = await action();
      if (!current()) return;
      if (next) status = next;
    } catch (error) {
      if (current()) failure = errorCode(error);
    } finally {
      if (current()) {
        busy = false; draw();
        const control = [...root.querySelectorAll<HTMLElement>("[data-writing-control]")].find((item) => item.dataset.writingControl === focused);
        (control?.matches("input:not(:disabled),button:not(:disabled)") ? control : control?.querySelector<HTMLElement>("input:not(:disabled),button:not(:disabled)"))?.focus();
      }
    }
  }

  async function setLearning(enabled: boolean) {
    if (!status || busy) return;
    if (enabled && status.started_at_ms === null && !await confirmDialog(l("settings.writing.enable_title"), l("settings.writing.disclosure"), l("settings.writing.enable_action"))) {
      if (current()) draw();
      return;
    }
    await update(() => api.setWritingProfileLearning(enabled));
  }

  async function setSharing(enabled: boolean) {
    await update(async () => {
      const next = await api.setWritingProfileSharing(enabled);
      if (current()) { connection = next; connectionFailure = null; }
    });
  }

  async function deleteProfile() {
    if (busy || !await confirmDialog(l("settings.writing.delete_title"), l("settings.writing.delete_help"))) return;
    await update(async () => {
      const next = await api.deleteWritingProfile();
      if (current() && connection) connection = { ...connection, enabled: false, designated: false };
      return next;
    });
  }

  function languageBlock(language: WritingProfileLanguage, snapshot: WritingProfileStatus): HTMLElement {
    const ready = language.status === "ready";
    const name = writingLanguageName(language.language);
    const details = h("details", { class: "writing-profile-language", open: openLanguages.has(language.language) || !changedLanguages.has(language.language) && snapshot.languages[0] === language });
    details.append(h("summary", { class: "row" }, h("strong", { class: "grow" }, name), h("span", { class: ready ? "badge success" : "badge" }, l(ready ? "settings.writing.ready" : "settings.writing.insufficient"))));
    details.addEventListener("toggle", () => {
      changedLanguages.add(language.language);
      if (details.open) openLanguages.add(language.language); else openLanguages.delete(language.language);
    });
    const progress = h("progress", { max: 1, value: writingProgress(language, snapshot), "aria-label": l("settings.writing.progress", () => ({ language: String(name) })) });
    const body = h("div", { class: "writing-profile-language-body" },
      h("p", { class: "help" }, l("settings.writing.evidence", () => ({ words: formatNumber(language.word_count), sessions: formatNumber(language.session_count) }))), progress);
    for (const option of snapshot.trait_options) {
      if (!isWritingTraitKey(option.key)) continue;
      const key = option.key;
      const trait = language.traits.find((item) => item.key === key);
      const choice = select(writingTraitChoices(key, option.values, trait && !trait.overridden ? trait.value : null), trait?.overridden ? trait.value : "", (value) => {
        void update(() => api.setWritingProfileTrait(language.language, key, value || null));
      }, "menu");
      choice.dataset.writingControl = `trait:${language.language}:${key}`;
      choice.setAttribute("aria-label", String(writingTraitLabel(key)));
      bindLocalization(choice, () => choice.setAttribute("aria-label", String(writingTraitLabel(key))));
      const support = trait?.overridden ? l("settings.writing.override") : trait ? l("settings.writing.support", () => ({ count: formatNumber(trait.support) })) : l("settings.writing.not_learned");
      body.append(h("div", { class: "row writing-profile-trait" },
        h("div", { class: "grow" }, h("span", {}, writingTraitLabel(key)), h("p", { class: "help" }, support)), choice));
    }
    details.append(body);
    return details;
  }

  function sources(snapshot: WritingProfileStatus): HTMLElement {
    const sessions = writingSourceSessions(snapshot.sources);
    const details = h("details", { class: "writing-profile-sources", open: openSources },
      h("summary", {}, l("settings.writing.sources", () => ({ count: formatNumber(sessions.length) }))),
      h("p", { class: "help" }, l("settings.writing.sources_help")));
    details.addEventListener("toggle", () => { openSources = details.open; });
    if (!sessions.length) details.append(h("p", { class: "help" }, l("settings.writing.sources_empty")));
    for (const source of sessions) {
      const title = source.kind === "dictation" ? l("settings.writing.source.dictation") : source.title;
      const state = source.excluded ? l("settings.writing.source.excluded") : !source.analyzed ? l("settings.writing.source.pending") : null;
      const include = toggle(l("settings.writing.source.use"), !source.excluded, (included) => {
        void update(() => api.setWritingProfileSourceExcluded(source.id, !included));
      });
      include.dataset.writingControl = `source:${source.id}`;
      details.append(h("div", { class: "writing-profile-source" },
        h("div", { class: "row" }, h("strong", { class: "grow" }, title), state ? h("span", { class: "badge" }, state) : null),
        h("p", { class: "help" }, liveText(() => `${formatDateTime(source.created_at_ms)} · ${formatNumber(source.word_count)} ${t("settings.writing.words")}`)),
        h("p", { class: "help" }, source.language ? writingLanguageName(source.language) : l("settings.writing.source.unknown_language")), include));
    }
    return details;
  }

  function sharing(): HTMLElement {
    const panel = section(l("settings.writing.sharing.title"), l("settings.writing.sharing.help"));
    if (!connection) {
      panel.append(h("p", { class: connectionFailure ? "error" : "help", role: "status" }, connectionFailure
        ? liveText(() => writingErrorText(connectionFailure)) : l("common.loading")));
      return panel;
    }
    const snapshot = connection;
    const enabled = toggle(l("settings.writing.sharing.enabled"), snapshot.enabled, (value) => { void setSharing(value); });
    enabled.dataset.writingControl = "sharing";
    const input = enabled.querySelector<HTMLInputElement>("input");
    if (input) input.disabled = !snapshot.signed_in;
    panel.append(enabled,
      h("p", { class: snapshot.enabled && snapshot.error || connectionFailure ? "error" : "help", role: "status" }, liveText(() => connectionFailure ? writingErrorText(connectionFailure) : writingConnectionText(snapshot))));
    if (snapshot.signed_in && !snapshot.designated) panel.append(controlButton({ class: "btn", onclick: () => { void setSharing(true); } }, l("settings.writing.sharing.designate")));
    if (snapshot.mcp_url) {
      panel.append(h("p", { class: "help" }, l("settings.writing.sharing.setup")),
        h("div", { class: "row" }, h("code", { class: "grow writing-profile-url" }, snapshot.mcp_url),
          controlButton({ class: "btn", onclick: async () => {
            try { await api.copyText(snapshot.mcp_url); if (current()) copied = true; }
            catch { if (current()) copied = false; }
            if (!current()) return;
            draw(); window.clearTimeout(copyTimer);
            copyTimer = window.setTimeout(() => { if (current()) { copied = null; draw(); } }, 2500);
          } }, l("settings.writing.sharing.copy"))));
    }
    if (copied !== null) panel.append(h("p", { class: copied ? "help" : "error", role: "status" }, l(copied ? "settings.writing.sharing.copied" : "settings.writing.sharing.copy_error")));
    return panel;
  }

  function draw() {
    if (disposed) return;
    const panel = section(l("settings.writing.title"), l("settings.writing.help"));
    if (!status) {
      panel.append(h("p", { class: failure ? "error" : "help", role: "status" }, failure ? liveText(() => writingErrorText(failure)) : l("common.loading")));
      root.replaceChildren(panel);
      return;
    }
    if (!status.available) {
      panel.append(h("p", { class: "help" }, l("settings.writing.error.unavailable")));
      root.replaceChildren(panel);
      return;
    }
    const snapshot = status;
    const learning = toggle(l("settings.writing.learning"), snapshot.learning_enabled, (value) => { void setLearning(value); });
    learning.dataset.writingControl = "learning";
    panel.append(learning,
      h("p", { class: "help" }, l("settings.writing.disclosure")),
      h("p", { class: "help" }, l("settings.writing.source.eligibility")),
      h("p", { class: "help" }, l("settings.writing.privacy")),
      h("p", { class: "help" }, l("settings.writing.threshold", () => ({ words: formatNumber(snapshot.min_words), sessions: formatNumber(snapshot.min_sessions) }))));
    if (!snapshot.learning_enabled && snapshot.started_at_ms !== null) panel.append(h("p", { class: "help", role: "status" }, l("settings.writing.paused")));
    if (busy || snapshot.processing) panel.append(h("p", { class: "help", role: "status" }, l(busy ? "settings.writing.updating" : "settings.writing.working")));
    const code = failure ?? snapshot.error;
    if (code) panel.append(h("p", { class: "error", role: "status" }, liveText(() => writingErrorText(code))));
    panel.append(h("div", { class: "row writing-profile-actions" },
      controlButton({ class: "btn", "data-writing-control": "refresh", disabled: !snapshot.learning_enabled, onclick: () => { void update(() => api.refreshWritingProfile()); } }, l("settings.writing.refresh")),
      controlButton({ class: "btn danger", "data-writing-control": "delete", disabled: snapshot.started_at_ms === null && !snapshot.sources.length, onclick: () => { void deleteProfile(); } }, l("settings.writing.delete"))));
    if (!snapshot.languages.length) panel.append(h("p", { class: "help" }, l("settings.writing.empty")));
    else panel.append(...snapshot.languages.map((language) => languageBlock(language, snapshot)));
    panel.append(h("p", { class: "help" }, l("settings.writing.limitations")), sources(snapshot));
    root.replaceChildren(panel, sharing());
    root.setAttribute("aria-busy", String(busy || !!snapshot.processing));
    if (busy) disabledControls(root);
  }

  async function load() {
    if (disposed || busy || loading) return;
    if (status && document.activeElement && root.contains(document.activeElement)) return;
    loading = true;
    const version = updateVersion;
    const [profile, access] = await Promise.allSettled([api.writingProfileStatus(), api.writingProfileConnection()]);
    loading = false;
    if (current() && version === updateVersion && !busy) {
      if (profile.status === "fulfilled") { status = profile.value; failure = null; }
      else failure = "load_failed";
      if (access.status === "fulfilled") { connection = access.value; connectionFailure = null; }
      else connectionFailure = "connection_failed";
      draw();
    }
  }

  draw();
  void load();
  let unlisten: (() => void) | null = null;
  void api.onWritingProfileChanged(() => { void load(); }).then((stop) => {
    if (disposed) stop(); else unlisten = stop;
  }).catch(() => { /* Periodic status refresh remains available if events are unavailable. */ });
  const timer = window.setInterval(() => { if (!document.hidden) void load(); }, STATUS_INTERVAL_MS);
  ctx.onDispose(() => { disposed = true; unlisten?.(); window.clearInterval(timer); window.clearTimeout(copyTimer); });
  return root;
}
