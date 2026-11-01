import { liveText, bindLocalization, l, t, localizeBackendText, formatNumber } from "../i18n.ts";
import { api, type ProviderAccount, type ProviderKind } from "../api";
import { controlButton, textField, errorCode, errorText, field, h, menuButton, section } from "../dom";
import { localView } from "./local";
import type { Context } from "../main";
import { VisibilityPoll } from "../window-lifecycle";
import { providerDestination } from "../processing-state";
import { providerVerification } from "../provider-verification";
import type { SetupTask } from "../task-setup-state";

let selected: string | null = null;

export async function providersView(ctx: Context): Promise<HTMLElement> {
  const { settings, kinds, keys } = ctx.info;
  if (ctx.param === "__local__") selected = settings.providers.find((provider) => provider.kind === "local")?.id ?? selected;
  if (!settings.providers.some((provider) => provider.id === selected)) selected = settings.providers[0]?.id ?? null;
  const account = settings.providers.find((provider) => provider.id === selected);
  const localReady = settings.providers.some((provider) => provider.kind === "local")
    && (await api.localModels(null, false)).some((model) => model.state.kind === "downloaded");
  const list = h("div", { class: "list", "aria-label": l("providers.title") }, settings.providers.map((provider) => {
    const kind = kinds.find((item) => item.kind === provider.kind);
    const ready = kind?.availability.status === "available" && (!kind.requires_key || !!keys[provider.id])
      && (!kind.is_cli_agent || !!provider.base_url) && (provider.kind !== "local" || localReady);
    return h("button", { "aria-current": provider.id === selected ? "true" : null, onclick: () => { selected = provider.id; if (ctx.param === "__local__") ctx.navigate("providers"); else ctx.rerender(); } },
      h("span", { class: "grow" }, h("span", { class: "provider-name" }, provider.name),
        h("span", { class: "provider-kind" }, l("providers.kind_status", () => ({ kind: localizeBackendText(kind?.label ?? provider.kind), status: ready ? t("providers.configured") : t("providers.not_configured") })))));
  }));
  const addable = kinds.filter((kind) => !((kind.kind === "local" || kind.kind === "apple_speech" || kind.kind === "apple_intelligence") && settings.providers.some((provider) => provider.kind === kind.kind)));
  const add = menuButton(l("providers.add"), addable.map((kind) => {
    const unsupported = kind.availability.status === "unsupported";
    return {
      label: liveText(() => unsupported && kind.availability.reason ? `${localizeBackendText(kind.label)} — ${localizeBackendText(kind.availability.reason)}` : localizeBackendText(kind.label)),
      disabled: unsupported, title: kind.availability.reason ? liveText(() => localizeBackendText(kind.availability.reason!)) : null,
      run: () => ctx.mutate(async () => {
        const next = await api.addProvider(kind.kind as ProviderKind);
        selected = next.settings.providers.at(-1)?.id ?? null;
        return next;
      }),
    };
  }), "provider-add");
  list.append(add);
  return h("div", {},
    ctx.info.migration_warning ? section(l("providers.api_keys"), null,
      h("p", { class: "help", role: "status" }, liveText(() => localizeBackendText(ctx.info.migration_warning ?? ""))),
      h("p", { class: "help" }, l("providers.migration_help"))) : null,
    h("div", { class: "providers" }, list,
      account ? await detail(ctx, account) : h("p", { class: "muted" }, l("providers.choose"))));
}

export async function providerDetails(ctx: Context, account: ProviderAccount, task?: SetupTask): Promise<HTMLElement> {
  return detail(ctx, account, task);
}

