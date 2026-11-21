import { api, type ModelChoice, type ProviderAccount, type ProviderKind, type SpeakerRecognitionMode } from "../api";
import type { ConfiguredDictationModel } from "../pill-model-state";
import { controlButton, errorText, h, menuButton, section, select, showPopupMenu, textField, toggle } from "../dom";
import { bindLocalization, formatNumber, l, liveText, localizeBackendText, t } from "../i18n.ts";
import { icon } from "../icons";
import type { Context } from "../main";
import { providerDestination } from "../processing-state";
import { recommendedSpeechModel } from "../dictation-home-state";
import { credentialAccess } from "../credential-access";
import { localView } from "./local";
import { providersView } from "./providers";
import { inferenceModelsView } from "./inference";
import { settingsTaskRows } from "./setup";
import { SpeakerModels, speakerDownload } from "./voices";

const cloudProviderKinds = new Set<ProviderKind>(["open_ai", "groq", "mistral", "open_router", "pennel_cloud"]);
const sameChoice = (a: ModelChoice, b: ModelChoice) => a.provider_id === b.provider_id && a.model === b.model;

type ActivePreset = "local" | "cloud" | "own";

function detectActivePreset(ctx: Context): ActivePreset {
  const { settings } = ctx.info;
  const stt = settings.providers.find((p) => p.id === settings.dictation_stt.provider_id);
  const llm = settings.providers.find((p) => p.id === settings.llm.provider_id);
  if (stt?.kind === "pennel_cloud" && llm?.kind === "pennel_cloud") return "cloud";
  const sttLocal = stt && ["local", "apple_speech"].includes(stt.kind);
  const llmLocal = llm && ["apple_intelligence", "ollama", "lm_studio"].includes(llm.kind);
  if (sttLocal && (settings.mode === "fast" || llmLocal)) return "local";
  return "own";
}

