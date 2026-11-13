export async function checkReviewProcessingUI() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) throw Error("Use the owned synthetic fixture page.");
  const { reviewProcessingChoice } = await import("../src/review-processing.ts");
  const { getInterfaceLanguage, setInterfaceLanguage } = await import("../src/i18n.ts");
  const client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client), info = window.__PENNEL_FIXTURE__.info();
  const language = getInterfaceLanguage(), target = document.getElementById("content"), previous = [...target.childNodes];
  const version = "perplexity/pplx-decider-v1.2-20261016";
  const cloudAccount = info.settings.providers.find(item => item.kind === "pennel_cloud");
  if (!cloudAccount) throw Error("The owned Cloud account fixture is missing.");
  info.settings.decision = null; info.cloud_signed_in = true; info.cloud_account_epoch = "owned-account-1";
  let profile = { available: true, reason_code: "available", profile: "supported_passages_v3", model: "pennel-review-v3",
    configured_model: "perplexity/pplx-decider-v1.2", expected_model: version, upstream: "perplexity", endpoint: "https://api.pennel.app/v1/decisions",
    limits: { max_input_bytes: 65536, max_questions: 64, max_batch_windows: 8, request_timeout_secs: 35 } };
  let local = { model_id: "pennel-decider-flash-v1.2-q8_0", display_name: "Decider Flash", available: false, qualified: false,
    state: "not_installed", reason_code: "not_installed", runtime_available: true, download_bytes: 5157837472, downloaded_bytes: 0, configuration: null };
  let presets = [{ provider_kind: "pennel_cloud", qualification_id: "owned-ui-fixture-only", language_policy: "nl", identity_evidence: "provider_reported_version",
    weights_independently_verified: false, actual_model: version, actual_provider: "Perplexity",
    configured: { protocol: "pennel_cloud", model: "pennel-review-v3", expected_model: version, upstream: null,
      endpoint: profile.endpoint, max_input_bytes: 65536, timeout_secs: 90 } }];
  const operations = [], errors = [], navigations = [], results = [];
  let cleanup = [], listeners = [], component, saveFailure = false, downloadFailure = false, presetFailure = false, blockedDownload = false, rejectDownload, pendingProfile;
  const assert = (value, message) => { if (!value) throw Error(message); };
  const settle = async () => { for (let index = 0; index < 4; index++) await new Promise(resolve => setTimeout(resolve, 0)); };
  const changed = () => { for (const listener of [...listeners]) listener(info); };
  const preserved = () => JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm, info.settings.reranking,
    info.settings.summary_automation, info.settings.suggestion_automation, info.settings.suggestion_language]);
  const before = preserved();
  client.invoke = async (command, args) => {
    operations.push({ command, args });
    if (command === "managed_meeting_review_profile") return pendingProfile ? pendingProfile() : structuredClone(profile);
    if (command === "managed_local_decider_status") return structuredClone(local);
    if (command === "qualified_finished_suggestion_presets") { if (presetFailure) throw Error("Owned metadata failure"); return structuredClone(presets); }
    if (command === "set_meeting_review_configuration") {
      assert(JSON.stringify(args.expectedDecision) === JSON.stringify(info.settings.decision), "Save did not carry the acknowledged prior connection.");
      assert(args.expectedCloudAccountEpoch === info.cloud_account_epoch, "Save did not carry its signed-in account epoch.");
      if (saveFailure) throw Error("Owned storage failure");
      info.settings.decision = structuredClone(args.config);
      if (args.expectedProvider === null) {
        assert(!info.settings.providers.some(item => item.kind === "pennel_cloud") && args.config.provider_id === "", "Cloud setup did not request an atomic first account.");
        info.settings.providers.push({ ...cloudAccount, id: "owned-cloud-created" });
        info.settings.decision.provider_id = "owned-cloud-created";
      }
      return structuredClone(info);
    }
    if (command === "download_managed_local_decider") {
      if (downloadFailure) throw Error("Owned download failure");
      if (blockedDownload) { local = { ...local, state: "downloading" }; return new Promise((_, reject) => { rejectDownload = reject; }); }
      local = { ...local, state: "unqualified", reason_code: "unqualified", downloaded_bytes: local.download_bytes };
      return null;
    }
    if (command === "cancel_managed_local_decider_download") {
      local = { ...local, state: "not_installed", reason_code: "download_cancelled" }; rejectDownload?.(new Error("Owned cancellation")); return null;
    }
    if (["begin_insights_analysis", "verify_inference", "inference_decide", "set_key", "download_model"].includes(command)) throw Error("Setup started unintended inference or account/model work.");
    return original(command, args);
  };
  const context = {
    get info() { return info; },
    async mutate(action) { const next = await action(); Object.assign(info, next); changed(); },
    async save() { throw Error("Review setup must use its scoped native writer."); },
    onDispose(action) { cleanup.push(action); }, onInfoChange(listener) { listeners.push(listener); },
    navigate(tab) { navigations.push(tab); }, rerender() {}, report(issue) { errors.push(String(issue)); },
  };
  async function mount() {
    cleanup.forEach(action => action()); cleanup = []; listeners = [];
    component = reviewProcessingChoice(context); target.replaceChildren(component.root); await settle();
  }
  const root = () => component.root;
  const source = name => root().querySelector(`[data-review-processing-source="${name}"]`);
  const save = () => root().querySelector("[data-review-processing-save]");
  const refresh = () => [...root().querySelectorAll("button")].find(item => ["Refresh", "Vernieuwen"].includes(item.textContent.trim()));
  try {
    setInterfaceLanguage("en"); await mount();
    assert(root().textContent.includes("Ready for manual Dutch Review") && !save().disabled, "A genuinely qualified Cloud fixture did not offer setup.");
    assert(info.settings.decision === null && !operations.some(x => x.command === "set_meeting_review_configuration"), "Opening setup assigned a connection.");
    source("local").click(); await settle();
    assert(root().textContent.includes("5.2 GB") && root().querySelector("[data-review-processing-download]").getClientRects().length, "Local setup did not disclose its managed model download.");
    assert(!operations.some(x => x.command === "download_managed_local_decider"), "Selecting local started a download.");
    source("local").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })); await settle();
    assert(source("cloud").getAttribute("aria-checked") === "true" && document.activeElement === source("cloud"), "Source keyboard navigation lost selection or focus.");
    save().click(); await settle();
    assert(info.settings.decision.model === "pennel-review-v3" && info.settings.decision.expected_model === version && root().textContent.includes("Selected for Review"), "Explicit save lost the managed alias/model pin.");
    assert(preserved() === before, "Review setup changed another task or consent policy.");
    results.push("Opening/choosing is metadata-only; explicit scoped save retains the account, model pin, ordinary tasks and consent.");

    const existingProviders = structuredClone(info.settings.providers);
    info.settings.providers = info.settings.providers.filter(item => item.kind !== "pennel_cloud"); info.settings.decision = null;
    await mount(); source("cloud").click();
    assert(!save().disabled && !info.settings.providers.some(item => item.kind === "pennel_cloud"), "A signed-in user was required to add an advanced Cloud connection.");
    save().click(); await settle();
    assert(info.settings.decision.provider_id === "owned-cloud-created" && info.settings.providers.filter(item => item.kind === "pennel_cloud").length === 1 && preserved() === before,
      "First Cloud save failed to create only its own managed account.");
    info.settings.providers = existingProviders;
    results.push("Signed-in users without an advanced Cloud connection get one only when they explicitly save Review setup.");

    info.settings.decision = null; changed(); await mount();
    source("cloud").click();
    info.settings.decision = { provider_id: "external-change", protocol: "mercury", model: "owned-other", expected_model: null, upstream: null, max_input_bytes: 16000, timeout_secs: 30 }; changed(); await settle();
    assert(save().disabled && root().textContent.includes("changed"), "A stale choice can overwrite a concurrent decision edit.");
    const concurrent = JSON.stringify(info.settings.decision); save().click(); await settle();
    assert(JSON.stringify(info.settings.decision) === concurrent, "A stale source changed the current connection.");
    source("cloud").click(); info.cloud_account_epoch = "owned-account-2"; changed(); await settle();
    assert(save().disabled, "Changing signed-in accounts revived a choice for the old account.");
    results.push("Concurrent connection and account changes keep Save closed until a fresh choice.");

    await mount(); source("cloud").click(); presets = []; refresh().click(); await settle();
    assert(save().disabled && root().textContent.includes("not passed Review checks"), "Service metadata granted qualification without an adopted preset.");
    presetFailure = true; refresh().click(); await settle();
    assert(save().disabled && root().textContent.includes("could not be checked"), "A failed metadata read was presented as an unqualified model."); presetFailure = false;
    profile = { ...profile, available: false, reason_code: "processing_region" }; refresh().click(); await settle();
    assert(root().textContent.includes("processing-region policy") && save().disabled, "Blocked Cloud region policy was not explained.");
    info.cloud_signed_in = false; changed(); await settle();
    assert(root().querySelector("[data-review-processing-sign-in]").getClientRects().length && save().disabled, "Signed-out setup did not offer sign-in.");
    root().querySelector("[data-review-processing-sign-in]").click(); assert(navigations.at(-1) === "cloud", "Cloud setup did not use the existing account view.");
    info.cloud_signed_in = true; profile = { ...profile, available: false, reason_code: "not_signed_in" }; refresh().click(); await settle();
    assert(root().querySelector("[data-review-processing-sign-in]").getClientRects().length, "An expired session hid sign-in behind stale credential metadata.");
    results.push("Unqualified, region-blocked and signed-out routes remain unavailable with their relevant action.");

    source("local").click(); downloadFailure = true; root().querySelector("[data-review-processing-download]").click(); await settle();
    assert(root().textContent.includes("Owned download failure") && info.settings.decision.provider_id === "external-change", "Failed local download changed the selected processing destination.");
    downloadFailure = false; blockedDownload = true; root().querySelector("[data-review-processing-download]").click(); await settle();
    root().querySelector("[data-review-processing-cancel]").click(); await settle();
    assert(root().textContent.includes("Download cancelled") && !root().textContent.includes("Owned cancellation") && info.settings.decision.provider_id === "external-change", "Cancelling a local download showed failure or switched destinations.");
    blockedDownload = false; root().querySelector("[data-review-processing-download]").click(); await settle();
    assert(save().disabled && root().textContent.includes("not passed Review checks"), "A downloaded model became qualified without evidence.");
    results.push("Local downloads are explicit; failure and completed-but-unqualified assets never select Cloud or approve inference.");

    info.cloud_signed_in = true; profile = { ...profile, available: true, reason_code: "available" }; presets = [{ provider_kind: "pennel_cloud", qualification_id: "owned-ui-fixture-only", language_policy: "nl", identity_evidence: "provider_reported_version", weights_independently_verified: false, actual_model: version, actual_provider: "Perplexity", configured: { protocol: "pennel_cloud", model: "pennel-review-v3", expected_model: version, upstream: null, endpoint: profile.endpoint, max_input_bytes: 65536, timeout_secs: 90 } }];
    await mount(); source("cloud").click(); saveFailure = true; save().click(); await settle();
    assert(root().textContent.includes("Owned storage failure") && info.settings.decision.provider_id === "external-change", "Failed persistence changed the acknowledged connection.");
    saveFailure = false;
    let finish; pendingProfile = () => new Promise(resolve => { finish = resolve; }); refresh().click(); await settle();
    cleanup.forEach(action => action()); cleanup = []; const detached = root().textContent;
    finish(structuredClone(profile)); await settle();
    assert(root().textContent === detached, "A late metadata response changed a disposed setup.");
    pendingProfile = null; await mount(); source("cloud").click();
    setInterfaceLanguage("nl"); assert(root().textContent.includes("Op deze computer") && root().textContent.includes("Instelling voor Beoordelen bewaren"), "Dutch labels did not update live.");
    root().style.width = "340px"; root().style.maxWidth = "100%";
    root().style.setProperty("--font-body", "26px"); root().style.setProperty("--font-label", "22px");
    await settle();
    const bounds = root().getBoundingClientRect();
    assert(root().scrollWidth <= root().clientWidth + 1, "Review setup overflows a 340px container with enlarged type.");
    for (const control of root().querySelectorAll("button")) {
      const rect = control.getBoundingClientRect();
      if (rect.width && rect.height) assert(rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1, "A Review setup action is clipped at 340px.");
    }
    assert(preserved() === before && errors.length === 0, "A failure path changed other settings or leaked errors.");
    assert(!operations.some(x => ["begin_insights_analysis", "verify_inference", "inference_decide", "set_key", "download_model"].includes(x.command)), "Setup performed inference or ordinary provider/model changes.");
    results.push("Failed persistence, detached metadata, live Dutch labels and 340px enlarged-type geometry preserve ownership and perform no inference.");
    window.__PENNEL_CONTROL_ACCEPTANCE__ = { review_processing: results, mocked_ui_only: true, production_qualification: false };
    return results;
  } finally {
    cleanup.forEach(action => action()); client.invoke = original; setInterfaceLanguage(language); target.replaceChildren(...previous);
  }
}
