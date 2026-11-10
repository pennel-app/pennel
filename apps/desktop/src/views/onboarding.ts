import { api, fixtureClient, listen, type DictationSessionSnapshot, type KindInfo, type LocalModel, type OnboardingGoal, type OnboardingStep, type PermissionInfo, type ProviderAccount, type ProviderKind, type SetupReadiness } from "../api";
import { controlButton, textField, errorText, formatBytes, h, isCancellation, select, type ChoiceElement } from "../dom";
import { OwnedDictationSession } from "../dictation-session";
import { icon } from "../icons";
import { defaultModelId, initialEngine, manualPracticeAllowed, modelOptionLabel, onboardingFlow, onboardingProgressChange, onboardingResumeStep, onboardingRouteIntent, onboardingSteps, stepIndex, type Engine } from "../onboarding-state";
import { trainedLabel } from "../speaker-state";
import { EnrollmentPanel, SpeakerModels, enrollmentBlocker, enrollmentIntro, speakerDownload } from "./voices";
import { hotkeyLabel, hotkeyKeyLabel } from "../presentation";
import { workspaceRoute } from "../workspace-state";
import { recommendedSpeechModel } from "../dictation-home-state";
import type { Context } from "../main";
import wordmarkSource from "../../../../assets/brand/wordmark.svg?raw";
import { VisibilityPoll } from "../window-lifecycle";
import { bindLocalization, getInterfaceLanguage, l, liveText, localizeBackendText, t, type LocalizedText, type TranslationKey, type UiText } from "../i18n";

function languageOptions(): [string, string | LocalizedText][] { return [["", l("onboarding.language_auto")], ["nl", l("onboarding.language_dutch")], ["en", l("onboarding.language_english")], ["fr", l("onboarding.language_french")], ["de", l("onboarding.language_german")]]; }
const phrases: Record<string, string> = { nl: "Zullen we morgen om tien uur afspreken?", en: "Shall we meet tomorrow at ten?", fr: "On se retrouve demain à dix heures ?", de: "Treffen wir uns morgen um zehn Uhr?" };

function validationError(key: TranslationKey): Error {
  return Object.defineProperty(new Error(), "message", { get: () => t(key), configurable: true });
}

/** The wordmark follows the text colour so it stays legible in dark mode; the full stop keeps the brand orange. */
function wordmark(): HTMLElement {
  const node = h("div", { class: "onboarding-wordmark", role: "img", "aria-label": "Pennel" });
  node.innerHTML = wordmarkSource.replace(/stroke="#2A2723"/gi, 'stroke="currentColor"').replace(/fill="#C2410C"/gi, 'fill="var(--accent)"');
  node.querySelector("svg")?.setAttribute("aria-hidden", "true");
  return node;
}

