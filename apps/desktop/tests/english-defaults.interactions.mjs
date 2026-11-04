const assert = (condition, message) => { if (!condition) throw new Error(message); };
const pause = () => new Promise(resolve => setTimeout(resolve, 30));
async function until(check, message) {
  for (let attempt = 0; attempt < 100; attempt++) { if (check()) return; await pause(); }
  throw new Error(message);
}

function ownedFixture() {
  const fixture = window.__PENNEL_FIXTURE__;
  const client = window.__PENNEL_PLATFORM__;
  assert(location.hostname === "127.0.0.1" && fixture && client, "Use the owned synthetic fixture server.");
  return { fixture, client };
}

async function switchLanguage(api, settings, language) {
  await api.saveSettings({ ...settings, ui_language: language });
  await until(() => document.documentElement.lang === language, "The interface did not switch language.");
}

export async function checkEnglishTemplateDefaults() {
  const { fixture, client } = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { t, getInterfaceLanguage } = await import("/src/i18n.ts");
  const before = await api.info();
  const initial = getInterfaceLanguage();
  const original = client.invoke.bind(client);
  const customName = "Mijn eigen sjabloon {name}";
  client.invoke = async (command, args) => {
    const result = await original(command, args);
    return command === "meeting_templates" ? [...result, { id: "qa-custom", name: customName, builtin: false, customized: false }] : result;
  };
  try {
    document.querySelector('[data-settings-category="dictation"]').click();
    await until(() => document.querySelector('[data-settings-category="dictation"][aria-current="page"]') && !document.querySelector('#content').inert && !document.querySelector('.template-row, .model-card'), "Dictation settings did not open.");
    document.querySelector('[data-settings-category="meeting-settings"]').click();
    await until(() => [...document.querySelectorAll(".template-row")].some(row => row.textContent.includes(customName)), "Templates did not finish mounting.");
    assert([...document.querySelectorAll(".template-row")].some(row => row.querySelector(".grow")?.textContent === t("settings.template.general")), "The built-in template did not use the interface language.");
    const newTemplate = [...document.querySelectorAll("button")].find(button => button.textContent === t("settings.template.new"));
    newTemplate.click();
    await until(() => document.querySelector(".template-editor[open]"), "The new template editor did not open.");
    const dialog = document.querySelector(".template-editor");
    const name = dialog.querySelector("input");
    const body = dialog.querySelector("textarea");
    assert(name.value === "" && body.value === "", "A new template was seeded with editable instructions or fixed-language headings.");
    assert(body.placeholder === t("settings.template.body_label"), "Template help did not follow the interface language.");
    name.value = customName;
    body.value = "## Mijn eigen kop\n- Mijn Nederlandse inhoud {name}";
    body.dispatchEvent(new Event("input", { bubbles: true }));
    await switchLanguage(api, before.settings, initial === "en" ? "nl" : "en");
    assert(dialog.isConnected && name.value === customName && body.value === "## Mijn eigen kop\n- Mijn Nederlandse inhoud {name}", "Language switching changed or discarded a custom template draft.");
    assert(body.placeholder === t("settings.template.body_label"), "The open template editor help did not relabel.");
    dialog.querySelector('button[type="button"]').click();
    await switchLanguage(api, before.settings, initial);
    assert([...document.querySelectorAll(".template-row")].some(row => row.querySelector(".grow")?.textContent === customName), "A custom Dutch template name was translated.");
    [...document.querySelectorAll("button")].find(button => button.textContent === t("settings.template.new")).click();
    await until(() => document.querySelector(".template-editor[open]"), "The empty editor did not reopen for capture.");
    assert(!fixture.unknown.length && !fixture.calls.includes("save_template"), "Template QA wrote content or called an unowned operation.");
    return { emptyStarter: true, liveHelp: true, customDraftPreserved: true, customNamePreserved: true };
  } finally { client.invoke = original; }
}

export async function checkEnglishModelMetadata() {
  const { fixture, client } = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { getInterfaceLanguage } = await import("/src/i18n.ts");
  const before = await api.info();
  const initial = getInterfaceLanguage();
  const original = client.invoke.bind(client);
  const vendorName = "Mijn eigen model";
  const makeModel = (id, name, description) => ({ id, name, description, size_bytes: 1000000, parameters: null, speed: 50, accuracy: 50, languages: ["en", "nl"], recommended: true, legacy: false, state: { kind: "downloaded" } });
  client.invoke = async (command, args) => command === "local_models" ? [
    makeModel("diar_streaming_sortformer_4spk-v2.1", "Sortformer (speakers)", "Recognizes up to 4 speakers in a meeting"),
    makeModel("qa-vendor", vendorName, "Vendor description"),
  ] : original(command, args);
  try {
    document.querySelector('[data-settings-category="dictation"]').click();
    await until(() => document.querySelector('[data-settings-category="dictation"][aria-current="page"]') && !document.querySelector('#content').inert && !document.querySelector('.template-row, .model-card'), "Dictation settings did not open.");
    fixture.emit("workspace-navigate", { section: "settings", tab: "local", meeting_id: null, jump_ms: null });
    await until(() => document.querySelector('[data-settings-category="advanced"][aria-current="page"]') && document.querySelector('[data-settings-detail="providers"][open]'), "Advanced local connections did not open.");
    await until(() => [...document.querySelectorAll(".model-card strong")].some(label => label.textContent.includes("Sortformer")), "Model cards did not finish mounting.");
    const label = [...document.querySelectorAll(".model-card strong")].find(label => label.textContent.includes("Sortformer"));
    const expected = language => language === "nl" ? "Sortformer (sprekers)" : "Sortformer (speakers)";
    assert(label.textContent === expected(initial), "The owned model name did not follow the interface language.");
    await switchLanguage(api, before.settings, initial === "en" ? "nl" : "en");
    assert(label.isConnected && label.textContent === expected(initial === "en" ? "nl" : "en"), "The owned model name did not relabel live.");
    assert([...document.querySelectorAll(".model-card strong")].some(label => label.textContent === vendorName), "A vendor model name was translated.");
    await switchLanguage(api, before.settings, initial);
    assert(!fixture.unknown.length, "Model QA called an unowned operation.");
    return { ownedNameLocalized: true, live: true, vendorNamePreserved: true };
  } finally { client.invoke = original; }
}

export async function checkEnglishRecordingErrors() {
  const { fixture } = ownedFixture();
  const { api } = await import("/src/api.ts");
  const { localizeBackendText } = await import("/src/i18n.ts");
  const before = await api.meetingRuntimeStatus();
  assert(before.error && before.terminal_error, "Use the recording-storage-error fixture.");
  await until(() => [...document.querySelectorAll(".error")].some(node => node.textContent.trim()), "The recording error did not render.");
  const text = document.body.innerText;
  assert(text.includes(localizeBackendText(before.error)), "The recording error did not use localized presentation.");
  const status = await api.meetingStatus();
  assert(status.notes.includes("Planning afronden"), "Error presentation changed the Dutch meeting notes.");
  const editor = document.querySelector("textarea");
  assert(!editor || editor.value === status.notes, "The visible note editor changed the stored Dutch notes.");
  assert(!fixture.unknown.length, "Recording error QA called an unowned operation.");
  return { errorLocalized: true, notesPreserved: true };
}
