import assert from "node:assert/strict";
import test from "node:test";
import { setInterfaceLanguage } from "../src/i18n.ts";
import { allSettingsEntries, searchSettings } from "../src/settings-search.ts";

test("settings search returns bilingual matches across EN and NL queries", () => {
  setInterfaceLanguage("nl");
  const nlApi = searchSettings("api sleutel");
  assert.ok(nlApi.length > 0);
  assert.equal(nlApi[0].entry.id, "ai-cloud-providers");
  assert.equal(nlApi[0].entry.category, "ai-models");
  assert.equal(nlApi[0].entry.anchor, "ai-providers");

  const nlOllama = searchSettings("ollama");
  assert.ok(nlOllama.some((match) => match.entry.id === "ai-advanced-providers"));

  const nlTheme = searchSettings("donker");
  assert.ok(nlTheme.some((match) => match.entry.id === "general-appearance" && match.entry.category === "appearance"));

  setInterfaceLanguage("en");
  const enWhisper = searchSettings("whisper");
  assert.ok(enWhisper.some((match) => match.entry.id === "ai-local-models"));

  const enReview = searchSettings("action items");
  assert.ok(enReview.some((match) => match.entry.id === "meetings-after" && match.entry.category === "meeting-settings"));
});

test("settings search returns empty array for blank query and lists all entries with resolved titles", () => {
  setInterfaceLanguage("en");
  assert.deepEqual(searchSettings(""), []);
  assert.deepEqual(searchSettings("   "), []);
  const all = allSettingsEntries();
  assert.ok(all.length >= 20);
  for (const item of all) {
    assert.ok(item.title.length > 0, `expected resolved title for ${item.entry.id}`);
    assert.ok(item.categoryLabel.length > 0, `expected resolved category label for ${item.entry.id}`);
  }
});
