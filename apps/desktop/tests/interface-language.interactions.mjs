/** Run only in the owned synthetic fixture server; no native restart, provider or user data is used. */
function ownedFixture() {
  if (!/^(127\.0\.0\.1|localhost)$/.test(location.hostname) || !window.__PENNEL_FIXTURE__ || !window.__PENNEL_PLATFORM__) {
    throw new Error("Language checks require the owned synthetic fixture server.");
  }
  return window.__PENNEL_FIXTURE__;
}
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 40));
  }
  throw new Error(message);
}
function choose(picker, value, document = globalThis.document) {
  assert(picker, "Language picker is missing.");
  const segmented = picker.querySelector(`[data-value="${value}"]`);
  if (segmented) { segmented.click(); return; }
  const option = picker.options?.find(option => option.value === value);
  assert(option, `Language ${value} is not offered.`);
  picker.click();
  const button = [...document.querySelectorAll('.choice-list [role="option"]')].find(node => node.textContent === option.label);
  assert(button, `Language ${value} is not reachable through the choice menu.`);
  button.click();
}

function showOtherOptions(document = globalThis.document) {
  const alternatives = document.querySelector("details.onboarding-other-options");
  if (alternatives && !alternatives.open) alternatives.querySelector("summary").click();
  const local = alternatives?.querySelector(".onboarding-local-options"); if (local && !local.open) local.querySelector("summary").click();
}

