import test from "node:test";
import assert from "node:assert/strict";
import { questionStarterText } from "../src/question-starters.ts";
import { setInterfaceLanguage } from "../src/i18n.ts";

test("question content follows the selected speech language independently of interface labels", () => {
  setInterfaceLanguage("en");
  const dutchQuestion = questionStarterText("decisions", "nl");
  assert.equal(dutchQuestion, "Welke besluiten zijn er genomen?");
  setInterfaceLanguage("nl");
  assert.equal(questionStarterText("decisions", "nl"), dutchQuestion);
  assert.equal(questionStarterText("actions", "en"), "What are the next steps?");
  setInterfaceLanguage("en");
});

test("automatic and unsupported content languages have a stable English fallback", () => {
  for (const language of [null, "", "auto", "de"]) {
    assert.equal(questionStarterText("questions", language), "Which questions remain unanswered?");
  }
  assert.equal(questionStarterText("actions", "nl-BE"), "Wat zijn de volgende stappen?");
});
