import test from "node:test";
import assert from "node:assert/strict";
import { catalogs, formatDateTime, formatNumber, getInterfaceLanguage, setInterfaceLanguage, t, l, liveText, bindLocalization, refreshLocalization, localizeBackendText } from "../src/i18n.ts";

test("unresolved or unsupported resolved-language values fall back to English", () => {
  setInterfaceLanguage(undefined);
  assert.equal(getInterfaceLanguage(), "en");
  assert.equal(t("common.cancel"), "Cancel");
  setInterfaceLanguage("unsupported");
  assert.equal(t("common.cancel"), "Cancel");
});

test("Dutch is selectable independently and catalog keys/placeholders match", () => {
  assert.deepEqual(Object.keys(catalogs.en).sort(), Object.keys(catalogs.nl).sort());
  for (const key of Object.keys(catalogs.en)) {
    const placeholders = value => [...value.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map(item => item[1]).sort();
    assert.deepEqual(placeholders(catalogs.en[key]), placeholders(catalogs.nl[key]), key);
  }
  setInterfaceLanguage("nl");
  assert.equal(t("common.cancel"), "Annuleren");
  setInterfaceLanguage("en");
});

test("locale formatting changes presentation without changing stored values", () => {
  const timestamp = "2026-10-25T09:00:00Z";
  const millis = Date.parse(timestamp);
  setInterfaceLanguage("en");
  const englishDate = formatDateTime(millis, { month: "long", timeZone: "UTC" });
  assert.equal(formatNumber(1.5), "1.5");
  setInterfaceLanguage("nl");
  assert.notEqual(formatDateTime(millis, { month: "long", timeZone: "UTC" }), englishDate);
  assert.equal(formatNumber(1.5), "1,5");
  assert.equal(Date.parse(timestamp), millis);
  setInterfaceLanguage("en");
});


test("live bindings update only their owned presentation and leave input content untouched", () => {
  setInterfaceLanguage("en");
  const owner = { isConnected: true, caption: "", value: "Kies een spraakmodel. {count} <script>" };
  const label = l("common.cancel");
  let calls = 0;
  const unbind = bindLocalization(owner, () => { calls++; owner.caption = String(label); });
  assert.equal(calls, 0);
  const computed = liveText(() => `${t("common.copy")} 1`);
  setInterfaceLanguage("nl");
  assert.equal(owner.caption, "Annuleren");
  assert.equal(String(computed), "Kopiëren 1");
  assert.equal(owner.value, "Kies een spraakmodel. {count} <script>");
  setInterfaceLanguage("en");
  assert.equal(owner.caption, "Cancel");
  assert.equal(calls, 2);
  unbind();
  setInterfaceLanguage("nl");
  assert.equal(calls, 2);
  setInterfaceLanguage("en");
});

test("known app-owned errors relabel in both directions without translating arbitrary content", () => {
  const source = "Kies een spraakmodel.";
  setInterfaceLanguage("en");
  const english = localizeBackendText(source);
  assert.notEqual(english, source);
  setInterfaceLanguage("nl");
  assert.equal(localizeBackendText(english), source);
  assert.equal(localizeBackendText(`Mijn notitie zegt: ${source}`), `Mijn notitie zegt: ${source}`);
  setInterfaceLanguage("en");
});

test("legacy dictation notices retain their Dutch-to-English compatibility mappings", () => {
  const notices = [
    ["API key rejected — saved in History", "API-sleutel geweigerd — bewaard in geschiedenis"],
    ["Cleanup failed — original text available (see log)", "Opkuisen mislukt — ruwe tekst beschikbaar (zie logboek)"],
    ["Cleanup failed — original text pasted (see log)", "Opkuisen mislukt — ruwe tekst geplakt (zie logboek)"],
    ["Cleanup timed out — original text available (see log)", "Opkuisen duurde te lang — ruwe tekst beschikbaar (zie logboek)"],
    ["Cleanup timed out — original text pasted (see log)", "Opkuisen duurde te lang — ruwe tekst geplakt (zie logboek)"],
    ["Copying failed — your text is in History", "Kopiëren mislukt — tekst staat in Geschiedenis"],
    ["Failed — saved in History", "Mislukt — bewaard in geschiedenis"],
    ["Model not downloaded — saved in History", "Model nog niet gedownload — bewaard in geschiedenis"],
    ["No connection — saved in History", "Geen verbinding — bewaard in geschiedenis"],
    ["Processor too old for local models — saved in History", "Processor te oud voor lokale modellen — bewaard in geschiedenis"],
    ["Text copied — paste with Ctrl+V", "Tekst gekopieerd — plak met Ctrl+V"],
    ["Cleanup could not preserve the wording and meaning — original text available", "Opschonen kon de bewoording en betekenis niet behouden — oorspronkelijke tekst beschikbaar"],
    ["Cleanup could not preserve the wording and meaning — original text pasted", "Opschonen kon de bewoording en betekenis niet behouden — oorspronkelijke tekst geplakt"],
  ];
  try {
    for (const [english, dutch] of notices) {
      setInterfaceLanguage("en");
      assert.equal(localizeBackendText(dutch), english);
      assert.equal(localizeBackendText(`My note says: ${dutch}`), `My note says: ${dutch}`);
      setInterfaceLanguage("nl");
      assert.equal(localizeBackendText(english), dutch);
    }
  } finally { setInterfaceLanguage("en"); }
});

test("recovery diagnostics follow the interface language while provider responses stay literal", () => {
  const diagnostics = [
    ["The recovery audio changed before recognition.", "De herstelaudio is gewijzigd voordat de spraakherkenning begon."],
    ["The recovery audio changed during recognition.", "De herstelaudio is tijdens de spraakherkenning gewijzigd."],
    ["Some recovery audio cannot be linked to its original transcript. Keep the files and review their sources before retrying recognition.", "Een deel van de herstelaudio kan niet aan het oorspronkelijke transcript worden gekoppeld. Bewaar de bestanden en controleer de bronnen voordat je de spraakherkenning opnieuw probeert."],
    ["Restore the transcript's source metadata before retrying recognition.", "Herstel de broninformatie van het transcript voordat je de spraakherkenning opnieuw probeert."],
  ];
  try {
    for (const language of ["en", "nl"]) {
      setInterfaceLanguage(language);
      for (const [english, dutch] of diagnostics) {
        assert.equal(localizeBackendText(english), language === "nl" ? dutch : english);
        assert.equal(localizeBackendText(dutch), language === "nl" ? dutch : english);
      }
      const response = "Provider response: The recovery audio changed before recognition. {message} <audio>";
      assert.equal(localizeBackendText(response), response);
    }
  } finally { setInterfaceLanguage("en"); }
});

test("microphone-owner refusals follow the interface without rewriting meeting content", () => {
  const refusals = [
    ["This meeting has no attributable personal microphone speech.", "Deze vergadering bevat geen bruikbare spraak van je eigen microfoon."],
    ["Review the microphone's existing manual speaker assignments before confirming its owner.", "Controleer eerst de bestaande handmatige sprekerstoewijzingen voor deze microfoon voordat je de eigenaar bevestigt."],
  ];
  try {
    for (const language of ["en", "nl"]) {
      setInterfaceLanguage(language);
      for (const [english, dutch] of refusals) {
        assert.equal(localizeBackendText(english), language === "nl" ? dutch : english);
        assert.equal(localizeBackendText(dutch), language === "nl" ? dutch : english);
        const note = `Meeting note: ${english}`;
        assert.equal(localizeBackendText(note), note);
      }
    }
  } finally { setInterfaceLanguage("en"); }
});


test("a view that finishes mounting after a language change catches up without rebuilding", () => {
  setInterfaceLanguage("en");
  const owner = { isConnected: false, caption: "Cancel", value: "unsaved", contains(node) { return node === this; } };
  let calls = 0;
  bindLocalization(owner, () => { owner.caption = t("common.cancel"); calls++; });
  setInterfaceLanguage("nl");
  assert.equal(calls, 0);
  owner.isConnected = true;
  refreshLocalization(owner);
  assert.equal(owner.caption, "Annuleren");
  assert.equal(owner.value, "unsaved");
  refreshLocalization(owner);
  assert.equal(calls, 1);
  owner.isConnected = false;
  setInterfaceLanguage("en");
});