async function quickSetupSection(ctx: Context, onOwnClick: () => void): Promise<HTMLElement> {
  const active = detectActivePreset(ctx);
  const status = h("p", { class: "help ai-quick-status", role: "status", hidden: true });
  let busy = false;

  const setStatus = (msg: string, isError = false) => {
    status.textContent = msg;
    status.className = isError ? "error ai-quick-status" : "help ai-quick-status";
    status.hidden = !msg;
  };

  const applyLocal = async () => {
    if (busy) return;
    busy = true;
    setStatus("");
    try {
      const models = await api.localModels(ctx.info.settings.language, true).catch(() => []);
      const downloaded = models.find((m) => m.state.kind === "downloaded");
      const recommended = downloaded ?? recommendedSpeechModel(models);
      const appleSpeechAvailable = ctx.info.kinds.find((k) => k.kind === "apple_speech")?.availability.status === "available";
      const appleTextAvailable = ctx.info.kinds.find((k) => k.kind === "apple_intelligence")?.availability.status === "available";
      await ctx.mutate(async () => {
        let next = await api.info();
        const ensureProvider = async (kind: ProviderKind) => {
          let found = next.settings.providers.find((p) => p.kind === kind);
          if (!found) {
            next = await api.addProvider(kind);
            found = next.settings.providers.find((p) => p.kind === kind);
          }
          return found;
        };
        const settings = structuredClone(next.settings);
        if (downloaded) {
          const local = await ensureProvider("local");
          if (local) {
            settings.providers = structuredClone(next.settings.providers);
            settings.dictation_stt = { provider_id: local.id, model: downloaded.id };
            settings.meeting_stt = { provider_id: local.id, model: downloaded.id };
          }
        } else if (appleSpeechAvailable) {
          const apple = await ensureProvider("apple_speech");
          const kindInfo = next.kinds.find((k) => k.kind === "apple_speech");
          if (apple) {
            settings.providers = structuredClone(next.settings.providers);
            settings.dictation_stt = { provider_id: apple.id, model: kindInfo?.default_stt_model ?? "" };
            settings.meeting_stt = { provider_id: apple.id, model: kindInfo?.default_stt_model ?? "" };
          }
        } else if (recommended) {
          const local = await ensureProvider("local");
          if (local) {
            settings.providers = structuredClone(next.settings.providers);
            settings.dictation_stt = { provider_id: local.id, model: recommended.id };
            settings.meeting_stt = { provider_id: local.id, model: recommended.id };
          }
        }
        if (appleTextAvailable) {
          const appleText = await ensureProvider("apple_intelligence");
          const kindInfo = next.kinds.find((k) => k.kind === "apple_intelligence");
          if (appleText) {
            settings.providers = structuredClone(next.settings.providers);
            settings.llm = { provider_id: appleText.id, model: kindInfo?.default_llm_model ?? "" };
          }
        } else {
          const localServer = next.settings.providers.find((p) => p.kind === "ollama" || p.kind === "lm_studio");
          const kindInfo = localServer ? next.kinds.find((k) => k.kind === localServer.kind) : null;
          if (localServer && kindInfo?.default_llm_model) {
            settings.llm = { provider_id: localServer.id, model: kindInfo.default_llm_model };
          }
        }
        settings.speaker_recognition = "local";
        settings.decision = null;
        return api.saveSettings(settings);
      });
      setStatus(t("settings.ai.quick_saved"));
      if (!downloaded && !appleSpeechAvailable) ctx.openSetup("dictation_stt");
    } catch (error) {
      setStatus(errorText(error), true);
    } finally {
      busy = false;
    }
  };

  const applyCloud = async () => {
    if (busy) return;
    if (!ctx.info.cloud_signed_in) {
      ctx.navigate("account");
      return;
    }
    busy = true;
    setStatus("");
    try {
      await ctx.mutate(async () => {
        let next = await api.info();
        if (!next.settings.providers.some((p) => p.kind === "pennel_cloud")) next = await api.addProvider("pennel_cloud");
        const settings = structuredClone(next.settings);
        const cloud = settings.providers.find((p) => p.kind === "pennel_cloud");
        const kind = next.kinds.find((k) => k.kind === "pennel_cloud");
        if (!cloud) return next;
        const sttModel = kind?.default_stt_model ?? "pennel-auto";
        const llmModel = kind?.default_llm_model ?? "pennel-auto";
        settings.dictation_stt = { provider_id: cloud.id, model: sttModel };
        settings.meeting_stt = { provider_id: cloud.id, model: sttModel };
        settings.llm = { provider_id: cloud.id, model: llmModel };
        settings.decision = null;
        return api.saveSettings(settings);
      });
      setStatus(t("settings.ai.quick_saved"));
    } catch (error) {
      setStatus(errorText(error), true);
    } finally {
      busy = false;
    }
  };

  const presetCard = (
    id: ActivePreset,
    iconName: Parameters<typeof icon>[0],
    titleKey: Parameters<typeof l>[0],
    helpKey: Parameters<typeof l>[0],
    actionLabel: ReturnType<typeof l>,
    onAction: () => void,
  ) => {
    const isActive = active === id;
    return h("div", { class: "ai-preset-card", "data-ai-preset": id, "data-active": String(isActive) },
      h("div", { class: "row ai-preset-head" },
        icon(iconName),
        h("strong", { class: "grow" }, l(titleKey)),
        isActive ? h("span", { class: "model-badge" }, l("settings.ai.preset.active")) : null),
      h("p", { class: "help" }, l(helpKey)),
      h("div", { class: "ai-preset-footer" },
        controlButton({ class: isActive ? "btn" : "btn primary", onclick: onAction }, actionLabel)));
  };

  const root = section(l("settings.ai.quick_title"), l("settings.ai.quick_help"),
    h("div", { class: "ai-preset-grid" },
      presetCard("local", "laptop", "settings.ai.preset.local_title", "settings.ai.preset.local_help", l("settings.ai.preset.use"), () => { void applyLocal(); }),
      presetCard("cloud", "globe", "settings.ai.preset.cloud_title", "settings.ai.preset.cloud_help",
        l(ctx.info.cloud_signed_in ? "settings.ai.preset.use" : "settings.ai.preset.sign_in"), () => { void applyCloud(); }),
      presetCard("own", "settings", "settings.ai.preset.own_title", "settings.ai.preset.own_help", l("settings.ai.preset.use"), onOwnClick)),
    status);
  root.dataset.settingsAnchor = "ai-quick";
  return root;
}