async function detail(ctx: Context, account: ProviderAccount, task?: SetupTask): Promise<HTMLElement> {
  const kind = ctx.info.kinds.find((item) => item.kind === account.kind);
  if (!kind) return section(account.name, l("providers.version_unavailable"));
  const status = h("p", { class: "help", role: "status" });
  let statusText = () => "";
  const showStatus = (text: () => string) => { statusText = text; status.textContent = text(); };
  bindLocalization(status, () => { status.textContent = statusText(); });
  const saveAccount = (key: "name" | "base_url", value: string) => {
    providerVerification.invalidate();
    return ctx.save((settings) => {
      const provider = settings.providers.find((item) => item.id === account.id);
      if (provider) provider[key] = value;
    });
  };
  const saveKey = (value: string) => {
    providerVerification.invalidate();
    return ctx.mutate(() => api.setKey(account.id, value));
  };
  const connectionField = (label: ReturnType<typeof l>, input: HTMLInputElement) => {
    if (!task) {
      input.addEventListener("change", () => { void saveAccount("base_url", input.value.trim()).catch(ctx.report); });
      return field(label, input);
    }
    let saved = account.base_url;
    const save = controlButton({ "data-setup-save": "connection", disabled: true, onclick: async () => {
      save.disabled = true;
      try { await saveAccount("base_url", input.value.trim()); saved = input.value.trim(); ctx.rerender(); }
      catch (error) { ctx.report(error); }
      finally { save.disabled = input.value.trim() === saved; }
    } }, l("settings.setup.save_connection"));
    input.addEventListener("input", () => { save.disabled = input.value.trim() === saved; });
    return field(label, h("div", { class: "setup-connection-field" }, input, save));
  };
  const name = textField({ type: "text", value: account.name, "aria-label": l("common.name") });
  name.addEventListener("change", () => { void saveAccount("name", name.value.trim() || account.name).catch(ctx.report); });
  const body = section(liveText(() => localizeBackendText(kind.label)), null, task ? null : field(l("common.name"), name));
  body.append(h("p", { class: "help" }, l("providers.processing_destination", () => ({ destination: providerDestination(account) }))));
  if (kind.availability.reason) body.append(h("p", { class: "help" }, liveText(() => localizeBackendText(kind.availability.reason ?? ""))));
  if (account.kind === "apple_speech") {
    body.append(h("p", { class: "help" }, l("providers.apple_speech_help")));
    if (kind.availability.status === "permission_required") {
      const speech = (await api.permissions()).find((permission) => permission.kind === "speech");
      let disposed = false;
      const poll = new VisibilityPoll(1000, async () => {
        const next = (await api.permissions()).find((permission) => permission.kind === "speech");
        if (!disposed && next?.status !== speech?.status) await ctx.refresh();
      }, (error) => { poll.dispose(); ctx.report(error); });
      poll.setVisible(ctx.visible, false);
      ctx.onVisibilityChange((visible) => poll.setVisible(visible));
      ctx.onDispose(() => { disposed = true; poll.dispose(); });
      body.append(controlButton({ class: "btn", onclick: async () => { await api.requestPermission("speech"); await ctx.refresh(); } },
        speech?.status === "denied" ? l("providers.open_settings") : l("providers.grant_access")));
    }
    if (!task) body.append(controlButton({ class: "btn", disabled: kind.availability.status !== "available", onclick: () => ctx.save((settings) => {
      settings.dictation_stt = { provider_id: account.id, model: kind.default_stt_model };
      settings.meeting_stt = { provider_id: account.id, model: kind.default_stt_model };
    }) }, l("providers.use_speech")));
  } else if (account.kind === "apple_intelligence") {
    body.append(h("p", { class: "help" }, l("providers.apple_text_help")));
    const test = controlButton({ class: "btn", disabled: kind.availability.status !== "available", onclick: async () => {
      test.disabled = true; showStatus(() => t("providers.testing"));
      try { await api.testAppleIntelligence(); showStatus(() => t("providers.apple_verified")); }
      catch (error) { showStatus(() => t("providers.test_failed", { error: errorText(error) })); }
      test.disabled = false;
    } }, l("providers.test_model"));
    body.append(h("div", { class: "row" },
      task ? null : controlButton({ class: "btn", disabled: kind.availability.status !== "available", onclick: () => ctx.save((settings) => {
        settings.llm = { provider_id: account.id, model: kind.default_llm_model };
      }) }, l("providers.use_text")),
      test,
      controlButton({ class: "btn", onclick: () => ctx.refresh().catch(ctx.report) }, l("providers.check_availability"))), status);
  } else if (kind.is_cli_agent) {
    body.append(h("p", { class: "help" }, l("providers.cli_help")));
    const executable = textField({ type: "text", value: account.base_url, placeholder: l("providers.cli_path"), "aria-label": l("providers.executable") });
    const detect = controlButton({ class: "btn", onclick: async () => {
      detect.disabled = true;
      try {
        const found = (await api.detectCliAgents()).find((agent) => agent.kind === account.kind);
        if (!found) showStatus(() => t("providers.cli_missing"));
        else { await saveAccount("base_url", found.path); }
      } catch (error) { showStatus(() => errorText(error)); }
      detect.disabled = false;
    } }, l("providers.find_cli"));
    const check = controlButton({ class: "btn", onclick: async () => {
      check.disabled = true; showStatus(() => t("providers.testing"));
      try { await api.checkCliAgent(account.id, kind.default_llm_model); showStatus(() => t("providers.verified")); }
      catch (error) { showStatus(() => t("providers.test_failed", { error: errorText(error) })); }
      check.disabled = false;
    } }, l("providers.test_cli"));
    body.append(connectionField(l("providers.executable"), executable), h("div", { class: "row" }, detect, check), status);
  } else if (account.kind === "pennel_cloud") {
    body.append(await cloudCard(ctx, status, showStatus));
  } else if (account.kind !== "local") {
    const baseUrl = textField({ type: "text", value: account.base_url, placeholder: kind.default_base_url || "https://…/v1", "aria-label": l("providers.base_url") });
    body.append(connectionField(l("providers.base_url"), baseUrl));
    if (kind.requires_key || account.kind === "custom") {
      const key = textField({ type: "password", placeholder: ctx.info.keys[account.id] ? l("providers.saved_key") : l("providers.paste_key"), autocomplete: "off", class: "grow", "aria-label": l("providers.api_key") });
      if (!task) key.addEventListener("change", () => { void saveKey(key.value).catch(ctx.report); });
      const save = task ? controlButton({ class: "btn", "data-setup-save": "key", disabled: true, onclick: async () => {
        if (!key.value) return;
        save!.disabled = true;
        try { await saveKey(key.value); key.value = ""; ctx.rerender(); }
        catch (error) { ctx.report(error); }
        finally { save!.disabled = !key.value; }
      } }, l("common.save")) : null;
      key.addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); if (save) save.click(); else key.blur(); } });
      if (save) key.addEventListener("input", () => { save.disabled = !key.value; });
      body.append(field(l("providers.api_key"), h("div", { class: "row" }, key,
        save,
        ctx.info.keys[account.id] ? controlButton({ class: "btn", onclick: () => saveKey("").catch(ctx.report) }, l("providers.clear_key")) : null)));
    }
    const test = controlButton({ class: "btn", onclick: async () => {
      test.disabled = true; showStatus(() => t("providers.testing"));
      try { const models = await api.providerModels(account.id); showStatus(() => t("providers.connection_verified", { count: formatNumber(models.length) })); }
      catch (error) { showStatus(() => t("providers.test_failed", { error: errorText(error) })); }
      test.disabled = false;
    } }, l("providers.test_connection"));
    body.append(h("div", { class: "row" }, status, h("span", { class: "grow" }), test));
  }
  if (task) return body;
  body.append(h("p", { class: "help" }, l("providers.models_help")),
    h("button", { class: "text-button", onclick: () => ctx.navigate("models") }, l("providers.open_models")));
  return h("div", {}, body, account.kind === "local" ? await localView(ctx, true) : null,
    h("div", { class: "row" }, h("span", { class: "grow" }), controlButton({ class: "btn danger", disabled: ctx.info.settings.providers.length < 2,
      onclick: () => ctx.mutate(() => api.removeProvider(account.id)),
    }, l("providers.remove"))));
}

async function cloudCard(ctx: Context, status: HTMLElement, showStatus: (text: () => string) => void): Promise<HTMLElement> {
  try {
    const { email } = await api.cloudAccount();
    return h("p", { role: "status" }, email ? l("providers.signed_in_as", () => ({ email })) : l("providers.signed_in"));
  } catch (error) {
    if (errorCode(error) !== "not_signed_in") {
      showStatus(() => errorText(error));
      return status;
    }
    return h("div", {}, h("p", {}, l("providers.cloud_sign_in")),
      h("div", { class: "row" }, controlButton({ class: "btn primary", onclick: () => ctx.navigate("account") }, l("providers.cloud_sign_in"))));
  }
}
