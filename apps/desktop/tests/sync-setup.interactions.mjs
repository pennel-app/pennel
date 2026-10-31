/** Run only in the owned fixture; every account, code and meeting here is synthetic. */
const assert = (value, message) => { if (!value) throw new Error(message); };
const visible = node => node?.getClientRects().length && !node.closest("details:not([open])");
const button = label => [...document.querySelectorAll("button")].find(node => visible(node) && node.textContent.trim() === label);
async function until(check, message) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return;
    await new Promise(resolve => setTimeout(resolve, 40));
  }
  throw new Error(message);
}
function fixture() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Synthetic fixture required.");
  return window.__PENNEL_FIXTURE__;
}

export async function checkSyncSetup() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const resume = button(t("account.setup.resume"));
  assert(resume && !resume.disabled, "A validated paused library has no Resume action.");
  assert(!document.querySelector("input[aria-label='Recovery code']"), "Resume unexpectedly requires recovery entry.");
  resume.click();
  await until(() => button(t("account.setup.pause")), "Resume did not activate the sync controls.");
  assert(state.calls.includes("cloud_sync_resume"), "Resume used the old create-vault operation.");
  assert(!state.calls.includes("cloud_sync_enable"), "Resume created a new library.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkDeviceApproval() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const approve = button(t("account.link.approve"));
  assert(approve && !approve.disabled, "Explicit device approval is missing.");
  assert(document.body.textContent.includes("481 926"), "The verification code is missing.");
  assert(!state.calls.includes("cloud_link_approve"), "Approval happened before a user action.");
  approve.click();
  await until(() => document.body.textContent.includes(t("account.setup.connected")), "Approved connection did not show its result.");
  assert(state.calls.filter(command => command === "cloud_link_approve").length === 1, "Approval was dispatched more than once.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkFirstSyncBackup() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const prepare = button(t("account.setup.create"));
  assert(prepare && !prepare.disabled, "First-device sync setup is missing.");
  prepare.click();
  await until(() => button(t("account.recovery.saved")), "The recovery backup step did not open.");
  assert(state.info().settings.cloud_sync === false, "Sync started before backup acknowledgment.");
  assert(state.calls.includes("cloud_sync_prepare") && !state.calls.includes("cloud_recovery_confirm"), "Backup preparation activated sync.");
  button(t("account.recovery.saved")).click();
  await until(() => button(t("account.setup.pause")), "Acknowledging the backup did not activate sync.");
  assert(state.calls.filter(command => command === "cloud_recovery_confirm").length === 1, "Activation was not dispatched once.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkLateRecoveryAfterLogout() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  button(t("account.setup.create")).click();
  await until(() => state.calls.includes("cloud_sync_prepare"), "Recovery preparation did not start.");
  document.querySelector(".account-management").open = true;
  const logout = button(t("account.logout.action"));
  assert(logout, "Sign-out is unavailable while preparation is pending.");
  logout.click();
  await until(() => button(t("account.setup.email")), "Sign-out did not reach setup.");
  await new Promise(resolve => setTimeout(resolve, 900));
  assert(!document.querySelector(".recovery-code"), "A late recovery response restored a signed-out secret.");
  assert(!state.calls.includes("cloud_recovery_code_context"), "The signed-out response continued into recovery context validation.");
  assert(!localStorage.getItem("pennel.recoveryCodeConfirmed"), "A stale operation restored acknowledgment bookkeeping after sign-out.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkRecoveryRotation() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const { api } = await import("/src/api.ts");
  button(t("account.setup.create")).click();
  await until(() => button(t("account.recovery.saved")), "The first recovery code was not displayed.");
  await api.cloudSyncNewRecoveryCode();
  button(t("account.recovery.saved")).click();
  await until(() => button(t("account.setup.replace_backup")), "An outdated displayed recovery code was not rejected.");
  assert(state.info().settings.cloud_sync === false, "Acknowledging an outdated code activated sync.");
  assert(localStorage.getItem("pennel.recoveryCodeConfirmed") === "false", "An outdated code was marked saved.");
  assert(!document.querySelector(".recovery-code"), "The invalidated recovery code remains displayed.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkConnectionUnavailable() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const connect = button(t("account.setup.connect"));
  const manual = button(t("account.link.manual"));
  assert(connect?.disabled && manual?.disabled, "Unavailable admission still offers an active create or join action.");
  const reason = t("account.error.unavailable");
  assert([...document.querySelectorAll("p")].filter(node => node.textContent === reason).length === 1, "Availability does not have one clear explanation.");
  assert(button(t("account.setup.email")) && !button(t("account.setup.email")).disabled, "Email recovery is unreachable while admission is disabled.");
  const before = state.calls.filter(command => command === "cloud_link_availability").length;
  button(t("account.link.check_again")).click();
  await until(() => state.calls.filter(command => command === "cloud_link_availability").length > before, "Check again did not recheck service capability.");
  await until(() => button(t("account.link.check_again")) && !button(t("account.link.check_again")).disabled, "The availability check did not finish.");
  assert(!state.calls.includes("cloud_link_start") && !state.calls.includes("cloud_link_join"), "Unavailable admission sent an unnecessary create or join.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkConnectionStartFailure() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const { accountFailureText } = await import("/src/account-errors.ts");
  button(t("account.setup.connect")).click();
  await until(() => button(t("account.link.retry")), "Failed creation has no retry action.");
  const alerts = [...document.querySelectorAll('[role="alert"]')].filter(visible);
  assert(alerts.length === 1, `One failed creation rendered ${alerts.length} error messages.`);
  assert(alerts[0].textContent === accountFailureText({ code: state.options.link_start_error }), "The latest classified failure was not displayed.");
  assert(!document.body.textContent.includes(t("account.link.phase.failed")), "The generic failed-phase copy duplicates the error.");
  button(t("account.link.manual")).click();
  await until(() => document.querySelector(".device-link-entry"), "Manual entry did not open.");
  assert(![...document.querySelectorAll('[role="alert"]')].some(visible), "Manual entry retained the preceding failed attempt.");
  button(t("common.cancel")).click();
  await until(() => button(t("account.setup.connect")), "Cancel did not return to connection setup.");
  button(t("account.setup.connect")).click();
  await until(() => document.querySelector(".device-link-code"), "A fresh creation did not display its connection code.");
  assert(![...document.querySelectorAll('[role="alert"]')].some(visible), "Successful retry retained the preceding failure.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkConnectionAccessDenied() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const alerts = [...document.querySelectorAll('[role="alert"]')].filter(visible);
  assert(alerts.length === 1 && alerts[0].textContent === t("account.error.service_access"), "Service access denial was not shown once with useful copy.");
  assert(button(t("account.setup.connect"))?.disabled, "Access-denied service still allows connection creation.");
  assert(button(t("account.setup.email")) && !button(t("account.setup.email")).disabled, "Access denial hides email recovery.");
  assert(button(t("account.link.check_again")), "Access denial has no recheck action.");
  assert(!state.calls.includes("cloud_link_start") && !state.calls.includes("cloud_link_join"), "Access denial attempted admission.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}

export async function checkConnectionAvailabilityLoginRace() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const { api } = await import("/src/api.ts");
  const original = api.cloudLinkAvailability;
  let release;
  let checks = 0;
  const pending = new Promise(resolve => { release = resolve; });
  api.cloudLinkAvailability = () => ++checks === 1 ? pending : Promise.resolve({ available: true, reason_code: null });
  try {
    button(t("account.link.check_again")).click();
    await until(() => checks === 1, "Availability retry did not start.");
    button(t("account.setup.email")).click();
    await until(() => button(t("account.login.cancel")), "Email fallback did not open while availability was pending.");
    button(t("account.login.cancel")).click();
    await until(() => button(t("account.setup.connect")) && !button(t("account.setup.connect")).disabled, "A superseded availability request left admission controls busy after login cancellation.");
    release({ available: false, reason_code: "pairing_unavailable" });
    await new Promise(resolve => setTimeout(resolve, 100));
    assert(!button(t("account.setup.connect")).disabled, "A late availability response replaced the current result.");
    assert(!document.body.textContent.includes(t("account.error.unavailable")), "The superseded availability response restored stale guidance.");
    assert(!state.calls.includes("cloud_link_start") && !state.calls.includes("cloud_link_join"), "Login cancellation unexpectedly admitted a connection.");
    assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
  } finally {
    release({ available: false, reason_code: "pairing_unavailable" });
    api.cloudLinkAvailability = original;
  }
}

export async function checkDelayedAccountEntry() {
  const state = fixture();
  const { t } = await import("/src/i18n.ts");
  const account = document.querySelector('[data-settings-category="account"]');
  assert(account && account.getAttribute("aria-current") !== "page", "Start this regression from another Settings category.");
  account.click();
  await until(() => document.querySelector('.settings-pane .settings-loading[role="status"]'), "Delayed account metadata did not expose its owned loading state.");
  await until(() => state.calls.includes("cloud_sync_setup"), "Account did not request sync setup.");
  await until(() => button(t("account.setup.create")), "Account stayed loading after device availability and delayed sync setup completed.");
  assert(document.querySelector('[data-settings-category="account"][aria-current="page"]'), "Finishing metadata navigated away from Account.");
  assert(!document.querySelector(".settings-pane .settings-loading"), "Account retained the loading placeholder beside its completed content.");
  assert(state.calls.filter(command => command === "cloud_link_availability").length === 1, "Overlapping Account renders repeated the availability request.");
  assert(!state.calls.includes("cloud_sync_prepare") && !state.calls.includes("cloud_link_start"), "Rendering Account performed a setup or connection mutation.");
  assert(!state.unknown.length, `Unknown commands: ${state.unknown.join(", ")}`);
}
