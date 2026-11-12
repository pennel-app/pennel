import { api, type FinishedSuggestionPreset, type InferenceAvailability, type InferenceCapability, type InferenceConfig, type InferenceProtocolInfo } from "../api";
import { errorText, field, h, section, select } from "../dom";
import { bindLocalization, getInterfaceLanguage, liveText, localizeBackendText } from "../i18n.ts";
import { inferenceConfig, inferenceFingerprint, inferenceProtocolsForProvider, InferenceVerificationState, testedSuggestionDraft } from "../inference-state.ts";
import type { Context } from "../main";
import { providerDestination } from "../processing-state";

const copy = {
  experimental: ["Experimental connections — setup and testing only", "Experimentele verbindingen — alleen instellen en testen"],
  experimental_help: ["These optional connections are for setup and sample testing. Pennel does not currently use them for dictation, meeting processing or search.", "Deze optionele verbindingen zijn bedoeld om in te stellen en met een voorbeeld te testen. Pennel gebruikt ze momenteel niet voor dicteren, het verwerken van vergaderingen of zoeken."],
  experimental_review: ["Experimental connections", "Experimentele verbindingen"],
  experimental_review_help: ["A qualified Decision setup can be used for manual Dutch Review. Other connections remain available for setup and sample testing.", "Een goedgekeurde beslissingsverbinding kan worden gebruikt om handmatig Nederlandse meetings te beoordelen. Andere verbindingen blijven beschikbaar om in te stellen en met een voorbeeld te testen."],
  tested_setup: ["Tested Review setup", "Geteste instelling voor Beoordelen"],
  use_tested: ["Use tested setup", "Geteste instelling gebruiken"],
  tested_help: ["Fill the tested model and limits, then save. Your selected connection and its API key are reused.", "Vul het geteste model en de limieten in en bewaar daarna. Je gekozen verbinding en de bijbehorende API-sleutel worden hergebruikt."],
  tested_need_router: ["Choose an existing OpenRouter connection to use this setup.", "Kies een bestaande OpenRouter-verbinding om deze instelling te gebruiken."],
  tested_add_router: ["Add OpenRouter connection", "OpenRouter-verbinding toevoegen"],
  tested_details: ["Verification details", "Verificatiegegevens"],
  tested_identity: ["Manual Dutch Review. The provider reports this exact model version; its weights are not independently verified. Automatic analysis needs separate qualification.", "Handmatig beoordelen van Nederlandse meetings. De provider meldt deze exacte modelversie; de modelgewichten zijn niet onafhankelijk gecontroleerd. Automatische analyse vereist een aparte goedkeuring."],
  tested_changed: ["This tested setup is no longer available for this connection. Refresh the settings and choose again.", "Deze geteste instelling is niet meer beschikbaar voor deze verbinding. Vernieuw de instellingen en kies opnieuw."],
  tested_unavailable: ["No tested manual Review setup is available on this computer yet.", "Er is nog geen geteste instelling om op deze computer handmatig meetings te beoordelen."],
  title_decision: ["Decision processing", "Beslissingen verwerken"],
  title_reranking: ["Passage ranking", "Passages rangschikken"],
  help_decision: ["Configure and test a service for structured choices.", "Stel een dienst voor gestructureerde keuzes in en test deze."],
  help_reranking: ["Configure and test a service that scores the relevance of text passages.", "Stel een dienst in die de relevantie van tekstpassages beoordeelt en test deze."],
  provider: ["Provider", "Provider"], format: ["API format", "API-indeling"], model: ["Model", "Model"],
  expected: ["Expected returned model", "Verwacht antwoordmodel"], upstream: ["Upstream provider", "Uitvoerende provider"],
  bytes: ["Max input bytes", "Maximale invoerbytes"], timeout: ["Timeout (seconds)", "Time-out (seconden)"],
  advanced: ["Advanced", "Geavanceerd"], save: ["Save configuration", "Configuratie bewaren"], disable: ["Disable", "Uitschakelen"],
  test: ["Test with sample", "Test met voorbeeld"], cancel: ["Cancel", "Annuleren"], testing: ["Processing the sample…", "Voorbeeld verwerken…"],
  passed: ["The sample test passed.", "De voorbeeldtest is geslaagd."], failed: ["The sample test could not be completed.", "De voorbeeldtest kon niet worden uitgevoerd."],
  disabled: ["Not configured", "Niet ingesteld"], configured: ["Configured; run the sample to verify.", "Ingesteld; test met het voorbeeld om te controleren."],
  checking: ["Checking this connection…", "Deze verbinding controleren…"], retry: ["Retry", "Opnieuw proberen"],
  check_failed: ["This connection could not be checked. Try again.", "Deze verbinding kon niet worden gecontroleerd. Probeer het opnieuw."],
  unavailable: ["Experimental connections are unavailable in this build.", "Experimentele verbindingen zijn niet beschikbaar in deze versie."],
  manage: ["Manage providers", "Providers beheren"], add: ["Add connection", "Verbinding toevoegen"],
  need_provider: ["Use Pennel Cloud, or add an OpenRouter or Custom server connection under Providers.", "Gebruik Pennel Cloud of voeg een OpenRouter- of aangepaste serververbinding toe bij Providers."],
  need_rerank_provider: ["Use Pennel Cloud, or add a Custom server connection for TEI under Providers.", "Gebruik Pennel Cloud of voeg een aangepaste serververbinding voor TEI toe bij Providers."],
  managed: ["Managed by Pennel Cloud. No API key or model ID is needed.", "Beheerd door Pennel Cloud. Je hebt geen API-sleutel of model-ID nodig."],
  cloud_sign_in: ["Sign in to Pennel Cloud before testing. Your other task settings stay the same.", "Meld je aan bij Pennel Cloud voor je test. De instellingen voor je andere taken blijven hetzelfde."],
  sign_in: ["Sign in to Pennel Cloud", "Aanmelden bij Pennel Cloud"], account: ["Account", "Account"],
  cloud_sample: ["The test sends only bundled synthetic text to Pennel Cloud and its configured model service. It uses your Cloud allowance.", "De test stuurt alleen meegeleverde fictieve tekst naar Pennel Cloud en de ingestelde modeldienst. Dit gebruikt je Cloud-tegoed."],
  sample: ["The test sends only synthetic example text to the selected server. It may incur provider charges.", "De test stuurt alleen fictieve voorbeeldtekst naar de gekozen server. De provider kan hiervoor kosten aanrekenen."],
  missing: ["Choose a provider, API format and model. OpenRouter also requires an upstream provider.", "Kies een provider, API-indeling en model. OpenRouter vereist ook een uitvoerende provider."],
  budgets: ["Use an input limit of 1–65,536 bytes and a timeout of 1–90 seconds.", "Gebruik een invoerlimiet van 1–65.536 bytes en een time-out van 1–90 seconden."],
  unsaved: ["Save these changes before testing.", "Bewaar deze wijzigingen voor je test."],
  missing_key: ["Add the API key to this connection under Providers before testing.", "Voeg de API-sleutel toe aan deze verbinding bij Providers voor je test."],
  destination: ["Server", "Server"], actual_model: ["Returned model", "Antwoordmodel"], actual_provider: ["Returned provider", "Antwoordprovider"],
  revision: ["Returned revision", "Antwoordrevisie"],
  request: ["Request ID", "Aanvraag-ID"], unknown: ["Not reported", "Niet gerapporteerd"],
} as const;
type CopyKey = keyof typeof copy;
const text = (key: CopyKey) => copy[key][getInterfaceLanguage() === "nl" ? 1 : 0];
const label = (key: CopyKey) => liveText(() => text(key));
let experimentalOpen = false;