function activeModelsSection(ctx: Context): HTMLElement {
  let disposed = false;
  const settings = ctx.info.settings;
  let showSeparateMeeting = !sameChoice(settings.dictation_stt, settings.meeting_stt);

  let speechModels: ConfiguredDictationModel[] = [];

  const quickSpeechLabel = h("span", { class: "menu-button-label" }, l("dictation.pill.models"));
  const quickSpeechPicker = controlButton({
    class: "menu-button speech-model-picker",
    "data-ai-speech-picker": true,
    "aria-haspopup": "menu",
    disabled: true,
    onclick: () => showPopupMenu(quickSpeechPicker, [
      ...speechModels.map((model) => ({
        value: JSON.stringify([model.choice.provider_id, model.choice.model]),
        label: model.label,
        run: async () => {
          await ctx.mutate(async () => {
            const latest = await api.info();
            const next = structuredClone(latest.settings);
            const linked = sameChoice(next.dictation_stt, next.meeting_stt) || !showSeparateMeeting;
            next.dictation_stt = model.choice;
            if (linked) next.meeting_stt = model.choice;
            return api.saveSettings(next);
          });
        },
      })),
      { value: "setup", label: l("dictation.pill.model_setup"), separatorBefore: true, run: () => ctx.openSetup("dictation_stt") },
    ], { current: JSON.stringify([ctx.info.settings.dictation_stt.provider_id, ctx.info.settings.dictation_stt.model]), report: ctx.report }),
  }, quickSpeechLabel, icon("chevronRight"));

  const rows = settingsTaskRows(ctx, ["dictation_stt", "llm"]);
  const sttRow = rows.querySelector<HTMLElement>('[data-task-row="dictation_stt"]');
  const llmRow = rows.querySelector<HTMLElement>('[data-task-row="llm"]');
  let meetingRowsHolder: HTMLElement | null = null;
  let meetingRow: HTMLElement | null = null;

  const ensureMeetingRow = () => {
    if (!meetingRow) {
      meetingRowsHolder = settingsTaskRows(ctx, ["meeting_stt"]);
      meetingRow = meetingRowsHolder.querySelector<HTMLElement>('[data-task-row="meeting_stt"]');
    }
    return meetingRow;
  };

  const syncRowLayout = () => {
    if (sttRow) {
      sttRow.querySelector("h3")?.replaceChildren(h("span", {}, l(showSeparateMeeting ? "settings.setup.dictation_title" : "settings.ai.speech_title")));
      const help = sttRow.querySelector(".help");
      if (help) help.textContent = t(showSeparateMeeting ? "settings.setup.dictation_help" : "settings.ai.speech_help");
      if (!sttRow.contains(quickSpeechPicker)) sttRow.append(quickSpeechPicker);
    }
    if (llmRow) {
      llmRow.querySelector("h3")?.replaceChildren(h("span", {}, l("settings.ai.text_title")));
      const help = llmRow.querySelector(".help");
      if (help) help.textContent = t("settings.ai.text_help");
    }
    if (showSeparateMeeting) {
      const row = ensureMeetingRow();
      if (row && llmRow && row.parentElement !== rows) rows.insertBefore(row, llmRow);
    } else if (meetingRow && meetingRow.parentElement === rows) {
      meetingRow.remove();
    }
  };

  const taskRowsContainer = h("div", { class: "ai-active-tasks" }, rows);

  const refreshQuickSpeech = async () => {
    const models = await api.configuredDictationModels().catch(() => [] as ConfiguredDictationModel[]);
    if (disposed) return;
    speechModels = models;
    const selected = models.find((m) => sameChoice(m.choice, ctx.info.settings.dictation_stt));
    quickSpeechLabel.textContent = selected?.label ?? t("dictation.pill.models");
    quickSpeechPicker.disabled = models.length === 0;
    quickSpeechPicker.hidden = models.length === 0;
  };

  const separateToggle = toggle(l("settings.ai.separate_meeting_speech"), showSeparateMeeting, async (checked) => {
    showSeparateMeeting = checked;
    if (!checked && !sameChoice(ctx.info.settings.dictation_stt, ctx.info.settings.meeting_stt)) {
      await ctx.save((next) => { next.meeting_stt = { ...next.dictation_stt }; });
    }
    syncRowLayout();
  });

  const overrideText = h("span", { class: "help grow" });
  const overrideNotice = h("div", { class: "row ai-review-override", hidden: true },
    overrideText,
    controlButton({ class: "btn compact", onclick: () => ctx.save((next) => { next.decision = null; }) }, l("settings.ai.review_use_default")));
  const syncOverride = () => {
    const decision = ctx.info.settings.decision;
    overrideNotice.hidden = !decision;
    if (decision) overrideText.textContent = t("settings.ai.review_override", { model: decision.model });
  };

  syncRowLayout();
  syncOverride();
  void refreshQuickSpeech();
  ctx.onInfoChange(() => {
    if (disposed) return;
    const isDiff = !sameChoice(ctx.info.settings.dictation_stt, ctx.info.settings.meeting_stt);
    if (isDiff && !showSeparateMeeting) {
      showSeparateMeeting = true;
      separateToggle.querySelector<HTMLInputElement>("input")!.checked = true;
      syncRowLayout();
    }
    syncOverride();
    void refreshQuickSpeech();
  });
  ctx.onDispose(() => { disposed = true; });

  const root = section(l("settings.ai.tasks_title"), null, taskRowsContainer, separateToggle, overrideNotice);
  root.dataset.settingsAnchor = "ai-tasks";
  return root;
}

