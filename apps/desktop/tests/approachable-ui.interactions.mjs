/** Run in an owned loopback fixture page; every recording, credential and note below is synthetic. */
function ownedFixture() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) {
    throw new Error("Approachable UI checks require the owned synthetic fixture server.");
  }
  return window.__PENNEL_FIXTURE__;
}

const assert = (condition, message) => { if (!condition) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 120; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 40));
  }
  throw new Error(message);
}
const visible = node => {
  if (!node?.isConnected || !node.getClientRects().length || node.closest("[hidden]")) return false;
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    if (parent.matches("details:not([open])") && !parent.querySelector(":scope > summary")?.contains(node)) return false;
  }
  return getComputedStyle(node).visibility !== "hidden";
};
const button = (scope, label) => [...scope.querySelectorAll("button")].find(node => node.textContent.trim() === label);
const click = node => { assert(node && !node.disabled, "Required user action is missing or disabled."); node.focus(); node.click(); };
const enter = (node, value) => { assert(node, "Required input is missing."); node.focus(); node.value = value; node.dispatchEvent(new Event("input", { bubbles: true })); };
const selectedTab = () => document.querySelector('.detail-tabs [aria-selected="true"]')?.dataset.tab;
const dialog = () => document.querySelector("dialog.task-setup-dialog[open]");

async function chooseSetupProvider(different = false, selector = "[data-setup-provider]") {
  const owner = dialog();
  const choice = owner?.querySelector(selector);
  assert(choice, "The task has no reachable provider/model chooser.");
  const before = choice.value;
  click(choice);
  await until(() => owner.querySelector('.choice-list [role="option"]'), "The task's option menu did not open.");
  const list = owner.querySelector(".choice-list");
  const choices = [...list.querySelectorAll('[role="option"]')];
  const selected = choices.find(node => !node.disabled && node.getAttribute("aria-selected") === (different ? "false" : "true"));
  const value = different ? choice.options[choices.indexOf(selected)]?.value : before;
  assert(list.closest("dialog[open]") === owner && visible(selected), "The modal's options are outside its active accessible layer.");
  assert(owner.contains(document.activeElement), "Opening options left focus outside the modal.");
  click(selected);
  await until(() => !owner.querySelector(".choice-list"), "Selecting the option did not close the menu.");
  return value;
}

async function pointerClick(node) {
  assert(node && !node.disabled, "The pending pointer action is unavailable.");
  node.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, composed: true, button: 0, buttons: 1 }));
  if (node.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, buttons: 1 }))) node.focus();
  await settle();
  assert(node.isConnected && !node.disabled, "A background refresh removed the key Save action between pointer down and click.");
  node.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, composed: true, button: 0 }));
  node.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  node.click();
}

async function refreshWhileTyping(api, fixture, observed, key) {
  const account = fixture.info().settings.llm.provider_id;
  await api.setKey(account, "");
  const before = observed.operations.filter(operation => operation.command === "app_info").length;
  fixture.emit("settings-changed", null);
  await until(() => observed.operations.filter(operation => operation.command === "app_info").length > before, "The background settings refresh did not reach the owning window.");
  await settle();
  assert(key.isConnected && document.activeElement === key && key.value, "Background refresh replaced a dirty focused key field.");
}

function observeCommands() {
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  const operations = [];
  let failKey = false;
  let failSettings = false;
  client.invoke = async (command, args = {}) => {
    operations.push({ command, args: structuredClone(args) });
    if (command === "set_key" && failKey) { failKey = false; throw new Error("Synthetic credential save failed."); }
    if (command === "save_settings" && failSettings) { failSettings = false; throw new Error("Synthetic settings save failed."); }
    return original(command, args);
  };
  return {
    operations,
    failNextKey: () => { failKey = true; },
    failNextSettings: () => { failSettings = true; },
    restore: () => { client.invoke = original; },
  };
}

