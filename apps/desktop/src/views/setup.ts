import { api, type AppInfo, type LocalModel, type PermissionInfo, type ProviderAccount, type ProviderTaskTestResult, type SetupReadiness } from "../api";
import { controlButton, errorText, field, h, menuButton, select } from "../dom";
import { bindLocalization, getInterfaceLanguage, l, localizeBackendText, refreshLocalization, t, type TranslationKey } from "../i18n.ts";
import type { Context } from "../main";
import { providerDestination } from "../processing-state";
import { choiceFingerprint, providerVerification, taskFingerprint } from "../provider-verification";
import { TaskModelDraft, TaskSetupScope, missingTaskAccess, processingLocation, taskConfiguration, type ProcessingLocation, type SetupTask, type TaskConfiguration } from "../task-setup-state";
import { accountView } from "./account";
import { modelPicker, providerModelCatalog, type ModelPickerState } from "./model-picker";
import { providerDetails } from "./providers";
import { localModelDisplayName } from "../settings-presentation";

const tasks: SetupTask[] = ["dictation_stt", "meeting_stt", "llm"];
const titleKeys: Record<SetupTask, TranslationKey> = {
  dictation_stt: "settings.setup.dictation_title", meeting_stt: "settings.setup.meeting_title", llm: "settings.setup.text_title",
};
const helpKeys: Record<SetupTask, TranslationKey> = {
  dictation_stt: "settings.setup.dictation_help", meeting_stt: "settings.setup.meeting_help", llm: "settings.setup.text_help",
};

function configurationLabel(configuration: TaskConfiguration): string {
  return configuration.reason ? localizeBackendText(configuration.reason) : t(`settings.setup.state.${configuration.state}`);
}

async function taskState(info: AppInfo, task: SetupTask): Promise<{ configuration: TaskConfiguration; speech: SetupReadiness | null }> {
  const account = info.settings.providers.find((provider) => provider.id === info.settings[task].provider_id);
  const local = account?.kind === "local";
  const downloaded = local ? (await api.localModels(null, false)).filter((model) => model.state.kind === "downloaded").map((model) => model.id) : [];
  let configuration = taskConfiguration(info, task, downloaded);
  if (account?.kind === "pennel_cloud" && configuration.state === "ready") {
    try { await api.cloudAccount(); } catch { configuration = { state: "account", reason: null }; }
  }
  const speech = task === "llm" ? null : await api.onboardingReadiness(task === "meeting_stt" ? "meeting" : "dictation_pad");
  return { configuration, speech };
}

export function settingsOverview(ctx: Context): HTMLElement {
  return settingsTaskOverview(ctx);
}

