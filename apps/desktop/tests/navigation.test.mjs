import assert from "node:assert/strict";
import test from "node:test";
import { localNavigation } from "../src/navigation.ts";

test("a provider broadcast is ignored by onboarding, history and library", () => {
  for (const window of ["onboarding", "history", "library", "live", "prompt"]) {
    assert.equal(localNavigation(window, "providers"), null);
  }
  assert.deepEqual(localNavigation("settings", "providers"), { kind: "settings", tab: "providers", param: undefined });
});

test("the account tab is a local settings route", () => {
  assert.deepEqual(localNavigation("settings", "account"), { kind: "settings", tab: "account", param: undefined });
  assert.equal(localNavigation("library", "account"), null);
});

test("settings routes apply locally and preserve the two compatibility aliases", () => {
  assert.deepEqual(localNavigation("settings", "dictionary"), { kind: "settings", tab: "dictionary", param: undefined });
  assert.deepEqual(localNavigation("settings", "meetings"), { kind: "settings", tab: "meeting-settings", param: undefined });
  assert.deepEqual(localNavigation("settings", "local"), { kind: "settings", tab: "providers", param: "__local__" });
  assert.equal(localNavigation("settings", "history"), null);
});

test("the library accepts meeting identifiers but ignores other window routes", () => {
  const meetingId = "2026-10-24 0900 Projectoverleg";
  assert.deepEqual(localNavigation("library", meetingId), { kind: "library", param: meetingId });
  assert.deepEqual(localNavigation("library", "chat"), { kind: "library", param: "__chat__" });
  for (const route of ["dictionary", "models", "dictation", "recording", "history", "onboarding", "settings", "live"]) {
    assert.equal(localNavigation("library", route), null);
  }
});

test("invalid meeting paths and non-string payloads are ignored", () => {
  for (const route of ["", " ", ".", "..", "../meeting", "a/b", "a\\b", "a:b", "a\n", null, {}, 42]) {
    assert.equal(localNavigation("library", route), null);
  }
});

test("a live recording route resets only its own window", () => {
  assert.deepEqual(localNavigation("live", "meeting"), { kind: "live" });
  assert.equal(localNavigation("live", "providers"), null);
});

test("task settings retain Advanced and Appearance links without treating them as meeting IDs", () => {
  for (const tab of ["advanced", "appearance", "overview"]) {
    assert.deepEqual(localNavigation("settings", tab), { kind: "settings", tab, param: undefined });
    assert.equal(localNavigation("library", tab), null);
  }
});