export async function checkDictationFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".dictation-home-status")?.textContent === t("dictation.home.ready"), "The ready dictation surface did not identify itself as ready.");
  const before = { settings: (await api.info()).settings.onboarding, pad: await api.padText(), history: await api.history(200) };
  const observed = observeCommands();
  try {
    const practice = button(document.querySelector(".dictation-home-actions"), t("dictation.home.try_here"));
    assert(visible(practice), "Safe in-app practice is not visible.");
    click(practice);
    await until(() => observed.operations.some(operation => operation.command === "show_window" && operation.args.kind === "onboarding" && operation.args.tab === "practice"), "Try it here did not request the owned practice window.");
    assert(!observed.operations.some(operation => ["toggle_dictation", "open_dictation_session", "copy_text"].includes(operation.command)), "Rehearsal started an external or pad recording before the owned practice window.");
    assert(await api.padText() === before.pad, "Opening safe practice changed the persistent pad.");
    assert(JSON.stringify(await api.history(200)) === JSON.stringify(before.history), "Opening safe practice created or changed history.");
    assert(JSON.stringify((await api.info()).settings.onboarding) === JSON.stringify(before.settings), "Opening rehearsal rewrote saved onboarding progress.");
    assert(visible(button(document, t("dictation.pad.settings"))), "Dictation settings became unreachable.");
    assert(!fixture.unknown.length, "Dictation used a non-fixture operation.");
    return { ready: true, ownedPractice: true, padPreserved: true, historyPreserved: true, userDataUsed: false, externalServicesUsed: false };
  } finally { observed.restore(); }
}

export async function checkOnboardingFlow() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector('.onboarding-shell[data-step="welcome"] .onboarding-goals'), "Use the newcomer Welcome fixture.");
  const before = fixture.info().settings;
  const observed = observeCommands();
  try {
    assert(visible(button(document, t("onboarding.try_dictation"))) && visible(button(document, t("onboarding.record_meeting"))), "The two first-use jobs are not visible.");
    click(button(document, t("onboarding.record_meeting")));
    await until(() => document.querySelector('.onboarding-shell[data-step="engine"]') && fixture.info().settings.onboarding.goal === "meeting", "Choosing meetings did not retain the task in speech setup.");
    let alternatives = document.querySelector("details.onboarding-other-options");
    assert(alternatives && !alternatives.open, "Technical alternatives are expanded before the first result.");
    assert(!visible(alternatives.querySelector(".engine-model-options")), "The local model catalogue is visible before Other options is opened.");
    click(alternatives.querySelector("summary"));
    await until(() => alternatives.open && visible(alternatives.querySelector(".engine-model-options")), "Other options did not reveal the local model controls.");
    assert(alternatives.textContent.includes(t("onboarding.own_provider")), "The advanced provider route was removed.");
    click(alternatives.querySelector("summary"));
    click(button(document, t("onboarding.back")));
    await until(() => document.querySelector('.onboarding-shell[data-step="welcome"]'), "Back did not return to the first task choice.");
    click(button(document, t("onboarding.try_dictation")));
    await until(() => document.querySelector('.onboarding-shell[data-step="engine"]') && fixture.info().settings.onboarding.goal === "dictation", "The dictation choice did not replace the first-use goal.");
    assert(fixture.info().settings.language === before.language && JSON.stringify(fixture.info().settings.providers) === JSON.stringify(before.providers), "Choosing a task changed speech language or provider accounts.");
    assert(!observed.operations.some(operation => ["start_meeting", "open_dictation_session", "download_model", "set_key"].includes(operation.command)), "Choosing a job started capture, downloaded a model or requested credentials automatically.");
    assert(!fixture.unknown.length, "Onboarding used a non-fixture operation.");
    return { goalChoice: true, technicalOptionsDeferred: true, existingConfigurationPreserved: true, externalServicesUsed: false, userDataUsed: false };
  } finally { observed.restore(); }
}