async function speakersCompactSection(ctx: Context): Promise<HTMLElement> {
  const setup = await api.speakerSetup();
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
  };
  const models = new SpeakerModels(setup.model_ids, updateModels);
  download = speakerDownload(models);
  ctx.onDispose(() => models.dispose());

  const choice = select<SpeakerRecognitionMode>(
    [["local", l("settings.speakers.route_local")], ["cloud", l("settings.speakers.route_cloud")]],
    mode,
    (value) => ctx.save((next) => { next.speaker_recognition = value; }),
    "segmented",
  );
  choice.setAttribute("aria-label", t("settings.speakers.route_label"));
  choice.options[0].disabled = !setup.local_available;

  const routeNodes = [
    setup.local_available ? null : h("p", { class: "help" }, l("settings.speakers.unavailable")),
    ...(mode === "cloud"
      ? [
          h("p", { class: "help" }, l("settings.speakers.cloud_help")),
          setup.signed_in ? null : controlButton({ class: "btn", onclick: () => ctx.navigate("account") }, l("settings.speakers.sign_in")),
        ]
      : setup.local_available
        ? [modelStatus, download.element, h("p", { class: "help" }, l("settings.speakers.local_help"))]
        : []),
  ];

  bindLocalization(modelStatus, updateModels);
  await models.start(ctx.info.active_model_downloads ?? []);

  const root = section(
    l("settings.ai.speakers_title"),
    l("settings.speakers.identity_help"),
    toggle(l("settings.speakers.automatic"), settings.diarization, (value) => ctx.save((next) => { next.diarization = value; }), ctx.info.capabilities.diarization),
    h("div", { class: "row speaker-route" }, choice),
    ...routeNodes,
  );
  root.dataset.settingsAnchor = "ai-speakers";
  return root;
}

