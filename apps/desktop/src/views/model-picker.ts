import { api, listen, type AppInfo, type LocalModel, type ModelChoice, type ProviderAccount, type RemoteModel } from "../api";
import { controlButton, errorText, field, formatBytes, h, isCancellation, select, textField } from "../dom";
import { l, liveText, localizeBackendText, t } from "../i18n";
import type { Context } from "../main";
import { ModelCatalogCache, providerFingerprint } from "../provider-verification";
import { localModelDisplayName } from "../settings-presentation";

const catalogs = new ModelCatalogCache<RemoteModel>();

export function providerModelCatalog(info: AppInfo, account: ProviderAccount): Promise<RemoteModel[]> {
  return catalogs.get(account.id, providerFingerprint(info, account), () => api.providerModels(account.id));
}

export interface ModelPickerState { search: string; manual: boolean }

export function modelPicker(ctx: Context, choice: ModelChoice, models: LocalModel[], ui: ModelPickerState, change: (model: string) => void): HTMLElement {
  const account = ctx.info.settings.providers.find((item) => item.id === choice.provider_id);
  const kind = ctx.info.kinds.find((item) => item.kind === account?.kind);
  const root = h("div", { class: "setup-model-picker" });
  if (!account || !kind) return root;
  let disposed = false;
  ctx.onDispose(() => { disposed = true; });

  if (["apple_speech", "apple_intelligence", "pennel_cloud"].includes(account.kind)) {
    root.append(field(l("common.model"), h("span", {}, l(account.kind === "pennel_cloud" ? "settings.setup.cloud_model" : "settings.setup.system_model"))));
    return root;
  }

  if (account.kind === "local") {
    const picker = select(models.map((model) => [model.id, liveText(() => `${localModelDisplayName(model)} · ${formatBytes(model.size_bytes)}${model.recommended ? ` · ${t("settings.setup.recommended")}` : ""}`)]), choice.model, change, "menu");
    picker.setAttribute("data-setup-local-model", "true");
    picker.disabled = !models.length || kind.availability.status === "unsupported";
    root.append(field(l("common.model"), picker));
    const model = models.find((item) => item.id === choice.model);
    if (!model) { root.append(h("p", { class: "help" }, l("onboarding.no_local_model"))); return root; }
    root.append(h("p", { class: "help" }, localizeBackendText(model.description)));
    if (model.state.kind === "downloaded") {
      root.append(h("p", { class: "help" }, l("settings.setup.downloaded", () => ({ size: formatBytes(model.size_bytes) }))));
      return root;
    }
    let downloading = ctx.info.active_model_downloads?.includes(model.id) ?? false;
    const progress = h("progress", { hidden: !downloading, "aria-label": l("providers.local.downloading_model", () => ({ model: String(localModelDisplayName(model)) })) });
    const outcome = h("p", { class: "help", role: "status" });
    const download = controlButton({ "data-setup-download": true, disabled: downloading || kind.availability.status !== "available", onclick: async () => {
      downloading = true; download.disabled = true; cancel.hidden = false; progress.hidden = false; outcome.textContent = "";
      try { await api.downloadModel(model.id); if (!disposed) await ctx.refresh(); }
      catch (error) { if (!disposed && !isCancellation(error)) { outcome.textContent = errorText(error); outcome.className = "error"; } }
      finally { if (!disposed) { downloading = false; download.disabled = false; cancel.hidden = true; progress.hidden = true; } }
    } }, l("settings.setup.download", () => ({ size: formatBytes(model.size_bytes) })));
    const cancel = controlButton({ hidden: !downloading, onclick: () => api.cancelDownload(model.id).catch(ctx.report) }, l("onboarding.cancel_download"));
    root.append(h("p", { class: "help" }, l("settings.setup.local_download_help")), h("div", { class: "row" }, download, cancel), progress, outcome);
    void listen<{ id: string; downloaded: number; total: number }>("download-progress", (event) => {
      if (!disposed && event.payload.id === model.id) { progress.max = event.payload.total || 1; progress.value = event.payload.downloaded; }
    }).then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
    void listen<{ id: string }>("download-done", (event) => { if (!disposed && event.payload.id === model.id) void ctx.refresh().catch(ctx.report); })
      .then((unlisten) => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(ctx.report);
    return root;
  }

  let known: RemoteModel[] = [];
  const selected = h("p", { class: "setup-selected-model", "data-setup-selected-model": true });
  const drawSelected = () => {
    const model = known.find((item) => item.id === choice.model);
    selected.textContent = model?.name && model.name !== model.id ? `${model.name} · ${model.id}` : choice.model.trim() || (kind.is_cli_agent ? t("providers.cli_default_model") : t("providers.choose_model"));
  };
  drawSelected();
  const manual = textField({ value: choice.model, "data-setup-manual": true, "aria-label": l("settings.setup.model_id"), placeholder: kind.is_cli_agent ? l("providers.cli_default_model") : l("providers.model_name"), spellcheck: "false", autocomplete: "off" });
  manual.addEventListener("input", () => { choice.model = manual.value; drawSelected(); change(manual.value); });
  const advanced = h("details", { class: "setup-model-advanced", open: ui.manual || kind.is_cli_agent },
    h("summary", {}, l("settings.setup.manual")), field(l("settings.setup.model_id"), manual),
    h("p", { class: "help" }, l("settings.setup.manual_help")));
  advanced.addEventListener("toggle", () => { ui.manual = advanced.open; });
  root.append(h("p", { class: "field-label" }, l("common.model")), selected);
  if (kind.is_cli_agent) { root.append(h("p", { class: "help" }, l("providers.cli_help")), advanced); return root; }

  const search = textField({ type: "search", value: ui.search, "data-setup-search": true, "aria-label": l("settings.setup.search_models"), placeholder: l("settings.setup.search_models"), autocomplete: "off" });
  const status = h("p", { class: "help", role: "status", "aria-live": "polite" });
  const list = h("div", { class: "setup-model-list", role: "group", "aria-label": l("settings.setup.available_models") });
  let loading = false;
  const drawModels = () => {
    const query = search.value.trim().toLocaleLowerCase();
    const matches = known.filter((model) => `${model.name ?? ""} ${model.id}`.toLocaleLowerCase().includes(query));
    list.replaceChildren(...matches.map((model) => {
      const button = h("button", { type: "button", class: "setup-model-option", "data-model-id": model.id, "aria-pressed": String(model.id === choice.model), onclick: () => {
        choice.model = model.id; manual.value = model.id; drawSelected();
        for (const option of list.querySelectorAll<HTMLButtonElement>("[data-model-id]")) option.setAttribute("aria-pressed", String(option.dataset.modelId === model.id));
        change(model.id);
      } }, h("span", { class: "setup-model-name" }, model.name || model.id),
      model.name && model.name !== model.id ? h("span", { class: "setup-model-id" }, model.id) : null);
      return button;
    }));
    if (!loading && retry.hidden) status.textContent = t(matches.length ? "settings.setup.catalog_help" : known.length ? "settings.setup.no_matches" : "settings.setup.no_models");
  };
  search.addEventListener("input", () => { ui.search = search.value; drawModels(); });
  const load = async () => {
    if (loading) return;
    loading = true; retry.hidden = true; status.textContent = t("common.loading");
    try {
      known = await providerModelCatalog(ctx.info, account);
      if (disposed) return;
      status.textContent = t("settings.setup.catalog_help");
    } catch (error) {
      if (disposed) return;
      status.textContent = t("providers.catalog_unavailable", { error: errorText(error) }); retry.hidden = false;
    } finally {
      loading = false;
      if (!disposed) { drawSelected(); drawModels(); if (!retry.hidden) status.textContent = t("settings.setup.catalog_failed"); }
    }
  };
  const retry = h("button", { class: "text-button", hidden: true, "data-setup-catalog-retry": true, onclick: () => { catalogs.invalidate(account.id); void load(); } }, l("providers.reload"));
  root.append(search, list, status, retry, advanced);
  if (kind.requires_key && !ctx.info.keys[account.id]) status.textContent = t("settings.setup.catalog_key");
  else void load();
  return root;
}
