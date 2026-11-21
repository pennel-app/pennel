import { bindLocalization, liveText, l, t, uiLocale, formatNumber, localizeBackendText, type LocalizedText } from "../i18n.ts";
import { localModelDisplayName, localModelLanguageName } from "../settings-presentation.ts";
import { api, listen, type LocalModel } from "../api";
import { controlButton, confirmDialog, errorText, field, h, isCancellation, section, select } from "../dom";
import type { Context } from "../main";

const progress = new Map<string, { downloaded: number; total: number }>();
const failures = new Map<string, string>();
const attached = new Set<string>();
let language: string | null = null;
let languageInitialized = false;
let supportedOnly = true;
let listening = false;
let refresh: (() => void) | null = null;

async function listenOnce() {
  if (listening) return;
  listening = true;
  await listen<{ id: string; downloaded: number; total: number }>("download-progress", (e) => {
    const appeared = !progress.has(e.payload.id);
    progress.set(e.payload.id, e.payload);
    const bar = document.querySelector<HTMLProgressElement>(`progress[data-model="${CSS.escape(e.payload.id)}"]`);
    if (bar) {
      bar.max = e.payload.total;
      bar.value = e.payload.downloaded;
    }
    if (appeared) refresh?.();
  });
  await listen<{ id: string; error: string | null }>("download-done", (e) => {
    progress.delete(e.payload.id);
    attached.delete(e.payload.id);
    if (e.payload.error && !isCancellation(e.payload.error)) failures.set(e.payload.id, e.payload.error);
    else failures.delete(e.payload.id);
    refresh?.();
  });
}

function bar(label: string | LocalizedText, value: number | null) {
  return h("div", { class: "model-score" },
    h("div", { class: "model-score-label" }, h("span", {}, label), h("span", {}, value === null ? l("providers.local.not_measured") : formatNumber(value))),
    h("div", { class: "model-score-track" }, h("span", { style: `width:${value ?? 0}%` })));
}


