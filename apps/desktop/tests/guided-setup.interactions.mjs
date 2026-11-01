const assert = (condition, message) => { if (!condition) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 120; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 35));
  }
  throw new Error(message);
}
function ownedFixture() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__ && window.__PENNEL_PLATFORM__, "Guided setup checks require an owned synthetic fixture.");
  return window.__PENNEL_FIXTURE__;
}
const dialog = () => document.querySelector("dialog[data-setup-task][open]");
const assignments = info => Object.fromEntries(["dictation_stt", "meeting_stt", "llm"].map(slot => [slot, info.settings[slot]]));
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);
const enter = (input, value) => {
  assert(input, "The expected guided setup input is missing.");
  input.focus(); input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
};
function click(node) {
  assert(node?.isConnected && !node.disabled && node.getClientRects().length && !node.closest("[hidden]"), "The expected guided setup action is unavailable.");
  node.focus(); node.click();
}
async function openTask(slot) {
  click(document.querySelector(`button[data-setup-task="${slot}"]`));
  await until(() => dialog()?.dataset.setupTask === slot && dialog().querySelector("[data-setup-apply]"), "The selected task did not open the shared guided chooser.");
  return dialog();
}
async function chooseModel(id) {
  await until(() => dialog()?.querySelector(`[data-model-id="${CSS.escape(id)}"]`), `The synthetic model ${id} is absent from the model list.`);
  const option = dialog().querySelector(`[data-model-id="${CSS.escape(id)}"]`);
  click(option);
  await settle();
  assert(option.isConnected && document.activeElement === option && option.getAttribute("aria-pressed") === "true", "Choosing a model lost keyboard focus or failed to update its selected state in place.");
}
function reveal(input) {
  assert(input, "The manual model control is missing.");
  for (let node = input.parentElement; node && node !== dialog(); node = node.parentElement) {
    if (node.matches("details") && !node.open) node.querySelector(":scope > summary").click();
  }
}