export async function inferenceModelsView(ctx: Context): Promise<HTMLElement[]> {
  const [protocols, candidates] = await Promise.all([api.inferenceProtocols().catch(() => []), api.qualifiedFinishedSuggestionPresets().catch(() => [])]);
  const presets = Array.isArray(candidates) ? candidates.filter(item => item && item.configured && item.identity_evidence === "provider_reported_version" && item.weights_independently_verified === false) : [];
  const supported = Array.isArray(protocols) ? protocols.filter((item) => Array.isArray(item.allowed_provider_kinds) && typeof item.managed === "boolean") : [];
  let renderRequested = false;
  const rerender = () => { if (!renderRequested) { renderRequested = true; queueMicrotask(() => ctx.rerender()); } };
  const sections = await Promise.all((["decision", "reranking"] as const).map((capability) => inferenceSection(ctx, capability, supported, rerender, capability === "decision" ? presets : [])));
  const root = h("details", { class: "experimental-connections", "data-inference-experimental": true, open: experimentalOpen },
    h("summary", {}, label(presets.length ? "experimental_review" : "experimental")), h("p", { class: "help" }, label(presets.length ? "experimental_review_help" : "experimental_help")), sections);
  root.addEventListener("toggle", () => { if (root.isConnected) experimentalOpen = root.open; });
  return [root];
}