export async function localView(ctx: Context, embedded = false): Promise<HTMLElement> {
  if (!languageInitialized) { language = ctx.info.settings.language ?? "nl"; languageInitialized = true; }
  await listenOnce();
  const models = await api.localModels(language, supportedOnly);
  for (const id of ctx.info.active_model_downloads ?? []) {
    if (attached.has(id)) continue;
    attached.add(id);
    const model = models.find((candidate) => candidate.id === id);
    progress.set(id, { downloaded: model?.state.kind === "partial" ? model.state.bytes : 0, total: model?.size_bytes ?? 0 });
    void api.joinModelDownload(id).then(async (joined) => {
      if (joined) return;
      attached.delete(id); progress.delete(id);
      if (ctx.visible) await ctx.refresh();
    }).catch((error) => {
      attached.delete(id); progress.delete(id);
      if (!isCancellation(error)) failures.set(id, errorText(error));
      ctx.rerender();
    });
  }
  const inUse = new Set(
    [ctx.info.settings.dictation_stt, ctx.info.settings.meeting_stt]
      .filter((c) => ctx.info.settings.providers.find((p) => p.id === c.provider_id)?.kind === "local")
      .map((c) => c.model),
  );
  const localId = ctx.info.settings.providers.find((p) => p.kind === "local")?.id;

  const use = (model: LocalModel) =>
    ctx.save((s) => {
      if (!localId) return;
      const linked = s.meeting_stt.provider_id === s.dictation_stt.provider_id && s.meeting_stt.model === s.dictation_stt.model;
      s.dictation_stt = { provider_id: localId, model: model.id };
      if (linked || s.meeting_stt.provider_id === localId) {
        s.meeting_stt = { provider_id: localId, model: model.id };
      }
    });

  const unsupported = ctx.info.capabilities.local_models.status === "unsupported" ? ctx.info.capabilities.local_models.reason : null;
  const card = (model: LocalModel) => {
    const active = progress.get(model.id);
    let actions: (Node | string)[];
    if (active) {
      actions = [
        controlButton({ class: "btn", onclick: () => api.cancelDownload(model.id) }, l("common.cancel")),
      ];
    } else if (model.state.kind === "downloaded") {
      actions = [
        ...inUse.has(model.id) ? [] : [controlButton({ class: "btn", disabled: !!unsupported, onclick: () => use(model) }, l("providers.local.use"))],
        h(
          "button",
          {
            class: "btn danger",
            onclick: async () => {
              if (inUse.has(model.id) && !await confirmDialog(l("providers.local.in_use_delete"), l("providers.local.in_use_delete_help"))) return;
              await ctx.mutate(() => api.deleteModel(model.id));
            },
          },
          l("providers.local.delete"),
        ),
      ];
    } else {
      const start = async (autoUse = false) => {
        failures.delete(model.id);
        progress.set(model.id, { downloaded: 0, total: model.size_bytes });
        try {
          await api.downloadModel(model.id);
          if (autoUse && localId) await use(model);
        } catch (error) { progress.delete(model.id); failures.set(model.id, errorText(error)); }
        ctx.rerender();
      };
      actions =
        model.state.kind === "partial"
          ? [
              controlButton({ class: "btn", disabled: !!unsupported, onclick: () => void start(false) }, l("providers.local.resume")),
              controlButton({ class: "btn danger", onclick: async () => await ctx.mutate(() => api.deleteModel(model.id)) }, l("settings.template.delete")),
            ]
          : [
              controlButton({ class: "btn primary", disabled: !!unsupported, onclick: () => void start(true) }, l("settings.ai.download_and_use")),
              controlButton({ class: "btn", disabled: !!unsupported, onclick: () => void start(false) }, l("providers.local.download")),
            ];
    }
    const inUseHere = inUse.has(model.id);
    const unmeasuredVariant = model.id === "whisper-large-v3-turbo-q5_0";
    const metadata = liveText(() => [localizeBackendText(model.description), `${formatNumber(Math.floor(model.size_bytes / 1e6))} MB`, model.parameters ? t("providers.local.parameters", { count: model.parameters }) : null].filter(Boolean).join(" · "));
    const languages = () => [...model.languages].map(localModelLanguageName).sort((a, b) => a.localeCompare(b, uiLocale()));
    const languageDetails = model.languages.length ? h("details", { class: "model-languages" }, h("summary", {}), h("p", {})) : null;
    if (languageDetails) {
      const update = () => {
        const names = languages();
        languageDetails.querySelector("summary")!.textContent = languageDetails.open ? t("providers.local.hide_languages")
          : names.length === 1 ? t("providers.local.one_language", { language: names[0] }) : t("providers.local.languages", { count: formatNumber(names.length) });
        languageDetails.querySelector("p")!.textContent = names.join(", ");
      };
      languageDetails.addEventListener("toggle", update);
      update(); bindLocalization(languageDetails, update);
    }
    return h("div", { class: "model-card" },
      h("div", { class: "model-card-heading" }, h("div", { class: "grow" },
        h("div", { class: "row" }, h("strong", {}, localModelDisplayName(model)), inUseHere || model.recommended ? h("span", { class: "model-badge" }, inUseHere ? l("providers.local.in_use") : l("providers.local.recommended")) : null),
        h("p", { class: "model-details" }, metadata)),
        h("div", { class: "row model-actions" }, actions)),
      h("div", { class: "model-scores" }, bar(l("providers.local.speed"), unmeasuredVariant ? null : model.speed), bar(l("providers.local.accuracy"), unmeasuredVariant ? null : model.accuracy)), languageDetails,
      active ? h("progress", { "data-model": model.id, max: active.total || 1, value: active.downloaded, "aria-label": l("providers.local.downloading_model", () => ({ model: String(localModelDisplayName(model)) })) }) : null,
      failures.has(model.id) ? h("div", { class: "error" }, l("providers.local.download_failed", () => ({ error: errorText(failures.get(model.id)) }))) : null);
  };

  const languagePicker = select<string>(
    [
      ["nl", liveText(() => localModelLanguageName("nl"))],
      ["en", liveText(() => localModelLanguageName("en"))],
      ["fr", liveText(() => localModelLanguageName("fr"))],
      ["de", liveText(() => localModelLanguageName("de"))],
      ["es", liveText(() => localModelLanguageName("es"))],
      ["it", liveText(() => localModelLanguageName("it"))],
      ["pt", liveText(() => localModelLanguageName("pt"))],
      ["pl", liveText(() => localModelLanguageName("pl"))],
    ],
    language ?? "",
    (value) => {
      language = value || null;
      ctx.rerender();
    },
    "menu",
  );
  languagePicker.setAttribute("aria-label", t("providers.local.accuracy_for"));
  bindLocalization(languagePicker, () => languagePicker.setAttribute("aria-label", t("providers.local.accuracy_for")));
  const onlyThis = h("input", {
    type: "checkbox",
    role: "switch",
    checked: supportedOnly,
    onchange: () => {
      supportedOnly = onlyThis.checked;
      ctx.rerender();
    },
  });

  const prominent = (model: LocalModel) => model.recommended || model.id === "whisper-large-v3-turbo-q5_0" || model.state.kind === "downloaded";
  const root = h(
    "div",
    {},
    embedded ? null : h("h1", {}, l("providers.local.title")),
    section(
      l("providers.local.models"),
      l("providers.local.offline_help"),
      unsupported ? h("p", { class: "help" }, unsupported) : null,
      h("p", { class: "help" }, l("providers.local.text_help")),
      h("div", { class: "row model-filters" }, h("div", { class: "model-language" }, h("span", { "aria-hidden": "true" }, l("providers.local.accuracy_for")), languagePicker), h("label", { class: "row" }, onlyThis, l("providers.local.only_language"))),
      h("p", { class: "help model-benchmark-note" }, l("providers.local.benchmark_help")),
      models.filter(prominent).map(card),
      h("details", { class: "all-models" }, h("summary", {}, l("providers.local.all_models", () => ({ count: formatNumber(models.filter((model) => !prominent(model)).length) }))), models.filter((model) => !prominent(model)).map(card)),
      field(
        l("providers.local.release_after"),
        select<string>(
          [
            ["1", "1 min"],
            ["5", "5 min"],
            ["15", "15 min"],
            ["60", l("providers.local.hour")],
          ],
          String(ctx.info.settings.local_unload_minutes),
          (v) => ctx.save((s) => (s.local_unload_minutes = Number(v))),
        ),
      ),
      h("p", { class: "help" }, l("providers.local.unload_help")),
    ),
  );
  const onRefresh = () => { if (root.isConnected) void ctx.refresh().catch(ctx.report); };
  refresh = onRefresh;
  ctx.onDispose(() => { if (refresh === onRefresh) refresh = null; });
  return root;
}
