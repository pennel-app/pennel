import { api, type FinishedSuggestionPreset, type InferenceConfig, type ManagedLocalDeciderStatus, type ManagedMeetingReviewProfile } from "./api";
import { confirmDialog, controlButton, errorText, h, section } from "./dom";
import { bindLocalization, getInterfaceLanguage, liveText, localizeBackendText } from "./i18n";
import type { Context } from "./main";
import { cloudReviewDraft, reviewProcessingSource, validManagedReviewProfile, type ReviewProcessingSource } from "./review-processing-state.ts";

const copy = {
  title: ["Review processing", "Verwerking voor Beoordelen"],
  help: ["Choose where Pennel looks for possible actions, decisions and open questions in your transcript.", "Kies waar Pennel mogelijke acties, beslissingen en open vragen in je transcript zoekt."],
  cloud: ["Pennel Cloud", "Pennel Cloud"],
  cloud_help: ["Use your Pennel account and Cloud allowance. No separate provider key is needed.", "Gebruik je Pennel-account en Cloud-tegoed. Je hebt geen aparte providersleutel nodig."],
  local: ["On this computer", "Op deze computer"],
  local_help: ["Pennel manages the Decider model. Transcript analysis stays on this computer.", "Pennel beheert het Decider-model. Het transcript wordt op deze computer beoordeeld."],
  advanced: ["Other providers…", "Andere providers…"],
  advanced_help: ["Keep using your own connection under Advanced settings.", "Blijf je eigen verbinding gebruiken via de geavanceerde instellingen."],
  sign_in: ["Sign in to Pennel Cloud", "Aanmelden bij Pennel Cloud"],
  checking: ["Checking availability…", "Beschikbaarheid controleren…"],
  unavailable: ["Availability could not be checked. Refresh and try again.", "De beschikbaarheid kon niet worden gecontroleerd. Vernieuw en probeer opnieuw."],
  disabled: ["Cloud Review is not available yet.", "Beoordelen via Cloud is nog niet beschikbaar."],
  unconfigured: ["Cloud Review needs setup. Your other Cloud features remain available.", "Beoordelen via Cloud moet nog worden ingesteld. Je andere Cloud-functies blijven beschikbaar."],
  region: ["This setup is unavailable under your account’s processing-region policy.", "Deze instelling is niet beschikbaar volgens het regiobeleid van je account."],
  qualification: ["This setup has not passed Review checks for this computer yet.", "Deze instelling is nog niet goedgekeurd voor Beoordelen op deze computer."],
  ready: ["Ready for manual Dutch Review.", "Klaar om Nederlandse meetings handmatig te beoordelen."],
  local_missing: ["Download Decider before using local Review.", "Download Decider om lokaal te beoordelen."],
  local_unsupported: ["Local Review is not supported by this build.", "Lokaal beoordelen wordt niet ondersteund door deze versie."],
  local_verifying: ["Verifying the Decider download…", "De Decider-download controleren…"],
  local_error: ["The local model could not be prepared. Try the download again.", "Het lokale model kon niet worden voorbereid. Probeer de download opnieuw."],
  download: ["Download Decider", "Decider downloaden"],
  download_size: ["Decider download: {size} GB.", "Decider-download: {size} GB."],
  remove: ["Remove Decider", "Decider verwijderen"],
  remove_help: ["Remove the local model download? Your meetings, notes and current processing choice are kept.", "De lokale modeldownload verwijderen? Je meetings, notities en huidige verwerkingskeuze blijven behouden."],
  cancel: ["Cancel download", "Download annuleren"],
  downloading: ["Downloading Decider…", "Decider downloaden…"],
  cancelled: ["Download cancelled. You can resume it later.", "Download geannuleerd. Je kunt deze later hervatten."],
  save: ["Save Review setup", "Instelling voor Beoordelen bewaren"],
  saved: ["Selected for Review", "Gekozen voor Beoordelen"],
  draft: ["Save to use this setup. Selecting a source does not start analysis.", "Bewaar om deze instelling te gebruiken. Een bron kiezen start geen beoordeling."],
  changed: ["Review setup or account changed. Refresh and choose again.", "De instelling of het account voor Beoordelen is gewijzigd. Vernieuw en kies opnieuw."],
  refresh: ["Refresh", "Vernieuwen"],
  automatic: ["Automatic analysis needs its own checks and consent. Choosing a setup does not enable it.", "Automatisch beoordelen vereist aparte controles en toestemming. Een instelling kiezen schakelt dit niet in."],
  destination: ["Processing destination", "Verwerkingslocatie"],
  cloud_destination: ["Pennel Cloud · Perplexity through OpenRouter", "Pennel Cloud · Perplexity via OpenRouter"],
} as const;
type CopyKey = keyof typeof copy;
const text = (key: CopyKey) => copy[key][getInterfaceLanguage() === "nl" ? 1 : 0];
const label = (key: CopyKey) => liveText(() => text(key));
const same = (left: InferenceConfig | null | undefined, right: InferenceConfig | null | undefined) => (!left && !right) || !!left && !!right
  && (["provider_id", "protocol", "model", "expected_model", "upstream", "max_input_bytes", "timeout_secs"] as const).every(key => left[key] === right[key]);