export async function checkSettingsFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelectorAll(".settings-overview [data-setup-task]").length === 3, "Settings did not open to three task-readiness actions.");
  const observed = observeCommands();
  try {
    const tasks = [...document.querySelectorAll(".settings-overview [data-setup-task]")].map(node => node.dataset.setupTask);
    assert(["dictation_stt", "meeting_stt", "llm"].every(task => tasks.includes(task)), "A core setup task is not reachable from the overview.");
    const textAction = document.querySelector('.settings-overview [data-setup-task="llm"]');
    click(textAction);
    await until(() => dialog()?.dataset.setupTask === "llm" && dialog().querySelector('input[type="password"]'), "Task setup did not expose the selected service's missing key in place.");
    click(dialog().querySelector("[data-setup-cancel]"));
    await until(() => !dialog(), "Back did not close task setup.");
    assert(textAction.isConnected && document.activeElement === textAction, "Task setup did not restore overview focus.");

    const speechAction = document.querySelector('.settings-overview [data-setup-task="dictation_stt"]');
    click(speechAction);
    await until(() => dialog()?.querySelector("[data-setup-provider]") && dialog().querySelector("[data-setup-local-model]"), "Local dictation setup did not expose its provider and model choices.");
    const speechBefore = structuredClone(fixture.info().settings.dictation_stt);
    await chooseSetupProvider();
    await settle();
    await chooseSetupProvider(false, "[data-setup-local-model]");
    await settle();
    assert(JSON.stringify(fixture.info().settings.dictation_stt) === JSON.stringify(speechBefore), "Selecting current modal options changed the speech configuration.");
    const changedProvider = await chooseSetupProvider(true);
    await until(() => dialog()?.querySelector("[data-setup-apply]") && !dialog().querySelector("[data-setup-apply]").disabled, "The available alternative provider did not become ready for explicit application.");
    assert(JSON.stringify(fixture.info().settings.dictation_stt) === JSON.stringify(speechBefore), "Browsing a different provider changed the active speech configuration.");
    observed.failNextSettings();
    click(dialog().querySelector("[data-setup-apply]"));
    await until(() => dialog()?.querySelector(".task-setup-message.error")?.textContent.includes("Synthetic settings save failed."), "A failed model-setting save has no task-local explanation.");
    click(dialog().querySelector("[data-setup-apply]"));
    await until(() => !dialog() && fixture.info().settings.dictation_stt.provider_id === changedProvider, "Use this setup did not retry the failed task assignment and return after it saved.");
    assert(document.activeElement === speechAction, "Modal option selection lost the originating Settings action.");
    const restored = await api.info();
    await api.saveSettings({ ...restored.settings, dictation_stt: speechBefore });

    const account = button(document.querySelector(".settings-categories"), t("navigation.account"));
    assert(account, "Account is missing from Settings navigation.");
    account.focus(); await settle();
    const bounds = account.getBoundingClientRect();
    assert(bounds.left >= 0 && bounds.right <= innerWidth && bounds.top >= 0 && bounds.bottom <= innerHeight, "Focused Account is clipped at the supported window size.");
    click(account);
    await until(() => button(document.querySelector(".settings-categories"), t("navigation.account"))?.getAttribute("aria-current") === "page", "Account did not become the selected Settings category.");
    fixture.emit("workspace-navigate", { section: "settings", tab: "models", meeting_id: null, jump_ms: null });
    await until(() => document.querySelector(".models-settings button[data-setup-task]"), "The existing Models deep link no longer opens its task overview.");
    fixture.emit("workspace-navigate", { section: "settings", tab: "providers", meeting_id: null, jump_ms: null });
    await until(() => button(document.querySelector(".settings-categories"), t("navigation.providers"))?.getAttribute("aria-current") === "page", "The existing Providers deep link no longer selects its editor.");
    click(button(document.querySelector(".settings-categories"), t("navigation.overview")));
    await until(() => document.querySelector(".settings-overview"), "Advanced configuration has no path back to task readiness.");
    assert(!observed.operations.some(operation => ["set_key", "test_provider_task", "test_provider_task_choice", "start_meeting", "toggle_dictation"].includes(operation.command)), "Browsing settings silently changed credentials, tested a provider or started capture.");
    assert(!fixture.unknown.length, "Settings used a non-fixture operation.");
    return { overview: true, taskLocalSetup: true, modalOptionsAccessible: true, settingsSaveRetried: true, focusRestored: true, accountReachable: true, legacyDeepLinks: true, externalServicesUsed: false, userDataUsed: false };
  } finally { observed.restore(); }
}