export function settingsTaskOverview(ctx: Context, models = false): HTMLElement {
  let disposed = false;
  const rows = new Map<SetupTask, { status: HTMLElement; selected: HTMLElement; destination: HTMLElement; action: HTMLButtonElement; revision: number; signature: string; pending: boolean; queued: boolean }>();
  const root = h("div", { class: "settings-overview" },
    models ? null : h("h2", {}, l("settings.setup.overview_title")), h("p", { class: "help" }, l(models ? "settings.setup.models_help" : "settings.setup.overview_help")),
    ...tasks.map((task) => {
      const status = h("p", { class: "settings-task-status help", role: "status" }, l("common.loading"));
      const selected = h("p", { class: "settings-task-selection", "data-task-selection": task });
      const destination = h("p", { class: "settings-task-status help" });
      const action = controlButton({ class: "btn", "data-setup-task": task, onclick: () => ctx.openSetup(task) }, l("settings.setup.finish"));
      rows.set(task, { status, selected, destination, action, revision: 0, signature: "", pending: false, queued: false });
      return h("section", { class: "settings-task-row" },
        h("div", { class: "settings-task-copy" }, h("h3", {}, l(titleKeys[task])), h("p", { class: "help" }, l(helpKeys[task])), selected, destination, status), action);
    }));
  const refreshTask = async (task: SetupTask) => {
    if (disposed) return;
    const info = ctx.info;
    const row = rows.get(task)!;
    const signature = JSON.stringify([taskFingerprint(info, task), getInterfaceLanguage(), info.interface_language, info.kinds, info.capabilities, info.active_model_downloads]);
    if (row.pending && signature === row.signature) { row.queued = true; return; }
    const changed = signature !== row.signature;
    row.signature = signature;
    row.pending = true;
    row.queued = false;
    const mine = ++row.revision;
      try {
        const choice = info.settings[task];
        const account = info.settings.providers.find((provider) => provider.id === choice.provider_id);
        let model = account?.kind === "pennel_cloud" ? t("settings.setup.cloud_model")
          : account?.kind === "apple_speech" || account?.kind === "apple_intelligence" ? t("settings.setup.system_model") : choice.model;
        const showSelection = () => { row.selected.textContent = account ? [account.name, model || t("providers.cli_default_model")].join(" · ") : t("providers.not_configured"); };
        if (changed || !row.selected.textContent) showSelection();
        row.destination.textContent = providerDestination(account);
        if (changed) { row.status.textContent = t("common.loading"); row.status.dataset.ready = "false"; }
        const { configuration, speech } = await taskState(info, task);
        if (disposed || mine !== row.revision) return;
        const ready = configuration.state === "ready" && (!speech || speech.can_record);
        const verified = providerVerification.get(task, taskFingerprint(info, task));
        row.status.textContent = ready ? t(verified?.verified ? "providers.test_passed" : "settings.setup.ready") : configuration.state !== "ready" ? configurationLabel(configuration) : speech?.reason ? localizeBackendText(speech.reason) : configurationLabel(configuration);
        row.status.dataset.ready = String(ready);
        row.action.textContent = t(ready ? "settings.setup.change" : "settings.setup.finish");
        if (account?.kind === "local") {
          const local = (await api.localModels(null, false)).find((item) => item.id === model);
          if (local) model = String(localModelDisplayName(local));
        } else if (account && !["pennel_cloud", "apple_speech", "apple_intelligence"].includes(account.kind)) {
          const kind = info.kinds.find((item) => item.kind === account.kind);
          if (kind && !kind.is_cli_agent && kind.availability.status === "available" && (!kind.requires_key || info.keys[account.id])) {
            try { model = (await providerModelCatalog(info, account)).find((item) => item.id === choice.model)?.name || model; } catch { /* A missing catalog must not hide the saved model. */ }
          }
        }
        if (disposed || mine !== row.revision) return;
        showSelection();
      } catch (error) {
        if (!disposed && mine === row.revision) row.status.textContent = errorText(error);
      } finally {
      if (!disposed && mine === row.revision) {
        row.pending = false;
        if (row.queued) { row.queued = false; void refreshTask(task); }
      }
    }
  };
  const refresh = () => Promise.all(tasks.map(refreshTask));
  ctx.onDispose(() => { disposed = true; for (const row of rows.values()) row.revision++; });
  ctx.onInfoChange(() => { void refresh(); });
  ctx.onFocusChange((focused) => { if (focused) void refresh(); });
  bindLocalization(root, () => { void refresh(); });
  void refresh();
  return root;
}

export interface TaskSetupDialog {
  notify: (info: AppInfo) => void;
  focusChanged: (focused: boolean) => void;
  visibilityChanged: (visible: boolean) => void;
  flush: () => Promise<void>;
  dispose: () => void;
  focus: () => void;
}

let dialogId = 0;
interface SetupViewScope {
  listeners: ((info: AppInfo) => void)[];
  focus: ((focused: boolean) => void)[];
  visibility: ((visible: boolean) => void)[];
  cleanup: (() => void)[];
  flushes: (() => Promise<void>)[];
}
const viewScope = (): SetupViewScope => ({ listeners: [], focus: [], visibility: [], cleanup: [], flushes: [] });