export async function checkGuidedSetup() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await until(() => document.querySelectorAll(".models-settings button[data-setup-task]").length === 3, "Models does not expose the three everyday processing tasks.");
  const experimental = document.querySelector("[data-inference-experimental]");
  assert(experimental && !experimental.open, "Experimental connection forms are expanded in everyday model setup.");
  assert(!fixture.setupTrace.samples.length, "Opening Models ran an inference sample automatically.");
  const opener = document.querySelector('button[data-setup-task="llm"]');
  await openTask("llm");
  assert(["device", "cloud", "own"].every(source => dialog().querySelector(`[data-setup-source="${source}"]`)), "The chooser does not expose the three processing locations.");
  await chooseModel("gpt-4.1-mini");
  assert(same(assignments(fixture.info()), before), "Browsing another model changed an active task assignment.");
  click(dialog().querySelector("[data-setup-test]"));
  await until(() => fixture.setupTrace.samples.length === 1 && !dialog().querySelector("[data-setup-test]").disabled, "The explicit draft sample did not finish.");
  assert(same(fixture.setupTrace.samples[0], { slot: "llm", choice: { provider_id: before.llm.provider_id, model: "gpt-4.1-mini" }, draft: true }), "The sample tested the saved model instead of the selected draft.");
  assert(same(assignments(fixture.info()), before), "Testing a draft changed an active task assignment.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Cancel did not close the model chooser.");
  assert(same(assignments(fixture.info()), before), "Cancelling model setup saved its draft.");
  assert(opener.isConnected && document.activeElement === opener, "Cancelling setup did not restore the originating task action.");
  await openTask("llm");
  await chooseModel("gpt-4.1-mini");
  click(dialog().querySelector("[data-setup-apply]"));
  await until(() => !dialog() && fixture.info().settings.llm.model === "gpt-4.1-mini", "Use this setup did not apply the selected draft.");
  const after = assignments(fixture.info());
  assert(same(after.dictation_stt, before.dictation_stt) && same(after.meeting_stt, before.meeting_stt), "Applying text processing changed a speech task.");
  assert(!fixture.calls.some(command => ["test_provider_task", "start_meeting", "toggle_dictation", "summarize_meeting", "ask_meetings"].includes(command)), "Model setup processed content or capture outside its explicit synthetic sample.");
  assert(!fixture.unknown.length, "Guided setup used a command outside the owned fixture.");
  return { draftSample: true, browsePreservesAssignments: true, cancelPreservesAssignments: true, oneTaskApplied: true, focusRestored: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupLateSample() {
  const fixture = ownedFixture();
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const before = structuredClone(assignments(fixture.info()));
  let finish;
  client.invoke = async (command, args = {}) => command === "test_provider_task_choice"
    ? new Promise(resolve => { finish = resolve; }) : original(command, args);
  try {
    await openTask("llm");
    click(dialog().querySelector("[data-setup-test]"));
    await until(() => finish, "The sample request did not reach its controlled responder.");
    await chooseModel("gpt-4.1-mini");
    finish({ verified: true, message: "Obsolete synthetic result for the previous model" });
    await settle();
    assert(!dialog().textContent.includes("Obsolete synthetic result for the previous model") && !dialog().querySelector("[data-setup-test]").disabled, "Editing the draft retained the previous model's late sample result.");
    assert(same(assignments(fixture.info()), before), "Editing a pending sample changed a saved assignment.");
    finish = null;
    click(dialog().querySelector("[data-setup-test]"));
    await until(() => finish, "The second explicit draft sample did not start.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await until(() => !dialog(), "Cancelling a pending sample did not return to Models.");
    await openTask("dictation_stt");
    finish({ verified: true, message: "Obsolete synthetic sample result" });
    await settle();
    assert(dialog()?.dataset.setupTask === "dictation_stt" && !dialog().textContent.includes("Obsolete synthetic sample result"), "A cancelled task's late sample reached the next task.");
    assert(same(assignments(fixture.info()), before), "A late sample saved or changed a task assignment.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await until(() => !dialog(), "The next task could not be cancelled.");
    assert(!fixture.unknown.length, "Late-response testing escaped the fixture.");
    return { editedSampleDiscarded: true, lateSampleDiscarded: true, nextTaskPreserved: true, activeAssignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
  } finally { finish?.({ verified: false, message: "Cancelled synthetic sample" }); client.invoke = original; }
}

export async function checkGuidedSetupCatalogRecovery() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("llm");
  await until(() => dialog()?.querySelector("[data-setup-catalog-retry]") && !dialog().querySelector("[data-setup-catalog-retry]").hidden, "A failed model catalog has no reachable retry action.");
  const failedRequests = fixture.setupTrace.catalogRequests.length;
  assert(failedRequests > 0, "Catalog recovery has no failed request to recover.");
  const manual = dialog().querySelector("input[data-setup-manual]");
  reveal(manual);
  enter(manual, "synthetic/manual-only-model");
  await settle();
  assert(same(assignments(fixture.info()), before), "Entering a manual model ID changed the saved configuration.");
  click(dialog().querySelector("[data-setup-catalog-retry]"));
  await until(() => dialog()?.querySelector('[data-model-id="gpt-4.1-mini"]'), "Explicit Retry did not recover the synthetic catalog.");
  assert(fixture.setupTrace.catalogRequests.length === failedRequests + 1, "Retry did not make exactly one fresh catalog request.");
  assert(dialog().querySelector("input[data-setup-manual]")?.value === "synthetic/manual-only-model", "Catalog recovery erased a manual model absent from its list.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Catalog recovery could not return to Models.");
  assert(same(assignments(fixture.info()), before), "Cancelling catalog recovery saved the manual model.");
  return { catalogFailureActionable: true, explicitRetry: true, manualModelRetained: true, assignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupMissingKey() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("llm");
  await until(() => dialog()?.querySelector('input[type="password"]'), "A missing provider key has no inline connection repair.");
  assert(dialog().querySelector("[data-setup-test]").disabled && dialog().querySelector("[data-setup-apply]").disabled, "A provider without credentials is shown as ready to use.");
  const key = dialog().querySelector('input[type="password"]');
  enter(key, "synthetic-setup-key-only");
  const save = dialog().querySelector('[data-setup-save="key"]');
  click(save);
  await until(() => fixture.info().keys[before.llm.provider_id], "The explicit connection-key action did not save its prerequisite.");
  assert(same(assignments(fixture.info()), before), "Saving connection credentials switched task assignments.");
  assert(!JSON.stringify(fixture.setupTrace).includes("synthetic-setup-key-only"), "The model setup trace retained credential text.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Cancelling after connecting did not return to Models.");
  assert(fixture.info().keys[before.llm.provider_id] && same(assignments(fixture.info()), before), "Cancel removed an explicitly saved connection or changed task choices.");
  return { missingKeyExplained: true, connectionSavedExplicitly: true, cancelPreservesTasks: true, noKeyInTrace: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupUnavailable() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("dictation_stt");
  const device = dialog().querySelector('[data-setup-source="device"]');
  assert(device?.disabled && device.parentElement.textContent.trim().length > device.textContent.trim().length, "An unavailable processing location lacks a visible reason.");
  assert(same(assignments(fixture.info()), before), "Opening an unavailable setup silently selected a replacement.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Unavailable setup did not offer a working cancel action.");
  return { unavailableVisible: true, reasonPresent: true, noFallback: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupCustomModel() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("llm");
  await until(() => dialog()?.querySelector("input[data-setup-manual]"), "A saved model absent from the catalog cannot be edited manually.");
  assert(dialog().querySelector("input[data-setup-manual]").value === before.llm.model && dialog().textContent.includes(before.llm.model), "Opening model setup lost the existing manually configured model.");
  assert(same(assignments(fixture.info()), before), "Opening a legacy manual model silently chose a catalog replacement.");
  return { savedModelRetained: true, manualModelEditable: true, noMigration: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupSampleFailure() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("llm");
  await chooseModel("gpt-4.1-mini");
  click(dialog().querySelector("[data-setup-test]"));
  await until(() => dialog()?.textContent.includes(fixture.options.sample_error), "A failed sample has no visible explanation.");
  assert(same(assignments(fixture.info()), before), "A failed sample changed the active model or selected a fallback.");
  assert(fixture.setupTrace.samples.length === 1 && fixture.setupTrace.samples[0].choice.model === "gpt-4.1-mini", "The failed draft sample was retried or substituted automatically.");
  return { failureVisible: true, noAutomaticRetry: true, assignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupPermissions() {
  const fixture = ownedFixture();
  const before = structuredClone(assignments(fixture.info()));
  await openTask("dictation_stt");
  await until(() => dialog()?.querySelector("[data-setup-access]"), "Denied microphone access has no task-local recovery action.");
  assert(dialog().querySelector("[data-setup-apply]").disabled, "A speech setup with denied microphone access is shown as ready to apply.");
  click(dialog().querySelector("[data-setup-access]"));
  await until(() => !dialog()?.querySelector("[data-setup-apply]")?.disabled, "Review access did not update readiness after the synthetic permission grant.");
  assert(fixture.calls.filter(command => command === "request_permission").length === 1 && same(assignments(fixture.info()), before), "Permission recovery requested unrelated access or changed a model assignment.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Permission recovery did not retain a working Cancel action.");
  return { deniedAccessExplained: true, relevantPermissionRequested: true, assignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupInterruptedDownload() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  const before = structuredClone(assignments(fixture.info()));
  await openTask("dictation_stt");
  await until(() => dialog()?.querySelector("[data-setup-download]"), "A selected partial model has no download controls.");
  assert(dialog().querySelector("[data-setup-download]").disabled && dialog().querySelector("[data-setup-apply]").disabled, "An unfinished model is shown as downloaded or ready to apply.");
  const cancelDownload = [...dialog().querySelectorAll(".setup-model-picker button")].find(node => node.textContent.trim() === t("onboarding.cancel_download"));
  click(cancelDownload);
  await until(() => dialog()?.querySelector("[data-setup-download]") && !dialog().querySelector("[data-setup-download]").disabled, "Cancelling the synthetic download retained its stale active state.");
  assert(dialog().querySelector("[data-setup-apply]").disabled && !fixture.calls.includes("download_model") && same(assignments(fixture.info()), before), "Interrupted model setup restarted a download or changed active choices.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Interrupted download setup could not return to Models.");
  return { interruptedDownloadRecoverable: true, noAutomaticRestart: true, assignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupCloudSignIn() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  const before = structuredClone(assignments(fixture.info()));
  await openTask("llm");
  click(dialog().querySelector('[data-setup-source="cloud"]'));
  await until(() => dialog()?.querySelector(".task-setup-add"), "An unconnected Cloud option has no inline Add connection action.");
  assert(!fixture.info().settings.providers.some(provider => provider.kind === "pennel_cloud") && same(assignments(fixture.info()), before), "Browsing Cloud created a connection or switched a task automatically.");
  click(dialog().querySelector(".task-setup-add"));
  await until(() => dialog()?.querySelector('.choice-list [role="menuitem"]'), "The inline connection menu did not open in the task dialog.");
  click([...dialog().querySelectorAll('.choice-list [role="menuitem"]')].find(node => node.textContent.trim() === "Pennel Cloud"));
  await until(() => [...dialog().querySelectorAll("button")].some(node => node.textContent.trim() === t("providers.cloud_sign_in")), "An explicitly added Cloud connection has no sign-in action.");
  assert(dialog().querySelector("[data-setup-apply]").disabled && same(assignments(fixture.info()), before), "Signed-out Cloud replaced a working model or appeared ready to use.");
  click([...dialog().querySelectorAll("button")].find(node => node.textContent.trim() === t("providers.cloud_sign_in")));
  await until(() => [...dialog().querySelectorAll("button")].some(node => node.textContent.trim() === t("account.setup.email")), "Cloud sign-in left the task dialog instead of opening contextual account setup.");
  assert(dialog().querySelector("[data-setup-apply]").disabled && !fixture.calls.includes("cloud_login_start"), "Opening account setup started an authentication request without its explicit action.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "Cloud account setup could not return to the originating task.");
  assert(fixture.info().settings.providers.some(provider => provider.kind === "pennel_cloud") && same(assignments(fixture.info()), before), "Cancelling removed an explicitly created connection or saved its unready task choice.");
  return { cloudSignInExplained: true, connectionCreatedExplicitly: true, contextualAccountSetup: true, assignmentsPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkGuidedSetupPendingConnection() {
  const fixture = ownedFixture();
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const before = structuredClone(assignments(fixture.info()));
  const count = fixture.info().settings.providers.length;
  let complete;
  client.invoke = async (command, args = {}) => command === "add_provider"
    ? new Promise((resolve, reject) => { complete = () => original(command, args).then(resolve, reject); }) : original(command, args);
  try {
    await openTask("llm");
    await until(() => !dialog().querySelector("[data-setup-test]").disabled && !dialog().querySelector("[data-setup-apply]").disabled, "The original synthetic setup is not ready before adding a connection.");
    click(dialog().querySelector(".task-setup-add"));
    await until(() => dialog()?.querySelector('.choice-list [role="menuitem"]'), "Add connection did not open its native-style menu.");
    click([...dialog().querySelectorAll('.choice-list [role="menuitem"]')].find(node => node.textContent.trim() === "OpenAI"));
    await until(() => complete, "The explicit connection request did not reach its delayed responder.");
    assert(dialog().querySelector("[data-setup-test]").disabled && dialog().querySelector("[data-setup-apply]").disabled, "Apply or Test remained enabled while the connection mutation was pending.");
    dialog().querySelector("[data-setup-test]").click();
    dialog().querySelector("[data-setup-apply]").click();
    assert(!fixture.setupTrace.samples.length && !fixture.setupTrace.savedChoices.length, "A pending connection allowed a test or task assignment to start.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await complete();
    await until(() => !dialog(), "Cancel did not settle the explicit connection request before returning.");
    assert(fixture.info().settings.providers.length === count + 1 && same(assignments(fixture.info()), before), "Settling a cancelled setup lost its explicit connection or applied its late provider choice.");
    return { pendingActionsDisabled: true, explicitConnectionRetained: true, lateChoiceNotApplied: true, externalServicesUsed: false, userDataUsed: false };
  } finally { client.invoke = original; }
}

export async function checkGuidedSetupFailedApplyCancel() {
  const fixture = ownedFixture();
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const before = structuredClone(assignments(fixture.info()));
  let attempts = 0;
  client.invoke = async (command, args = {}) => {
    if (command === "save_settings" && ++attempts === 1) throw new Error("Synthetic task assignment failed once.");
    return original(command, args);
  };
  try {
    await openTask("llm");
    await chooseModel("gpt-4.1-mini");
    click(dialog().querySelector("[data-setup-apply]"));
    await until(() => dialog()?.querySelector(".task-setup-message.error")?.textContent.includes("Synthetic task assignment failed once."), "A failed Apply did not remain visible inside the same setup.");
    assert(attempts === 1 && same(assignments(fixture.info()), before), "The failed Apply changed a saved assignment or retried itself.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await until(() => !dialog(), "Cancel could not leave a failed Apply.");
    fixture.emit("workspace-navigate", { section: "settings", tab: "providers", meeting_id: null, jump_ms: null });
    await until(() => document.querySelector(".providers"), "Providers navigation did not flush and leave Models.");
    fixture.emit("workspace-navigate", { section: "settings", tab: "models", meeting_id: null, jump_ms: null });
    await until(() => document.querySelector(".models-settings button[data-setup-task]"), "The model overview did not reopen after a cancelled failed Apply.");
    await openTask("llm");
    await until(() => dialog()?.querySelector("[data-setup-selected-model]"), "The original model did not reopen after navigation.");
    assert(attempts === 1 && same(assignments(fixture.info()), before), "Cancel or subsequent navigation retried and saved the discarded model draft.");
    assert(dialog().querySelector("input[data-setup-manual]")?.value === before.llm.model, "Reopening setup did not restore the original model ID.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await until(() => !dialog(), "The restored original setup could not be cancelled.");
    return { failedApplyVisible: true, cancelledDraftDiscarded: true, noDeferredSave: true, originalModelRestored: true, externalServicesUsed: false, userDataUsed: false };
  } finally { client.invoke = original; }
}

export async function checkGuidedSetupLoginIsolation() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  const before = structuredClone(assignments(fixture.info()));
  assert(Object.values(before).every(choice => !choice.provider_id), "Use the synthetic profile with three unassigned tasks.");
  await openTask("llm");
  click(dialog().querySelector('[data-setup-source="cloud"]'));
  await until(() => dialog()?.querySelector(".task-setup-add"), "Cloud has no inline connection action.");
  click(dialog().querySelector(".task-setup-add"));
  await until(() => dialog()?.querySelector('.choice-list [role="menuitem"]'), "The Cloud connection menu did not open.");
  click([...dialog().querySelectorAll('.choice-list [role="menuitem"]')].find(node => node.textContent.trim() === "Pennel Cloud"));
  await until(() => [...dialog().querySelectorAll("button")].some(node => node.textContent.trim() === t("providers.cloud_sign_in")), "The new Cloud connection has no account setup action.");
  click([...dialog().querySelectorAll("button")].find(node => node.textContent.trim() === t("providers.cloud_sign_in")));
  await until(() => [...dialog().querySelectorAll("button")].some(node => node.textContent.trim() === t("account.setup.email")), "Contextual account setup has no explicit sign-in action.");
  click([...dialog().querySelectorAll("button")].find(node => node.textContent.trim() === t("account.setup.email")));
  await until(() => fixture.calls.includes("cloud_login_poll") && dialog()?.querySelector(".account-management"), "The owned synthetic login did not return to contextual account management.");
  const emptyTasks = new Intl.ListFormat([document.documentElement.lang], { style: "long", type: "conjunction" }).format([t("account.slot.dictation"), t("account.slot.text"), t("account.slot.meetings")]);
  assert(!dialog().textContent.includes(t("account.cloud_offer", { tasks: emptyTasks })), "Task-local sign-in offers to adopt Cloud for every unassigned task.");
  assert(same(assignments(fixture.info()), before) && !fixture.setupTrace.savedChoices.length, "Signing in from one task configured other empty task slots.");
  click(dialog().querySelector("[data-setup-cancel]"));
  await until(() => !dialog(), "The completed synthetic sign-in could not return to Models.");
  assert(same(assignments(fixture.info()), before), "Cancelling after sign-in applied the Cloud draft.");
  return { syntheticLoginCompleted: true, noGlobalAdoptionPrompt: true, noOtherTaskAssignments: true, cancelledDraftDiscarded: true, externalServicesUsed: false, userDataUsed: false };
}
