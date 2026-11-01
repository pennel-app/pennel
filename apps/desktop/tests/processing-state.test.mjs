import { setInterfaceLanguage } from "../src/i18n.ts";
setInterfaceLanguage("nl");
import assert from "node:assert/strict";
import test from "node:test";
import { processingSummary, providerDestination } from "../src/processing-state.ts";
import { choiceFingerprint, ModelCatalogCache, ProviderVerification, taskFingerprint } from "../src/provider-verification.ts";

const providers = [
  { id: "speech", kind: "local", name: "Lokaal", base_url: "" },
  { id: "text", kind: "custom", name: "Mijn server", base_url: "https://username:secret@example.com/v1?key=secret" },
];
const settings = { providers, dictation_stt: { provider_id: "speech", model: "whisper" }, meeting_stt: { provider_id: "speech", model: "whisper" }, llm: { provider_id: "text", model: "cleanup" }, mode: "clean", language: "nl" };

test("processing explains local recognition and remote text processing separately", () => {
  assert.deepEqual(processingSummary(settings, "dictation"), [
    { label: "Audio", destination: "Op deze computer" },
    { label: "Tekst", destination: "Mijn server (example.com)" },
  ]);
  assert.equal(processingSummary({ ...settings, mode: "fast" }, "dictation")[1].destination, "Geen opkuisbeurt (Snel)");
  assert.match(processingSummary({ ...settings, mode: "fast" }, "meeting")[1].destination, /Mijn server/);
});

test("provider destinations hide credentials and do not claim a configurable server is offline", () => {
  const destination = providerDestination(providers[1]);
  assert.equal(destination, "Mijn server (example.com)");
  assert.doesNotMatch(destination, /secret|username|key=/);
  assert.equal(providerDestination({ id: "ollama", kind: "ollama", name: "Ollama", base_url: "https://remote.example/v1" }), "Ollama (remote.example)");
  assert.match(providerDestination({ id: "cli", kind: "codex", name: "Codex", base_url: "/bin/codex" }), /CLI-instellingen/);
  assert.equal(providerDestination(undefined), "Nog niet ingesteld");
});

test("task verification becomes stale after a model, endpoint, language or credential revision change", () => {
  const info = { settings, keys: { text: true }, provider_revisions: { text: 1 } };
  const fingerprint = taskFingerprint(info, "llm");
  const verification = new ProviderVerification();
  verification.set("llm", fingerprint, { verified: true, message: "Test geslaagd" });
  assert.equal(verification.get("llm", fingerprint)?.verified, true);
  for (const changed of [
    { ...info, settings: { ...settings, llm: { ...settings.llm, model: "another" } } },
    { ...info, settings: { ...settings, providers: providers.map((provider) => ({ ...provider, base_url: "https://new.example/v1" })) } },
    { ...info, settings: { ...settings, language: "en" } },
    { ...info, provider_revisions: { text: 2 } },
  ]) assert.notEqual(taskFingerprint(changed, "llm"), fingerprint);
  assert.equal(verification.get("llm", taskFingerprint({ ...info, provider_revisions: { text: 2 } }, "llm")), null);
  assert.equal(verification.get("llm", fingerprint), null);
});

test("failed model catalog loads can retry and an obsolete response cannot replace newer configuration", async () => {
  const cache = new ModelCatalogCache();
  let calls = 0;
  await assert.rejects(cache.get("provider", "first", async () => { calls++; throw new Error("offline"); }));
  assert.deepEqual(await cache.get("provider", "first", async () => { calls++; return ["first"]; }), ["first"]);
  assert.equal(calls, 2);
  let finishOld;
  const old = cache.get("provider", "old", () => new Promise((resolve) => { finishOld = resolve; }));
  assert.deepEqual(await cache.get("provider", "new", async () => ["new"]), ["new"]);
  finishOld(["old"]);
  await old;
  assert.deepEqual(await cache.get("provider", "new", async () => assert.fail("current result should be cached")), ["new"]);
  cache.invalidate("provider");
  assert.deepEqual(await cache.get("provider", "new", async () => ["refreshed"]), ["refreshed"]);
});

test("Cloud task and draft verification follows account identity while preserving routine token refreshes", () => {
  const cloud = { id: "cloud", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "https://api.pennel.app/v1" };
  const choice = { provider_id: cloud.id, model: "pennel-auto" };
  const info = { settings: { ...settings, providers: [...providers, cloud], llm: choice }, keys: {}, provider_revisions: { cloud: 1 }, cloud_account_epoch: "account-1", cloud_signed_in: true };
  const fingerprint = taskFingerprint(info, "llm");
  const draftFingerprint = choiceFingerprint(info, choice);
  const rotated = { ...info, keys: { cloud: true }, provider_revisions: { cloud: 2 } };
  assert.equal(taskFingerprint(rotated, "llm"), fingerprint);
  assert.equal(choiceFingerprint(rotated, choice), draftFingerprint);
  const verification = new ProviderVerification();
  verification.set("llm", fingerprint, { verified: true, message: "Synthetic Cloud test passed" });
  assert.equal(verification.get("llm", taskFingerprint(rotated, "llm"))?.verified, true);
  for (const change of [{ cloud_account_epoch: "account-2" }, { cloud_signed_in: false }]) {
    const changed = { ...rotated, ...change };
    assert.notEqual(taskFingerprint(changed, "llm"), fingerprint);
    assert.notEqual(choiceFingerprint(changed, choice), draftFingerprint);
  }
  assert.equal(verification.get("llm", taskFingerprint({ ...rotated, cloud_account_epoch: "account-2" }, "llm")), null);
});