export async function checkInterfaceLanguageUI() {
  const fixture = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t, getInterfaceLanguage } = await import("/src/i18n.ts");
  const before = await api.info();
  const initial = getInterfaceLanguage();
  assert(document.documentElement.lang === before.interface_language && initial === before.interface_language, "Boot locale and document disagree.");
  const picker = [...document.querySelectorAll('.choice[aria-label]')].find(node => node.getAttribute("aria-label")?.startsWith(t("interface.language") + ":"));
  assert(picker, "Settings has no accessible interface-language selector.");
  const content = await api.meetingDetail(fixture.options.meeting_id);
  const pad = await api.padText();
  const history = await api.history(200);
  const next = initial === "en" ? "nl" : "en";
  choose(picker, next);
  await until(() => fixture.info().settings.ui_language === next && document.documentElement.lang === next, "Preference did not save and apply immediately.");
  const saved = await api.info();
  assert(saved.interface_language === next && getInterfaceLanguage() === next, "The process did not apply its saved preference.");
  assert(picker.isConnected && [...document.querySelectorAll('.choice')].includes(picker), "Live switching replaced the language control.");
  assert(picker.getAttribute("aria-label")?.startsWith(t("interface.language") + ":"), "Accessible label did not switch immediately.");
  assert(JSON.stringify(saved.settings) === JSON.stringify({ ...before.settings, ui_language: next }), "Interface selection changed other settings.");
  assert(JSON.stringify(await api.meetingDetail(fixture.options.meeting_id)) === JSON.stringify(content), "Interface selection rewrote meeting data.");
  assert(await api.padText() === pad && JSON.stringify(await api.history(200)) === JSON.stringify(history), "Interface selection rewrote dictation data.");
  choose(picker, "system");
  await until(() => document.documentElement.lang === before.system_interface_language && fixture.info().settings.ui_language === "system", "System-language option did not restore the resolved system language.");
  assert(picker.textContent.includes(before.system_interface_language === "nl" ? "Nederlands" : "English"), "System-language option does not identify its resolved language.");
  assert(!fixture.calls.includes("restart_app") && !fixture.unknown.length, "Language selection invoked an unowned native operation.");
  return { initial, selected: next, restored: before.system_interface_language, live: true, contentPreserved: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkOnboardingLanguageUI() {
  const fixture = ownedFixture();
  assert(fixture.options.step === "language", "Use the onboarding-language fixture.");
  const { api } = await import("/src/api.ts");
  const { t, getInterfaceLanguage } = await import("/src/i18n.ts");
  const before = await api.info();
  assert(before.settings.ui_language === "system", "First-run interface preference does not default to System.");
  assert(document.documentElement.lang === before.system_interface_language, "First screen did not use the system language.");
  const picker = document.querySelector('.onboarding-language .onboarding-interface-language .choice');
  assert(picker?.value === "system", "First-run language picker did not select System.");
  const next = getInterfaceLanguage() === "en" ? "nl" : "en";
  choose(picker, next);
  await until(() => document.documentElement.lang === next && fixture.info().settings.ui_language === next, "Onboarding language change was not immediate and persistent.");
  assert(document.querySelector('.onboarding-heading h1')?.textContent === t("onboarding.interface_title"), "Onboarding heading did not translate in place.");
  assert(picker.isConnected, "Changing language replaced the onboarding selector.");
  assert(JSON.stringify((await api.info()).settings) === JSON.stringify({ ...before.settings, ui_language: next }), "Language screen changed speech/provider/model/progress settings.");
  document.querySelector('.onboarding-next').click();
  await until(() => fixture.info().settings.onboarding.resume_step === "welcome", "Language did not advance to Welcome.");
  document.querySelector('[data-onboarding-goal="dictation"]').click();
  document.querySelector(".onboarding-next").click();
  await until(() => fixture.info().settings.onboarding.resume_step === "engine" && document.querySelector('[aria-label]') && document.querySelector('.engine-language'), "Welcome did not advance to speech recognition.");
  showOtherOptions();
  const modelPicker = [...document.querySelectorAll('.choice[aria-label]')].find(node => node.getAttribute("aria-label")?.startsWith(t("onboarding.speech_model") + ":"));
  assert(modelPicker?.value === before.settings.dictation_stt.model, "Language onboarding reset the existing model choice.");
  const engineSettings = await api.info();
  const reverse = next === "en" ? "nl" : "en";
  await api.saveSettings({ ...engineSettings.settings, ui_language: reverse });
  await until(() => document.documentElement.lang === reverse, "External settings change did not relabel speech setup.");
  assert(modelPicker.isConnected && modelPicker.value === before.settings.dictation_stt.model, "Live switching replaced the speech-model control or reset its choice.");
  assert(fixture.info().settings.language === before.settings.language, "Interface choice changed speech language.");
  assert(!fixture.unknown.length, "Onboarding used an unowned service.");
  return { system: before.system_interface_language, selected: next, selectedModel: modelPicker.value, speechLanguage: before.settings.language, live: true, externalServicesUsed: false, userDataUsed: false };
}

export async function checkOnboardingModelPicker() {
  const fixture = ownedFixture();
  assert(fixture.options.download_active, "Use the owned active-download fixture.");
  const { t, getInterfaceLanguage } = await import("/src/i18n.ts");
  const picker = () => [...document.querySelectorAll('.choice[aria-label]')].find(node => node.getAttribute("aria-label")?.startsWith(t("onboarding.speech_model") + ":"));
  const id = fixture.info().settings.dictation_stt.model;
  showOtherOptions();
  assert(picker()?.value === id, "Onboarding reset the saved model ID.");
  assert(fixture.completeDownload(id), "Active model download was not owned by the fixture.");
  await until(() => document.querySelector('[data-download-state]')?.textContent.trim() === t("onboarding.downloaded"), "Downloaded model badge did not update.");
  assert(picker()?.value === id && fixture.info().settings.language === "nl", "Download changed the selected model or speech language.");
  return { locale: getInterfaceLanguage(), selectedId: id, completionLabel: t("onboarding.downloaded"), externalServicesUsed: false, userDataUsed: false };
}

/** Two real owned fixture windows share an isolated synthetic profile through the storage-event seam. */
export async function checkCrossWindowLanguagePreservation() {
  ownedFixture();
  const { catalogs } = await import("/src/i18n.ts");
  const text = (win, key) => catalogs[win.document.documentElement.lang][key];
  const profile = `language-preservation-${crypto.randomUUID()}`;
  const profileKey = `pennel-synthetic-fixture:${profile}`;
  const frames = [];
  async function open(name, { waitReady = true, modelDelay = 0 } = {}) {
    const url = new URL("/index.html", location.origin);
    url.searchParams.set("qa", name); url.searchParams.set("locale", "nl"); url.searchParams.set("profile", profile);
    url.searchParams.set("window", name.startsWith("settings-") ? "settings" : "workspace");
    url.searchParams.set("section", name.startsWith("settings-") ? "settings" : name === "recording" ? "recording" : "meetings");
    if (name.startsWith("settings-")) url.searchParams.set("tab", name === "settings-appearance" ? "appearance" : "meeting-settings");
    if (modelDelay) url.searchParams.set("model_state_delay_ms", String(modelDelay));
    if (name === "meeting-notes") { url.searchParams.set("meeting_id", "2026-10-24 0900 Projectoverleg"); url.searchParams.set("meeting_tab", "notes"); }
    const frame = document.createElement("iframe");
    frame.style.cssText = "position:fixed;left:-10000px;width:980px;height:720px;border:0";
    frame.src = url.href; document.body.append(frame); frames.push(frame);
    await until(() => frame.contentWindow?.__PENNEL_FIXTURE__ && (!waitReady || (frame.contentWindow.__PENNEL_FIXTURE_READY__ && frame.contentDocument?.getElementById("app")?.textContent.trim())), `${name} did not boot.`);
    return frame.contentWindow;
  }
  try {
    const settingsWindow = await open("settings-appearance");
    const recordingWindow = await open("recording");
    const notesWindow = await open("meeting-notes");
    await until(() => recordingWindow.document.querySelector("textarea") && notesWindow.document.querySelector("textarea"), "Owned note fields did not render.");
    const recordingInput = recordingWindow.document.querySelector("textarea");
    const notesInput = notesWindow.document.querySelector("textarea");
    const literal = "Annuleren — unsaved synthetic notes {count} <b>literal</b>";
    for (const input of [recordingInput, notesInput]) {
      input.value = literal; input.dispatchEvent(new Event("input", { bubbles: true })); input.setSelectionRange(3, 9);
    }
    const before = await recordingWindow.__PENNEL_PLATFORM__.invoke("meeting_status");
    assert(before.recording && before.session_id, "Recording fixture has no active session.");
    const picker = [...settingsWindow.document.querySelectorAll('.choice[aria-label]')].find(node => node.getAttribute("aria-label")?.startsWith(text(settingsWindow, "interface.language") + ":"));
    choose(picker, "en", settingsWindow.document);
    await until(() => [settingsWindow, recordingWindow, notesWindow].every(win => win.document.documentElement.lang === "en"), "Settings change did not propagate to other owned windows.");
    assert(recordingWindow.document.querySelector("textarea") === recordingInput && notesWindow.document.querySelector("textarea") === notesInput, "Cross-window locale change replaced a note input.");
    for (const input of [recordingInput, notesInput]) {
      assert(input.value === literal && input.selectionStart === 3 && input.selectionEnd === 9, "Cross-window relabeling changed unsaved notes or selection.");
    }
    const after = await recordingWindow.__PENNEL_PLATFORM__.invoke("meeting_status");
    assert(after.recording && after.session_id === before.session_id && after.id === before.id, "Cross-window relabeling interrupted or replaced the active recording.");
    assert(!recordingWindow.__PENNEL_FIXTURE__.calls.includes("stop_meeting"), "Locale switching stopped the recording.");
    assert(recordingWindow.__PENNEL_FIXTURE__.info().settings.language === "nl", "UI change altered the frozen speech language.");
    const reopened = await open("settings-appearance");
    assert(reopened.document.documentElement.lang === "en" && reopened.__PENNEL_FIXTURE__.info().settings.ui_language === "en", "Reopened interface did not retain its explicit language.");
    const delayed = await open("settings-meetings", { waitReady: false, modelDelay: 900 });
    await until(() => delayed.__PENNEL_FIXTURE__.calls.includes("model_state"), "Asynchronous settings fixture did not reach its owned model-state delay.");
    choose(picker, "nl", settingsWindow.document);
    await until(() => delayed.document.documentElement.lang === "nl", "Pending settings window did not receive the new language.");
    await until(() => delayed.document.querySelector('.speaker-route') && !delayed.document.querySelector('.settings-loading'), "Delayed speaker controls did not finish rendering.");
    const heading = [...delayed.document.querySelectorAll('.pane-section h2,.pane-section h3')].find(node => node.textContent === text(delayed, "settings.speakers.title"));
    assert(heading, "Asynchronously inserted speaker controls retained the previous interface language.");
    assert(delayed.document.querySelector('.speaker-route .choice')?.getAttribute("aria-label") === text(delayed, "settings.speakers.route_label"), "Asynchronously inserted speaker controls retained a stale accessible name.");
    choose(picker, "en", settingsWindow.document);
    await until(() => [recordingWindow, notesWindow, delayed].every(win => win.document.documentElement.lang === "en"), "Repeated cross-window language switch did not apply.");
    assert(recordingWindow.document.querySelector("textarea") === recordingInput && recordingInput.value === literal, "Repeated relabeling replaced the recording note field or its content.");
    assert(notesWindow.document.querySelector("textarea") === notesInput && notesInput.value === literal, "Repeated relabeling replaced the meeting note field or its content.");
    return { windows: 5, persisted: true, noteNodesPreserved: true, noteSelectionPreserved: true, asynchronousInsertionLocalized: true, repeatedSwitching: true, recordingSession: before.session_id, externalServicesUsed: false, userDataUsed: false };
  } finally {
    frames.forEach(frame => frame.remove());
    localStorage.removeItem(profileKey);
  }
}
