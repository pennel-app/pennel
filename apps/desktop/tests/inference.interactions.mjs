export async function checkInferenceUI() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) throw new Error("Use the owned synthetic fixture page.");
  const { inferenceModelsView } = await import("../src/views/inference.ts");
  const { setInterfaceLanguage, getInterfaceLanguage } = await import("../src/i18n.ts");
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const info = window.__PENNEL_FIXTURE__.info();
  info.settings.providers.push({ id: "typed-fixture", name: "Synthetic decision server", kind: "custom", base_url: "https://decision.example/v1" });
  info.keys["typed-fixture"] = true;
  info.provider_revisions = { "typed-fixture": 1 };
  info.settings.decision = null; info.settings.reranking = null;
  const originalChoices = JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]);
  const operations = [];
  const protocols = [
    { protocol: "type_safe", name: "TypeSafe System One", default_base_url: "https://api.typesafe.ai/v1", decision: true, reranking: false, extraction: false, requires_key: true, allowed_provider_kinds: ["custom"], managed: false },
    { protocol: "open_router_decisions", name: "OpenRouter Decisions", default_base_url: "https://openrouter.ai/api/v1", decision: true, reranking: false, extraction: false, requires_key: true, allowed_provider_kinds: ["custom", "open_router"], managed: false },
    { protocol: "tei", name: "TEI", default_base_url: "http://127.0.0.1:8080", decision: false, reranking: true, extraction: false, requires_key: false, allowed_provider_kinds: ["custom"], managed: false },
    { protocol: "pennel_cloud", name: "Pennel Cloud", default_base_url: "https://api.pennel.app", decision: true, reranking: true, extraction: false, requires_key: false, allowed_provider_kinds: ["pennel_cloud"], managed: true },
  ];
  const identity = { protocol: "type_safe", endpoint: "https://decision.example/v1/systemone", requested_model: "fixture-model", requested_upstream: null, actual_model: "fixture-model-v1", actual_provider: "Fixture", actual_revision: "revision-1", request_id: "fixture-request" };
  let responder = async () => ({ verified: true, message: "Sample passed", identity });
  let cloudService = true;
  let availabilityResponder = null;
  const availabilityCalls = [];
  client.invoke = async (command, args) => {
    if (command === "inference_protocols") return protocols;
    if (command === "inference_availability") {
      availabilityCalls.push(args.capability);
      const config = info.settings[args.capability];
      const cloud = config?.protocol === "pennel_cloud";
      const value = { capability: args.capability, configured: !!config, available: !!config && (!cloud || cloudService && info.cloud_signed_in), reason: config ? cloud && !cloudService ? "Pennel Cloud models are temporarily unavailable." : "Configured" : "Not configured", provider_id: config?.provider_id ?? null, model: config?.model ?? null, protocol: config?.protocol ?? null, endpoint: config ? cloud ? `https://api.pennel.app/v1/${args.capability === "decision" ? "decisions" : "rerank"}` : identity.endpoint : null };
      return availabilityResponder ? availabilityResponder(args, value) : value;
    }
    if (command === "verify_inference") { operations.push({ command, ...args }); return responder(args); }
    if (command === "cancel_inference") { operations.push({ command, ...args }); return true; }
    return original(command, args);
  };
  const target = document.getElementById("content");
  const previous = [...target.childNodes];
  const previousLanguage = getInterfaceLanguage();
  let cleanup = [], listeners = [], pendingMount = Promise.resolve();
  const errors = [], navigations = [];
  let saveFailure = null;
  const context = {
    get info() { return info; },
    async save(change) { if (saveFailure) throw saveFailure; change(info.settings); for (const listener of [...listeners]) listener(info); },
    onDispose(action) { cleanup.push(action); },
    onInfoChange(listener) { listeners.push(listener); },
    rerender() { pendingMount = pendingMount.then(mount); },
    navigate(route) { navigations.push(route); }, report(error) { errors.push(String(error)); },
  };
  async function mount() {
    cleanup.forEach(action => action()); cleanup = []; listeners = [];
    target.replaceChildren(...await inferenceModelsView(context));
  }
  const settle = async () => { await new Promise(resolve => setTimeout(resolve, 0)); await pendingMount; await new Promise(resolve => setTimeout(resolve, 0)); };
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const decision = () => target.querySelector('[data-inference-capability="decision"]');
  const experimental = () => target.querySelector("details[data-inference-experimental]");
  const button = name => [...decision().querySelectorAll("button")].find(item => item.textContent.trim() === name);
  const reranking = () => target.querySelector('[data-inference-capability="reranking"]');
  const rerankButton = name => [...reranking().querySelectorAll("button")].find(item => item.textContent.trim() === name);
  const results = [];
  try {
    setInterfaceLanguage("en");
    const configuredProviders = info.settings.providers;
    info.settings.providers = configuredProviders.filter(account => !["custom", "open_router"].includes(account.kind));
    await mount();
    assert(experimental() && !experimental().open && experimental().querySelector("summary").textContent === "Experimental connections — setup and testing only", "Experimental setup is not collapsed and clearly labelled by default.");
    assert(experimental().textContent.includes("does not currently use them"), "Experimental connections imply active product processing.");
    experimental().open = true;
    assert(!decision().querySelector("input,.choice") && button("Add connection"), "Missing provider shows a blank form instead of an actionable empty state.");
    button("Add connection").click();
    assert(navigations.at(-1) === "providers" && operations.length === 0, "Adding a connection did not open setup, or started inference.");
    results.push("Experimental connections start collapsed and empty setup offers a labelled action without blank controls.");
    info.settings.providers = configuredProviders;
    await settle(); await mount(); experimental().open = true; await settle();
    assert(operations.length === 0 && decision().textContent.includes("Not configured"), "Mounting enabled or tested an optional connection.");
    const model = decision().querySelector('[data-inference-field="model"]'); model.value = "fixture-model"; model.dispatchEvent(new Event("input", { bubbles: true }));
    button("Save configuration").click(); await settle();
    assert(experimental().open, "Saving collapses the experimental setup the user is working in.");
    assert(info.settings.decision?.model === "fixture-model", "Explicit configuration was not saved.");
    assert(decision().textContent.includes(identity.endpoint), "Custom setup lost its explicit configured endpoint.");
    assert(JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]) === originalChoices, "Existing STT/LLM selections changed.");
    assert(operations.length === 0, "Saving configuration ran inference automatically.");
    results.push("Optional configuration saves atomically without changing or invoking existing tasks.");
    button("Test with sample").click(); await settle();
    assert(decision().textContent.includes("The sample test passed.") && decision().textContent.includes("fixture-model-v1") && decision().textContent.includes("revision-1"), "Sample outcome or actual identity missing.");
    results.push("Explicit sample verification exposes actual identity and revision.");
    let finish;
    responder = () => new Promise(resolve => { finish = resolve; });
    button("Test with sample").click(); await settle();
    info.provider_revisions["typed-fixture"]++;
    for (const listener of [...listeners]) listener(info);
    await settle(); finish({ verified: true, message: "obsolete", identity: { ...identity, request_id: "obsolete-result" } }); await settle();
    assert(!decision().textContent.includes("The sample test passed.") && !decision().textContent.includes("obsolete-result") && operations.some(item => item.command === "cancel_inference"), "A late response verified changed credentials.");
    results.push("Credential changes cancel verification and suppress stale outcomes.");
    button("Test with sample").click(); await settle();
    button("Cancel").click(); await settle();
    assert(!decision().textContent.includes("Processing the sample…"), "Cancelled verification retained an active-processing message.");
    finish({ verified: true, message: "late", identity }); await settle();
    assert(!decision().textContent.includes("The sample test passed."), "Cancelled verification produced a late success.");
    results.push("Cancelling discards an already-running sample result.");
    const edited = decision().querySelector('[data-inference-field="model"]'); edited.value = "not-saved"; edited.dispatchEvent(new Event("input", { bubbles: true }));
    assert(button("Test with sample").disabled && info.settings.decision.model === "fixture-model", "Unsaved input changed the selected provider or remained testable.");
    saveFailure = new Error("Synthetic storage failure"); button("Save configuration").click(); await settle();
    assert(decision().textContent.includes("Synthetic storage failure") && info.settings.decision.model === "fixture-model", "A failed save lost its explanation or replaced the acknowledged configuration.");
    saveFailure = null;
    setInterfaceLanguage("nl");
    assert(decision().textContent.includes("Beslissingen verwerken") && button("Configuratie bewaren"), "Dutch presentation did not relabel the controls.");
    setInterfaceLanguage("en");
    button("Disable").click(); await settle();
    assert(info.settings.decision === null && JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]) === originalChoices, "Disabling changed another task or selected a fallback.");
    assert(errors.length === 0, errors.join("\n"));
    results.push("Unsaved changes, live EN/NL labels and disabling preserve task ownership.");
    info.settings.providers.push({ id: "router-fixture", name: "Synthetic OpenRouter connection", kind: "open_router", base_url: "https://openrouter.example/api/v1" });
    info.keys["router-fixture"] = true;
    info.settings.decision = { provider_id: "router-fixture", protocol: "open_router_decisions", model: "fixture-model", upstream: null, expected_model: null, max_input_bytes: 16000, timeout_secs: 30 };
    await mount();
    const upstream = decision().querySelector('[data-inference-field="upstream"]');
    assert(upstream?.required && !upstream.closest(".field").hidden && upstream.closest("details") === experimental(), "Required OpenRouter route is hidden in Advanced.");
    button("Save configuration").click(); await settle();
    assert(decision().textContent.includes("OpenRouter also requires an upstream provider."), "Missing required OpenRouter route has no useful validation.");
    upstream.value = "Fixture upstream"; upstream.dispatchEvent(new Event("input", { bubbles: true }));
    button("Save configuration").click(); await settle();
    assert(info.settings.decision?.upstream === "Fixture upstream" && JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]) === originalChoices, "Saving an explicit route changed ordinary processing.");
    results.push("Required OpenRouter upstream is visible outside Advanced and validated before saving.");

    info.settings.providers = [{ id: "cloud-fixture", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "https://untrusted-provider-field.example/v1" }, configuredProviders.find(account => account.id === "typed-fixture")];
    info.settings.decision = info.settings.reranking = null;
    info.cloud_account_epoch = "signed-out"; info.cloud_signed_in = false;
    await mount(); experimental().open = true;
    assert(decision().querySelector('[data-inference-managed]') && reranking().querySelector('[data-inference-managed]'), "Cloud is not available for both experimental capabilities.");
    for (const root of [decision(), reranking()]) {
      for (const name of ["model", "protocol", "expected_model", "upstream"]) assert(root.querySelector(`[data-inference-field="${name}"]`).closest(".field").hidden, `Managed Cloud exposes editable ${name}.`);
      assert(root.textContent.includes("Sign in to Pennel Cloud before testing.") && !root.textContent.includes("untrusted-provider-field.example"), "Cloud sign-in or trusted destination is missing.");
    }
    assert(info.settings.decision === null && info.settings.reranking === null, "Opening Cloud setup enabled an experimental connection.");
    const pickProvider = async (name) => {
      decision().querySelector('[data-inference-field="provider"]').click(); await settle();
      const option = [...document.querySelectorAll('[role="option"]')].find(item => item.textContent.trim() === name);
      assert(option && !option.disabled, `Provider ${name} is unavailable.`); option.click(); await settle();
    };
    const providerControl = decision().querySelector('[data-inference-field="provider"]');
    providerControl.focus(); providerControl.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })); await settle();
    const menu = document.querySelector('[role="listbox"]');
    assert(menu && menu.contains(document.activeElement), "Keyboard navigation cannot open the managed provider choice.");
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await settle();
    assert(document.activeElement === providerControl && !document.querySelector('[role="listbox"]'), "Escape does not restore focus to the provider choice.");
    await pickProvider("Synthetic decision server");
    const manualDraft = decision().querySelector('[data-inference-field="model"]');
    assert(!manualDraft.closest(".field").hidden, "Choosing a custom connection hides its required model control.");
    manualDraft.value = "unsaved-manual-model"; manualDraft.dispatchEvent(new Event("input", { bubbles: true }));
    const pickProtocol = async (name) => {
      decision().querySelector('[data-inference-field="protocol"]').click(); await settle();
      [...document.querySelectorAll('[role="option"]')].find(item => item.textContent.trim() === name).click(); await settle();
    };
    await pickProtocol("OpenRouter Decisions");
    const manualUpstream = decision().querySelector('[data-inference-field="upstream"]');
    assert(!manualUpstream.closest(".field").hidden && manualUpstream.required, "A custom OpenRouter protocol hides its required route.");
    manualUpstream.value = "fixture-route"; manualUpstream.dispatchEvent(new Event("input", { bubbles: true }));
    await pickProtocol("TypeSafe System One");
    assert(manualUpstream.closest(".field").hidden && manualUpstream.value === "", "Changing protocols leaves an unsupported hidden OpenRouter route.");
    await pickProvider("Pennel Cloud"); await pickProvider("Synthetic decision server");
    assert(decision().querySelector('[data-inference-field="model"]').value === "unsaved-manual-model", "Browsing Cloud lost the existing custom model draft.");
    await pickProvider("Pennel Cloud");
    assert(info.settings.decision === null && info.settings.reranking === null, "Browsing providers saved or activated a connection.");
    results.push("Keyboard provider selection restores focus and browsing Cloud preserves unsaved custom drafts without activating them.");
    button("Sign in to Pennel Cloud").click();
    assert(navigations.at(-1) === "account", "Cloud sign-in did not open the existing account flow.");
    button("Save configuration").click(); await settle();
    assert(info.settings.decision?.protocol === "pennel_cloud" && info.settings.decision.model === "pennel-auto" && info.settings.decision.expected_model === null && info.settings.decision.upstream === null, "Managed Cloud did not save its fixed model contract.");
    assert(button("Test with sample").disabled, "Signed-out Cloud can test a saved connection.");
    results.push("Both connections support keyless managed Cloud with an explicit existing-account sign-in prerequisite and trusted destination.");

    info.cloud_account_epoch = "account-1"; info.cloud_signed_in = true;
    let releaseOldAvailability;
    availabilityResponder = (args, value) => {
      if (args.capability === "decision" && !releaseOldAvailability) return new Promise(resolve => { releaseOldAvailability = () => resolve({ ...value, endpoint: "https://obsolete-cloud-capability.example" }); });
      return value;
    };
    const callsBefore = availabilityCalls.filter(item => item === "decision").length;
    for (const listener of [...listeners]) listener(info);
    await settle();
    assert(decision().querySelector('[data-inference-managed]') && button("Test with sample").disabled && decision().textContent.includes("Checking this connection…"), "A pending Cloud capability check blocks rendering or claims readiness.");
    for (let i = 0; i < 3; i++) for (const listener of [...listeners]) listener(info);
    await settle();
    assert(availabilityCalls.filter(item => item === "decision").length === callsBefore + 1, "Identical info notifications restart a Cloud capability check.");
    cloudService = false; info.cloud_account_epoch = "account-1-new";
    for (const listener of [...listeners]) listener(info);
    await settle(); releaseOldAvailability(); await settle();
    assert(button("Test with sample").disabled && decision().textContent.includes("temporarily unavailable") && !decision().textContent.includes("obsolete-cloud-capability.example"), "A prior account's capability response publishes stale readiness.");
    availabilityResponder = null; cloudService = true;
    button("Retry").click(); await settle();
    assert(!button("Test with sample").disabled && decision().textContent.includes("Configured; run the sample to verify."), "Signed-in configuration claims verification or stays blocked.");
    assert(decision().textContent.includes("api.pennel.app") && !decision().textContent.includes("/v1/decisions"), "Managed Cloud exposes its internal API route instead of a readable processing destination.");
    results.push("Pending Cloud checks render immediately, survive identical notifications and reject stale account capabilities; Retry rechecks current support.");
    responder = async () => ({ verified: true, message: "Sample passed", identity: { ...identity, protocol: "pennel_cloud", endpoint: "https://api.pennel.app/v1/decisions", actual_provider: null, actual_model: "managed-model-v1" } });
    button("Test with sample").click(); await settle();
    assert(decision().textContent.includes("managed-model-v1") && decision().textContent.includes("Returned provider: Not reported"), "Cloud fabricates a returned provider or hides the actual model.");
    assert(decision().textContent.includes("bundled synthetic text") && decision().textContent.includes("Cloud allowance"), "Cloud sample disclosure is missing.");
    rerankButton("Save configuration").click(); await settle();
    assert(info.settings.reranking?.protocol === "pennel_cloud" && info.settings.reranking.model === "pennel-auto", "Cloud reranking could not be configured independently.");
    assert(reranking().textContent.includes("api.pennel.app") && !reranking().textContent.includes("/v1/rerank"), "Cloud reranking exposes its internal route in the processing destination.");
    assert(JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]) === originalChoices, "Cloud setup changed ordinary task assignments.");
    results.push("Explicit signed-in Cloud samples report actual model and unknown upstream honestly, with independent assignments.");

    responder = () => new Promise(resolve => { finish = resolve; });
    const cancellationsBeforeRefresh = operations.filter(item => item.command === "cancel_inference").length;
    button("Test with sample").click(); await settle();
    info.provider_revisions["cloud-fixture"] = 2;
    for (const listener of [...listeners]) listener(info);
    await settle();
    assert(decision().textContent.includes("Processing the sample…") && operations.filter(item => item.command === "cancel_inference").length === cancellationsBeforeRefresh, "Routine same-account credential rotation cancels its own Cloud sample.");
    finish({ verified: true, message: "same Cloud account", identity: { ...identity, protocol: "pennel_cloud", actual_model: "same-account-model" } }); await settle();
    assert(decision().textContent.includes("same-account-model") && decision().textContent.includes("The sample test passed."), "Routine credential rotation discards a current-account result.");
    results.push("Routine same-account token rotation preserves an in-flight Cloud sample and its current result.");

    button("Test with sample").click(); await settle();
    info.cloud_account_epoch = "account-2";
    for (const listener of [...listeners]) listener(info);
    await settle();
    finish({ verified: true, message: "obsolete Cloud account", identity: { ...identity, actual_model: "obsolete-cloud-model" } }); await settle();
    assert(!decision().textContent.includes("obsolete-cloud-model") && !decision().textContent.includes("The sample test passed."), "A previous Cloud account published a stale verification result.");
    responder = async () => { throw new Error("Pennel Cloud models are temporarily unavailable."); };
    button("Test with sample").click(); await settle();
    assert(decision().textContent.includes("temporarily unavailable") && !decision().textContent.includes("The sample test passed."), "Cloud failure retained success or lost its sanitized explanation.");
    cloudService = false;
    info.cloud_account_epoch = "account-3";
    for (const listener of [...listeners]) listener(info);
    await settle();
    assert(button("Test with sample").disabled && decision().textContent.includes("temporarily unavailable"), "Unavailable Cloud service is reported as ready.");
    results.push("Cloud account changes discard late results and unavailable services preserve a useful failure state.");

    setInterfaceLanguage("nl");
    assert(decision().textContent.includes("Beheerd door Pennel Cloud") && decision().textContent.includes("meegeleverde fictieve tekst"), "Managed Cloud presentation is not localized in Dutch.");
    setInterfaceLanguage("en");
    info.settings.providers = info.settings.providers.filter(account => account.kind === "pennel_cloud");
    const cloudProtocol = protocols.pop();
    await mount();
    assert(!decision().querySelector('[data-inference-field="provider"]') && !button("Save configuration"), "Cloud remains eligible without native capability metadata.");
    protocols.push(cloudProtocol);
    assert(JSON.stringify([info.settings.dictation_stt, info.settings.meeting_stt, info.settings.llm]) === originalChoices, "A Cloud failure or missing capability changed another task.");
    results.push("English/Dutch Cloud copy and missing-capability handling preserve ordinary task ownership.");
    return { passed: results.length, results, externalServicesUsed: false, userDataUsed: false };
  } finally {
    cleanup.forEach(action => action()); client.invoke = original; setInterfaceLanguage(previousLanguage); target.replaceChildren(...previous);
  }
}
