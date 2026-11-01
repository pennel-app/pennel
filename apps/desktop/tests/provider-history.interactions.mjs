/** Run in an owned local fixture page: import('/tests/provider-history.interactions.mjs').then(module => module.checkProviderHistoryUI()). */
export async function checkProviderHistoryUI() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) {
    throw new Error("Interaction checks require the owned synthetic fixture server.");
  }
  const fixture = window.__PENNEL_FIXTURE__;
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const operations = [];
  let info = fixture.info();
  info.provider_revisions = { groq: 0 };
  let entries = [{ id: "synthetic-unsaved", created_at_ms: 1, app_bundle_id: null, raw: "Voorbeeld", text: "Dit is alleen synthetische hersteltekst.", status: "unsaved", error: "synthetische opslagfout", has_audio: false }];
  let refreshHook = null;
  let joinResponder = () => Promise.resolve(false);
  let modelResponder = null;
  client.invoke = async (command, args = {}) => {
    operations.push({ command, args });
    if (command === "history") return structuredClone(entries);
    if (command === "save_history") { entries = entries.map(entry => entry.id === args.id ? { ...entry, status: "done" } : entry); return null; }
    if (command === "delete_history") { entries = entries.filter(entry => entry.id !== args.id); return null; }
    if (command === "copy_text") return null;
    if (command === "join_model_download") return joinResponder(args.id);
    if (command === "local_models" && modelResponder) return modelResponder();
    return original(command, args);
  };
  const { historyView } = await import("/src/views/history.ts");
  const { modelsView } = await import("/src/views/models.ts");
  const { localView } = await import("/src/views/local.ts");
  const { onboardingView } = await import("/src/views/onboarding.ts");
  const { t } = await import("/src/i18n.ts");
  const { providerDestination } = await import("/src/processing-state.ts");
  const target = document.getElementById("content");
  const previous = [...target.childNodes];
  let current = historyView;
  let cleanup = [];
  let infoListeners = [];
  let refreshes = 0;
  const errors = [];
  const context = {
    get info() { return info; }, visible: true,
    save: async change => { change(info.settings); await mount(current); },
    mutate: async action => { const next = await action(); if (next) info = next; await mount(current); },
    update: next => { info = next; },
    refresh: async () => {
      refreshes++;
      if (refreshHook) refreshHook();
      if (infoListeners.length) infoListeners.forEach(listener => listener(info));
      else await mount(current);
    },
    rerender: () => { void mount(current); },
    navigate() {}, openSetup() {}, report: error => errors.push(String(error)),
    onDispose: action => cleanup.push(action), beforeLeave() {}, onFlush() {},
    onInfoChange: listener => infoListeners.push(listener), onFocusChange() {}, onVisibilityChange() {},
  };
  async function mount(view) {
    current = view;
    cleanup.forEach(action => action()); cleanup = []; infoListeners = [];
    const node = await view(context);
    target.replaceChildren(node);
  }
  const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const button = (label, root = target) => [...root.querySelectorAll("button")].find(node => node.textContent.trim() === label);
  const textSection = () => target.querySelector('[data-setup-task="llm"]')?.closest("section");
  const results = [];
  try {
    await mount(historyView);
    assert(target.textContent.includes(t("dictation.history.unsaved")) && target.textContent.includes(t("dictation.history.temporary")), "Unsaved status and memory-only explanation missing.");
    button(t("common.copy")).click(); await settle();
    assert(operations.some(operation => operation.command === "copy_text" && operation.args.text === entries[0].text), "Copy did not use retained text.");
    button(t("dictation.history.save_again")).click(); await settle();
    assert(entries[0].status === "done" && operations.filter(operation => operation.command === "save_history").length === 1, "Save did not persist exactly once.");
    assert(!operations.some(operation => ["retry_history", "toggle_dictation", "dictation_session_start"].includes(operation.command)), "Save unexpectedly ran recognition or capture.");
    results.push("Unsaved History copy and save preserve text without recognition or paste.");

    entries = [{ ...entries[0], id: "synthetic-discard", status: "unsaved" }];
    await mount(historyView);
    [...target.querySelectorAll("button")].find(node => node.getAttribute("aria-label") === t("dictation.history.delete")).click(); await settle();
    assert(document.querySelector("dialog[open]")?.textContent.includes(t("dictation.history.discard_title")), "Discard confirmation missing.");
    button(t("common.cancel"), document.querySelector("dialog[open]")).click(); await settle();
    assert(entries.length === 1 && !operations.some(operation => operation.command === "delete_history"), "Cancel discarded retained text.");
    [...target.querySelectorAll("button")].find(node => node.getAttribute("aria-label") === t("dictation.history.delete")).click(); await settle();
    button(t("common.delete"), document.querySelector("dialog[open]")).click(); await settle();
    assert(entries.length === 0 && operations.filter(operation => operation.command === "delete_history").length === 1, "Explicit discard failed.");
    results.push("Unsaved History discard requires confirmation; cancellation preserves the entry.");

    info.keys.groq = true;
    const settingsBefore = JSON.stringify(info.settings);
    await mount(modelsView); await settle();
    assert(target.querySelectorAll('button[data-setup-task]').length === 3, "Models does not expose all three task-specific setup actions.");
    assert(textSection().textContent.includes(providerDestination(info.settings.providers.find(account => account.id === info.settings.llm.provider_id))), "The text processing summary omits its selected destination.");
    assert(!operations.some(operation => ["test_provider_task", "test_provider_task_choice"].includes(operation.command)), "Opening the model overview ran a sample automatically.");
    assert(JSON.stringify(info.settings) === settingsBefore, "Opening the model overview changed the saved settings.");
    results.push("Models summarizes all three tasks and their destination without testing or changing settings.");

    const models = await original("local_models", {});
    const modelId = models[0].id;
    modelResponder = () => models.map(model => model.id === modelId ? { ...model, state: { kind: "partial", bytes: 123 } } : model);
    info.active_model_downloads = [modelId];
    refreshHook = () => { info.active_model_downloads = []; };
    const joinBefore = operations.filter(operation => operation.command === "join_model_download").length;
    await mount(context => localView(context, true)); await settle();
    assert(operations.filter(operation => operation.command === "join_model_download").length === joinBefore + 1, "A cancelled snapshot attempted repeated joins.");
    assert(!operations.some(operation => operation.command === "download_model"), "Joining an obsolete download started a fresh download.");
    assert(!button(t("common.cancel")), "A cancelled download kept a stale active indicator.");
    results.push("A stale active-download snapshot does not start or retain a cancelled download.");

    info.active_model_downloads = [modelId];
    refreshHook = null;
    joinResponder = async id => {
      fixture.emit("download-progress", { id, downloaded: 456, total: 1000 });
      return true;
    };
    const activeBefore = operations.filter(operation => operation.command === "join_model_download").length;
    const refreshesBefore = refreshes;
    await mount(context => localView(context, true)); await settle();
    assert(operations.filter(operation => operation.command === "join_model_download").length === activeBefore + 1, "An existing worker was joined repeatedly.");
    assert(refreshes === refreshesBefore, "An immediately successful join refreshed the view during mounting.");
    assert(target.querySelector("progress")?.value === 456 && button(t("common.cancel")), "Existing progress was not replayed into the model controls.");
    info.active_model_downloads = [];
    fixture.emit("download-done", { id: modelId, error: "cancelled" }); await settle();
    results.push("An existing download replays progress through a single join and cancellation clears it.");

    info.settings.onboarding = { version: 1, status: "pending", resume_step: "engine" };
    info.settings.dictation_stt = { provider_id: "local", model: modelId };
    info.active_model_downloads = [modelId];
    const cancelledBefore = operations.filter(operation => operation.command === "cancel_download").length;
    const wizardJoinsBefore = operations.filter(operation => operation.command === "join_model_download").length;
    const wizardRefreshesBefore = refreshes;
    await mount(onboardingView); await settle();
    assert(operations.filter(operation => operation.command === "join_model_download").length === wizardJoinsBefore + 1, "A successful initial wizard join caused repeated attachments.");
    assert(refreshes === wizardRefreshesBefore, "A successful initial wizard join refreshed before mounting completed.");
    results.push("An immediately successful initial join does not refresh or remount the wizard.");
    const alternatives = target.querySelector("details.onboarding-other-options");
    if (alternatives && !alternatives.open) alternatives.querySelector("summary").click();
    button(t("onboarding.apple_dictation")).click(); await settle();
    assert(operations.filter(operation => operation.command === "cancel_download").length === cancelledBefore, "Choosing another engine cancelled a shared download.");
    info.active_model_downloads = [];
    fixture.emit("download-done", { id: modelId, error: "cancelled" }); await settle();
    results.push("Choosing Apple dictation does not implicitly cancel a shared model download.");

    assert(errors.length === 0, errors.join("\n"));
    return { passed: results.length, results, externalServicesUsed: false, userDataUsed: false, operations: operations.map(operation => operation.command) };
  } finally {
    cleanup.forEach(action => action());
    client.invoke = original;
    target.replaceChildren(...previous);
  }
}