class OnboardingWizard {
  readonly root = h("div", { class: "onboarding-shell" });
  private readonly body = h("div", { class: "onboarding-body" });
  private readonly progress = h("div", { class: "onboarding-progress", role: "img" });
  private readonly next = controlButton({ class: "btn primary onboarding-next", onclick: () => { void this.advance().catch(this.fail); } });
  private readonly back = h("button", { class: "text-button", onclick: () => { void this.previous().catch(this.fail); } }, l("onboarding.back"));
  private readonly skip = h("button", { class: "text-button", onclick: () => { void this.leave().catch(this.fail); } });
  private readonly count = h("span", { class: "help onboarding-count" });
  private readonly topControls = h("div", { class: "onboarding-top-controls" });
  private readonly cancelDownload = h("button", { class: "text-button onboarding-cancel-download", hidden: true, onclick: () => { void this.cancelCurrentDownload().catch(this.fail); } }, l("onboarding.cancel_download"));
  private readonly theme = h("button", { class: "button-icon", "aria-label": l("onboarding.change_appearance"), onclick: () => {
    const dark = this.ctx.info.settings.appearance === "dark" || (this.ctx.info.settings.appearance === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    void this.ctx.save((settings) => { settings.appearance = dark ? "light" : "dark"; }).catch(this.fail);
  } });
  private readonly message = h("p", { class: "onboarding-message error", role: "status", "aria-live": "polite", hidden: true });
  private readonly readyRow = h("div", { class: "onboarding-permissions" });
  private readonly engineDetail = h("section", { class: "onboarding-engine-detail", id: "onboarding-engine-detail" });
  private readonly advancedModel = h("div", { class: "engine-model-options" });
  private readonly localButtons = new Map<Engine, HTMLButtonElement>();
  private readonly goalButtons = new Map<OnboardingGoal, HTMLButtonElement>();
  private index: number;
  private get step(): OnboardingStep { return onboardingSteps[this.index]; }
  private goal: OnboardingGoal;
  private legacyStep: OnboardingStep | undefined;
  private preserveProgress = false;
  private permissionRepair = false;
  private showOptions = false;
  private get flow(): OnboardingStep[] { return this.preserveProgress ? ["engine", "permissions", "practice"] : onboardingFlow(this.legacyStep); }
  private engine: Engine;
  private language: string;
  private localModelId: string;
  private providerChoice: string;
  private providerModel: string;
  private providerBaseUrl: string;
  private models: LocalModel[] = [];
  private recommendedModelId: string | null = null;
  private modelPicker: ChoiceElement<string> | null = null;
  private modelPickerSignature = "";
  private preferRecommendation: boolean;
  private continueAfterDownload: string | null = null;
  private permissions: PermissionInfo[] = [];
  private readiness: SetupReadiness | null = null;
  private downloading = false;
  private readonly attachedDownloads = new Set<string>();
  private downloadProgress = { downloaded: 0, total: 0 };
  private downloadError: string | null = null;
  private lastError: unknown = null;
  private actionBusy = false;
  private disposed = false;
  private renderGeneration = 0;
  private modelsGeneration = 0;
  private permissionSignature = "";
  private poll: VisibilityPoll | undefined;
  private readonly cleanup: (() => void)[] = [];
  private practice: OwnedDictationSession | null = null;
  private practiceDone = false;
  private practiceStart: Promise<void> = Promise.resolve();
  private held = false;
  private practiceText: HTMLTextAreaElement | null = null;
  private practiceStatus: HTMLParagraphElement | null = null;
  private practiceButton: HTMLButtonElement | null = null;
  private practiceKey: HTMLButtonElement | null = null;
  private voicePanel: EnrollmentPanel | null = null;
  private interfacePicker: ChoiceElement<"system" | "en" | "nl"> | null = null;
  private topLanguagePicker: ChoiceElement<"system" | "en" | "nl"> | null = null;
  private refreshPracticePresentation: (() => void) | null = null;
  private voiceModels: SpeakerModels | null = null;
  private speechChoiceSignature: string;

  constructor(private readonly ctx: Context) {
    const settings = ctx.info.settings;
    const requested = new URLSearchParams(location.search).get("tab");
    const intent = onboardingRouteIntent(requested, settings.onboarding);
    this.goal = intent.goal;
    this.preserveProgress = intent.preserveProgress;
    this.preferRecommendation = !intent.preserveProgress && settings.onboarding.status !== "completed" && settings.onboarding.resume_step === "welcome";
    this.permissionRepair = intent.preserveProgress && intent.step === "permissions";
    const choice = this.goal === "meeting" ? settings.meeting_stt : settings.dictation_stt;
    this.speechChoiceSignature = JSON.stringify(choice);
    const account = settings.providers.find((provider) => provider.id === choice.provider_id);
    this.engine = initialEngine(account?.kind, ctx.info.kinds.find((kind) => kind.kind === "local")?.availability.status !== "unsupported");
    this.language = settings.language ?? "";
    this.localModelId = account?.kind === "local" && !this.preferRecommendation ? choice.model : "";
    this.providerChoice = account && this.customAccounts().some((item) => item.id === account.id) ? account.id
      : this.customAccounts()[0]?.id ?? `new:${this.customKinds()[0]?.kind ?? "open_ai"}`;
    const providerKind = this.customKind();
    this.providerModel = this.engine === "custom" ? choice.model : providerKind?.default_stt_model ?? "";
    this.providerBaseUrl = this.customAccount()?.base_url ?? providerKind?.default_base_url ?? "";
    this.legacyStep = settings.onboarding?.resume_step;
    this.index = stepIndex(intent.step === "resume" ? "practice" : intent.step);
    if (requested && onboardingSteps.includes(requested as OnboardingStep)) {
      if (requested === "language" || requested === "voice") this.legacyStep = requested;
    }
    const fixture = fixtureClient()?.fixture?.name ?? "";
    if (fixture.includes("engine-apple")) this.engine = "apple";
    if (fixture.includes("engine-custom")) this.engine = "custom";
    this.showOptions = this.engine !== "local";
    if (fixture.includes("download-error")) this.downloadError = t("onboarding.download_error");
    this.progress.append(...onboardingSteps.map(() => h("span")));
    this.topControls.append(this.interfaceLanguageControl()); this.topLanguagePicker = this.interfacePicker;
    this.root.append(this.topControls, this.progress,
      this.body, this.message, h("footer", { class: "onboarding-footer" }, h("div", { class: "onboarding-footer-left" }, this.theme, this.back, this.skip, this.count), this.cancelDownload, this.next));
    ctx.onInfoChange(() => {
      const choice = this.goal === "meeting" ? ctx.info.settings.meeting_stt : ctx.info.settings.dictation_stt;
      const signature = JSON.stringify(choice);
      if (this.step === "engine" && signature !== this.speechChoiceSignature) { this.preferRecommendation = false; this.configureGoal(this.goal); this.renderEngineDetail(); }
      this.speechChoiceSignature = signature;
      this.updateCommon(); if (this.step === "permissions") void this.refreshPermissions().catch(this.fail);
    });
    ctx.onFocusChange((focused) => {
      if (this.step === "permissions" && focused) void this.refreshPermissions().catch(this.fail);
      if (this.step === "practice") {
        if (!focused) { if (this.practiceText) this.practiceText.disabled = true; void this.practice?.close().catch(this.fail); }
        else void this.practice?.open().catch(this.fail);
      }
    });
    ctx.onVisibilityChange((visible) => {
      this.poll?.setVisible(visible);
      if (visible && this.step === "practice") {
        if (this.practice) void this.practice.open().catch(this.fail);
        else void this.renderStep().catch(this.fail);
      }
    });
    ctx.beforeLeave(async () => { await this.closePractice(); });
    ctx.onFlush(() => this.persist(this.step));
    ctx.onDispose(() => this.dispose());
    bindLocalization(this.root, () => { this.updateCommon(); this.updateDownload(); this.refreshPracticePresentation?.(); });
  }

  private readonly fail = (error: unknown) => {
    if (this.disposed) return;
    this.lastError = error; this.message.textContent = localizeBackendText(errorText(error)); this.message.hidden = false;
    this.actionBusy = false; this.updateCommon();
  };

  async start(): Promise<void> {
    this.cleanup.push(await listen<{ id: string; downloaded: number; total: number }>("download-progress", (event) => {
      if (event.payload.id !== this.localModelId || this.disposed) return;
      this.downloading = true; this.downloadProgress = event.payload; this.updateDownload(); this.updateCommon();
    }));
    this.cleanup.push(await listen<{ id: string; error: string | null }>("download-done", (event) => {
      this.attachedDownloads.delete(event.payload.id);
      if (event.payload.id !== this.localModelId || this.disposed) return;
      this.downloading = false;
      this.downloadError = event.payload.error && !isCancellation(event.payload.error) ? event.payload.error : null;
      if (event.payload.error) this.continueAfterDownload = null;
      void this.ctx.refresh().then(() => this.loadModels()).then(() => { if (this.step === "engine" && this.engine === "local") this.renderEngineDetail(); this.updateCommon(); }).catch(this.fail);
    }));
    const route = (event: Event) => {
      const step = (event as CustomEvent<unknown>).detail;
      if (step === "practice" || step === "dictation-setup" || step === "dictation-permissions" || step === "resume" || onboardingSteps.includes(step as OnboardingStep)) {
        const intent = onboardingRouteIntent(step, this.ctx.info.settings.onboarding);
        this.preserveProgress = intent.preserveProgress;
        this.permissionRepair = intent.preserveProgress && intent.step === "permissions";
        this.configureGoal(intent.goal);
        if (step === "language" || step === "voice") this.legacyStep = step;
        if (intent.step === "resume" || intent.preserveProgress) void this.resume().catch(this.fail);
        else void this.move(stepIndex(intent.step)).catch(this.fail);
      }
    };
    window.addEventListener("pennel-onboarding-route", route);
    this.cleanup.push(() => window.removeEventListener("pennel-onboarding-route", route));
    if (new URLSearchParams(location.search).get("tab") === "resume" || this.preserveProgress) await this.resume(); else await this.renderStep();
  }

  private async resume() {
    const readiness = await api.onboardingReadiness(this.goal === "meeting" ? "meeting" : "practice");
    this.readiness = readiness;
    await this.move(stepIndex(onboardingResumeStep(readiness, this.ctx.info.settings.onboarding.resume_step, {
      goal: this.goal, preserveProgress: this.preserveProgress, step: this.permissionRepair ? "permissions" : "resume",
    })));
  }

  private async persist(step: OnboardingStep, status?: "skipped" | "completed") {
    if (this.preserveProgress || !onboardingProgressChange(this.ctx.info.settings.onboarding, step, status, this.goal)) return;
    await this.ctx.save((settings) => {
      const next = onboardingProgressChange(settings.onboarding, step, status, this.goal);
      if (next) settings.onboarding = next;
    });
  }

  private async previous() {
    const at = this.flow.indexOf(this.step);
    await this.move(stepIndex(this.flow[Math.max(0, at - 1)]));
  }

  private async move(index: number) {
    if (this.actionBusy || this.disposed) return;
    this.continueAfterDownload = null;
    this.actionBusy = true; this.updateCommon();
    try {
      await this.closePractice(); this.closeVoice();
      await this.persist(onboardingSteps[index]);
      this.index = index; this.message.hidden = true;
      await this.renderStep();
    } finally { this.actionBusy = false; this.updateCommon(); }
  }

  private async closePractice() {
    this.held = false;
    if (this.practiceText) this.practiceText.disabled = true;
    await this.practice?.close(); this.practice = null;
  }

  private closeVoice() {
    this.voicePanel?.dispose(); this.voicePanel = null;
    this.voiceModels?.dispose(); this.voiceModels = null;
  }

  private async leave() {
    this.continueAfterDownload = null;
    this.actionBusy = true; this.updateCommon();
    // The voice step is optional, so skipping it still completes the onboarding.
    const status = onboardingSteps[this.index] === "voice" ? "completed" : "skipped";
    try { await this.closePractice(); this.closeVoice(); await this.persist(onboardingSteps[this.index], status); await api.showWorkspace(workspaceRoute(this.goal === "meeting" ? "meetings" : "dictation")); }
    finally { this.actionBusy = false; this.updateCommon(); }
  }

  private async renderStep() {
    const generation = ++this.renderGeneration;
    this.poll?.dispose(); this.poll = undefined;
    this.root.dataset.step = onboardingSteps[this.index];
    this.body.className = `onboarding-body onboarding-${this.step}`;
    this.interfacePicker = null; this.refreshPracticePresentation = null;
    if (this.step === "language") this.renderLanguage();
    if (this.step === "welcome") this.renderWelcome();
    if (this.step === "engine") { await this.loadModels(); if (generation !== this.renderGeneration || this.disposed) return; this.renderEngine(); }
    if (this.step === "permissions") {
      this.body.replaceChildren(this.title(l(this.goal === "meeting" ? "onboarding.meeting_permissions_title" : "onboarding.permissions_title"), l(this.goal === "meeting" ? "onboarding.meeting_permissions_intro" : "onboarding.permissions_intro")), this.readyRow,
        h("p", { class: "help onboarding-permission-help" }, l(this.goal === "meeting" ? "onboarding.meeting_permissions_later" : "onboarding.permissions_later")));
      this.permissionSignature = ""; await this.refreshPermissions();
      this.poll = new VisibilityPoll(1500, () => this.refreshPermissions(), this.fail);
      this.poll.setVisible(this.ctx.visible, false);
    }
    if (this.step === "practice") { if (this.goal === "meeting") await this.renderMeetingReady(); else await this.renderPractice(); }
    if (this.step === "voice") { await this.renderVoice(generation); if (generation !== this.renderGeneration || this.disposed) return; }
    this.updateCommon();
  }

  private ownedStatus(read: () => string): HTMLElement {
    const node = h("span", {}, localizeBackendText(read()));
    bindLocalization(node, () => { node.textContent = localizeBackendText(read()); });
    return node;
  }

  private title(text: string | LocalizedText, description: string | LocalizedText) {
    return h("div", { class: "onboarding-heading" }, h("h1", {}, text), h("p", { class: "muted" }, description));
  }

  private hotkey(): string {
    return hotkeyLabel(this.ctx.info.settings.hotkey, this.ctx.info.platform);
  }

  private keycap(): HTMLButtonElement {
    return h("button", { class: "keycap", type: "button", "aria-label": l("onboarding.hold_to_dictate") }, icon("globe"), h("span", {}, this.keyLabel()));
  }

  private keyLabel(): string {
    return hotkeyKeyLabel(this.ctx.info.settings.hotkey, this.ctx.info.platform);
  }

  private renderLanguage() {
    const system = this.ctx.info.system_interface_language === "nl" ? "Nederlands" : "English";
    const picker = select<"system" | "en" | "nl">([
      ["system", l("onboarding.interface_system", { language: system })], ["en", "English"], ["nl", "Nederlands"],
    ], this.ctx.info.settings.ui_language ?? "system", (value) => {
      void this.ctx.save((settings) => { settings.ui_language = value; }).catch(this.fail);
    }, "menu");
    picker.setAttribute("aria-label", t("onboarding.interface_language"));
    bindLocalization(picker, () => picker.setAttribute("aria-label", t("onboarding.interface_language")));
    this.interfacePicker = picker;
    this.body.replaceChildren(wordmark(), this.title(l("onboarding.interface_title"), l("onboarding.interface_intro")),
      h("div", { class: "preference-row onboarding-interface-language" }, h("span", {}, l("onboarding.interface_language")), picker),
      h("p", { class: "help" }, l("onboarding.interface_speech_note")));
  }

  private renderWelcome() {
    this.goalButtons.clear();
    this.body.replaceChildren(wordmark(),
      h("h1", {}, l("onboarding.welcome_title")), h("p", { class: "onboarding-intro" }, l("onboarding.welcome_intro")),
      h("div", { class: "onboarding-goals", role: "group", "aria-label": l("onboarding.first_task") },
        this.goalAction("dictation", l("onboarding.try_dictation"), l("onboarding.dictation_goal_help")),
        this.goalAction("meeting", l("onboarding.record_meeting"), l("onboarding.meeting_goal_help"))));
  }

  private goalAction(goal: OnboardingGoal, label: LocalizedText, description: LocalizedText): HTMLElement {
    const button = h("button", { class: "onboarding-goal", "data-onboarding-goal": goal, "aria-pressed": String(goal === this.goal), "aria-label": label, onclick: () => {
      void this.chooseGoal(goal).catch(this.fail);
    } }, h("span", { class: "onboarding-goal-icon" }, icon(goal === "meeting" ? "notebook" : "mic")),
      h("span", { class: "onboarding-goal-check", "aria-hidden": true }, icon("check")),
      h("span", { class: "onboarding-goal-copy" }, h("strong", {}, label), h("span", { class: "help" }, description)));
    this.goalButtons.set(goal, button); return button;
  }

  private interfaceLanguageControl(): HTMLElement {
    const system = this.ctx.info.system_interface_language === "nl" ? "Nederlands" : "English";
    const picker = select<"system" | "en" | "nl">([
      ["system", l("onboarding.interface_system", { language: system })], ["en", "English"], ["nl", "Nederlands"],
    ], this.ctx.info.settings.ui_language ?? "system", (value) => {
      void this.ctx.save((settings) => { settings.ui_language = value; }).catch(this.fail);
    }, "menu");
    picker.setAttribute("aria-label", t("onboarding.interface_language"));
    bindLocalization(picker, () => picker.setAttribute("aria-label", t("onboarding.interface_language")));
    this.interfacePicker = picker;
    return h("div", { class: "onboarding-interface-language" }, icon("globe"), picker);
  }

  private async chooseGoal(goal: OnboardingGoal) {
    if (this.actionBusy) return;
    this.legacyStep = undefined;
    this.configureGoal(goal);
    this.updateCommon();
  }

  private configureGoal(goal: OnboardingGoal) {
    this.continueAfterDownload = null;
    this.goal = goal;
    const settings = this.ctx.info.settings;
    const choice = goal === "meeting" ? settings.meeting_stt : settings.dictation_stt;
    const account = settings.providers.find((provider) => provider.id === choice.provider_id);
    this.engine = initialEngine(account?.kind, this.ctx.info.kinds.find((kind) => kind.kind === "local")?.availability.status !== "unsupported");
    this.localModelId = account?.kind === "local" && !this.preferRecommendation ? choice.model : "";
    this.providerChoice = account && this.customAccounts().some((item) => item.id === account.id) ? account.id : this.customAccounts()[0]?.id ?? `new:${this.customKinds()[0]?.kind ?? "open_ai"}`;
    this.providerModel = this.engine === "custom" ? choice.model : this.customKind()?.default_stt_model ?? "";
    this.providerBaseUrl = this.customAccount()?.base_url ?? this.customKind()?.default_base_url ?? "";
    this.showOptions = this.engine !== "local";
    this.downloading = false;
    this.downloadError = null;
    this.downloadProgress = { downloaded: 0, total: 0 };
  }

  private async loadModels() {
    const generation = ++this.modelsGeneration;
    const [models, compatible] = await Promise.all([api.localModels(this.language || null, false), api.localModels(this.language || null, true)]);
    if (this.disposed || generation !== this.modelsGeneration) return;
    this.models = models;
    this.recommendedModelId = recommendedSpeechModel(compatible)?.id ?? null;
    this.localModelId = defaultModelId(models, this.localModelId, compatible);
    await this.attachDownload();
  }

  private async attachDownload() {
    const id = this.localModelId;
    if (!this.ctx.info.active_model_downloads?.includes(id) || this.attachedDownloads.has(id)) return;
    this.attachedDownloads.add(id);
    const model = this.localModel();
    this.downloading = true;
    this.downloadProgress = { downloaded: model?.state.kind === "partial" ? model.state.bytes : 0, total: model?.size_bytes ?? 0 };
    void api.joinModelDownload(id).then(async (joined) => {
      if (this.disposed || joined) return;
      this.attachedDownloads.delete(id);
      if (id === this.localModelId) this.downloading = false;
      await this.ctx.refresh();
      await this.loadModels();
      if (this.step === "engine" && this.engine === "local") this.renderEngineDetail();
      this.updateCommon();
    }).catch((error) => {
      this.attachedDownloads.delete(id);
      if (id === this.localModelId) this.downloading = false;
      if (isCancellation(error)) { this.updateCommon(); return; }
      this.fail(error);
    });
  }

  private async chooseModel(id: string) {
    if (id === this.localModelId) return;
    this.preferRecommendation = false;
    this.continueAfterDownload = null;
    const previous = this.localModelId;
    if (this.downloading) await api.cancelDownload(previous).catch(() => undefined);
    this.attachedDownloads.delete(previous);
    this.localModelId = id; this.downloading = false; this.downloadError = null; this.downloadProgress = { downloaded: 0, total: 0 };
    await this.attachDownload();
    if (this.disposed) return;
    this.renderEngineDetail(); this.updateCommon();
  }

  private localModel(): LocalModel | undefined { return this.models.find((model) => model.id === this.localModelId); }
  private customKinds(): KindInfo[] { return this.ctx.info.kinds.filter((kind) => kind.stt && !["local", "apple_speech", "pennel_cloud"].includes(kind.kind)); }
  private customAccounts(): ProviderAccount[] { return this.ctx.info.settings.providers.filter((account) => this.customKinds().some((kind) => kind.kind === account.kind)); }
  private customAccount(): ProviderAccount | undefined { return this.customAccounts().find((account) => account.id === this.providerChoice); }
  private customKind(): KindInfo | undefined {
    const kind = this.providerChoice.startsWith("new:") ? this.providerChoice.slice(4) : this.customAccount()?.kind;
    return this.customKinds().find((item) => item.kind === kind);
  }

  private renderEngine() {
    const language = select(languageOptions(), this.language, (value) => {
      this.language = value;
      void this.loadModels().then(() => { if (this.engine === "local") this.renderEngineDetail(); this.updateCommon(); }).catch(this.fail);
    }, "menu");
    language.setAttribute("aria-label", t("onboarding.language"));
    bindLocalization(language, () => language.setAttribute("aria-label", t("onboarding.language")));
    const choices = h("div", { class: "engine-choices", role: "group", "aria-label": l("onboarding.speech_recognition") });
    const remoteChoices: HTMLElement[] = [];
    this.localButtons.clear();
    for (const [value, label] of [["local", l("onboarding.local")], ["apple", l("onboarding.apple_dictation")], ["cloud", l("settings.setup.source.cloud")], ["custom", l("onboarding.own_provider")]] as [Engine, string | LocalizedText][]) {
      const button = h("button", { class: "engine-choice", "aria-controls": "onboarding-engine-detail", onclick: () => { void this.chooseEngine(value).catch(this.fail); } }, label);
      button.addEventListener("keydown", (event) => {
        const order: Engine[] = ["local", "apple", "cloud", "custom"];
        const index = order.indexOf(value);
        const next = event.key === "ArrowRight" ? (index + 1) % order.length : event.key === "ArrowLeft" ? (index + order.length - 1) % order.length : event.key === "Home" ? 0 : event.key === "End" ? order.length - 1 : -1;
        if (next >= 0) { event.preventDefault(); const target = order[next]; if (!this.localButtons.get(target)?.disabled) void this.chooseEngine(target).then(() => this.localButtons.get(target)?.focus()).catch(this.fail); }
      });
      if (value !== "custom") bindLocalization(button, () => { button.title = this.ctx.info.kinds.find((kind) => kind.kind === (value === "local" ? "local" : value === "cloud" ? "pennel_cloud" : "apple_speech"))?.availability.reason ?? ""; });
      this.localButtons.set(value, button);
      if (value === "cloud" || value === "custom") {
        button.classList.add("onboarding-option-row"); bindLocalization(button, () => button.setAttribute("aria-label", String(label))); button.setAttribute("aria-label", String(label));
        button.replaceChildren(h("span", { class: "grow" }, h("strong", {}, label), h("span", { class: "help" }, l(value === "cloud" ? "settings.setup.source.cloud_help" : "onboarding.other_provider_help"))), icon("chevronRight"));
        remoteChoices.push(button);
      } else choices.append(button);
    }
    const localChoices = h("details", { class: "onboarding-local-options" },
      h("summary", {}, h("span", {}, h("strong", {}, l("onboarding.other_local")), h("span", { class: "help" }, l("onboarding.other_local_help"))), icon("chevronRight")), choices, this.advancedModel);
    const alternatives = h("details", { class: "onboarding-other-options", open: this.showOptions }, h("summary", {}, l("onboarding.other_options")),
      h("div", { class: "onboarding-alternative-rows" }, localChoices, ...remoteChoices),
      h("div", { class: "engine-language" }, h("span", {}, l("onboarding.language")), language),
      ...this.ctx.info.kinds.filter((kind) => ["local", "apple_speech"].includes(kind.kind) && kind.availability.status === "unsupported" && kind.availability.reason)
        .map((kind) => h("p", { class: "help" }, this.ownedStatus(() => this.ctx.info.kinds.find((item) => item.kind === kind.kind)?.availability.reason ?? ""))));
    alternatives.addEventListener("toggle", () => { this.showOptions = alternatives.open; });
    this.body.replaceChildren(this.title(l("onboarding.engine_title"), l("onboarding.engine_intro")),
      this.engineDetail, alternatives);
    this.renderEngineDetail();
  }

  private async chooseEngine(engine: Engine) {
    if (engine !== this.engine) this.continueAfterDownload = null;
    this.engine = engine; this.message.hidden = true; this.renderEngineDetail(); this.updateCommon();
  }

  private async cancelCurrentDownload() {
    const id = this.localModelId;
    this.continueAfterDownload = null;
    await api.cancelDownload(id);
    if (!this.disposed && this.localModelId === id) { this.downloading = false; this.updateDownload(); this.updateCommon(); }
  }

  private renderEngineDetail() {
    this.engineDetail.dataset.engine = this.engine;
    this.advancedModel.hidden = this.engine !== "local";
    this.localButtons.forEach((button, engine) => button.setAttribute("aria-pressed", String(engine === this.engine)));
    const apple = this.ctx.info.kinds.find((kind) => kind.kind === "apple_speech");
    const appleButton = this.localButtons.get("apple");
    if (appleButton) { appleButton.disabled = apple?.availability.status === "unsupported"; if (apple?.availability.reason) appleButton.title = apple.availability.reason; }
    const local = this.ctx.info.kinds.find((kind) => kind.kind === "local");
    const localButton = this.localButtons.get("local");
    if (localButton) { localButton.disabled = local?.availability.status === "unsupported"; if (local?.availability.reason) localButton.title = local.availability.reason; }
    if (this.engine === "local") {
      const model = this.localModel();
      const downloaded = model?.state.kind === "downloaded";
      const signature = JSON.stringify(this.models.map(item => [item.id, item.name, item.size_bytes, item.recommended]));
      if (!this.modelPicker || signature !== this.modelPickerSignature) {
        const focused = this.modelPicker === document.activeElement;
        const picker = select(this.models.map((item): [string, UiText] => [item.id, liveText(() => modelOptionLabel(item, formatBytes))]), this.localModelId, (value) => { void this.chooseModel(value).catch(this.fail); }, "menu");
        picker.setAttribute("aria-label", t("onboarding.speech_model"));
        bindLocalization(picker, () => picker.setAttribute("aria-label", t("onboarding.speech_model")));
        this.advancedModel.replaceChildren(h("label", { class: "preference-row" }, l("onboarding.speech_model"), picker));
        this.modelPicker = picker; this.modelPickerSignature = signature;
        if (focused) picker.focus({ preventScroll: true });
      }
      const picker = this.modelPicker;
      picker.value = this.localModelId;
      picker.disabled = this.models.length === 0;
      const name = h("div", {}, h("div", { class: "engine-model-heading" }, h("strong", {}, model?.name ?? l("onboarding.recommended_setup")), model?.id === this.recommendedModelId ? h("span", { class: "onboarding-recommended-badge" }, l("onboarding.recommended")) : null),
        h("small", {}, model ? (downloaded ? liveText(() => formatBytes(model.size_bytes)) : l("onboarding.model_download_size", () => ({ size: formatBytes(model.size_bytes) }))) : l("onboarding.no_local_model")));
      const status = h("span", { class: "engine-model-status", "data-download-state": true });
      const track = h("progress", { class: "engine-download-progress", "aria-label": l("onboarding.download_model"), hidden: !this.downloading });
      const caption = h("div", { class: "engine-download-caption", hidden: !this.downloading }, h("span", { "data-download-caption": true }));
      this.engineDetail.replaceChildren(h("div", { class: "engine-model" }, icon("mic"), name, status),
        h("p", { class: "onboarding-destination-chip" }, icon("laptop"), l("dictation.processing.local")),
        ...(model ? [h("p", { class: "help" }, this.ownedStatus(() => model.description))] : []),
        ...(local?.availability.reason ? [h("p", { class: "help" }, this.ownedStatus(() => this.ctx.info.kinds.find(kind => kind.kind === "local")?.availability.reason ?? ""))] : []),
        h("p", { class: "help" }, l(this.goal === "meeting" ? "onboarding.meeting_speech_only" : "onboarding.practice_speech_only")),
        track, caption,
        ...(this.downloadError ? [h("div", { class: "engine-error", role: "alert" }, h("p", {}, this.ownedStatus(() => this.downloadError ?? "")),
          h("button", { class: "text-button", disabled: apple?.availability.status === "unsupported", onclick: () => { void this.chooseEngine("apple"); } }, l("onboarding.use_apple")))] : []));
      this.updateDownload();
    } else if (this.engine === "apple") {
      this.engineDetail.replaceChildren(h("h2", {}, l("onboarding.apple_title")),
        h("p", { class: "engine-description" }, l("onboarding.apple_intro")),
        h("p", { class: "help" }, l(this.goal === "meeting" ? "onboarding.meeting_speech_only" : "onboarding.practice_speech_only")),
        h("div", { class: "engine-model" }, icon("audioLines"), h("div", {}, h("strong", {}, l("onboarding.apple_dictation")), h("small", {}, l("onboarding.apple_available")))),
        ...(apple?.availability.reason ? [h("p", { class: "help" }, this.ownedStatus(() => this.ctx.info.kinds.find((kind) => kind.kind === "apple_speech")?.availability.reason ?? ""))] : []));
    } else if (this.engine === "cloud") {
      this.engineDetail.replaceChildren(h("h2", {}, l("settings.setup.source.cloud")), h("p", { class: "engine-description" }, l("settings.setup.source.cloud_help")),
        h("p", { class: "help" }, l("onboarding.cloud_setup_help")),
        controlButton({ class: "btn", "data-onboarding-cloud-setup": true, onclick: () => this.ctx.openSetup(this.goal === "meeting" ? "meeting_stt" : "dictation_stt") }, l("onboarding.cloud_setup")));
    } else this.renderProvider();
  }

  private renderProvider() {
    const options: [string, UiText][] = [
      ...this.customAccounts().map((account): [string, string] => [account.id, account.name]),
      ...this.customKinds().filter((kind) => !this.customAccounts().some((account) => account.kind === kind.kind)).map((kind): [string, UiText] => [`new:${kind.kind}`, liveText(() => this.ctx.info.kinds.find((current) => current.kind === kind.kind)?.label ?? kind.label)]),
    ];
    const picker = select(options, this.providerChoice, (value) => {
      this.providerChoice = value;
      const account = this.customAccount(); const kind = this.customKind();
      const choice = this.goal === "meeting" ? this.ctx.info.settings.meeting_stt : this.ctx.info.settings.dictation_stt;
      this.providerModel = account?.id === choice.provider_id ? choice.model : kind?.default_stt_model ?? "";
      this.providerBaseUrl = account?.base_url ?? kind?.default_base_url ?? "";
      this.renderProvider(); this.updateCommon();
    }, "menu");
    picker.setAttribute("aria-label", t("onboarding.own_provider"));
    bindLocalization(picker, () => picker.setAttribute("aria-label", t("onboarding.own_provider")));
    const kind = this.customKind(); const account = this.customAccount();
    const keySet = !!account && this.ctx.info.keys[account.id];
    const key = textField({ type: "password", "data-onboarding-key": true, "aria-label": l("onboarding.api_key"), autocomplete: "off", placeholder: keySet ? l("onboarding.key_saved") : l("onboarding.paste_key") });
    const model = textField({ type: "text", value: this.providerModel, "aria-label": l("onboarding.model"), placeholder: l("onboarding.model_name"), oninput: (event) => { this.providerModel = (event.target as HTMLInputElement).value; } });
    const url = textField({ type: "text", value: this.providerBaseUrl, "aria-label": l("onboarding.server_address"), placeholder: "https://…", oninput: (event) => { this.providerBaseUrl = (event.target as HTMLInputElement).value; } });
    for (const option of picker.options) {
      const optionKind = option.value.startsWith("new:") ? this.customKinds().find((item) => item.kind === option.value.slice(4)) : this.customKinds().find((item) => item.kind === this.customAccounts().find((item) => item.id === option.value)?.kind);
      option.disabled = optionKind?.availability.status === "unsupported";
    }
    this.engineDetail.replaceChildren(h("h2", {}, l("onboarding.provider_title")), h("p", { class: "engine-description" }, l("onboarding.provider_intro")),
      h("div", { class: "onboarding-provider-fields" }, h("label", { class: "preference-row" }, l("onboarding.provider"), picker),
        kind?.requires_key ? h("label", { class: "preference-row" }, l("onboarding.api_key"), key) : null,
        h("label", { class: "preference-row" }, l("onboarding.model"), model),
        ["custom", "ollama", "lm_studio"].includes(kind?.kind ?? "") ? h("label", { class: "preference-row" }, l("onboarding.server_address"), url) : null),
      h("p", { class: "help provider-key-note" }, l("onboarding.key_storage_note")));
  }

  private updateDownload() {
    if (this.engine !== "local" || this.step !== "engine") return;
    const progress = this.engineDetail.querySelector<HTMLProgressElement>("progress");
    if (progress) { progress.hidden = !this.downloading; progress.max = this.downloadProgress.total || this.localModel()?.size_bytes || 1; progress.value = this.downloadProgress.downloaded; }
    const caption = this.engineDetail.querySelector<HTMLElement>(".engine-download-caption");
    if (caption) caption.hidden = !this.downloading;
    const label = this.engineDetail.querySelector("[data-download-caption]");
    if (label) label.textContent = this.downloadProgress.total ? `${Math.min(100, Math.round(this.downloadProgress.downloaded / this.downloadProgress.total * 100))}%` : t("onboarding.preparing_download");
    const status = this.engineDetail.querySelector("[data-download-state]");
    if (status) status.replaceChildren(...(this.localModel()?.state.kind === "downloaded" ? [icon("check"), document.createTextNode(t("onboarding.downloaded"))] : []));
  }

  private async refreshPermissions() {
    const [permissions, readiness] = await Promise.all([api.permissions(), api.onboardingReadiness(this.goal === "meeting" ? "meeting" : "practice")]);
    if (this.disposed || this.step !== "permissions") return;
    this.permissions = permissions; this.readiness = readiness;
    const signature = JSON.stringify([permissions, readiness]);
    if (signature !== this.permissionSignature) {
      this.permissionSignature = signature;
      const kinds: [PermissionInfo["kind"], string | LocalizedText, string | LocalizedText][] = [["microphone", l("onboarding.microphone"), l("onboarding.microphone_description")]];
      if (this.goal === "meeting") kinds.push(["system_audio", l("onboarding.system_audio"), l("onboarding.system_audio_description")]);
      else kinds.push(["accessibility", l("onboarding.accessibility"), l("onboarding.accessibility_description")]);
      if (this.engine === "apple") kinds.push(["speech", l("onboarding.speech_recognition"), l("onboarding.apple_permission_description")]);
      this.readyRow.replaceChildren(...kinds.map(([kind, label, description]) => this.permissionRow(kind, label, description)),
        ...(this.goal === "dictation" && !readiness.can_use_shortcut ? [h("p", { class: "help" }, this.ctx.info.capabilities.global_hotkey.reason ? this.ownedStatus(() => this.ctx.info.capabilities.global_hotkey.reason ?? "") : l("onboarding.manual_shortcut_help"))] : []));
    }
    this.updateCommon();
  }

  private permissionRow(kind: PermissionInfo["kind"], label: string | LocalizedText, description: string | LocalizedText): HTMLElement {
    const permission = this.permissions.find((item) => item.kind === kind);
    const granted = permission?.status === "granted" || permission?.status === "not_required";
    const unsupported = permission?.status === "unsupported";
    const unknown = permission?.status === "unknown";
    const status = h("span", { class: "permission-status" }, ...(granted ? [icon("check"), h("span", {}, l("onboarding.permission_ready"))] : []));
    const button = controlButton({ class: "btn", disabled: unsupported || (this.ctx.info.platform === "linux" && kind === "microphone"), onclick: async () => {
      button.disabled = true;
      try { await api.requestPermission(kind); await this.refreshPermissions(); }
      catch (error) { this.fail(error); button.disabled = false; }
    } }, permission?.status === "denied" || kind === "accessibility" || this.ctx.info.platform === "windows" ? l("onboarding.open_settings") : l("onboarding.grant_access"));
    return h("div", { class: "permission-row" }, icon(kind === "microphone" ? "mic" : kind === "speech" || kind === "system_audio" ? "audioLines" : "keyboard"),
      h("div", { class: "grow" }, h("strong", {}, label), h("p", { class: "help" }, description),
        granted ? null : unsupported ? h("p", { class: "help" }, l("onboarding.permission_unsupported"))
          : unknown && this.ctx.info.platform === "linux" ? h("p", { class: "help" }, l("onboarding.microphone_checked_later")) : button,
        kind === "microphone" && unknown && this.ctx.info.platform === "windows" ? h("p", { class: "help" }, l("onboarding.microphone_checked_later")) : null), status);
  }

  private async renderPractice() {
    this.practiceDone = false;
    this.practiceText = h("textarea", { class: "practice-text", "aria-label": l("onboarding.first_dictation"), placeholder: l("onboarding.practice_placeholder"), spellcheck: true, disabled: true });
    this.practiceStatus = h("p", { class: "practice-status help", role: "status", "aria-live": "polite" });
    this.practiceKey = this.keycap();
    this.practiceButton = h("button", { class: "text-button", onclick: () => { void this.practice?.toggle().catch(this.fail); } }, l("onboarding.start_dictation"));
    const apply = (snapshot: DictationSessionSnapshot) => {
      if (this.disposed || this.step !== "practice" || !this.practiceText || !this.practiceStatus || !this.practiceButton) return;
      const busy = snapshot.phase === "listening" || snapshot.phase === "working";
      this.practiceText.disabled = busy;
      if (snapshot.phase === "done") this.practiceText.value = snapshot.text;
      this.refreshPracticePresentation = () => {
        if (!this.practiceStatus || !this.practiceButton || this.step !== "practice") return;
        this.practiceStatus.textContent = snapshot.error ? localizeBackendText(snapshot.error) : snapshot.phase === "listening" ? t("onboarding.listening_release") : snapshot.phase === "working" ? t("onboarding.writing_words") : snapshot.phase === "done" ? t("onboarding.first_dictation_done") : t("onboarding.practice_gesture_help", { shortcut: this.hotkey() });
        this.practiceButton.textContent = snapshot.phase === "listening" ? t("onboarding.stop_dictation") : snapshot.phase === "working" ? t("onboarding.processing_text") : t("onboarding.start_dictation");
        if (this.practiceDone) {
          this.body.querySelector("h1")!.textContent = t("onboarding.practice_success_title");
          const introduction = this.body.querySelector(".onboarding-heading p");
          if (introduction) introduction.textContent = t("onboarding.practice_success_intro");
        }
      };
      this.refreshPracticePresentation();
      this.practiceStatus.classList.toggle("error", !!snapshot.error);
      this.practiceKey?.setAttribute("data-pressed", String(snapshot.phase === "listening"));
      this.practiceButton.disabled = snapshot.phase === "working";
      if (snapshot.phase === "done" && snapshot.text.trim() && !this.practiceDone) {
        this.practiceDone = true;
        this.body.querySelector("h1")!.textContent = t("onboarding.practice_success_title");
        const introduction = this.body.querySelector(".onboarding-heading p");
        if (introduction) introduction.textContent = t("onboarding.practice_success_intro");
        void this.persist("practice", "completed").catch(this.fail);
      }
      this.updateCommon();
    };
    this.practice = new OwnedDictationSession("practice", apply);
    const press = () => {
      this.held = true;
      this.practiceStart = (async () => { await this.practice?.open(); if (this.practice?.state.snapshot?.phase !== "listening") await this.practice?.toggle(); })().catch(this.fail);
    };
    const release = () => {
      if (!this.held) return;
      this.held = false;
      void this.practiceStart.then(async () => { if (this.practice?.state.snapshot?.phase === "listening") await this.practice.toggle(); }).catch(this.fail);
    };
    this.practiceKey.addEventListener("pointerdown", (event) => { event.preventDefault(); this.practiceKey?.setPointerCapture(event.pointerId); press(); });
    this.practiceKey.addEventListener("pointerup", release);
    this.practiceKey.addEventListener("pointercancel", () => { this.held = false; void this.practice?.cancel().catch(this.fail); });
    this.practiceKey.addEventListener("keydown", (event) => { if ([" ", "Enter"].includes(event.key)) { event.preventDefault(); if (!event.repeat) press(); } });
    this.practiceKey.addEventListener("keyup", (event) => { if ([" ", "Enter"].includes(event.key)) { event.preventDefault(); release(); } });
    this.practiceKey.addEventListener("click", (event) => { if (event.detail === 0 && !this.held) void this.practice?.toggle().catch(this.fail); });
    this.body.replaceChildren(this.title(l("onboarding.practice_title"), l("onboarding.practice_gesture", { shortcut: this.hotkey() })),
      h("div", { class: "practice-gesture" }, this.practiceKey, h("div", {}, h("p", { class: "practice-phrase" }, liveText(() => `“${phrases[this.language] ?? phrases[getInterfaceLanguage()]}”`)), h("p", { class: "help" }, l("onboarding.practice_free_speech")))),
      this.practiceText, this.practiceStatus, h("div", { class: "row practice-help" }, this.practiceButton,
        h("details", {}, h("summary", {}, l("onboarding.shortcut_not_working", { shortcut: this.hotkey() })), h("p", { class: "help" }, this.ctx.info.platform === "macos"
          ? l("onboarding.mac_shortcut_help")
          : this.ctx.info.capabilities.global_hotkey.reason ? this.ownedStatus(() => this.ctx.info.capabilities.global_hotkey.reason ?? "") : l("onboarding.manual_practice_help")))));
    try { await this.practice.open(); } catch (error) { this.fail(error); }
  }

  private async renderMeetingReady() {
    this.readiness = await api.onboardingReadiness("meeting");
    if (this.disposed || this.step !== "practice") return;
    this.body.replaceChildren(this.title(l("onboarding.meeting_ready_title"), l("onboarding.meeting_ready_intro")),
      h("div", { class: "onboarding-meeting-ready" }, icon("notebook"), h("h2", {}, l("onboarding.meeting_not_recording")),
        h("p", { class: "help" }, l("onboarding.meeting_ready_help"))),
      ...(!this.readiness.can_record ? [h("p", { class: "error", role: "status" }, this.ownedStatus(() => this.readiness?.reason ?? "")),
        controlButton({ class: "btn", onclick: () => { void this.resume().catch(this.fail); } }, l("onboarding.check_setup"))] : []));
  }

  private async renderVoice(generation: number) {
    const setup = await api.speakerSetup();
    if (generation !== this.renderGeneration || this.disposed) return;
    const mode = this.ctx.info.settings.speaker_recognition === "cloud" ? "cloud" : "local";
    const detail = h("div", { class: "voice-step" });
    const models = new SpeakerModels(setup.model_ids, () => draw());
    const download = speakerDownload(models);
    let shown = "";
    const draw = () => {
      if (this.disposed || this.voiceModels !== models) return;
      download.update(); this.updateCommon();
      const needsModels = mode === "local" && setup.local_available && !models.ready;
      const blocker = needsModels ? null : enrollmentBlocker(setup, mode, models);
      const view = needsModels ? "download" : blocker ? "blocked" : "record";
      if (view === shown) return;
      shown = view;
      if (view === "download") {
        detail.replaceChildren(h("section", { class: "onboarding-engine-detail" }, h("h2", {}, l("onboarding.voice_models_title")),
          h("p", { class: "engine-description" }, l("onboarding.voice_models_intro")),
          download.element));
      } else if (view === "blocked") {
        detail.replaceChildren(h("section", { class: "onboarding-engine-detail" }, h("h2", {}, mode === "cloud" ? l("onboarding.voice_cloud_title") : l("onboarding.voice_unavailable_title")),
          h("p", { class: "engine-description" }, mode === "cloud" ? l("onboarding.voice_cloud_intro") : this.ownedStatus(() => enrollmentBlocker(setup, mode, models) ?? "")),
          h("p", { class: "help" }, l("onboarding.voice_later"))));
      } else {
        this.voicePanel ??= new EnrollmentPanel(setup.enrollment_text, () => {}, () => this.updateCommon());
        detail.replaceChildren(this.voicePanel.element,
          ...setup.profile ? [h("p", { class: "help voice-step-note" }, l("onboarding.voice_update_note", () => ({ profile: trainedLabel(setup.profile!) })))] : []);
      }
    };
    this.voiceModels = models;
    this.body.replaceChildren(this.title(l("onboarding.voice_title"), enrollmentIntro()), detail);
    await models.start(this.ctx.info.active_model_downloads ?? []);
  }

  private async commitEngine() {
    let account: ProviderAccount | undefined;
    let model: string;
    if (this.engine === "local") {
      account = this.ctx.info.settings.providers.find((item) => item.kind === "local"); model = this.localModelId;
      if ((await api.modelState(model)).kind !== "downloaded") { await this.loadModels(); this.renderEngineDetail(); throw validationError("onboarding.model_download_required"); }
    }
    else if (this.engine === "apple") { account = this.ctx.info.settings.providers.find((item) => item.kind === "apple_speech"); model = this.ctx.info.kinds.find((item) => item.kind === "apple_speech")?.default_stt_model ?? "dictation"; }
    else if (this.engine === "cloud") {
      await api.cloudAccount();
      account = this.ctx.info.settings.providers.find((item) => item.kind === "pennel_cloud");
      const current = this.goal === "meeting" ? this.ctx.info.settings.meeting_stt : this.ctx.info.settings.dictation_stt;
      model = current.provider_id === account?.id ? current.model : this.ctx.info.kinds.find((item) => item.kind === "pennel_cloud")?.default_stt_model ?? "auto";
    }
    else { account = this.customAccount(); model = this.providerModel.trim(); }
    const kind: ProviderKind = this.engine === "local" ? "local" : this.engine === "apple" ? "apple_speech" : this.engine === "cloud" ? "pennel_cloud" : this.customKind()?.kind ?? "open_ai";
    if (!model) throw validationError("onboarding.model_required");
    const infoKind = this.ctx.info.kinds.find((item) => item.kind === kind);
    if (this.engine === "custom" && ["custom", "ollama", "lm_studio"].includes(kind)) {
      try { if (!["http:", "https:"].includes(new URL(this.providerBaseUrl.trim()).protocol)) throw new Error(); }
      catch { throw validationError("onboarding.server_required"); }
    }
    const key = this.body.querySelector<HTMLInputElement>("[data-onboarding-key]")?.value.trim() ?? "";
    if (infoKind?.requires_key && !key && (!account || !this.ctx.info.keys[account.id])) throw validationError("onboarding.key_required");
    if (!account) {
      const next = await api.addProvider(kind); this.ctx.update(next);
      account = [...next.settings.providers].reverse().find((item) => item.kind === kind);
    }
    if (!account) throw validationError("onboarding.provider_add_failed");
    const id = account.id;
    if (key) this.ctx.update(await api.setKey(id, key));
    await this.ctx.save((settings) => {
      if (this.engine === "custom") {
        const provider = settings.providers.find((item) => item.id === id);
        if (provider && this.providerBaseUrl.trim()) provider.base_url = this.providerBaseUrl.trim();
      }
      if (this.goal === "meeting") settings.meeting_stt = { provider_id: id, model };
      else settings.dictation_stt = { provider_id: id, model };
      settings.language = this.language || null;
    });
  }

  private async advance() {
    if (this.actionBusy || this.disposed) return;
    this.actionBusy = true; this.message.hidden = true; this.updateCommon();
    try {
      if (this.step === "engine") {
        const model = this.localModel();
        if (this.engine === "local" && model?.state.kind !== "downloaded") {
          if (!model || this.downloading) return;
          this.continueAfterDownload = model.id;
          this.downloadError = null; this.downloading = true;
          this.downloadProgress = { downloaded: model.state.kind === "partial" ? model.state.bytes : 0, total: model.size_bytes };
          this.renderEngineDetail(); this.updateCommon();
          try { await api.downloadModel(model.id); } catch (error) { this.continueAfterDownload = null; this.downloading = false; this.downloadError = errorText(error); this.renderEngineDetail(); throw error; }
          return;
        }
        await this.commitEngine();
      }
      if (this.step === "practice") {
        if (this.goal === "meeting") {
          this.readiness = await api.onboardingReadiness("meeting");
          if (!this.readiness.can_record) { await this.renderMeetingReady(); return; }
          await this.persist("practice", "completed");
          await api.showWorkspace(workspaceRoute("recording"));
          return;
        }
        if (!this.practiceDone) return;
        if (!this.flow.includes("voice")) {
          await this.closePractice(); await this.persist("practice", "completed");
          await api.showWorkspace(workspaceRoute("dictation"));
          return;
        }
      }
      if (this.step === "voice") {
        if (this.voicePanel?.busy) return;
        this.closeVoice(); await this.persist("voice", "completed"); await api.showWorkspace(workspaceRoute(this.goal === "meeting" ? "meetings" : "dictation")); return;
      }
      const next = stepIndex(this.flow[this.flow.indexOf(this.step) + 1]);
      await this.closePractice(); await this.persist(onboardingSteps[next]); this.index = next; await this.renderStep();
    } finally { this.actionBusy = false; this.updateCommon(); }
  }

  private updateCommon() {
    if (this.disposed) return;
    if (this.interfacePicker) this.interfacePicker.value = this.ctx.info.settings.ui_language ?? "system";
    if (this.topLanguagePicker) this.topLanguagePicker.value = this.ctx.info.settings.ui_language ?? "system";
    this.topControls.hidden = this.step === "language";
    this.goalButtons.forEach((button, goal) => button.setAttribute("aria-pressed", String(goal === this.goal)));
    if (!this.message.hidden && this.lastError) this.message.textContent = localizeBackendText(errorText(this.lastError));
    this.body.inert = this.actionBusy;
    const current = Math.max(0, this.flow.indexOf(this.step));
    this.progress.hidden = this.step === "language" || this.step === "welcome";
    if (this.progress.childElementCount !== this.flow.length) this.progress.replaceChildren(...this.flow.map(() => h("span")));
    this.progress.setAttribute("aria-label", t("onboarding.step_accessibility", { current: current + 1, total: this.flow.length }));
    [...this.progress.children].forEach((segment, index) => segment.setAttribute("data-active", String(index <= current)));
    this.back.hidden = current === 0; this.back.disabled = this.actionBusy;
    this.skip.textContent = this.step === "practice" ? t("onboarding.practice_later") : this.step === "voice" ? t("onboarding.skip") : t("onboarding.setup_later");
    this.skip.disabled = this.actionBusy || !!this.voicePanel?.busy;
    this.count.textContent = t("onboarding.step_count", { current: current + 1, total: this.flow.length });
    this.count.hidden = false;
    this.next.hidden = false;
    this.cancelDownload.hidden = this.step !== "engine" || !this.downloading;
    const dark = this.ctx.info.settings.appearance === "dark" || (this.ctx.info.settings.appearance === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    this.theme.replaceChildren(icon(dark ? "sun" : "moon"));
    let label = this.step === "voice" ? t("onboarding.done") : t("onboarding.continue");
    let disabled = this.actionBusy;
    if (this.step === "engine" && this.engine === "local") {
      const model = this.localModel(); disabled ||= !model || this.downloading;
      if (model?.state.kind !== "downloaded") label = this.downloading ? this.downloadProgress.total ? t("onboarding.downloading_progress", { percent: Math.max(0, Math.min(100, Math.round(this.downloadProgress.downloaded / this.downloadProgress.total * 100))) }) : t("onboarding.preparing_download") : this.downloadError ? t("onboarding.retry") : t("onboarding.download_continue");
    }
    if (this.step === "engine" && this.engine === "custom") { label = t("onboarding.use_provider"); disabled ||= !this.customKind(); }
    if (this.step === "engine" && this.engine === "cloud") {
      const choice = this.goal === "meeting" ? this.ctx.info.settings.meeting_stt : this.ctx.info.settings.dictation_stt;
      const configured = this.ctx.info.settings.providers.some((item) => item.kind === "pennel_cloud" && item.id === choice.provider_id);
      disabled ||= !configured;
    }
    if (this.step === "engine" && this.engine === "apple") {
      const availability = this.ctx.info.kinds.find((kind) => kind.kind === "apple_speech")?.availability;
      disabled ||= !availability || ["unsupported", "unavailable"].includes(availability.status);
    }
    if (this.step === "permissions") {
      const manualAllowed = manualPracticeAllowed(this.readiness, this.permissions, this.ctx.info.settings, this.goal);
      disabled ||= !manualAllowed;
      if (this.goal === "dictation" && manualAllowed && this.ctx.info.platform === "macos" && this.readiness?.can_use_shortcut === false) label = t("onboarding.continue_without_shortcut", { shortcut: this.hotkey() });
    }
    if (this.step === "practice") {
      if (this.goal === "meeting") { disabled ||= !this.readiness?.can_record; label = t("onboarding.open_recording"); }
      else { disabled ||= !this.practiceDone; label = t("onboarding.open_dictation"); }
    }
    if (this.step === "voice") disabled ||= !!this.voicePanel?.busy || !!this.voiceModels?.downloading;
    this.next.disabled = disabled; this.next.replaceChildren(label, icon("arrowRight"));
    if (this.continueAfterDownload === this.localModelId && this.step === "engine" && this.engine === "local" && !this.actionBusy && !this.downloading && this.localModel()?.state.kind === "downloaded") {
      this.continueAfterDownload = null;
      void this.advance().catch(this.fail);
    }
  }

  private dispose() {
    this.continueAfterDownload = null;
    this.disposed = true; ++this.renderGeneration; this.poll?.dispose(); this.closeVoice();
    this.cleanup.forEach((cleanup) => cleanup());
    void this.closePractice().catch(this.ctx.report);
  }
}

export async function onboardingView(ctx: Context): Promise<HTMLElement> {
  const wizard = new OnboardingWizard(ctx);
  await wizard.start();
  return wizard.root;
}
