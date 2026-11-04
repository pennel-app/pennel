/** Owned synthetic renderer checks: no native Keychain or user data is accessed. */
const assert = (value, message) => { if (!value) throw new Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 150; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 35));
  }
  throw new Error(message);
}
function ownedFixture() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__ && window.__PENNEL_PLATFORM__, "Saved-access checks require the owned synthetic fixture server.");
  return window.__PENNEL_FIXTURE__;
}
const panel = () => document.querySelector("[data-credential-access]");
const action = () => panel()?.querySelector("[data-credential-update]");

export async function checkSavedAccessMigration() {
  const fixture = ownedFixture();
  const { t, setInterfaceLanguage, getInterfaceLanguage } = await import("/src/i18n.ts");
  await until(() => action(), "Pending saved access has no update action.");
  assert(!fixture.calls.includes("migrate_saved_credentials"), "Opening settings prompted for legacy access automatically.");
  assert(panel().querySelector('[role="status"][aria-live="polite"]'), "Saved access lacks accessible status feedback.");
  const language = getInterfaceLanguage();
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  let requests = 0;
  let finish;
  let remaining = structuredClone(fixture.info().credential_store);
  client.invoke = async (command, args = {}) => {
    if (command === "app_info") return { ...await original(command, args), credential_store: structuredClone(remaining) };
    if (command !== "migrate_saved_credentials") return original(command, args);
    requests++;
    if (requests === 1) {
      remaining = { status: "denied", pending: 2 };
      throw new Error("Synthetic credential migration failure");
    }
    await new Promise(resolve => { finish = resolve; });
    const result = await original(command, args);
    remaining = structuredClone(result.credential_store);
    return result;
  };
  try {
    action().click();
    await until(() => panel()?.querySelector(".error") && !action()?.disabled, "Failed migration has no usable retry or error feedback.");
    assert(panel().querySelector(".error").textContent === t("credentials.denied"), "Partial migration failure did not refresh the remaining access status.");
    assert(panel().textContent.includes(t("credentials.pending", { count: 2 })), "Partial migration lost completed progress or reports an obsolete pending count.");
    action().click();
    await until(() => finish && action()?.disabled, "Explicit migration did not disable its action while pending.");
    action().click();
    assert(requests === 2 && panel().getAttribute("aria-busy") === "true", "A pending migration admitted a duplicate or has no busy state.");
    setInterfaceLanguage(language === "en" ? "nl" : "en");
    assert(action().textContent === t("credentials.updating"), "The in-progress action does not relabel with the interface language.");
    setInterfaceLanguage(language);
    fixture.emit("workspace-navigate", { section: "settings", tab: "account", meeting_id: null, jump_ms: null });
    await until(() => panel()?.getAttribute("aria-busy") === "true" && document.querySelector(".settings-pane h1")?.textContent === "Pennel Cloud", "Changing category lost the pending migration state.");
    assert(action().disabled && requests === 2, "The Account category admits a second migration.");
    assert(!fixture.calls.includes("cloud_account"), "Account read Cloud credentials while saved access required migration.");
    finish();
    await until(() => !panel() && fixture.calls.includes("cloud_account"), "The current category did not resume after saved access was updated.");
    assert(fixture.info().credential_store.status === "ready", "Migration did not update the current credential state.");
    assert(!fixture.unknown.length, "Migration used unsupported fixture IPC.");
    return { explicitOnly: true, retry: true, duplicateSuppressed: true, busyLocalization: true, navigationSafe: true, accountPausedUntilReady: true, secretsExposed: false, userDataUsed: false };
  } finally { client.invoke = original; setInterfaceLanguage(language); }
}

export async function checkSavedAccessStatus() {
  const fixture = ownedFixture();
  const { t } = await import("/src/i18n.ts");
  await until(() => action(), "The saved-access error state has no recovery action.");
  const state = fixture.info().credential_store.status;
  assert(["locked", "denied", "unavailable"].includes(state), "Use an unavailable saved-access fixture.");
  assert(panel().querySelector("[data-credential-message]").textContent === t(`credentials.${state}`), "Saved-access recovery does not explain the current failure.");
  assert(action().textContent === t("credentials.retry") && !action().disabled, "The saved-access state cannot be retried.");
  assert(!fixture.calls.includes("migrate_saved_credentials") && !fixture.calls.includes("cloud_account"), "Account recovery accessed saved credentials automatically.");
  assert(!panel().textContent.match(/provision|entitlement|B538|access.group|SecItem/i), "The product recovery flow exposes signing implementation details.");
  return { state, retryAvailable: true, noAutomaticRead: true, userDataUsed: false };
}
