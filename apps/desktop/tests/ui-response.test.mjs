import test from "node:test";
import assert from "node:assert/strict";
import { getInterfaceLanguage, setInterfaceLanguage } from "../src/i18n.ts";
import { localizeUiResponse } from "../src/ui-response.ts";

test("metadata follows the response locale without changing runtime state or speech configuration", () => {
  const settings = { ui_language: "nl", language: "nl", mode: "fast", dictionary: ["Kopiëren"], providers: [{ name: "Mijn provider" }] };
  setInterfaceLanguage("en");
  const response = localizeUiResponse("save_settings", { interface_language: "nl", settings, kinds: [], capabilities: {}, hotkey_error: "Choose a speech model." });
  assert.equal(response.hotkey_error, "Kies een spraakmodel.");
  assert.equal(getInterfaceLanguage(), "en");
  assert.deepEqual(response.settings, settings);
  localizeUiResponse("save_settings", { interface_language: "en", system_interface_language: "en", settings: { ...settings, ui_language: "system" }, kinds: [], capabilities: {} });
  assert.equal(getInterfaceLanguage(), "en");
  setInterfaceLanguage("en");
});

test("saved-access migration localizes only owned metadata and preserves the credential status", () => {
  const credential_store = { status: "migration_required", pending: 2 };
  const settings = { ui_language: "nl", language: "en", dictionary: ["Choose a speech model."] };
  const response = localizeUiResponse("migrate_saved_credentials", {
    interface_language: "nl", settings, kinds: [], capabilities: {}, credential_store,
    migration_warning: "Choose a speech model.",
  });
  assert.equal(response.migration_warning, "Kies een spraakmodel.");
  assert.deepEqual(response.credential_store, credential_store);
  assert.deepEqual(response.settings, settings);
});

test("only known UI message fields translate; identical user words and machine states do not", () => {
  setInterfaceLanguage("en");
  const content = "Kies een spraakmodel.";
  const status = { error: content, notes: content, title: content, transcript: [{ text: content }], phase: "error", session_id: content };
  const result = localizeUiResponse("meeting_status", status);
  assert.notEqual(result.error, content);
  assert.equal(result.notes, content);
  assert.equal(result.title, content);
  assert.equal(result.session_id, content);
  assert.equal(result.phase, "error");
  assert.deepEqual(result.transcript, status.transcript);
  assert.deepEqual(status.error, content);
  assert.deepEqual(localizeUiResponse("history", [{ text: content, raw: content }]), [{ text: content, raw: content }]);
  const arbitrary = { settings: { ui_language: "nl" }, kinds: [], hotkey_error: content };
  assert.deepEqual(localizeUiResponse("user_document", arbitrary), arbitrary);
  assert.equal(getInterfaceLanguage(), "en");
  assert.deepEqual(localizeUiResponse("download-done", { error: "cancelled" }), { error: "cancelled" });
});

test("meeting retranscription metadata translates without rewriting meeting content", () => {
  setInterfaceLanguage("en");
  const text="Er is geen bewaarde audio om opnieuw te transcriberen.";
  const value={ notes:text, transcript:[{text}], retranscription_plan:{available:false,reason:text}, retranscription_status:{running:false,error:text,warning:null} };
  const result=localizeUiResponse("meeting_detail",value);
  assert.equal(result.retranscription_plan.reason,"There is no saved audio to transcribe again.");
  assert.equal(result.retranscription_status.error,"There is no saved audio to transcribe again.");
  assert.equal(result.notes,text);
  assert.equal(result.transcript[0].text,text);
});

test("lightweight recording metadata localizes errors while preserving identity and revision", () => {
  setInterfaceLanguage("en");
  const status = { title: "Kies een spraakmodel.", error: "Kies een spraakmodel.", session_id: "qa-session", transcript_revision: 12, busy: true };
  const value = localizeUiResponse("meeting_runtime_status", status);
  assert.equal(value.error, "Choose a speech model.");
  assert.equal(value.title, status.title);
  assert.equal(value.session_id, status.session_id);
  assert.equal(value.transcript_revision, 12);
  assert.equal(Object.hasOwn(value, "transcript"), false);
});

test("local model metadata localizes owned labels while preserving vendor names and IDs", () => {
  setInterfaceLanguage("nl");
  const models = [
    { id: "diar_streaming_sortformer_4spk-v2.1", name: "Sortformer (speakers)", description: "Recognizes up to 4 speakers in a meeting", languages: ["en", "nl"] },
    { id: "vendor-model", name: "Sortformer (speakers)", description: "Vendor description", languages: ["en"] },
  ];
  const result = localizeUiResponse("local_models", models);
  assert.equal(result[0].name, "Sortformer (sprekers)");
  assert.equal(result[0].description, "Herkent tot 4 sprekers in een meeting");
  assert.equal(result[0].id, models[0].id);
  assert.deepEqual(result[0].languages, models[0].languages);
  assert.deepEqual(result[1], models[1]);
  assert.equal(models[0].name, "Sortformer (speakers)");
  setInterfaceLanguage("en");
});