export function reviewProcessingChoice(ctx: Context) {
  let active = true, loading = true, pending = false, downloading = false, cancelling = false, dirty = false, conflicted = false, presetsFailed = false, failure = "", generation = 0;
  let source: ReviewProcessingSource = reviewProcessingSource(ctx.info.settings.decision, ctx.info.settings.providers) ?? "cloud";
  let baseline = structuredClone(ctx.info.settings.decision ?? null), epoch = ctx.info.cloud_account_epoch ?? null;
  let cloud: ManagedMeetingReviewProfile | null = null, local: ManagedLocalDeciderStatus | null = null, presets: FinishedSuggestionPreset[] = [];
  let poll: ReturnType<typeof setTimeout> | undefined;
  const status = h("p", { class: "help review-processing-status", role: "status" });
  const destination = h("p", { class: "help review-processing-destination" });
  const size = h("p", { class: "help" });
  const notice = h("p", { class: "help" });
  const error = h("p", { class: "error", role: "status", hidden: true });
  const choose = (value: ReviewProcessingSource) => {
    if (pending) return;
    source = value; dirty = true; conflicted = false; failure = "";
    baseline = structuredClone(ctx.info.settings.decision ?? null); epoch = ctx.info.cloud_account_epoch ?? null;
    draw();
  };
  const option = (value: "cloud" | "local") => controlButton({ class: "review-processing-option", role: "radio", "data-review-processing-source": value,
    onclick: () => choose(value) }, h("strong", {}, label(value)), h("span", { class: "help" }, label(value === "cloud" ? "cloud_help" : "local_help")));
  const cloudOption = option("cloud"), localOption = option("local");
  const signIn = controlButton({ class: "btn", "data-review-processing-sign-in": true, onclick: () => ctx.navigate("cloud") }, label("sign_in"));
  const download = controlButton({ class: "btn", "data-review-processing-download": true, onclick: () => { void acquire(); } }, label("download"));
  const cancel = controlButton({ class: "btn", "data-review-processing-cancel": true, onclick: async () => {
    cancelling = true;
    try { await api.cancelManagedLocalDeciderDownload(); await load(); }
    catch (issue) { failure = localizeBackendText(errorText(issue)); cancelling = false; draw(); }
  } }, label("cancel"));
  const remove = controlButton({ class: "text-button", onclick: async () => {
    if (pending || !await confirmDialog(label("remove"), label("remove_help"), label("remove"))) return;
    pending = true; failure = ""; draw();
    try { await api.removeManagedLocalDecider(); await load(); }
    catch (issue) { failure = localizeBackendText(errorText(issue)); }
    finally { pending = false; if (active) draw(); }
  } }, label("remove"));
  const save = controlButton({ class: "btn primary", "data-review-processing-save": true, onclick: () => { void commit(); } }, label("save"));
  const refresh = controlButton({ class: "btn", onclick: () => { baseline = structuredClone(ctx.info.settings.decision ?? null); epoch = ctx.info.cloud_account_epoch ?? null; conflicted = false; failure = ""; void load(); } }, label("refresh"));
  const root = section(label("title"), label("help"),
    h("div", { class: "review-processing-options", role: "radiogroup", "aria-label": label("title") }, cloudOption, localOption),
    status, destination, size, h("div", { class: "row review-processing-actions" }, signIn, download, cancel, save, refresh, remove), notice, error,
    h("p", { class: "help review-processing-automatic" }, label("automatic")),
    h("details", { class: "review-processing-advanced", open: source === "advanced" }, h("summary", {}, label("advanced")), h("p", { class: "help" }, label("advanced_help")),
      controlButton({ class: "text-button", onclick: () => ctx.navigate("models") }, label("advanced"))));
  root.classList.add("review-processing");
  root.dataset.reviewProcessing = source;
  function candidate(): InferenceConfig | null {
    if (source === "cloud") return ctx.info.cloud_signed_in ? cloudReviewDraft(cloud, presets, ctx.info.settings.providers.find(item => item.kind === "pennel_cloud")) : null;
    if (source === "local" && local?.available && local.qualified && local.configuration && ctx.info.settings.providers.some(item => item.id === local!.configuration!.provider_id && item.kind === "local")) return local.configuration;
    return null;
  }
  function draw() {
    root.dataset.reviewProcessing = source;
    for (const [value, button] of [["cloud", cloudOption], ["local", localOption]] as const) {
      button.setAttribute("aria-checked", String(source === value)); button.disabled = pending;
      button.tabIndex = source === value || source === "advanced" && value === "cloud" ? 0 : -1;
    }
    const next = candidate(), signedIn = ctx.info.cloud_signed_in === true && cloud?.reason_code !== "not_signed_in";
    let message: CopyKey;
    if (loading) message = "checking";
    else if (source === "cloud") message = !signedIn ? "sign_in" : !cloud || presetsFailed || cloud.available && !validManagedReviewProfile(cloud) ? "unavailable" : !cloud.available
      ? cloud.reason_code === "processing_region" ? "region" : cloud.reason_code === "unconfigured" ? "unconfigured" : "disabled" : next ? "ready" : "qualification";
    else if (source === "local") {
      if (!local) message = "unavailable";
      else if (local.state === "unsupported" || !local.runtime_available) message = "local_unsupported";
      else if (local.state === "downloading" || downloading) message = "downloading";
      else if (local.state === "verifying") message = "local_verifying";
      else if (["cancelled", "download_cancelled"].includes(local.reason_code)) message = "cancelled";
      else if (local.state === "error") message = "local_error";
      else if (!local.available) message = local.state === "not_installed" ? "local_missing" : "qualification";
      else message = next ? "ready" : "qualification";
    }
    else message = "advanced_help";
    status.textContent = text(message);
    if (source === "local" && local && (local.state === "downloading" || downloading) && local.download_bytes > 0) status.textContent += ` ${Math.min(100, Math.floor(local.downloaded_bytes / local.download_bytes * 100))}%`;
    destination.hidden = source === "advanced";
    destination.textContent = `${text("destination")}: ${text(source === "cloud" ? "cloud_destination" : "local")}`;
    size.hidden = source !== "local" || !local?.download_bytes;
    size.textContent = text("download_size").replace("{size}", new Intl.NumberFormat(getInterfaceLanguage(), { maximumFractionDigits: 1 }).format((local?.download_bytes ?? 0) / 1e9));
    signIn.hidden = source !== "cloud" || signedIn;
    download.hidden = source !== "local" || !local?.runtime_available || ["downloading", "verifying", "ready", "unqualified"].includes(local.state);
    download.disabled = pending || downloading || loading;
    cancel.hidden = source !== "local" || !(downloading || local?.state === "downloading"); cancel.disabled = pending || cancelling;
    remove.hidden = source !== "local" || !local?.downloaded_bytes || downloading || ["downloading", "verifying"].includes(local.state); remove.disabled = pending || loading;
    save.hidden = source === "advanced"; save.disabled = pending || loading || conflicted || !next || same(ctx.info.settings.decision, next);
    refresh.disabled = pending || loading;
    notice.textContent = conflicted ? text("changed") : next && same(ctx.info.settings.decision, next) ? text("saved") : dirty ? text("draft") : "";
    error.textContent = failure; error.hidden = !failure;
  }
  async function load() {
    const ticket = ++generation, accountEpoch = ctx.info.cloud_account_epoch;
    clearTimeout(poll);
    loading = true; cloud = null; presets = []; presetsFailed = false; draw();
    const values = await Promise.allSettled([api.managedMeetingReviewProfile(), api.managedLocalDeciderStatus(), api.qualifiedFinishedSuggestionPresets()]);
    if (!active || ticket !== generation || accountEpoch !== ctx.info.cloud_account_epoch) return;
    cloud = values[0].status === "fulfilled" ? values[0].value : null;
    local = values[1].status === "fulfilled" ? values[1].value : null;
    presets = values[2].status === "fulfilled" && Array.isArray(values[2].value) ? values[2].value : [];
    presetsFailed = values[2].status !== "fulfilled" || !Array.isArray(values[2].value);
    loading = false; draw();
    scheduleLocalPoll(ticket);
  }
  function scheduleLocalPoll(ticket: number) {
    clearTimeout(poll);
    if (!active || !(downloading || local && ["downloading", "verifying"].includes(local.state))) return;
    poll = setTimeout(async () => {
      try {
        const status = await api.managedLocalDeciderStatus();
        if (!active || ticket !== generation) return;
        local = status; draw();
      } catch { /* A failed status read never changes the selected processing destination. */ }
      if (active && ticket === generation) scheduleLocalPoll(ticket);
    }, 750);
  }
  async function acquire() {
    if (pending || downloading) return;
    failure = ""; downloading = true; draw(); scheduleLocalPoll(generation);
    try { await api.downloadManagedLocalDecider(); await load(); }
    catch (issue) { if (!cancelling) failure = localizeBackendText(errorText(issue)); }
    finally { downloading = false; cancelling = false; if (active) draw(); }
  }
  async function commit() {
    if (pending || conflicted || !candidate()) return;
    const chosen = source, expected = structuredClone(baseline), expectedEpoch = epoch;
    pending = true; failure = ""; draw();
    try {
      await load();
      if (!active || chosen !== source || !same(ctx.info.settings.decision, expected) || (chosen === "cloud" && (ctx.info.cloud_account_epoch ?? null) !== expectedEpoch)) throw new Error(text("changed"));
      const config = candidate(), account = ctx.info.settings.providers.find(item => item.id === config?.provider_id);
      if (!config || !account && chosen !== "cloud") throw new Error(text("changed"));
      await ctx.mutate(() => api.setMeetingReviewConfiguration(config, expected, chosen === "cloud" ? expectedEpoch : null, account ? structuredClone(account) : null));
      if (!active) return;
      dirty = false; conflicted = false; baseline = structuredClone(ctx.info.settings.decision ?? null); epoch = ctx.info.cloud_account_epoch ?? null;
    } catch (issue) { if (active) failure = localizeBackendText(errorText(issue)); }
    finally { pending = false; if (active) draw(); }
  }
  let infoSignature = JSON.stringify([ctx.info.cloud_account_epoch, ctx.info.cloud_signed_in, ctx.info.settings.providers]);
  for (const button of [cloudOption, localOption]) button.addEventListener("keydown", event => {
    if (pending || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === "Home" ? "cloud" : event.key === "End" ? "local" : source === "cloud" ? "local" : "cloud";
    choose(next); (next === "cloud" ? cloudOption : localOption).focus();
  });
  ctx.onInfoChange(() => {
    if (!active) return;
    if (dirty && (!same(ctx.info.settings.decision, baseline) || source === "cloud" && (ctx.info.cloud_account_epoch ?? null) !== epoch)) conflicted = true;
    if (!dirty) { source = reviewProcessingSource(ctx.info.settings.decision, ctx.info.settings.providers) ?? source; baseline = structuredClone(ctx.info.settings.decision ?? null); epoch = ctx.info.cloud_account_epoch ?? null; }
    const next = JSON.stringify([ctx.info.cloud_account_epoch, ctx.info.cloud_signed_in, ctx.info.settings.providers]);
    if (next !== infoSignature) { infoSignature = next; void load(); } else draw();
  });
  ctx.onDispose(() => { active = false; generation++; clearTimeout(poll); });
  bindLocalization(root, draw); draw(); void load();
  return { root };
}
