import { bindLocalization, l, t, localizeBackendText, formatDateTime } from "../i18n.ts";
import { api, listen, type DictationSessionSnapshot, type HistoryEntry, type PillState, type SetupReadiness } from "../api";
import { SerialQueue } from "../async";
import { controlButton, errorText, h } from "../dom";
import { OwnedDictationSession } from "../dictation-session";
import { PadDraftState, PadNavigationBlocked } from "../dictation-session-state";
import { icon } from "../icons";
import type { Context } from "../main";
import { hotkeyLabel } from "../presentation";
import { shortcutAccessHint } from "../pad-shortcut-state";
import { processingSummary } from "../processing-state";
import { cleanupVerificationCurrent } from "../cleanup-verification-state.ts";
import { recentDictations } from "../dictation-home-state";

const draft = new PadDraftState();
const saves = new SerialQueue();

function updateProcessing(node: HTMLElement, ctx: Context): void {
  node.replaceChildren(...processingSummary(ctx.info.settings, "dictation", ctx.info.cleanup_verification).map((stage, index) =>
    h("div", { "data-processing-stage": index === 0 ? "recognition" : index === 1 ? "cleanup" : "verification" }, h("dt", {}, stage.label), h("dd", {}, stage.destination))));
}

export async function padView(ctx: Context): Promise<HTMLElement> {
  if (ctx.param !== "__pad__") return dictationHome(ctx);
  if (!draft.dirty) draft.hydrate(await api.padText());
  let mounted = true;
  let saving: number | undefined;
  let readiness: SetupReadiness | null = null;
  let transition = false;
  let writeError: string | null = null;
  let navigationNotice = false;
  const back = h("button", { class: "text-button dictation-pad-back", onclick: () => ctx.navigate("dictation-home") }, l("dictation.pad.back"));
  const status = h("p", { class: "pad-status help", role: "status", "aria-live": "polite" });
  const target = h("p", { class: "help pad-target" });
  const textarea = h("textarea", { class: "dictation-pad", value: draft.value, lang: ctx.info.settings.language ?? "", "aria-label": l("dictation.pad.title"), placeholder: l("dictation.pad.placeholder"), spellcheck: true });
  const start = controlButton({ class: "btn primary" }, icon("mic"), l("dictation.pad.start"));
  const cancel = h("button", { class: "text-button", hidden: true, onclick: () => { void session.cancel().catch(showError); } }, l("common.cancel"));
  const copy = controlButton({ class: "btn", onclick: async () => {
    try { await api.copyText(textarea.value); status.textContent = t("dictation.pad.copied"); }
    catch (error) { showError(error); }
  } }, icon("copy"), l("common.copy"));
  const setup = h("div", { class: "pad-setup", hidden: true }, icon("warning"), h("span", {}, l("dictation.pad.not_ready")),
    controlButton({ class: "btn", onclick: () => {
      if (readiness?.configured) void api.showWindow("onboarding", "dictation-permissions").catch(showError);
      else ctx.openSetup("dictation_stt");
    } }, l("dictation.pad.setup")));
  const shortcutText = h("span");
  const shortcutRecovery = h("button", { class: "text-button", onclick: () => api.showWindow("onboarding", "dictation-permissions") }, l("dictation.access.check"));
  const shortcutHelp = h("div", { class: "pad-setup pad-shortcut-help", hidden: true, role: "status", "aria-live": "polite" }, icon("warning"), shortcutText, shortcutRecovery);
  const shortcutKey = h("kbd", {}, hotkeyLabel(ctx.info.settings.hotkey, ctx.info.platform));
  const shortcutInstruction = h("span", {}, l("dictation.pad.shortcut_help"));
  const processing = h("dl", { class: "help processing-path compact-processing", "aria-label": l("dictation.processing.title") });
  const currentVerification = h("p", { class: "help", role: "status", hidden: true });
  const refreshProcessing = () => updateProcessing(processing, ctx);
  refreshProcessing();

  const flush = async () => {
    window.clearTimeout(saving);
    await saves.drain();
    if (!draft.dirty) return;
    const value = draft.value;
    try { await saves.run(() => api.savePadText(value)); }
    catch (error) { writeError = errorText(error); throw error; }
    writeError = null;
    session.acceptLocalSavedText(value);
    draft.acknowledge(value);
  };
  const showError = (error: unknown) => { if (mounted) { status.textContent = errorText(error); status.classList.add("error"); start.disabled = false; } };
  const apply = (snapshot: DictationSessionSnapshot) => {
    if (!mounted) return;
    const busy = snapshot.phase === "listening" || snapshot.phase === "working";
    currentVerification.hidden = !busy || !snapshot.cleanup_verification;
    currentVerification.textContent = snapshot.cleanup_verification ? cleanupVerificationCurrent(snapshot.cleanup_verification) : "";
    if (!busy && !transition) navigationNotice = false;
    back.disabled = busy;
    textarea.disabled = busy;
    start.disabled = transition || snapshot.phase === "working" || readiness?.can_record === false;
    start.replaceChildren(icon(snapshot.phase === "listening" ? "square" : "mic"), snapshot.phase === "listening" ? t("dictation.pad.stop") : snapshot.phase === "working" ? t("dictation.pad.processing") : t("dictation.pad.start"));
    cancel.hidden = !busy;
    copy.disabled = busy || !textarea.value;
    if (draft.accept(snapshot)) {
      textarea.value = draft.value;
      copy.disabled = !draft.value;
    }
    status.textContent = writeError ?? (snapshot.error ? localizeBackendText(snapshot.error) : null) ?? (navigationNotice ? t("dictation.pad.finish_before_leaving") : snapshot.phase === "listening" ? t("dictation.pad.listening") : snapshot.phase === "working" ? t("dictation.pad.transcribing") : "");
    status.classList.toggle("error", !!writeError || !!snapshot.error);
    target.textContent = ctx.info.settings.dictation_pad_auto_paste
      ? snapshot.external_paste_available && snapshot.target_name ? t("dictation.pad.external_target", { app: snapshot.target_name }) : snapshot.external_paste_reason ? localizeBackendText(snapshot.external_paste_reason) : t("dictation.pad.no_target")
      : t("dictation.pad.local_target");
  };
  const session = new OwnedDictationSession("pad", apply);
  const refreshReadiness = async () => {
    try {
      const result = await api.onboardingReadiness("dictation_pad");
      if (!mounted) return;
      readiness = result;
      start.disabled = transition || !result.can_record;
      setup.hidden = result.can_record;
      const detail = setup.querySelector("span"); if (detail) detail.textContent = result.reason ? localizeBackendText(result.reason) : t("dictation.pad.not_ready");
      const shortcut = shortcutAccessHint(result, ctx.info.capabilities.global_hotkey);
      shortcutHelp.hidden = shortcut === null;
      shortcutText.textContent = shortcut?.message ?? "";
      shortcutRecovery.hidden = !shortcut?.recoverable;
      shortcutKey.textContent = hotkeyLabel(ctx.info.settings.hotkey, ctx.info.platform);
      shortcutInstruction.textContent = result.can_use_shortcut ? t("dictation.pad.shortcut_help") : t("dictation.pad.button_help");
      if (session.state.snapshot) apply(session.state.snapshot);
    } catch (error) { showError(error); }
  };
  start.addEventListener("click", async () => {
    transition = true; start.disabled = true;
    try { await flush(); await session.toggle(); }
    catch (error) { showError(error); }
    finally { transition = false; if (session.state.snapshot) apply(session.state.snapshot); }
  });
  textarea.addEventListener("input", () => {
    session.invalidateRefresh(); draft.edit(textarea.value); copy.disabled = !textarea.value;
    window.clearTimeout(saving); saving = window.setTimeout(() => { void flush().catch(showError); }, 160);
  });
  textarea.addEventListener("blur", () => { void flush().catch(showError); });
  ctx.onFlush(flush);
  ctx.beforeLeave(async () => {
    await flush();
    let left = false;
    try { left = !transition && await session.leave(); }
    catch (error) { showError(error); throw new PadNavigationBlocked(errorText(error)); }
    if (!left) {
      navigationNotice = true;
      if (session.state.snapshot) apply(session.state.snapshot);
      else status.textContent = t("dictation.pad.finish_before_leaving");
      throw new PadNavigationBlocked(t("dictation.pad.finish_before_leaving"));
    }
  });
  ctx.onDispose(() => { mounted = false; window.clearTimeout(saving); void flush().catch(ctx.report); void session.close().catch(ctx.report); });
  ctx.onInfoChange(() => { textarea.lang = ctx.info.settings.language ?? ""; refreshProcessing(); void refreshReadiness(); if (session.state.snapshot) apply(session.state.snapshot); });
  const restoreSession = async () => {
    if (!mounted || !ctx.visible) return;
    try { await session.open(); await session.refresh(); }
    catch (error) {
      if (session.state.busy) throw error;
      await session.close();
      if (mounted && ctx.visible) await session.open();
    }
    if (mounted && ctx.visible) await refreshReadiness();
  };
  ctx.onFocusChange((focused) => { if (focused) void restoreSession().catch(showError); });
  ctx.onVisibilityChange((visible) => { if (visible) void restoreSession().catch(showError); });
  const root = h("div", { class: "pad-workspace" }, back, setup,
    h("div", { class: "page-heading" }, h("div", {}, h("h1", {}, l("dictation.pad.title")), h("p", { class: "muted" }, l("dictation.pad.intro"))), start),
    h("div", { class: "shortcut-instruction" }, shortcutKey, shortcutInstruction), shortcutHelp, processing, currentVerification,
    textarea, h("div", { class: "row pad-actions" }, copy, cancel, h("span", { class: "grow" }),
      h("button", { class: "text-button", onclick: () => ctx.navigate("dictation") }, icon("settings"), l("dictation.pad.settings"))), status, target);
  bindLocalization(root, () => {
    refreshProcessing();
    void refreshReadiness();
    if (session.state.snapshot) apply(session.state.snapshot);
  });
  try { if (ctx.visible) await session.open(); } catch (error) { showError(error); }
  await refreshReadiness();
  return root;
}