export async function checkMeetingFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".meeting-detail .meeting-summary-setup"), "Use a finished meeting with an existing summary and no configured text service.");
  const observed = observeCommands();
  try {
    const record = document.querySelector(".library-record");
    assert(visible(record) && record.dataset.phase === "new" && record.textContent.includes(t("meetings.new_recording")), "The populated library lost its New recording action.");
    assert(visible(button(document.querySelector(".library-shortcuts"), t("meetings.open_dictation"))) && visible(button(document.querySelector(".library-shortcuts"), t("meetings.open_history"))), "Dictation or History is not visible from the library.");
    const article = document.querySelector(".meeting-detail");
    const panel = article.querySelector(".meeting-panel");
    const ownNotes = article.querySelector(".own-notes");
    ownNotes.open = true;
    const notes = ownNotes.querySelector("textarea");
    const originalNotes = notes.value;
    const synthetic = "Synthetic draft for task-local setup.\nThis text belongs only to the owned QA fixture.";
    notes.value = synthetic; notes.dispatchEvent(new Event("input", { bubbles: true }));
    notes.focus(); notes.setSelectionRange(4, 17); await settle();
    panel.scrollTop = panel.scrollHeight;
    const position = panel.scrollTop;
    const setup = article.querySelector(".meeting-summary-setup");
    const opener = button(setup, t("meetings.set_up_summaries"));
    assert(visible(setup) && setup.querySelector(".meeting-setup-reason")?.textContent.trim() && visible(opener), "An existing summary has no visible setup reason and recovery action.");
    const regenerate = button(article, t("meetings.regenerate"));
    assert(regenerate?.disabled, "Regeneration is enabled without a usable text service.");
    click(opener);
    await until(() => dialog()?.dataset.setupTask === "llm" && dialog().querySelector('input[type="password"]'), "Summary setup did not open beside the meeting.");
    assert(article.isConnected && document.querySelector(".meeting-detail") === article, "Setup replaced the originating meeting.");
    let key = dialog().querySelector('input[type="password"]');
    enter(key, "fixture-only-discarded-key");
    await refreshWhileTyping(api, fixture, observed, key);
    dialog().dispatchEvent(new Event("cancel", { cancelable: true }));
    await until(() => !dialog(), "Escape/cancel did not return from task setup.");
    assert(!fixture.info().keys[fixture.info().settings.llm.provider_id], "Cancelling setup saved an unfinished service key.");
    assert(document.activeElement === opener && notes.isConnected && notes.value === synthetic && notes.selectionStart === 4 && notes.selectionEnd === 17, "Cancelling setup lost note content, caret or return focus.");
    assert(panel.scrollTop === position && selectedTab() === "notes", "Cancelling setup lost the selected tab or reading position.");

    click(article.querySelector('[data-tab="ask"]'));
    await until(() => article.querySelector(".chat-setup"), "Questions does not expose contextual setup.");
    assert(!visible(setup) && visible(article.querySelector(".chat-setup")), "Questions shows duplicate summary and question setup notices.");
    click(article.querySelector('[data-tab="notes"]'));
    click(opener);
    await until(() => dialog()?.querySelector('input[type="password"]'), "Summary setup did not reveal the service key repair.");
    key = dialog().querySelector('input[type="password"]');
    enter(key, "fixture-only-background-key");
    await refreshWhileTyping(api, fixture, observed, key);
    await pointerClick(button(key.closest(".row"), t("common.save")));
    await until(() => fixture.info().keys[fixture.info().settings.llm.provider_id] && !regenerate.disabled, "The explicit key Save click was lost after background refresh.");
    click(dialog().querySelector("[data-setup-apply]"));
    await until(() => !dialog(), "The refreshed setup did not return to its meeting.");
    await api.setKey(fixture.info().settings.llm.provider_id, "");
    fixture.emit("settings-changed", null);
    await until(() => visible(opener) && regenerate.disabled, "An external key change did not update the same meeting's setup action.");
    click(opener);
    await until(() => dialog()?.querySelector('input[type="password"]'), "Summary setup did not reopen after an external key change.");
    observed.failNextKey();
    key = dialog().querySelector('input[type="password"]');
    enter(key, "fixture-only-key");
    click(dialog().querySelector('[data-setup-save="key"]'));
    await until(() => dialog()?.querySelector(".task-setup-message.error")?.textContent.includes("Synthetic credential save failed."), "A failed key save has no visible task-local explanation.");
    await settle();
    assert(dialog() && notes.isConnected && notes.value === synthetic, "A failed connection save dismissed setup or lost the originating draft.");
    key = dialog().querySelector('input[type="password"]');
    enter(key, "fixture-only-retry-key");
    click(dialog().querySelector('[data-setup-save="key"]'));
    await until(() => fixture.info().keys[fixture.info().settings.llm.provider_id] && !dialog().querySelector("[data-setup-apply]").disabled, "Retry did not save the connection key and enable explicit application.");
    click(dialog().querySelector("[data-setup-apply]"));
    await until(() => !dialog() && !regenerate.disabled, "Use this setup did not return to the repaired meeting action.");
    assert(document.querySelector(".meeting-detail") === article && notes.value === synthetic && selectedTab() === "notes", "Successful setup navigated away from the same meeting or lost its notes.");
    assert(!visible(setup) && !regenerate.disabled, "The repaired meeting still presents a blocked summary action.");
    assert(!observed.operations.some(operation => ["summarize_meeting", "ask_meetings", "start_meeting", "test_provider_task", "test_provider_task_choice"].includes(operation.command)), "Setup processed meeting content or started a recording without a user action.");
    notes.value = originalNotes; notes.dispatchEvent(new Event("input", { bubbles: true }));
    notes.blur(); await settle();
    const title = article.querySelector("h1").textContent;
    click(record);
    await until(() => document.querySelector(".recording-view") && button(document.querySelector(".recording-view"), t("common.start_recording")), "New recording did not open the next meeting's ready-to-record surface.");
    assert(!observed.operations.some(operation => operation.command === "start_meeting"), "Opening the next recording started capture without explicit confirmation.");
    const previousMeeting = document.querySelector(`.meeting-row[data-id="${CSS.escape(fixture.options.meeting_id)}"] .meeting-select`);
    click(previousMeeting);
    await until(() => document.querySelector(".meeting-detail h1")?.textContent === title, "The previous meeting became unreachable after opening the next recording.");
    assert(!fixture.unknown.length, "Meeting recovery used a non-fixture operation.");
    return { persistentRecording: true, secondMeetingEntry: true, taskEntries: true, visibleRecovery: true, cancelPreservesContext: true, unsavedKeyDiscarded: true, keyClickSurvivesRefresh: true, failedSaveRetained: true, retryUpdatesInPlace: true, externalServicesUsed: false, userDataUsed: false };
  } finally { observed.restore(); }
}