function cloudProvidersSection(ctx: Context): HTMLElement {
  const { settings, kinds, keys } = ctx.info;
  const cloudAccounts = settings.providers.filter((p) => cloudProviderKinds.has(p.kind));
  const cards = cloudAccounts.map((account: ProviderAccount) => {
    const isCloud = account.kind === "pennel_cloud";
    const ready = isCloud ? ctx.info.cloud_signed_in === true : !!keys[account.id];
    const status = h("p", { class: "help", role: "status", hidden: true });
    let body: HTMLElement;
    if (isCloud) {
      body = h("div", { class: "row" },
        h("span", { class: "help grow" }, l(ready ? "providers.signed_in" : "providers.cloud_sign_in")),
        controlButton({ class: "btn", onclick: () => ctx.navigate("account") }, l(ready ? "navigation.account" : "providers.cloud_sign_in")));
    } else {
      const keyInput = textField({
        type: "password",
        class: "grow",
        placeholder: ready ? l("providers.saved_key") : l("providers.paste_key"),
        autocomplete: "off",
        "aria-label": l("providers.api_key"),
      });
      const saveBtn = controlButton({
        class: "btn",
        disabled: true,
        onclick: async () => {
          if (!keyInput.value.trim()) return;
          saveBtn.disabled = true;
          try {
            await ctx.mutate(() => api.setKey(account.id, keyInput.value.trim()));
            keyInput.value = "";
            status.textContent = t("providers.configured");
            status.className = "help";
            status.hidden = false;
          } catch (error) {
            status.textContent = errorText(error);
            status.className = "error";
            status.hidden = false;
          } finally {
            saveBtn.disabled = !keyInput.value.trim();
          }
        },
      }, l("common.save"));
      keyInput.addEventListener("input", () => { saveBtn.disabled = !keyInput.value.trim(); });
      keyInput.addEventListener("keydown", (event) => { if (event.key === "Enter" && !saveBtn.disabled) { event.preventDefault(); saveBtn.click(); } });
      const testBtn = controlButton({
        class: "btn",
        disabled: !ready,
        onclick: async () => {
          testBtn.disabled = true;
          status.textContent = t("providers.testing");
          status.className = "help";
          status.hidden = false;
          try {
            const models = await api.providerModels(account.id);
            status.textContent = t("providers.connection_verified", { count: formatNumber(models.length) });
          } catch (error) {
            status.textContent = t("providers.test_failed", { error: errorText(error) });
            status.className = "error";
          } finally {
            testBtn.disabled = false;
          }
        },
      }, l("providers.test_connection"));
      const clearBtn = ready
        ? controlButton({ class: "text-button", onclick: () => ctx.mutate(() => api.setKey(account.id, "")) }, l("providers.clear_key"))
        : null;
      body = h("div", { class: "ai-provider-controls" },
        h("div", { class: "row" }, keyInput, saveBtn, testBtn, clearBtn),
        status);
    }
    return h("div", { class: "ai-provider-row", "data-provider-id": account.id },
      h("div", { class: "row ai-provider-head" },
        h("strong", {}, account.name),
        h("span", { class: "help grow" }, liveText(() => providerDestination(account))),
        h("span", { class: ready ? "model-badge" : "badge" }, l(ready ? "providers.configured" : "providers.not_configured"))),
      body);
  });

  const addable = kinds.filter((k) => cloudProviderKinds.has(k.kind) && !(k.kind === "pennel_cloud" && settings.providers.some((p) => p.kind === "pennel_cloud")));
  const addMenu = addable.length
    ? menuButton(l("providers.add"), addable.map((kind) => ({
        label: liveText(() => localizeBackendText(kind.label)),
        disabled: kind.availability.status === "unsupported",
        run: () => ctx.mutate(() => api.addProvider(kind.kind as ProviderKind)),
      })), "provider-add")
    : null;

  const root = section(
    l("settings.ai.providers_title"),
    l("settings.ai.providers_help"),
    credentialAccess(ctx),
    ...cards,
    addMenu ? h("div", { class: "row ai-provider-add" }, addMenu) : null,
  );
  root.dataset.settingsAnchor = "ai-providers";
  return root;
}

export async function aiModelsView(ctx: Context, initialTab: string | null = null): Promise<HTMLElement> {
  const cloudSection = cloudProvidersSection(ctx);
  const [quick, speakers, localModels, advancedProviders, specializedReview] = await Promise.all([
    quickSetupSection(ctx, () => {
      cloudSection.scrollIntoView({ behavior: "smooth", block: "start" });
      cloudSection.querySelector<HTMLElement>("input,button")?.focus({ preventScroll: true });
    }),
    speakersCompactSection(ctx),
    localView(ctx, true),
    providersView({ ...ctx, get info() { return ctx.info; }, param: initialTab === "local" ? "__local__" : undefined }),
    inferenceModelsView(ctx),
  ]);
  localModels.dataset.settingsAnchor = "ai-local";

  const usesAdvancedProvider = [ctx.info.settings.dictation_stt, ctx.info.settings.meeting_stt, ctx.info.settings.llm].some((choice) => {
    const provider = ctx.info.settings.providers.find((p) => p.id === choice.provider_id);
    return provider && !cloudProviderKinds.has(provider.kind) && !["local", "apple_speech", "apple_intelligence"].includes(provider.kind);
  });
  const openAdvanced = initialTab === "providers" || usesAdvancedProvider;
  const advancedSection = h("details", { class: "task-settings-detail ai-advanced-providers", open: openAdvanced, "data-settings-anchor": "ai-advanced" },
    h("summary", {}, l("settings.ai.advanced_providers_title")),
    h("p", { class: "help" }, l("settings.ai.advanced_providers_help")),
    advancedProviders,
  );

  return h("div", { class: "task-settings ai-models-hub" },
    h("p", { class: "help ai-hub-subtitle" }, l("settings.ai.subtitle")),
    quick,
    activeModelsSection(ctx),
    speakers,
    localModels,
    cloudSection,
    advancedSection,
    ...specializedReview,
  );
}
