import test from "node:test";
import assert from "node:assert/strict";
import { setInterfaceLanguage } from "../src/i18n.ts";
import { isWritingTraitKey, writingConnectionText, writingErrorText, writingLanguageName, writingProgress, writingSourceSessions, writingTraitChoices } from "../src/writing-profile-state.ts";

test("both evidence gates limit readiness progress, including a large single meeting", () => {
  const limits = { min_words: 1000, min_sessions: 3 };
  assert.equal(writingProgress({ word_count: 8000, session_count: 1 }, limits), 1 / 3);
  assert.equal(writingProgress({ word_count: 500, session_count: 8 }, limits), 0.5);
  assert.equal(writingProgress({ word_count: 3000, session_count: 5 }, limits), 1);
  assert.equal(writingProgress({ word_count: 100, session_count: 1 }, { min_words: 0, min_sessions: 3 }), 0);
});

test("trait controls allow only supported values and retain stable values across UI languages", () => {
  const offered = ["direct", "balanced", "indirect", "PRIVATE_SOURCE_FACT"];
  setInterfaceLanguage("en");
  const options = writingTraitChoices("directness", offered, "direct");
  assert.deepEqual(options.map(([value]) => value), ["", "direct", "balanced", "indirect"]);
  assert.equal(String(options[0][1]), "Learned: Direct");
  setInterfaceLanguage("nl");
  assert.equal(String(options[0][1]), "Geleerd: Direct");
  assert.deepEqual(options.map(([value]) => value), ["", "direct", "balanced", "indirect"]);
  assert.equal(isWritingTraitKey("source_quotation"), false);
  assert.equal(isWritingTraitKey("constructor"), false);
  setInterfaceLanguage("en");
});

test("unknown failures cannot display source text or provider output", () => {
  const secret = "SOURCE_ONLY_PERSONAL_FACT_174c";
  assert.equal(writingErrorText(null), "");
  assert.equal(writingErrorText(secret), writingErrorText("analysis_failed"));
  assert.ok(!writingErrorText(secret).includes(secret));
  assert.match(writingErrorText("provider_unavailable"), /selected text model/i);
});

test("sharing reflects sign-in and designated source rather than only the local switch", () => {
  const connection = { enabled: true, signed_in: true, designated: true, mcp_url: "https://example.test/mcp/style" };
  assert.match(writingConnectionText(connection), /This computer/);
  assert.match(writingConnectionText({ ...connection, designated: false }), /Another computer/);
  assert.match(writingConnectionText({ ...connection, enabled: false }), /off/);
  assert.match(writingConnectionText({ ...connection, enabled: false, error: "connection_unavailable" }), /off/);
  assert.match(writingConnectionText({ ...connection, signed_in: false }), /Sign in/);
  assert.ok(!writingConnectionText({ ...connection, error: "PRIVATE_SOURCE_FACT" }).includes("PRIVATE_SOURCE_FACT"));
});

test("language labels relabel without changing the profile's language identity", () => {
  setInterfaceLanguage("en");
  const name = writingLanguageName("nl");
  assert.equal(String(name), "Dutch");
  setInterfaceLanguage("nl");
  assert.equal(String(name), "Nederlands");
  setInterfaceLanguage("en");
});

test("long captures appear once while retaining their full word count and exclusion anchor", () => {
  const source = { id: "first-chunk", session_id: "one-recording", kind: "meeting", created_at_ms: 1, title: "Keep this title", fingerprint: "f", word_count: 300, language: "en", excluded: false, analyzed: true };
  const sources = [source, { ...source, id: "second-chunk", word_count: 250 }, { ...source, id: "another-session", session_id: "another-recording", word_count: 100 }];
  const grouped = writingSourceSessions(sources);
  assert.equal(grouped.length, 2);
  assert.equal(grouped[0].id, "first-chunk");
  assert.equal(grouped[0].word_count, 550);
  assert.equal(grouped[0].title, source.title);
  assert.equal(sources[0].word_count, 300);
  const mixed = writingSourceSessions([source, { ...source, id: "mixed", language: "nl", analyzed: false, excluded: true }]);
  assert.equal(mixed[0].language, null);
  assert.equal(mixed[0].analyzed, false);
  assert.equal(mixed[0].excluded, true);
});