async function inferenceSection(ctx: Context, capability: InferenceCapability, protocols: InferenceProtocolInfo[], rerender: () => void, presets: FinishedSuggestionPreset[]): Promise<HTMLElement> {
  const availableProtocols = protocols.filter((item) => item[capability]);
  const eligible = ctx.info.settings.providers.filter((account) => inferenceProtocolsForProvider(protocols, account, capability).length);
  const saved = inferenceConfig(ctx.info, capability);
  const first = eligible.find((account) => account.id === saved?.provider_id) ?? eligible.find((account) => account.kind === "pennel_cloud") ?? eligible[0];
  const providerProtocols = (id: string) => {
    const account = eligible.find((item) => item.id === id);
    return account ? inferenceProtocolsForProvider(protocols, account, capability) : [];
  };
  const newDraft = (id: string): InferenceConfig => {
    const choices = providerProtocols(id);
    const protocol = choices.find((item) => item.managed)?.protocol ?? choices.find((item) => item.protocol === "type_safe")?.protocol ?? choices[0]?.protocol ?? (capability === "reranking" ? "tei" : "type_safe");
    return { provider_id: id, protocol, model: protocol === "pennel_cloud" ? "pennel-auto" : "", upstream: null, expected_model: null, max_input_bytes: 16_000, timeout_secs: 30 };
  };
  const draft = saved ? structuredClone(saved) : newDraft(first?.id ?? "");
  const providerDrafts = new Map<string, InferenceConfig>();
  const state = new InferenceVerificationState();
  let disposed = false;
  let dirty = false;
  let saving = false;
  let applyingPreset = false;
  let selectedPreset = presets[0]?.qualification_id ?? "";
  let errorMessage: string | null = null;
  let fingerprint = inferenceFingerprint(ctx.info, capability);
  let accountState = JSON.stringify([ctx.info.cloud_account_epoch, ctx.info.cloud_signed_in, ctx.info.settings.providers]);
  let configurationState = JSON.stringify(inferenceConfig(ctx.info, capability));
  let providerState = JSON.stringify(ctx.info.settings.providers);
  let availability: InferenceAvailability | null = null;
  let availabilityGeneration = 0;
  let checkingAvailability = false;
  let availabilityError: string | null = null;
  const notice = h("p", { class: "help", role: "status", "aria-live": "polite" });
  const status = h("p", { class: "help", role: "status" });
  const identity = h("div", { class: "help" });
  const destination = h("p", { class: "help" });
  const managedHelp = h("p", { class: "help", "data-inference-managed": true }, label("managed"));
  const sampleHelp = h("p", { class: "help" });
  const accountAction = h("button", { class: "btn", onclick: () => ctx.navigate("account") });
  const retryAvailability = h("button", { class: "btn", onclick: () => { void checkAvailability(); } }, label("retry"));
  const cancelRun = () => {
    const id = state.invalidate();
    if (id) void api.cancelInference(id).catch(() => {});
  };
  const changed = () => { dirty = true; errorMessage = null; cancelRun(); notice.textContent = ""; draw(); };
  const provider = select(eligible.map((item) => [item.id, item.name]), draft.provider_id, (id) => {
    providerDrafts.set(draft.provider_id, structuredClone(draft));
    Object.assign(draft, providerDrafts.get(id) ?? newDraft(id));
    model.value = draft.model; expected.value = draft.expected_model ?? ""; upstream.value = draft.upstream ?? "";
    bytes.value = String(draft.max_input_bytes); timeout.value = String(draft.timeout_secs);
    protocol.value = draft.protocol;
    updateProtocolOptions(); changed();
  }, "menu");
  const protocol = select(availableProtocols.map((item) => [item.protocol, item.name]), draft.protocol, (value) => {
    draft.protocol = value;
    if (!value.startsWith("open_router")) { draft.upstream = null; upstream.value = ""; }
    changed();
  }, "menu");
  provider.dataset.inferenceField = "provider";
  protocol.dataset.inferenceField = "protocol";
  function updateProtocolOptions() {
    const allowed = providerProtocols(draft.provider_id);
    for (const option of protocol.options) option.disabled = !allowed.some((item) => item.protocol === option.value);
  }
  updateProtocolOptions();
  const input = (value: string, change: (value: string) => void, numeric = false) => {
    const control = h("input", { type: numeric ? "number" : "text", value });
    control.addEventListener("input", () => { change(control.value); changed(); });
    return control;
  };
  const model = input(draft.model, (value) => { draft.model = value; });
  const expected = input(draft.expected_model ?? "", (value) => { draft.expected_model = value.trim() || null; });
  const upstream = input(draft.upstream ?? "", (value) => { draft.upstream = value.trim() || null; });
  const upstreamField = field(label("upstream"), upstream);
  const modelField = field(label("model"), model);
  const protocolField = field(label("format"), protocol);
  const expectedField = field(label("expected"), expected);
  model.required = true;
  const bytes = input(String(draft.max_input_bytes), (value) => { draft.max_input_bytes = Number(value); }, true);
  const timeout = input(String(draft.timeout_secs), (value) => { draft.timeout_secs = Number(value); }, true);
  for (const [name, control] of [["model", model], ["expected_model", expected], ["upstream", upstream], ["max_input_bytes", bytes], ["timeout_secs", timeout]] as const) control.dataset.inferenceField = name;
  bytes.min = "1"; bytes.max = "65536"; bytes.step = "1";
  timeout.min = "1"; timeout.max = "90"; timeout.step = "1";
  const tested = select(presets.map(preset => [preset.qualification_id, `${preset.actual_provider} · ${preset.actual_model}`]), selectedPreset,
    value => { selectedPreset = value; draw(); }, "menu");
  const testedHelp = h("p", { class: "help" });
  const testedDetails = h("details", { "data-tested-suggestion-details": true }, h("summary", {}, label("tested_details")), h("p", { class: "help" }, label("tested_identity")),
    h("p", { class: "help" }, presets.map(preset => `${preset.actual_provider} · ${preset.actual_model}`).join("\n")));
  const testedApply = h("button", { class: "btn", "data-tested-suggestion-apply": true, onclick: async () => {
    if (saving || applyingPreset || state.running) return;
    const id = draft.provider_id, requested = selectedPreset, previousDraft = JSON.stringify(draft);
    const shown = presets.find(item => item.qualification_id === requested);
    applyingPreset = true; errorMessage = null; draw();
    try {
      const fresh = await api.qualifiedFinishedSuggestionPresets();
      if (disposed || draft.provider_id !== id || selectedPreset !== requested || JSON.stringify(draft) !== previousDraft) return;
      const preset = fresh.find(item => item.qualification_id === requested), account = ctx.info.settings.providers.find(item => item.id === id);
      const next = preset && shown && JSON.stringify(preset) === JSON.stringify(shown) && account ? testedSuggestionDraft(preset, account) : null;
      if (!next || !providerProtocols(id).some(item => item.protocol === next.protocol)) throw new Error(text("tested_changed"));
      Object.assign(draft, next);
      model.value = draft.model; expected.value = draft.expected_model ?? ""; upstream.value = draft.upstream ?? "";
      bytes.value = String(draft.max_input_bytes); timeout.value = String(draft.timeout_secs); protocol.value = draft.protocol;
      updateProtocolOptions(); changed();
    } catch (error) { if (!disposed) errorMessage = errorText(error); }
    finally { applyingPreset = false; if (!disposed) draw(); }
  } }, label("use_tested"));
  const testedAdd = h("button", { class: "text-button", onclick: () => ctx.navigate("providers") }, label("tested_add_router"));
  const testedSetup = capability === "decision" ? h("div", { "data-tested-suggestion-setup": true },
    presets.length ? field(label("tested_setup"), tested) : h("h3", {}, label("tested_setup")), testedHelp,
    h("div", { class: "row" }, testedApply, testedAdd), presets.length ? testedDetails : null) : null;
  const save = h("button", { class: "btn", onclick: async () => {
    if (saving) return;
    if (!draft.provider_id || !draft.model.trim() || !providerProtocols(draft.provider_id).some((item) => item.protocol === draft.protocol) || (draft.protocol.startsWith("open_router") && !draft.upstream?.trim())) { errorMessage = text("missing"); draw(); return; }
    if (!Number.isInteger(draft.max_input_bytes) || draft.max_input_bytes < 1 || draft.max_input_bytes > 65536 || !Number.isInteger(draft.timeout_secs) || draft.timeout_secs < 1 || draft.timeout_secs > 90) { errorMessage = text("budgets"); draw(); return; }
    errorMessage = null; saving = true; draw();
    try {
      cancelRun();
      const config = { ...draft, model: draft.model.trim(), ...(draft.protocol === "pennel_cloud" ? { model: "pennel-auto", expected_model: null, upstream: null } : {}) };
      await ctx.save((settings) => { settings[capability] = config; });
      if (!disposed) { dirty = false; rerender(); }
    } catch (error) { if (!disposed) errorMessage = errorText(error); }
    finally { saving = false; if (!disposed) draw(); }
  } }, label("save"));
  const disable = h("button", { class: "btn", onclick: async () => {
    if (saving) return;
    errorMessage = null; saving = true; draw();
    cancelRun();
    try { await ctx.save((settings) => { settings[capability] = null; }); if (!disposed) { dirty = false; rerender(); } }
    catch (error) { if (!disposed) errorMessage = errorText(error); }
    finally { saving = false; if (!disposed) draw(); }
  } }, label("disable"));
  const test = h("button", { class: "btn", onclick: async () => {
    const id = crypto.randomUUID();
    const current = inferenceFingerprint(ctx.info, capability);
    if (!state.begin(id, current)) return;
    errorMessage = null;
    draw();
    try {
      const result = await api.verifyInference(capability, id);
      if (!disposed && state.finish(id, inferenceFingerprint(ctx.info, capability), result)) draw();
    } catch (error) {
      if (!disposed && state.finish(id, inferenceFingerprint(ctx.info, capability), { verified: false, message: errorText(error), identity: null })) draw();
    }
  } }, label("test"));
  const cancel = h("button", { class: "btn", onclick: () => { cancelRun(); errorMessage = null; notice.textContent = ""; draw(); } }, label("cancel"));
  function draw() {
    const current = inferenceConfig(ctx.info, capability);
    const result = state.get(inferenceFingerprint(ctx.info, capability));
    const account = ctx.info.settings.providers.find((item) => item.id === draft.provider_id);
    const managed = draft.protocol === "pennel_cloud" && account?.kind === "pennel_cloud";
    const preset = presets.find(item => item.qualification_id === selectedPreset);
    const compatible = !!preset && !!account && !!testedSuggestionDraft(preset, account) && providerProtocols(account.id).some(item => item.protocol === preset.configured.protocol);
    tested.disabled = saving || state.running || applyingPreset || !presets.length;
    testedApply.disabled = saving || state.running || applyingPreset || !compatible;
    testedHelp.textContent = text(!presets.length ? "tested_unavailable" : compatible ? "tested_help" : "tested_need_router");
    testedAdd.hidden = !presets.length || presets.some(preset => ctx.info.settings.providers.some(account => !!testedSuggestionDraft(preset, account)));
    const signedIn = ctx.info.cloud_signed_in === true;
    const keyMissing = !!current && protocols.find((item) => item.protocol === current.protocol)?.requires_key && !ctx.info.keys[current.provider_id];
    status.textContent = !protocols.length ? text("unavailable") : managed && !signedIn ? text("cloud_sign_in") : !current ? text("disabled") : keyMissing ? text("missing_key") : checkingAvailability ? text("checking") : availability?.available ? text("configured") : localizeBackendText(availabilityError ?? availability?.reason ?? text("check_failed"));
    let endpoint = !dirty ? availability?.endpoint : null;
    if (managed && endpoint) {
      try { endpoint = new URL(endpoint).host || null; } catch { endpoint = null; }
    }
    destination.textContent = `${text("destination")}: ${managed ? "Pennel Cloud" : providerDestination(account)}${endpoint ? ` · ${endpoint}` : ""}`;
    const matchesSaved = current && (["provider_id", "protocol", "model", "expected_model", "upstream", "max_input_bytes", "timeout_secs"] as const).every((key) => current[key] === draft[key]);
    test.disabled = saving || applyingPreset || dirty || state.running || !matchesSaved || !availability?.available || !!keyMissing || managed && !signedIn;
    cancel.hidden = !state.running;
    disable.disabled = saving || applyingPreset || !current;
    save.disabled = saving || applyingPreset || state.running || !eligible.length || !availableProtocols.length;
    provider.disabled = saving || applyingPreset || !eligible.length || !protocols.length;
    protocol.disabled = saving || applyingPreset || !availableProtocols.length || managed;
    for (const control of [model, expected, upstream, bytes, timeout]) control.disabled = saving || applyingPreset || !eligible.length || !protocols.length;
    modelField.hidden = protocolField.hidden = expectedField.hidden = managed;
    model.disabled ||= managed; expected.disabled ||= managed;
    managedHelp.hidden = accountAction.hidden = !managed;
    accountAction.disabled = saving || applyingPreset || state.running;
    accountAction.textContent = text(signedIn ? "account" : "sign_in");
    retryAvailability.hidden = !current || !!availability?.available || checkingAvailability || !!keyMissing || managed && !signedIn;
    retryAvailability.disabled = saving || state.running;
    sampleHelp.textContent = text(managed ? "cloud_sample" : "sample");
    const needsUpstream = draft.protocol.startsWith("open_router");
    upstreamField.hidden = !needsUpstream;
    upstream.required = needsUpstream;
    upstream.disabled ||= !needsUpstream;
    notice.className = errorMessage || (result && !result.verified) ? "error" : "help";
    if (errorMessage) notice.textContent = errorMessage;
    else if (state.running) notice.textContent = text("testing");
    else if (result) notice.textContent = result.verified ? text("passed") : localizeBackendText(result.message) || text("failed");
    else if (dirty) notice.textContent = text("unsaved");
    else notice.textContent = "";
    identity.replaceChildren();
    if (result?.identity) {
      for (const [key, value] of [["actual_model", result.identity.actual_model], ["actual_provider", result.identity.actual_provider], ["revision", result.identity.actual_revision], ["request", result.identity.request_id]] as const) identity.append(h("p", {}, `${text(key)}: ${value ?? text("unknown")}`));
    }
  }
  const advanced = h("details", {}, h("summary", {}, label("advanced")), expectedField, field(label("bytes"), bytes), field(label("timeout"), timeout));
  const body = eligible.length && availableProtocols.length
    ? [field(label("provider"), provider), testedSetup, protocolField, modelField, upstreamField, managedHelp, advanced, destination, status,
      h("div", { class: "row" }, accountAction, retryAvailability, h("button", { class: "text-button", onclick: () => ctx.navigate("providers") }, label("manage"))),
      h("div", { class: "row" }, save, disable), sampleHelp, h("div", { class: "row" }, test, cancel), notice, identity]
    : [testedSetup, status, protocols.length && !eligible.length ? h("p", { class: "help" }, label(capability === "reranking" ? "need_rerank_provider" : "need_provider")) : null,
      protocols.length && !eligible.length ? h("button", { class: "btn", onclick: () => ctx.navigate("providers") }, label("add")) : null,
      saved ? h("div", { class: "row" }, disable) : null, notice];
  const root = section(label(capability === "decision" ? "title_decision" : "title_reranking"), label(capability === "decision" ? "help_decision" : "help_reranking"), body);
  root.dataset.inferenceCapability = capability;
  ctx.onDispose(() => { disposed = true; cancelRun(); });
  ctx.onInfoChange(() => {
    const next = inferenceFingerprint(ctx.info, capability);
    const nextAccountState = JSON.stringify([ctx.info.cloud_account_epoch, ctx.info.cloud_signed_in, ctx.info.settings.providers]);
    if (next === fingerprint && nextAccountState === accountState) return;
    const nextConfigurationState = JSON.stringify(inferenceConfig(ctx.info, capability));
    const nextProviderState = JSON.stringify(ctx.info.settings.providers);
    const replaceControls = nextConfigurationState !== configurationState || nextProviderState !== providerState;
    configurationState = nextConfigurationState; providerState = nextProviderState;
    fingerprint = next; accountState = nextAccountState; cancelRun();
    availabilityGeneration++; checkingAvailability = false; availabilityError = null;
    availability = null;
    draw();
    if (!dirty && replaceControls) { rerender(); return; }
    void checkAvailability();
  });
  async function checkAvailability() {
    if (!protocols.length || !inferenceConfig(ctx.info, capability)) return;
    const generation = ++availabilityGeneration;
    const captured = inferenceFingerprint(ctx.info, capability);
    const current = () => !disposed && generation === availabilityGeneration && captured === inferenceFingerprint(ctx.info, capability);
    checkingAvailability = true; availabilityError = null; availability = null; draw();
    try {
      const value = await api.inferenceAvailability(capability);
      if (current()) availability = value;
    } catch (error) { if (current()) availabilityError = errorText(error); }
    finally { if (current()) { checkingAvailability = false; draw(); } }
  }
  bindLocalization(root, draw);
  draw();
  void checkAvailability();
  return root;
}
