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
    { protocol: "type_safe", name: "TypeSafe System One", default_base_url: "https://api.typesafe.ai/v1", decision: true, reranking: false, extraction: false, requires_key: true },
    { protocol: "open_router_decisions", name: "OpenRouter Decisions", default_base_url: "https://openrouter.ai/api/v1", decision: true, reranking: false, extraction: false, requires_key: true },
    { protocol: "tei", name: "TEI", default_base_url: "http://127.0.0.1:8080", decision: false, reranking: true, extraction: false, requires_key: false },
  ];
  const identity = { protocol: "type_safe", endpoint: "https://decision.example/v1/systemone", requested_model: "fixture-model", requested_upstream: null, actual_model: "fixture-model-v1", actual_provider: "Fixture", actual_revision: "revision-1", request_id: "fixture-request" };
  let responder = async () => ({ verified: true, message: "Sample passed", identity });
  client.invoke = async (command, args) => {
    if (command === "inference_protocols") return protocols;
    if (command === "inference_availability") {
      const config = info.settings[args.capability];
      return { capability: args.capability, configured: !!config, available: !!config, reason: config ? "Configured" : "Not configured", provider_id: config?.provider_id ?? null, model: config?.model ?? null, protocol: config?.protocol ?? null, endpoint: config ? identity.endpoint : null };
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
    return { passed: results.length, results, externalServicesUsed: false, userDataUsed: false };
  } finally {
    cleanup.forEach(action => action()); client.invoke = original; setInterfaceLanguage(previousLanguage); target.replaceChildren(...previous);
  }
}
