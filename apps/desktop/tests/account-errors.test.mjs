import assert from "node:assert/strict";
import test from "node:test";
import { accountFailureText } from "../src/account-errors.ts";
import { setInterfaceLanguage, t } from "../src/i18n.ts";

test("connection and setup failures have actionable copy in both interface languages", () => {
  for (const language of ["en", "nl"]) {
    setInterfaceLanguage(language);
    for (const [codes, key] of [
      [["pairing_unavailable"], "account.error.unavailable"],
      [["pairing_incompatible"], "account.error.incompatible"],
      [["http_403"], "account.error.service_access"],
      [["pairing_storage", "storage"], "account.error.storage"],
      [["pairing_claimed", "pairing_used", "pairing_busy"], "account.error.fresh_code"],
      [["pairing_account", "pairing_existing"], "account.error.account"],
      [["pairing_invalid", "pairing_changed", "pairing_transcript", "pairing_message", "pairing_phase", "pairing_approval"], "account.error.verify"],
      [["network", "internal", "pairing_retry", "http_408", "http_500", "http_504", "http_599"], "account.error.network"],
      [["rate_limited", "http_429"], "account.error.rate"],
      [["device_limit"], "account.error.device_limit"],
      [["generation", "pairing_library_changed"], "account.error.library_changed"],
      [["pairing_pending"], "account.error.pending"],
    ]) {
      for (const code of codes) assert.equal(accountFailureText({ code, message: "Canonical English should not leak" }), t(key), `${language}: ${code}`);
    }
    const storage = accountFailureText({ code: "pairing_storage" });
    assert.match(storage, language === "en" ? /same pending connection/i : /dezelfde .*verbinding/i);
    const unavailable = accountFailureText({ code: "pairing_unavailable" });
    assert.match(unavailable, language === "en" ? /email.*recovery code/i : /e-mail.*herstelcode/i);
  }
  setInterfaceLanguage("en");
});

test("unknown account errors remain localized and do not expose raw upstream text", () => {
  setInterfaceLanguage("nl");
  for (const error of [new Error("raw private body"), "raw private body", { code: "pairing_future", message: "raw private body" }]) {
    assert.equal(accountFailureText(error), t("account.error.generic"));
    assert.ok(!accountFailureText(error).includes("raw private body"));
  }
  setInterfaceLanguage("en");
});