/** The origin stays mounted behind this task-specific setup, including its draft and reading position. */
export function openTaskSetupDialog(base: Context, task: SetupTask, beforeClose: () => Promise<void>, onClose: () => void): TaskSetupDialog {
  const id = `task-setup-${++dialogId}`;
  const invoker = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const state = new TaskSetupScope();
  const draft = new TaskModelDraft(base.info.settings[task]);
  const currentAccount = base.info.settings.providers.find((account) => account.id === draft.choice.provider_id);
  let location: ProcessingLocation = currentAccount ? processingLocation(currentAccount.kind, currentAccount.base_url) : "device";
  const pickerUi: ModelPickerState = { search: "", manual: false };
  const dialog = h("dialog", { class: "task-setup-dialog", "aria-modal": "true", "aria-labelledby": `${id}-title`, "aria-describedby": `${id}-help`, "data-setup-task": task });
  const body = h("div", { class: "task-setup-body" }, h("p", { class: "help", role: "status" }, l("common.loading")));
  const message = h("p", { class: "task-setup-message help", role: "status", "aria-live": "polite" });
  const status = h("p", { class: "settings-task-status help", role: "status" });
  const back = controlButton({ class: "btn", "data-setup-cancel": true, onclick: () => { void close(false); } }, l("common.cancel"));
  const done = controlButton({ class: "btn primary", "data-setup-apply": true, disabled: true, onclick: () => { void close(true); } }, l("settings.setup.apply"));
  const test = controlButton({ "data-setup-test": true, disabled: true, onclick: () => { void runSample(); } }, l("settings.setup.test"));
  const testResult = h("p", { class: "help", role: "status", "aria-live": "polite" });
  let closed = false;
  let closing = false;
  let accountPage = false;
  let providerOpen = false;
  let deferred = false;
  let pointerInFlight = false;
  let pointerGeneration = 0;
  let signature = "";
  let statusText = () => "";
  let localModels: LocalModel[] = [];
  let permissions: PermissionInfo[] = [];
  let cloudReady: boolean | null = null;
  let cloudReadinessEpoch = 0;
  let cloudReadyFingerprint: string | null = null;
  let cloudCheck: { fingerprint: string; promise: Promise<boolean> } | null = null;
  let cloudStatus: HTMLElement | null = null;
  let cloudAccountAction: HTMLButtonElement | null = null;
  let sampleEpoch = 0;
  let sampleRunning = false;
  let sample: { fingerprint: string; result: ProviderTaskTestResult } | null = null;
  let currentView = viewScope();
  const report = (error: unknown) => { if (!closed) { message.textContent = errorText(error); message.className = "task-setup-message error"; } };
  const saved = () => {
    if (closed) return;
    if (state.failure) report(state.failure);
    else { message.textContent = ""; message.className = "task-setup-message help"; }
    requestRender();
  };
  const typing = () => body.contains(document.activeElement) && !!document.activeElement?.matches("input,textarea,[contenteditable=true]");
  const enteredKey = () => [...body.querySelectorAll<HTMLInputElement>('input[type="password"]')].some((input) => !!input.value);
  const preservingInput = () => typing() || enteredKey() || pointerInFlight;
  const currentSignature = (info: AppInfo) => JSON.stringify([choiceFingerprint(info, draft.choice), info.settings.providers, info.keys, info.provider_revisions, info.interface_language, info.active_model_downloads ?? [], info.kinds.map((kind) => [kind.kind, kind.availability])]);
  const disposeView = (view: SetupViewScope) => { view.cleanup.splice(0).forEach((action) => action()); };

  const child = (view: SetupViewScope): Context => ({
    ...base,
    get info() { return base.info; },
    get visible() { return base.visible; },
    param: undefined, jumpMs: undefined, sourceTarget: undefined,
    save: (change) => {
      const pending = state.track(base.save(change), "settings");
      drawActions();
      return pending.then(saved).catch((error: unknown) => { report(error); drawActions(); throw error; });
    },
    mutate: (work) => {
      const pending = state.track(base.mutate(work));
      drawActions();
      return pending.then(saved).catch((error: unknown) => { report(error); drawActions(); throw error; });
    },
    refresh: () => {
      resetCloudReadiness(); drawActions();
      return base.refresh().then(() => { requestRender(); });
    },
    update: (info) => { base.update(info); requestRender(); },
    rerender: () => requestRender(),
    navigate: (tab) => {
      if (tab === "account") { accountPage = true; requestRender(); return; }
      if (["providers", "local", "models"].includes(tab)) {
        providerOpen = true;
        const details = body.querySelector<HTMLDetailsElement>(".task-setup-provider");
        if (details) { details.open = true; details.scrollIntoView({ block: "nearest" }); details.querySelector<HTMLElement>("input,button")?.focus(); }
        else requestRender();
        return;
      }
      report(new Error(t("settings.setup.stay_here")));
    },
    openSetup: () => undefined,
    report,
    onDispose: (action) => view.cleanup.push(action),
    beforeLeave: (action) => view.flushes.push(action),
    onFlush: (action) => view.flushes.push(action),
    onInfoChange: (action) => view.listeners.push(action),
    onFocusChange: (action) => view.focus.push(action),
    onVisibilityChange: (action) => view.visibility.push(action),
  });

  function requestRender() {
    if (closed) return;
    if (preservingInput()) { deferred = true; return; }
    void render().catch(report);
  }

  function invalidateSample() {
    sampleEpoch++; sampleRunning = false; sample = null;
  }

  function resetCloudReadiness() {
    cloudReadinessEpoch++;
    cloudReady = null;
    cloudCheck = null;
  }

  function changed(renderView = true) {
    invalidateSample();
    if (renderView) requestRender();
    drawActions();
  }

  function chooseAccount(account: ProviderAccount | undefined) {
    const kind = base.info.kinds.find((item) => item.kind === account?.kind);
    draft.chooseProvider(account?.id ?? "", (task === "llm" ? kind?.default_llm_model : kind?.default_stt_model) ?? "");
    pickerUi.search = ""; pickerUi.manual = false; providerOpen = false;
    changed();
  }

  function missingAccess(): PermissionInfo | undefined {
    const account = base.info.settings.providers.find((item) => item.id === draft.choice.provider_id);
    return missingTaskAccess(task, account?.kind, permissions);
  }

  function drawActions() {
    if (closed) return;
    const choice = draft.choice;
    const configuration = taskConfiguration(base.info, task, localModels.filter((model) => model.state.kind === "downloaded").map((model) => model.id), choice);
    const account = base.info.settings.providers.find((item) => item.id === choice.provider_id);
    const fingerprint = choiceFingerprint(base.info, draft.choice);
    if (account?.kind === "pennel_cloud" && cloudReadyFingerprint !== fingerprint) { resetCloudReadiness(); cloudReadyFingerprint = fingerprint; }
    const needsCloud = account?.kind === "pennel_cloud" && cloudReady !== true;
    const connectionPending = !!body.querySelector("button[data-setup-save]:not(:disabled)");
    const ready = configuration.state === "ready" && !needsCloud;
    if (sample?.fingerprint !== fingerprint) sample = null;
    statusText = () => needsCloud ? t(cloudReady === null ? "common.loading" : "settings.setup.cloud_sign_in") : configuration.state !== "ready" ? configurationLabel(configuration)
      : missingAccess() ? t("settings.setup.state.permission") : t("settings.setup.draft_ready");
    status.textContent = statusText();
    done.disabled = closing || accountPage || state.busy || !ready || !!missingAccess() || connectionPending || sampleRunning;
    test.disabled = closing || accountPage || state.busy || !ready || connectionPending || sampleRunning;
    test.textContent = t(sampleRunning ? "providers.testing" : "settings.setup.test");
    testResult.textContent = sampleRunning ? t("providers.processing_sample") : sample ? localizeBackendText(sample.result.message) : "";
    testResult.className = sample && !sample.result.verified ? "error" : "help";
    if (account?.kind === "pennel_cloud") {
      if (cloudStatus?.isConnected) cloudStatus.textContent = t(cloudReady === null ? "common.loading" : cloudReady ? "providers.signed_in" : "settings.setup.cloud_sign_in");
      if (cloudAccountAction?.isConnected) cloudAccountAction.textContent = t(cloudReady === false ? "providers.cloud_sign_in" : "navigation.account");
    }
  }

  function checkCloudReadiness(mine: number) {
    const choice = draft.choice;
    const account = base.info.settings.providers.find((item) => item.id === choice.provider_id);
    if (accountPage || account?.kind !== "pennel_cloud" || cloudReady !== null) return;
    const fingerprint = choiceFingerprint(base.info, choice);
    const epoch = cloudReadinessEpoch;
    if (cloudCheck?.fingerprint !== fingerprint) {
      const check = { fingerprint, promise: api.cloudAccount().then(() => true, () => false) };
      cloudCheck = check;
      void check.promise.then(() => { if (cloudCheck === check) cloudCheck = null; });
    }
    void cloudCheck.promise.then((ready) => {
      if (closed || epoch !== cloudReadinessEpoch || !state.current(mine) || fingerprint !== choiceFingerprint(base.info, draft.choice)) return;
      cloudReady = ready;
      cloudReadyFingerprint = fingerprint;
      drawActions();
    });
  }

  async function runSample() {
    if (test.disabled || sampleRunning) return;
    const epoch = ++sampleEpoch;
    const choice = { ...draft.choice, model: draft.choice.model.trim() };
    const fingerprint = choiceFingerprint(base.info, draft.choice);
    sampleRunning = true; sample = null; drawActions();
    try {
      const result = await api.testProviderTaskChoice(task, choice);
      if (!closed && epoch === sampleEpoch && fingerprint === choiceFingerprint(base.info, draft.choice)) sample = { fingerprint, result };
    } catch (error) {
      if (!closed && epoch === sampleEpoch && fingerprint === choiceFingerprint(base.info, draft.choice)) sample = { fingerprint, result: { verified: false, message: errorText(error) } };
    } finally {
      if (!closed && epoch === sampleEpoch) { sampleRunning = false; drawActions(); }
    }
  }

  async function render() {
    const mine = state.next();
    signature = currentSignature(base.info);
    const nextView = viewScope();
    const ctx = child(nextView);
    try {
      let node: HTMLElement;
      if (accountPage) {
        node = h("div", {}, h("button", { class: "text-button", onclick: () => { accountPage = false; resetCloudReadiness(); requestRender(); } }, l("settings.setup.back_to_models")), await accountView(ctx, { taskSetup: true }));
      } else {
        const info = ctx.info;
        const choice = draft.choice;
        const account = info.settings.providers.find((provider) => provider.id === choice.provider_id);
        if (account) location = processingLocation(account.kind, account.base_url);
        const kind = info.kinds.find((item) => item.kind === account?.kind);
        const [nextModels, nextPermissions] = await Promise.all([
          account?.kind === "local" ? api.localModels(null, false) : Promise.resolve([] as LocalModel[]),
          task === "llm" ? Promise.resolve([] as PermissionInfo[]) : api.permissions(),
        ]);
        if (!state.current(mine)) { disposeView(nextView); return; }
        localModels = nextModels; permissions = nextPermissions;
        const configuration = taskConfiguration(info, task, localModels.filter((model) => model.state.kind === "downloaded").map((model) => model.id), choice);
        const compatible = info.kinds.filter((item) => task === "llm" ? item.llm : item.stt);
        const sources = h("div", { class: "setup-sources", role: "group", "aria-label": l("settings.setup.where") },
          ...(["device", "cloud", "own"] as const).map((source) => {
            const options = compatible.filter((item) => processingLocation(item.kind, item.default_base_url) === source
              || info.settings.providers.some((account) => account.kind === item.kind && processingLocation(account.kind, account.base_url) === source));
            const supported = options.some((item) => item.availability.status !== "unsupported");
            const reason = !supported ? options.find((item) => item.availability.reason)?.availability.reason : null;
            return h("div", { class: "setup-source" },
              controlButton({ "data-setup-source": source, "aria-pressed": String(location === source), disabled: !supported, onclick: () => {
                location = source;
                const candidates = info.settings.providers.filter((item) => processingLocation(item.kind, item.base_url) === source && options.some((option) => option.kind === item.kind));
                chooseAccount(candidates.find((item) => compatible.find((option) => option.kind === item.kind)?.availability.status === "available") ?? candidates[0]);
              } }, l(`settings.setup.source.${source}`)),
              !supported ? h("p", { class: "help" }, reason ? localizeBackendText(reason) : l("settings.setup.source.unavailable")) : null);
          }));
        const accounts = info.settings.providers.filter((item) => processingLocation(item.kind, item.base_url) === location && compatible.some((option) => option.kind === item.kind));
        const provider = accounts.length ? select(accounts.map((item) => [item.id, item.name]), choice.provider_id, (id) => chooseAccount(accounts.find((item) => item.id === id)), "menu") : null;
        if (provider) {
          provider.setAttribute("data-setup-provider", "true");
          provider.setAttribute("aria-label", t("settings.setup.process_with"));
          for (const option of provider.options) option.disabled = compatible.find((item) => item.kind === accounts.find((candidate) => candidate.id === option.value)?.kind)?.availability.status === "unsupported";
        }
        const addable = compatible.filter((item) => processingLocation(item.kind, item.default_base_url) === location
          && !(["local", "apple_speech", "apple_intelligence", "pennel_cloud"].includes(item.kind) && info.settings.providers.some((provider) => provider.kind === item.kind)));
        const add = addable.length ? menuButton(l("settings.setup.add_connection"), addable.map((kind) => ({
          label: kind.availability.status === "unsupported" && kind.availability.reason ? `${localizeBackendText(kind.label)} — ${localizeBackendText(kind.availability.reason)}` : localizeBackendText(kind.label),
          disabled: kind.availability.status === "unsupported",
          run: async () => {
            try {
              providerOpen = true;
              await ctx.mutate(async () => {
                const next = await api.addProvider(kind.kind);
                const created = next.settings.providers.at(-1);
                if (created && !closed) {
                  draft.chooseProvider(created.id, task === "llm" ? kind.default_llm_model : kind.default_stt_model);
                  pickerUi.search = ""; pickerUi.manual = false; invalidateSample();
                }
                return next;
              });
            } catch (error) { report(error); }
          },
        })), "task-setup-add") : null;
        node = h("div", { class: "guided-setup" }, h("h3", {}, l("settings.setup.where")), sources,
          h("p", { class: "help setup-source-help" }, l(`settings.setup.source.${location}_help`)),
          provider ? field(l("settings.setup.process_with"), h("div", { class: "row" }, provider, add))
            : h("div", {}, h("p", { class: "help" }, l("settings.setup.no_connection")), add ? h("div", { class: "row setup-add-connection" }, add) : null));
        if (account && kind) {
          if (kind.availability.reason) node.append(h("p", { class: "help" }, localizeBackendText(kind.availability.reason)));
          if (kind.is_cli_agent) node.append(h("p", { class: "help" }, l("settings.setup.cli_destination")));
          node.append(h("p", { class: "setup-destination" }, l("settings.setup.processing_at", () => ({ destination: providerDestination(account) }))));
          if (account.kind === "pennel_cloud") {
            cloudStatus = h("p", { class: "help" });
            cloudAccountAction = controlButton({ onclick: () => { accountPage = true; requestRender(); } }, l("navigation.account"));
            node.append(cloudStatus, cloudAccountAction);
          } else if (!["local", "apple_speech", "apple_intelligence"].includes(account.kind)) {
            const affected = tasks.filter((item) => info.settings[item].provider_id === account.id).map((item) => t(titleKeys[item])).join(", ");
            const details = h("details", { class: "task-setup-provider", open: providerOpen || ["key", "address"].includes(configuration.state) },
              h("summary", {}, l("settings.setup.connection_details")),
              h("p", { class: "help" }, affected ? l("settings.setup.shared_connection", { tasks: affected }) : l("settings.setup.connection_saved")),
              await providerDetails(ctx, account, task));
            details.addEventListener("toggle", () => { providerOpen = details.open; });
            node.append(details);
          }
          node.append(modelPicker(ctx, choice, localModels, pickerUi, (model) => { draft.setModel(model); changed(account.kind === "local"); }));
        }
        if (missingAccess()) {
          node.append(h("p", { class: "help" }, l("settings.setup.access_help")), controlButton({ "data-setup-access": true, onclick: async () => {
            try {
              const required = missingAccess();
              if (required) await api.requestPermission(required.kind);
              await ctx.refresh();
            } catch (error) { report(error); }
          } }, l("settings.setup.check_access")));
        }
        node.append(h("div", { class: "setup-sample" }, h("p", { class: "help" }, l("settings.setup.sample_help")), h("div", { class: "row" }, test), testResult),
          h("p", { class: "help setup-prerequisites" }, l("settings.setup.prerequisites")));
      }
      if (!state.current(mine)) { disposeView(nextView); return; }
      if (preservingInput()) { disposeView(nextView); deferred = true; return; }
      disposeView(currentView); currentView = nextView;
      const scroll = body.scrollTop;
      body.replaceChildren(node); refreshLocalization(node); body.scrollTop = scroll;
      deferred = false;
      if (!state.failure) { message.textContent = ""; message.className = "task-setup-message help"; }
      drawActions();
      checkCloudReadiness(mine);
    } catch (error) {
      disposeView(nextView);
      if (state.current(mine)) { signature = ""; throw error; }
    }
  }

  async function flush() {
    await state.settle();
    await Promise.all(currentView.flushes.map((action) => action()));
  }

  async function close(finish: boolean) {
    if (closed || closing || (finish && done.disabled)) return;
    const candidate = { ...draft.choice, model: draft.choice.model.trim() };
    const expected = choiceFingerprint(base.info, candidate);
    closing = true; back.disabled = true; done.disabled = true;
    body.inert = true;
    try {
      if (document.activeElement instanceof HTMLElement && dialog.contains(document.activeElement)) document.activeElement.blur();
      try { await flush(); } catch (error) { if (finish) throw error; }
      await beforeClose();
      state.settingsSaved();
      if (finish && state.failure) throw state.failure;
      if (finish) {
        const tested = sample;
        await state.track(base.mutate(async () => {
          const next = await api.info();
          if (choiceFingerprint(next, candidate) !== expected) throw new Error(t("settings.setup.changed_review"));
          const configuration = taskConfiguration(next, task, localModels.filter((model) => model.state.kind === "downloaded").map((model) => model.id), candidate);
          if (configuration.state !== "ready") throw new Error(configurationLabel(configuration));
          return api.applyTaskModelChoice(task, candidate);
        }), "settings");
        const applied = taskFingerprint(base.info, task);
        if (tested?.result.verified && tested.fingerprint === expected && applied === tested.fingerprint) providerVerification.set(task, applied, tested.result);
      }
      dispose();
      onClose();
    } catch (error) { report(error); }
    finally { closing = false; if (!closed) { body.inert = false; back.disabled = false; drawActions(); } }
  }

  function dispose() {
    if (closed) return;
    closed = true; invalidateSample(); state.dispose(); disposeView(currentView);
    dialog.close(); dialog.remove();
    if (invoker?.isConnected) invoker.focus({ preventScroll: true });
  }

  body.addEventListener("focusout", () => { queueMicrotask(() => { if (deferred && !preservingInput()) requestRender(); }); });
  dialog.addEventListener("pointerdown", () => { pointerGeneration++; pointerInFlight = true; }, true);
  dialog.addEventListener("pointerup", () => {
    const gesture = pointerGeneration;
    requestAnimationFrame(() => {
      if (closed || gesture !== pointerGeneration) return;
      pointerInFlight = false;
      if (deferred) requestRender();
    });
  });
  dialog.addEventListener("pointercancel", () => { pointerInFlight = false; if (deferred) requestRender(); });
  dialog.addEventListener("click", () => { pointerInFlight = false; if (deferred) queueMicrotask(requestRender); });
  body.addEventListener("input", drawActions);
  bindLocalization(status, drawActions);
  dialog.addEventListener("cancel", (event) => { event.preventDefault(); void close(false); });
  dialog.append(h("header", { class: "task-setup-header" }, h("h2", { id: `${id}-title` }, l(titleKeys[task])), h("p", { id: `${id}-help`, class: "help" }, l(helpKeys[task])), status),
    body, message, h("footer", { class: "task-setup-footer row" }, back, h("span", { class: "grow" }), done));
  document.body.append(dialog); dialog.showModal();
  requestRender();
  return {
    notify: (info) => {
      currentView.listeners.forEach((listener) => listener(info));
      if (currentSignature(info) !== signature) { invalidateSample(); drawActions(); requestRender(); }
    },
    focusChanged: (focused) => {
      currentView.focus.forEach((listener) => listener(focused));
      if (focused) {
        resetCloudReadiness(); drawActions();
        void base.refresh().then(() => { requestRender(); }).catch(report);
      }
    },
    visibilityChanged: (visible) => { currentView.visibility.forEach((listener) => listener(visible)); },
    flush, dispose, focus: () => { (dialog.querySelector<HTMLElement>("button,input,select") ?? done).focus(); },
  };
}