function dictationHome(ctx: Context): HTMLElement {
  const key = h("kbd", {}, hotkeyLabel(ctx.info.settings.hotkey, ctx.info.platform));
  const detail = h("p", { class: "help", hidden: true });
  const readiness = h("p", { class: "dictation-home-status help", role: "status", "aria-live": "polite" }, l("dictation.home.checking"));
  let latestReadiness: SetupReadiness | null = null;
  const introduction = h("p", { class: "dictation-home-intro muted" }, l("dictation.home.intro"));
  const shortcutInstruction = h("span", { class: "muted" }, l("dictation.home.shortcut"));
  let practicePending = false;
  const repair = controlButton({ class: "btn dictation-home-repair", hidden: true, onclick: () => {
    if (latestReadiness?.configured) void api.showWindow("onboarding", "dictation-permissions").catch(ctx.report);
    else ctx.openSetup("dictation_stt");
  } }, l("dictation.pad.setup"));
  const practice = controlButton({ class: "btn dictation-home-practice", disabled: true, title: l("dictation.home.practice_help"), onclick: async () => {
    practicePending = true;
    practice.disabled = true;
    try { await api.showWindow("onboarding", "practice"); }
    catch (error) { ctx.report(error); }
    finally { practicePending = false; practice.disabled = !latestReadiness?.can_record; }
  } }, icon("mic"), l("dictation.home.try_here"));
  const processing = h("dl", { class: "help processing-path compact-processing", "aria-label": l("dictation.processing.title") });
  const currentVerification = h("p", { class: "help", role: "status", hidden: true });
  let activeDictation: PillState | null = null;
  const updateVerification = () => {
    const active = activeDictation && !["hidden", "inserted", "copied", "problem"].includes(activeDictation.state);
    currentVerification.hidden = !active || !activeDictation?.cleanup_verification;
    currentVerification.textContent = activeDictation?.cleanup_verification ? cleanupVerificationCurrent(activeDictation.cleanup_verification) : "";
  };
  const recent = h("div", { class: "recent-dictations", role: "list", "data-recent-dictations": true });
  const recentLoadStatus = h("p", { class: "error recent-load-status", role: "status", hidden: true });
  const recentFeedback = h("p", { class: "help recent-copy-status", role: "status", "aria-live": "polite", hidden: true });
  const processingDetails = h("details", { class: "dictation-processing-details" }, h("summary", {}, l("dictation.home.processing_details")), processing);
  const root = h("div", { class: "pad-workspace dictation-home", "data-dictation-readiness": "checking" },
    h("h1", {}, l("dictation.home.title")),
    introduction,
    h("div", { class: "dictation-home-shortcut" }, key, shortcutInstruction),
    readiness, detail,
    h("div", { class: "dictation-home-actions row" }, repair, practice,
      h("button", { class: "text-button dictation-home-settings", onclick: () => ctx.navigate("dictation") }, l("dictation.pad.settings"))),
    currentVerification, processingDetails,
    h("section", { class: "dictation-recent" }, h("div", { class: "row" }, h("h2", { class: "grow" }, l("dictation.home.recent")),
      h("button", { class: "text-button", "data-action": "all-history", onclick: () => ctx.navigate("history") }, l("dictation.home.all_history"))), recent, recentLoadStatus, recentFeedback),
    h("div", { class: "dictation-pad-option" }, h("button", { class: "text-button", onclick: () => ctx.navigate("dictation-pad") }, l("dictation.home.open_pad")),
      h("p", { class: "help" }, l("dictation.home.pad_help"))));
  let disposed = false;
  let revision = 0;
  let recentRevision = 0;
  let recentRowId = 0;
  const recentRows = new Map<string, { row: HTMLElement; text: HTMLElement; time: HTMLElement; entry: HistoryEntry }>();
  ctx.onDispose(() => { disposed = true; });
  const refreshRecent = async () => {
    const mine = ++recentRevision;
    try {
      const entries = recentDictations(await api.history(200));
      if (disposed || mine !== recentRevision) return;
      recentLoadStatus.hidden = true; recentLoadStatus.textContent = "";
      const ids = new Set(entries.map(entry => entry.id));
      for (const [id, item] of recentRows) {
        if (!ids.has(id)) {
          if (item.row.contains(document.activeElement)) root.querySelector<HTMLButtonElement>('[data-action="all-history"]')?.focus({ preventScroll: true });
          item.row.remove(); recentRows.delete(id);
        }
      }
      recent.querySelector(".recent-dictations-empty")?.remove();
      for (const [index, entry] of entries.entries()) {
        let item = recentRows.get(entry.id);
        if (!item) {
          const text = h("span", { id: `recent-dictation-${++recentRowId}` });
          const time = h("small", { class: "help" });
          const row = h("div", { class: "recent-dictation-entry", role: "listitem", "data-recent-id": entry.id });
          const current = { row, text, time, entry };
          const copy = controlButton({ class: "text-button recent-dictation-copy", "aria-label": l("dictation.history.copy"), "aria-describedby": text.id, onclick: async () => {
            copy.disabled = true;
            try {
              await api.copyText(current.entry.text);
              if (!disposed) { recentFeedback.textContent = t("dictation.pad.copied"); recentFeedback.classList.remove("error"); }
            } catch (error) {
              if (!disposed) { recentFeedback.textContent = errorText(error); recentFeedback.classList.add("error"); }
            } finally { copy.disabled = false; if (!disposed) recentFeedback.hidden = false; }
          } }, icon("copy"), l("common.copy"));
          row.append(h("button", { class: "recent-dictation", onclick: () => ctx.navigate("history", current.entry.id) }, text, time), copy);
          recentRows.set(entry.id, current); item = current;
        }
        item.entry = entry;
        item.text.textContent = entry.text;
        item.time.textContent = formatDateTime(entry.created_at_ms, { dateStyle: "medium", timeStyle: "short" });
        if (recent.children[index] !== item.row) recent.insertBefore(item.row, recent.children[index] ?? null);
      }
      if (!entries.length) recent.append(h("p", { class: "help recent-dictations-empty" }, l("dictation.home.recent_empty")));
    } catch (error) { if (!disposed && mine === recentRevision) { recentLoadStatus.textContent = errorText(error); recentLoadStatus.hidden = false; } }
  };
  void listen("history-changed", () => { void refreshRecent(); }).then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
  void listen<PillState>("dictation-state", (event) => {
    if (!disposed) { activeDictation = event.payload; updateVerification(); }
  }).then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
  void api.dictationState().then((value) => {
    if (!disposed) { activeDictation = value; updateVerification(); }
  }).catch(ctx.report);
  const refresh = async () => {
    const mine = ++revision;
    key.textContent = hotkeyLabel(ctx.info.settings.hotkey, ctx.info.platform);
    updateProcessing(processing, ctx);
    updateVerification();
    try {
      const ready = await api.onboardingReadiness("dictation_pad");
      if (disposed || mine !== revision) return;
      latestReadiness = ready;
      const shortcut = shortcutAccessHint(ready, ctx.info.capabilities.global_hotkey);
      const anywhere = ready.can_record && ready.can_use_shortcut && ready.can_paste_externally;
      root.dataset.dictationReadiness = ready.can_record ? "ready" : "blocked";
      readiness.textContent = ready.can_record ? t(anywhere ? "dictation.home.ready" : "dictation.home.ready_here") : t("dictation.home.needs_setup");
      readiness.classList.toggle("success", anywhere);
      introduction.textContent = t(ready.can_record && !ready.can_paste_externally ? "dictation.home.copy_intro" : ready.can_record ? "dictation.home.ready_intro" : "dictation.home.intro");
      shortcutInstruction.textContent = t(ready.can_record && !ready.can_paste_externally ? "dictation.home.shortcut_copy" : "dictation.home.shortcut");
      detail.textContent = ready.can_record ? shortcut?.message ?? "" : ready.reason ? localizeBackendText(ready.reason) : t("dictation.home.speech_setup");
      detail.hidden = !detail.textContent;
      repair.hidden = ready.can_record && !shortcut?.recoverable;
      repair.textContent = ready.can_record ? t("dictation.access.check") : t("dictation.home.finish_setup");
      repair.classList.toggle("primary", !repair.hidden);
      practice.classList.toggle("primary", repair.hidden);
      practice.disabled = !ready.can_record || practicePending;
    } catch (error) { if (!disposed && mine === revision) {
      latestReadiness = null;
      root.dataset.dictationReadiness = "blocked";
      readiness.textContent = t("dictation.home.needs_setup"); readiness.classList.remove("success");
      detail.textContent = errorText(error); detail.hidden = false;
      repair.hidden = false; repair.textContent = t("dictation.home.finish_setup"); repair.classList.add("primary");
      practice.classList.remove("primary"); practice.disabled = true;
    } }
  };
  ctx.onInfoChange(() => { void refresh(); });
  bindLocalization(root, () => { void refresh(); void refreshRecent(); });
  ctx.onFocusChange((focused) => { if (focused) { void refresh(); void refreshRecent(); } });
  void refresh(); void refreshRecent();
  return root;
}
