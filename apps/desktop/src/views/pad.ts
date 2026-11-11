import { playMotion } from "../motion";
import { bindLocalization, l, t, localizeBackendText, formatDateTime } from "../i18n.ts";
import { api, listen, type DictationSessionSnapshot, type HistoryEntry, type LocalModel, type PillState, type SetupReadiness } from "../api";
import { SerialQueue } from "../async";
import { controlButton, errorText, formatBytes, h } from "../dom";
import { OwnedDictationSession } from "../dictation-session";
import { HomePracticeDraft, PadDraftState, PadNavigationBlocked } from "../dictation-session-state";
import { icon } from "../icons";
import type { Context } from "../main";
import { shortcutRow, shortcutText as effectiveShortcutText } from "../shortcut-presentation";
import { shortcutAccessHint } from "../pad-shortcut-state";
import { processingSummary } from "../processing-state";
import { cleanupVerificationCurrent } from "../cleanup-verification-state.ts";
import { recentDictations, recommendedSpeechModel } from "../dictation-home-state";

const draft = new PadDraftState();
const saves = new SerialQueue();
const homeDraft = new HomePracticeDraft();

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
  const shortcutKey = h("kbd", {}, effectiveShortcutText(ctx.info, "dictate"));
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
      shortcutKey.textContent = effectiveShortcutText(ctx.info, "dictate");
      shortcutKey.hidden = !shortcutRow(ctx.info, "dictate")?.binding;
      shortcutInstruction.textContent = result.can_use_shortcut && !shortcutKey.hidden ? t("dictation.pad.shortcut_help") : t("dictation.pad.button_help");
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
  let disposed = false;
  let latestReadiness: SetupReadiness | null = null;
  let revision = 0;
  let transition = false;
  let notice = false;
  let practiceActivated = false;
  let actionError: string | null = null;
  let activeDictation: PillState | null = null;
  let recommendedSize: string | null = null;
  let speechModels: LocalModel[] = [];
  const timers = new Set<number>();
  const later = (action: () => void, delay: number) => {
    const timer = window.setTimeout(() => { timers.delete(timer); if (!disposed) action(); }, delay);
    timers.add(timer); return timer;
  };
  const key = h("kbd", {}, effectiveShortcutText(ctx.info, "dictate"));
  const instructionLead = h("span", {}, l("dictation.home.hold"));
  const instructionTail = h("span", {}, l("dictation.home.and_speak"));
  const instruction = h("h1", { class: "dictation-home-instruction" }, instructionLead, key, instructionTail);
  const detail = h("p", { class: "help dictation-home-detail", hidden: true });
  const readinessText = h("span", {}, l("dictation.home.checking"));
  const readiness = h("p", { class: "dictation-home-status", role: "status", "aria-live": "polite" }, icon("mic"), readinessText);
  const introduction = h("p", { class: "dictation-home-intro muted", id: "home-practice-intro" }, l("dictation.home.intro"));
  const status = h("p", { class: "help home-practice-status", role: "status", "aria-live": "polite", hidden: true });
  const textarea = h("textarea", { class: "home-practice-field", value: homeDraft.value, spellcheck: true,
    lang: ctx.info.settings.language ?? "", "aria-label": l("dictation.home.message"), placeholder: l("dictation.home.message_placeholder") });
  const currentVerification = h("p", { class: "help", role: "status", hidden: true });
  const showError = (error: unknown) => {
    if (!disposed) { actionError = errorText(error); status.textContent = actionError; status.classList.add("error"); status.hidden = false; }
  };
  const repair = controlButton({ class: "btn dictation-home-repair", hidden: true, onclick: () => {
    if (latestReadiness?.configured) void api.showWindow("onboarding", "dictation-permissions").catch(showError);
    else ctx.openSetup("dictation_stt");
  } }, l("dictation.home.finish_setup"));
  const practice = controlButton({ class: "btn dictation-home-practice", disabled: true, title: l("dictation.home.practice_help"), onclick: () => {
    practiceActivated = true;
    void session.open().then(() => { if (!disposed) textarea.focus(); }).catch(showError);
  } }, icon("mic"), l("dictation.home.try_here"));
  const start = controlButton({ class: "btn home-practice-start", disabled: true, onclick: () => { void toggle(); } }, icon("mic"), l("dictation.pad.start"));
  const handsFree = controlButton({ class: "btn dictation-home-hands-free", disabled: true, onclick: () => { void toggle(); } }, l("dictation.home.hands_free"));
  const cancel = controlButton({ class: "btn home-practice-cancel", hidden: true, onclick: () => { actionError = null; void session.cancel().catch(showError); } }, l("common.cancel"));
  const copyPractice = controlButton({ class: "text-button home-practice-copy", disabled: !homeDraft.value, onclick: async () => {
    try { await api.copyText(textarea.value); status.textContent = t("dictation.pad.copied"); status.classList.remove("error"); status.hidden = false; }
    catch (error) { showError(error); }
  } }, icon("copy"), l("common.copy"));
  const updateVerification = () => {
    const owned = session.state.snapshot;
    const busy = owned && ["listening", "working"].includes(owned.phase);
    const active = activeDictation && !["hidden", "inserted", "copied", "problem"].includes(activeDictation.state);
    const verification = busy ? owned.cleanup_verification : active ? activeDictation?.cleanup_verification : null;
    currentVerification.hidden = !verification;
    currentVerification.textContent = verification ? cleanupVerificationCurrent(verification) : "";
  };
  const apply = (snapshot: DictationSessionSnapshot) => {
    if (disposed) return;
    homeDraft.accept(snapshot);
    if (textarea.value !== homeDraft.value) textarea.value = homeDraft.value;
    const busy = session.state.busy;
    if (!busy && !transition) notice = false;
    textarea.disabled = busy;
    practice.disabled = transition || busy || !latestReadiness?.can_record;
    start.disabled = transition || snapshot.phase === "working" || !latestReadiness?.can_record;
    start.replaceChildren(icon(snapshot.phase === "listening" ? "square" : "mic"), t(snapshot.phase === "listening" ? "dictation.pad.stop" : snapshot.phase === "working" ? "dictation.pad.processing" : "dictation.pad.start"));
    handsFree.hidden = busy; handsFree.disabled = transition || !latestReadiness?.can_record;
    cancel.hidden = !busy; cancel.disabled = transition;
    copyPractice.disabled = busy || !textarea.value;
    status.textContent = actionError ?? (snapshot.error ? localizeBackendText(snapshot.error) : notice ? t("dictation.pad.finish_before_leaving")
      : snapshot.phase === "listening" ? t("dictation.pad.listening") : snapshot.phase === "working" ? t("dictation.pad.transcribing")
      : snapshot.phase === "done" ? t("dictation.home.result_here") : "");
    status.classList.toggle("error", !!actionError || !!snapshot.error || notice); status.hidden = !status.textContent;
    panel.dataset.active = String(practiceActivated || busy || !!textarea.value);
    updateVerification();
  };
  const session = new OwnedDictationSession("home_practice", apply);
  const toggle = async () => {
    if (transition || disposed) return;
    actionError = null; practiceActivated = true; transition = true; start.disabled = true; handsFree.disabled = true; practice.disabled = true;
    try { await session.toggle(); }
    catch (error) { showError(error); }
    finally { transition = false; if (session.state.snapshot) apply(session.state.snapshot); else void refresh(); }
  };
  textarea.addEventListener("focus", () => {
    practiceActivated = true; panel.dataset.active = "true";
    void session.open().catch(showError);
  });
  textarea.addEventListener("input", () => { homeDraft.value = textarea.value; copyPractice.disabled = !textarea.value; });
  const practiceHint = h("p", { class: "help home-practice-hint" }, l("dictation.home.field_help"));
  const panel = h("section", { class: "home-practice-panel", "aria-label": l("dictation.home.practice_title"), "data-active": String(!!homeDraft.value) },
    h("div", { class: "home-practice-message" }, h("label", { for: "home-practice-message", class: "help" }, l("dictation.home.message")), textarea),
    practiceHint, h("div", { class: "row home-practice-actions" }, start, copyPractice), status);
  textarea.id = "home-practice-message";
  const changeProcessing = h("button", { class: "text-button dictation-home-settings", onclick: () => ctx.navigate("dictation") }, l("dictation.home.change"));
  const processing = h("div", { class: "home-processing-chips", "aria-label": l("dictation.processing.title") }, changeProcessing);
  const updateChips = () => {
    const settings = ctx.info.settings;
    const stages = processingSummary(settings, "dictation", ctx.info.cleanup_verification);
    processing.querySelectorAll(".home-processing-chip").forEach(chip => chip.remove());
    for (const [index, stage] of stages.entries()) {
      const choice = index === 0 ? settings.dictation_stt : settings.llm;
      const account = settings.providers.find(provider => provider.id === choice.provider_id);
      const local = index === 1 && settings.mode === "fast" || account && ["local", "apple_speech", "apple_intelligence"].includes(account.kind);
      const label = index === 0 ? local ? t("dictation.home.speech_local") : t("dictation.home.speech_by", { provider: account?.name ?? stage.destination })
        : index === 1 ? settings.mode === "fast" ? t("dictation.processing.fast") : local ? t("dictation.home.cleanup_local") : t("dictation.home.cleanup_by", { provider: account?.name ?? stage.destination })
        : `${stage.label}: ${stage.destination}`;
      const destination = `${stage.label}: ${stage.destination}`;
      processing.insertBefore(h("span", { class: "home-processing-chip", title: destination, "aria-label": destination, "data-processing-stage": index === 0 ? "recognition" : index === 1 ? "cleanup" : "verification" },
        icon(local ? "laptop" : "globe"), h("span", {}, label)), changeProcessing);
    }
  };
  const recent = h("div", { class: "recent-dictations", role: "list", "data-recent-dictations": true });
  const recentError = h("span");
  const recentLoadStatus = h("div", { class: "error recent-load-status", role: "status", hidden: true }, recentError,
    h("button", { class: "text-button", onclick: () => { void refreshRecent(); } }, l("common.retry")));
  const recentFeedback = h("p", { class: "help recent-copy-status", role: "status", "aria-live": "polite", hidden: true });
  const root = h("div", { class: "pad-workspace dictation-home", "data-dictation-readiness": "checking" },
    h("div", { class: "dictation-home-content" },
      h("section", { class: "dictation-home-hero" }, h("div", { class: "dictation-home-guidance" }, readiness, instruction, introduction,
        h("div", { class: "dictation-home-actions row" }, repair, practice, handsFree, cancel), detail, processing, currentVerification), panel),
      h("section", { class: "dictation-recent" }, h("div", { class: "row" }, h("h2", { class: "grow" }, l("dictation.home.recent")),
        h("button", { class: "text-button", "data-action": "all-history", onclick: () => ctx.navigate("history") }, l("dictation.home.all_history"))), recent, recentLoadStatus, recentFeedback)));
  let recentRevision = 0;
  let recentRowId = 0;
  let recentLoaded = false;
  const recentRows = new Map<string, { row: HTMLElement; text: HTMLElement; time: HTMLElement; entry: HistoryEntry }>();
  const refreshRecent = async () => {
    const mine = ++recentRevision;
    try {
      const entries = recentDictations(await api.history(200));
      if (disposed || mine !== recentRevision) return;
      recentLoadStatus.hidden = true; recentError.textContent = "";
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
          let feedbackTimer: number | undefined;
          const copy = controlButton({ class: "text-button recent-dictation-copy", "aria-label": l("dictation.history.copy"), "aria-describedby": text.id, onclick: async () => {
            copy.disabled = true;
            try {
              await api.copyText(current.entry.text);
              if (!disposed) {
                window.clearTimeout(feedbackTimer); if (feedbackTimer) timers.delete(feedbackTimer);
                copy.replaceChildren(icon("check")); playMotion(copy.firstElementChild as HTMLElement, "motion-feedback"); copy.dataset.copied = "true";
                feedbackTimer = later(() => { copy.replaceChildren(icon("copy")); delete copy.dataset.copied; }, 1200);
                recentFeedback.textContent = t("dictation.pad.copied"); recentFeedback.classList.remove("error");
              }
            } catch (error) {
              if (!disposed) { recentFeedback.textContent = errorText(error); recentFeedback.classList.add("error"); }
            } finally { copy.disabled = false; if (!disposed) recentFeedback.hidden = false; }
          } }, icon("copy"));
          row.append(h("button", { class: "recent-dictation", onclick: () => ctx.navigate("history", current.entry.id) }, text, time), copy);
          if (recentLoaded) { row.classList.add("recent-dictation-new"); later(() => row.classList.remove("recent-dictation-new"), 1600); }
          recentRows.set(entry.id, current); item = current;
        }
        item.entry = entry;
        item.text.textContent = entry.text;
        item.time.textContent = formatDateTime(entry.created_at_ms, { timeStyle: "short" });
        item.time.title = formatDateTime(entry.created_at_ms);
        if (recent.children[index] !== item.row) recent.insertBefore(item.row, recent.children[index] ?? null);
      }
      recentLoaded = true;
      if (!entries.length) recent.append(h("p", { class: "help recent-dictations-empty" }, l("dictation.home.recent_empty")));
    } catch (error) { if (!disposed && mine === recentRevision) { recentError.textContent = errorText(error); recentLoadStatus.hidden = false; } }
  };
  void listen("history-changed", () => { void refreshRecent(); }).then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
  void listen<PillState>("dictation-state", (event) => {
    if (!disposed) { activeDictation = event.payload; updateVerification(); }
  }).then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
  void api.dictationState().then((value) => { if (!disposed) { activeDictation = value; updateVerification(); } }).catch(ctx.report);
  const refresh = async () => {
    const mine = ++revision;
    key.textContent = effectiveShortcutText(ctx.info, "dictate");
    const hasShortcut = !!shortcutRow(ctx.info, "dictate")?.binding;
    textarea.lang = ctx.info.settings.language ?? "";
    updateChips(); updateVerification();
    try {
      const ready = await api.onboardingReadiness("dictation_pad");
      if (disposed || mine !== revision) return;
      latestReadiness = ready;
      const shortcut = shortcutAccessHint(ready, ctx.info.capabilities.global_hotkey);
      const anywhere = hasShortcut && ready.can_record && ready.can_use_shortcut && ready.can_paste_externally;
      const speech = ctx.info.settings.dictation_stt;
      const account = ctx.info.settings.providers.find(provider => provider.id === speech.provider_id);
      const provider = ctx.info.kinds.find(kind => kind.kind === account?.kind);
      const model = speechModels.find(model => model.id === speech.model);
      const download = !ready.configured && account?.kind === "local" && provider?.availability.status === "available"
        && model && model.state.kind !== "downloaded";
      root.dataset.dictationReadiness = ready.can_record ? "ready" : "blocked";
      readiness.replaceChildren(icon(ready.can_record ? "check" : "warning"), readinessText);
      readinessText.textContent = t(ready.can_record ? anywhere ? "dictation.home.ready" : "dictation.home.ready_here" : "dictation.home.needs_setup");
      readiness.classList.toggle("success", ready.can_record); readiness.classList.toggle("error", !ready.can_record);
      instructionLead.textContent = t(!ready.can_record && !ready.configured ? download ? "dictation.home.download_title" : "dictation.home.setup_title" : hasShortcut ? "dictation.home.hold" : "shortcuts.button");
      key.hidden = !hasShortcut || !ready.can_record && !ready.configured; instructionTail.hidden = key.hidden;
      introduction.textContent = !ready.can_record ? download ? t(recommendedSize ? "dictation.home.download_intro" : "dictation.home.download_intro_unknown", { size: recommendedSize ?? "" })
        : ready.reason ? localizeBackendText(ready.reason) : t("dictation.home.speech_setup")
        : t(!ready.can_paste_externally ? "dictation.home.copy_intro" : ready.can_use_shortcut && hasShortcut ? "dictation.home.release_help" : "dictation.home.ready_intro");
      detail.textContent = ready.can_record ? shortcut?.message ?? "" : download && ready.reason ? localizeBackendText(ready.reason) : t("dictation.home.finish_to_try");
      detail.hidden = !detail.textContent;
      repair.hidden = ready.can_record && !shortcut?.recoverable;
      repair.textContent = t(ready.can_record ? "dictation.access.check" : "dictation.home.finish_setup");
      repair.classList.toggle("primary", !ready.can_record);
      practice.classList.toggle("primary", ready.can_record);
      practice.disabled = transition || !ready.can_record || session.state.busy;
      practice.setAttribute("aria-describedby", "home-practice-intro home-practice-readiness");
      detail.id = "home-practice-readiness";
      handsFree.disabled = transition || !ready.can_record; handsFree.hidden = !ready.can_record || session.state.busy;
      start.disabled = transition || !ready.can_record;
      textarea.disabled = session.state.busy;
      if (session.state.snapshot) apply(session.state.snapshot);
    } catch (error) { if (!disposed && mine === revision) {
      latestReadiness = null; root.dataset.dictationReadiness = "blocked";
      readiness.replaceChildren(icon("warning"), readinessText);
      readinessText.textContent = t("dictation.home.needs_setup"); readiness.classList.remove("success"); readiness.classList.add("error");
      detail.textContent = errorText(error); detail.hidden = false;
      repair.hidden = false; repair.textContent = t("dictation.home.finish_setup"); repair.classList.add("primary");
      practice.classList.remove("primary"); practice.disabled = true; start.disabled = true; handsFree.hidden = true;
    } }
  };
  const restoreSession = async () => {
    if (disposed || !ctx.visible || !practiceActivated) return;
    try { await session.open(); await session.refresh(); }
    catch (error) {
      if (session.state.busy) throw error;
      await session.close();
      if (!disposed && ctx.visible) await session.open();
    }
  };
  ctx.beforeLeave(async () => {
    try {
      if (!transition && await session.leave()) return;
    } catch (error) { showError(error); throw new PadNavigationBlocked(errorText(error)); }
    notice = true;
    if (session.state.snapshot) apply(session.state.snapshot);
    else { status.textContent = t("dictation.pad.finish_before_leaving"); status.hidden = false; }
    throw new PadNavigationBlocked(t("dictation.pad.finish_before_leaving"));
  });
  ctx.onDispose(() => { disposed = true; for (const timer of timers) window.clearTimeout(timer); void session.close().catch(ctx.report); });
  ctx.onInfoChange(() => { void refresh(); void refreshRecent(); });
  bindLocalization(root, () => { void refresh(); void refreshRecent(); });
  ctx.onFocusChange((focused) => { if (focused) { void refresh(); void refreshRecent(); void restoreSession().catch(showError); } });
  ctx.onVisibilityChange((visible) => { if (visible) void restoreSession().catch(showError); });
  void api.localModels(ctx.info.settings.language, true).then(models => {
    if (disposed) return;
    speechModels = models;
    const model = recommendedSpeechModel(models); recommendedSize = model ? formatBytes(model.size_bytes) : null;
    void refresh();
  }).catch(ctx.report);
  void refresh(); void refreshRecent();
  return root;
}