export async function checkActiveRecordingFlow() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector('.library-record[data-phase="active"]'), "Use the active recording fixture.");
  const before = await api.meetingStatus();
  const observed = observeCommands();
  try {
    const record = document.querySelector(".library-record");
    assert(visible(record) && record.textContent.includes(t("meetings.return_recording")), "An active meeting has no clear return action.");
    click(record); await settle();
    const after = await api.meetingStatus();
    assert(after.recording && after.id === before.id && after.session_id === before.session_id, "Returning to recording replaced or stopped the active session.");
    assert(!observed.operations.some(operation => ["start_meeting", "stop_meeting"].includes(operation.command)), "Returning to the active recording started or stopped capture.");
    assert(!fixture.unknown.length, "Returning to recording used a non-fixture operation.");
    return { activeSessionPreserved: true, noSecondCapture: true, externalServicesUsed: false, userDataUsed: false };
  } finally { observed.restore(); }
}

export async function checkDictationRepairFlow() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  const before = structuredClone(fixture.info().settings.onboarding);
  assert(before.status === "completed" && before.goal === "meeting", "Use the completed meeting-first dictation-permissions fixture.");
  const observed = observeCommands();
  const dictationPermissions = () => {
    const labels = [...document.querySelectorAll(".onboarding-permissions .permission-row strong")].map(node => node.textContent);
    return labels.includes(t("onboarding.microphone")) && labels.includes(t("onboarding.accessibility")) && !labels.includes(t("onboarding.system_audio"));
  };
  const unchanged = () => JSON.stringify(fixture.info().settings.onboarding) === JSON.stringify(before);
  try {
    await until(() => document.querySelector('.onboarding-shell[data-step="permissions"]') && dictationPermissions(), "A newly opened dictation repair showed meeting permissions instead of Accessibility.");
    assert(unchanged(), "Opening dictation repair overwrote the saved meeting goal or completion.");
    for (const route of ["dictation-setup", "dictation-permissions"]) {
      const requests = observed.operations.filter(operation => operation.command === "onboarding_readiness").length;
      window.dispatchEvent(new CustomEvent("pennel-onboarding-route", { detail: route }));
      await until(() => observed.operations.filter(operation => operation.command === "onboarding_readiness").length > requests && dictationPermissions(), "Reusing the onboarding window did not apply the dictation repair intent.");
      assert(unchanged(), "A delivered dictation repair route rewrote saved onboarding progress.");
    }
    assert(!observed.operations.some(operation => ["toggle_dictation_session", "toggle_dictation", "start_meeting"].includes(operation.command)), "A repair route began capturing before an explicit action.");
    for (const label of [t("onboarding.microphone"), t("onboarding.accessibility")]) {
      const row = [...document.querySelectorAll(".onboarding-permissions .permission-row")].find(node => node.querySelector("strong")?.textContent === label);
      const action = row?.querySelector("button");
      assert(action, "The fixture's missing permission has no recovery action.");
      click(action);
      await until(() => [...document.querySelectorAll(".onboarding-permissions .permission-row")].find(node => node.querySelector("strong")?.textContent === label)?.querySelector(".permission-status")?.textContent.includes(t("onboarding.permission_ready")), "The fixture permission did not become ready.");
    }
    click(document.querySelector(".onboarding-next"));
    await until(() => document.querySelector("textarea.practice-text") && button(document, t("onboarding.start_dictation")), "Repair did not lead to owned dictation practice.");
    const practice = button(document, t("onboarding.start_dictation"));
    click(practice);
    await until(() => practice.textContent === t("onboarding.stop_dictation"), "The explicit practice action did not enter listening state.");
    click(practice);
    await until(() => document.querySelector("textarea.practice-text")?.value.trim() && document.querySelector(".onboarding-next")?.textContent === t("onboarding.open_dictation"), "The owned fixture practice did not produce a dictation result.");
    click(document.querySelector(".onboarding-next"));
    await until(() => observed.operations.some(operation => operation.command === "show_workspace" && operation.args.route?.section === "dictation"), "Completed dictation repair returned to meetings instead of Dictation.");
    assert(unchanged(), "Finishing dictation repair overwrote the saved meeting-first progress.");
    assert(!observed.operations.some(operation => operation.command === "start_meeting" || operation.command === "toggle_dictation" || operation.command === "copy_text" || operation.command === "open_dictation_session" && operation.args.kind !== "practice"), "Dictation repair escaped its owned practice session.");
    assert(!fixture.unknown.length, "Dictation repair used a non-fixture operation.");
    return { constructorIntent: true, reusedWindowIntent: true, meetingGoalPreserved: true, ownedPractice: true, returnToDictation: true, externalServicesUsed: false, userDataUsed: false };
  } finally { observed.restore(); }
}
