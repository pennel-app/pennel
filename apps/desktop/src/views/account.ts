import { api, type ClaudeAccessView, type ClaudeStatus, type CloudAccount, type CloudDevice, type CloudLinkAvailability, type CloudLinkRole, type CloudLinkView, type CloudSyncSetup, type Settings, type SyncReport } from "../api";
import { claudeInstructions, claudeStatusText } from "../claude-access";
import { accountScreenFor, afterSignOut, copyStatusText, formatUsage, initialAccountUi, planSyncRedraw, pollLogin, recoveryNeedsConfirmation, relativeTime, type LoginPoll } from "../cloud-login";
import { controlButton, textField, confirmDialog, errorCode, errorText, h, section, toggle } from "../dom";
import type { Context } from "../main";
import { bindLocalization, liveText, l, localizeBackendText, t, uiLocale, type UiText } from "../i18n";
import { connectionFeedback, connectionRole, linkIsPending, qrImageSource, syncRequiresBackup, syncSetupAction } from "../device-link";
import { AccountActionEpoch } from "../account-actions";
import { accountFailureText } from "../account-errors";
import "../sync.css";

const LOGIN_LIFETIME_MS = 15 * 60 * 1000;
const SYNC_NOW_TIMEOUT_MS = 60 * 1000;
const CONFIRMED_KEY = "pennel.recoveryCodeConfirmed";

function confirmedFlag(): boolean | null {
  const value = localStorage.getItem(CONFIRMED_KEY);
  return value === null ? null : value === "true";
}

function setConfirmed(value: boolean) { localStorage.setItem(CONFIRMED_KEY, String(value)); }

function showCode(code: string, context: string) {
  ui.shownCode = code;
  recoveryContext = context;
  setConfirmed(false);
}

function heading() { return h("h1", {}, "Pennel Cloud"); }

function plainSection(...children: (Node | null | false)[]) {
  return h("section", { class: "pane-section" }, ...children);
}

let ui = initialAccountUi();
let loginRun = 0;
let syncingTimer: number | undefined;
let redraw: (() => void) | null = null;
let listening = false;
let shownReport: SyncReport | null = null;
let syncBlock: { element: HTMLElement; build: (report: SyncReport) => HTMLElement } | null = null;
let claudeShown: { line: HTMLElement; enabled: boolean; status: ClaudeStatus } | null = null;
let listeningClaude = false;
let linkView: CloudLinkView | null = null;
let linkTimer: number | undefined;
let linkRun = 0;
let linkBusy = false;
let linkManual = false;
let linkDraft = "";
let linkFailure: unknown | null = null;
let linkAvailability: CloudLinkAvailability | null = null;
let linkAvailabilityFailure: unknown | null = null;
let linkAvailabilityBusy = false;
let linkAvailabilityRun = 0;
let linkAvailabilityRequest: Promise<void> | null = null;
let accountSignedIn = false;
const accountActions = new AccountActionEpoch();
let recoveryContext: string | null = null;
let allowCachedLink = true;
let accountViewRun = 0;
let noticeFailure: unknown | null = null;

function clearNotice() { noticeFailure = null; ui.notice = null; }
function setNoticeError(error: unknown) { noticeFailure = error; ui.notice = accountFailureText(error); }
function noticeText(): string { return noticeFailure ? accountFailureText(noticeFailure) : ui.notice ?? ""; }

function update() { redraw?.(); }

function renamingDevice(): boolean {
  return [...document.querySelectorAll<HTMLInputElement>("input.device-name")]
    .some((input) => input === document.activeElement || input.value !== input.dataset.original);
}

function listenForSync() {
  if (listening) return;
  listening = true;
  void api.onSyncChanged((report) => {
    if (report.status.kind === "signed_out" && observeAccountIdentity(null)) { update(); return; }
    const wasSyncing = ui.syncing;
    ui.syncing = false;
    window.clearTimeout(syncingTimer);
    const plan = planSyncRedraw(shownReport, report, wasSyncing, renamingDevice());
    if (plan === "skip") return;
    if (plan === "status" && syncBlock?.element.isConnected) {
      shownReport = report;
      const next = syncBlock.build(report);
      syncBlock.element.replaceWith(next);
      syncBlock.element = next;
      return;
    }
    update();
  }).catch(() => { listening = false; });
}

function claudeLine(status: ClaudeStatus, enabled: boolean): string | null {
  return claudeStatusText(status, enabled, (then) => relativeTime(then, Date.now()));
}

function listenForClaude() {
  if (listeningClaude) return;
  listeningClaude = true;
  void api.onClaudeAccessChanged((status) => {
    if (!claudeShown?.line.isConnected) return;
    claudeShown.status = status;
    const text = claudeLine(status, claudeShown.enabled);
    claudeShown.line.textContent = text ?? "";
    claudeShown.line.className = status.kind === "error" ? "error" : "muted";
  }).catch(() => { listeningClaude = false; });
}

async function beginLogin(ctx: Context) {
  accountActions.invalidate();
  linkAvailabilityRun++;
  linkAvailabilityBusy = false;
  linkAvailabilityRequest = null;
  const run = ++loginRun;
  clearNotice();
  try {
    const start = await api.cloudLoginStart();
    if (run !== loginRun) return;
    ui.login = { kind: "code", start };
    update();
    await api.openUrl(start.verification_uri).catch(ctx.report);
    const outcome = await pollLogin(async (): Promise<LoginPoll> => {
      try { return await api.cloudLoginPoll(start.device_code); }
      catch (error) {
        if (errorCode(error) === "error") return { kind: "pending" };
        throw error;
      }
    }, {
      intervalMs: Math.max(1, start.interval_secs) * 1000,
      expiresAtMs: Date.now() + LOGIN_LIFETIME_MS,
      now: Date.now,
      sleep: (ms) => new Promise((resolve) => window.setTimeout(resolve, ms)),
      cancelled: () => run !== loginRun,
    });
    if (outcome === "cancelled") return;
    if (outcome === "expired") { ui.login = { kind: "expired" }; update(); return; }
    ui.login = { kind: "idle" };
    ui.offerCloud = true;
    await ctx.refresh();
  } catch (error) {
    if (run !== loginRun) return;
    ui.login = { kind: "idle" };
    setNoticeError(error);
    update();
  }
}

function cancelLogin() {
  loginRun++;
  ui.login = { kind: "idle" };
  update();
}

function resetLocalState() {
  accountActions.invalidate();
  loginRun++;
  linkRun++;
  linkAvailabilityRun++;
  linkAvailability = null;
  linkAvailabilityFailure = null;
  linkAvailabilityBusy = false;
  linkAvailabilityRequest = null;
  window.clearTimeout(linkTimer);
  window.clearTimeout(syncingTimer);
  linkView = null;
  linkDraft = "";
  linkFailure = null;
  linkManual = false;
  linkBusy = false;
  recoveryContext = null;
  noticeFailure = null;
  allowCachedLink = false;
  ui = afterSignOut();
  localStorage.removeItem(CONFIRMED_KEY);
}

function observeAccountIdentity(identity: string | null): boolean {
  accountSignedIn = identity !== null;
  if (!accountActions.observe(identity)) return false;
  resetLocalState();
  return true;
}

function checkConnectionAvailability(refresh = false): Promise<void> {
  if (linkAvailabilityRequest) return linkAvailabilityRequest;
  if (!refresh && (linkAvailability || linkAvailabilityFailure)) return Promise.resolve();
  const run = ++linkAvailabilityRun;
  const current = accountActions.captureIdentity();
  linkAvailabilityBusy = true;
  if (refresh) {
    linkAvailability = null;
    linkAvailabilityFailure = null;
    linkFailure = null;
    if (linkView?.phase === "failed") { linkView = null; allowCachedLink = false; }
  }
  const request = (async () => {
    try {
      const availability = await api.cloudLinkAvailability();
      if (run === linkAvailabilityRun && current()) linkAvailability = availability;
    } catch (error) {
      if (run === linkAvailabilityRun && current()) linkAvailabilityFailure = error;
    } finally {
      if (run === linkAvailabilityRun) {
        linkAvailabilityBusy = false;
        linkAvailabilityRequest = null;
        if (current()) update();
      }
    }
  })();
  linkAvailabilityRequest = request;
  if (refresh) update();
  return request;
}

function connectionAvailabilityIssue(): unknown | null {
  return linkAvailabilityFailure ?? (linkAvailability && !linkAvailability.available ? { code: linkAvailability.reason_code ?? "pairing_unavailable" } : null);
}

function connectionAdmissionDisabled(): boolean { return linkBusy || linkAvailabilityBusy || linkAvailability?.available !== true; }

function connectionAvailabilityNotice(): HTMLElement | null {
  if (linkAvailabilityBusy) return h("p", { role: "status", class: "help" }, l("account.link.checking"));
  const failure = connectionAvailabilityIssue();
  return failure ? h("p", { role: linkAvailabilityFailure ? "alert" : "status", class: linkAvailabilityFailure ? "error" : "help" }, liveText(() => accountFailureText(failure))) : null;
}

function checkAvailabilityAction(): HTMLElement | null {
  return connectionAvailabilityIssue() ? controlButton({ class: "btn", disabled: linkAvailabilityBusy, onclick: () => void checkConnectionAvailability(true) }, l("account.link.check_again")) : null;
}

function connectionVisible(): boolean { return Boolean(linkView || linkManual || linkBusy || linkFailure); }

async function restoreConnection(identity: string | null, ownsView: () => boolean) {
  if (!allowCachedLink || linkView || linkBusy || linkManual) return;
  const current = accountActions.captureIdentity();
  const cached = await api.cloudLinkView().catch(() => null);
  if (!current() || !ownsView() || !cached || cached.phase === "idle") return;
  if ((identity === null && cached.role === "trusted_device") || (identity !== null && cached.account_email && cached.account_email !== identity)) return;
  linkView = cached;
}

export async function accountView(ctx: Context): Promise<HTMLElement> {
  listenForSync();
  listenForClaude();
  const mine = ctx.rerender;
  const viewRun = ++accountViewRun;
  const ownsView = () => viewRun === accountViewRun;
  redraw = mine;
  ctx.onDispose(() => { if (ownsView()) { accountViewRun++; if (redraw === mine) redraw = null; window.clearTimeout(linkTimer); } });
  syncBlock = null;
  shownReport = null;
  if (ui.login.kind !== "idle") return signedOut(ctx);

  let account: CloudAccount | null = null;
  let accountError: unknown = null;
  try { account = await api.cloudAccount(); if (!ownsView()) return h("div"); observeAccountIdentity(account.email); }
  catch (error) {
    if (!ownsView()) return h("div");
    if (accountScreenFor(errorCode(error)) === "signed_out") {
      observeAccountIdentity(null);
      await restoreConnection(null, ownsView);
      if (!ownsView()) return h("div");
      if (!linkView || !linkIsPending(linkView.phase)) void checkConnectionAvailability();
      scheduleLinkPoll(ctx);
      return signedOut(ctx);
    }
    accountError = error;
  }
  await restoreConnection(account?.email ?? null, ownsView);
  if (!ownsView()) return h("div");
  if (!linkView || !linkIsPending(linkView.phase)) void checkConnectionAvailability();
  scheduleLinkPoll(ctx);
  const current = accountActions.captureIdentity();
  const [report, devices, warning, claude, setup] = await Promise.all([
    api.cloudSyncLast(),
    account ? api.cloudDevices().catch(() => null) : Promise.resolve(null),
    api.cloudFolderWarning().catch(() => null),
    account ? api.claudeAccess().catch(() => null) : Promise.resolve(null),
    api.cloudSyncSetup().catch((error) => { if (current()) setNoticeError(error); return null; }),
  ]);
  if (!current() || !ownsView()) return h("div", {}, heading());
  const enabled = ctx.info.settings.cloud_sync === true;
  const build = (current: SyncReport) => syncSection(ctx, current, enabled, warning, !unlockVisible(current, enabled), setup);
  const syncElement = build(report);
  shownReport = report;
  syncBlock = { element: syncElement, build };

  return h("div", {},
    heading(),
    syncElement,
    ui.shownCode ? recoverySheet(ctx, ui.shownCode) : null,
    unlockSection(ctx, report, enabled),
    connectionVisible() ? connectionSection(ctx, setup ? connectionRole(setup) : linkView?.role ?? "new_device") : null,
    !ui.shownCode ? h("details", { class: "account-management" },
      h("summary", {}, l("account.setup.admin")),
      accountSection(ctx, account, accountError),
      account ? usageSection(account) : null,
      enabled && !needsVault(report) ? recoverySection(ctx) : null,
      cloudPrompt(ctx),
      claude ? claudeSection(ctx, claude) : null,
      devices ? devicesSection(ctx, devices) : null,
      deleteSection(ctx)) : null);
}

function needsVault(report: SyncReport): boolean {
  return ["needs_setup", "needs_recovery_code", "vault_reset"].includes(report.status.kind);
}

function signedOut(ctx: Context): HTMLElement {
  if (connectionVisible()) return h("div", {}, heading(), connectionSection(ctx, "new_device"));
  if (ui.login.kind === "code") {
    const { start } = ui.login;
    return h("div", {}, heading(), plainSection(
      h("p", {}, l("account.login.browser_code")),
      h("p", { class: "login-code", "aria-live": "polite" }, start.user_code),
      h("div", { class: "row" },
        controlButton({ class: "btn", onclick: () => void api.openUrl(start.verification_uri).catch(ctx.report) }, l("account.login.reopen_browser")),
        controlButton({ class: "btn", onclick: cancelLogin }, l("account.login.cancel")))));
  }
  return h("div", {}, heading(), plainSection(
    ...(ui.login.kind === "expired"
      ? [h("p", { class: "error", role: "alert" }, l("account.login.expired")),
         controlButton({ class: "btn primary", onclick: () => void beginLogin(ctx) }, l("account.login.start_again"))]
      : [h("p", {}, l("account.setup.intro")),
         ui.notice ? h("p", { class: "error", role: "alert" }, liveText(noticeText)) : null,
         h("div", { class: "row" },
           controlButton({ class: "btn primary", disabled: connectionAdmissionDisabled(), onclick: () => void beginConnection(ctx, "new_device") }, l("account.setup.connect")),
           controlButton({ class: "btn", disabled: connectionAdmissionDisabled(), onclick: () => { linkView = null; linkFailure = null; allowCachedLink = false; linkManual = true; update(); } }, l("account.link.manual"))),
         connectionAvailabilityNotice(),
         checkAvailabilityAction(),
         h("p", { class: "help" }, l("account.setup.first_device")),
         controlButton({ class: "btn", onclick: () => void beginLogin(ctx) }, l("account.setup.email"))])));
}

function scheduleLinkPoll(ctx: Context) {
  window.clearTimeout(linkTimer);
  if (linkBusy || !linkView || !linkIsPending(linkView.phase) || linkFailure || !redraw) return;
  const run = linkRun;
  linkTimer = window.setTimeout(() => { void refreshConnection(ctx, run); }, 2000);
}

async function refreshConnection(ctx: Context, run = linkRun) {
  if (linkBusy) return;
  try {
    const next = await api.cloudLinkPoll();
    if (run !== linkRun) return;
    const changed = JSON.stringify(next) !== JSON.stringify(linkView);
    linkView = next;
    linkFailure = null;
    if (next.phase === "complete") {
      setConfirmed(true);
      ui.offerCloud = false;
      if (redraw) await ctx.refresh();
    } else if (changed) update();
  } catch (error) {
    if (run !== linkRun) return;
    linkFailure = error;
    update();
  }
  scheduleLinkPoll(ctx);
}

async function beginConnection(ctx: Context, role: CloudLinkRole, code?: string) {
  if (connectionAdmissionDisabled()) return;
  const run = ++linkRun;
  window.clearTimeout(linkTimer);
  linkBusy = true;
  linkFailure = null;
  linkView = null;
  linkManual = false;
  allowCachedLink = false;
  clearNotice();
  update();
  try {
    const view = code === undefined ? await api.cloudLinkStart(role) : await api.cloudLinkJoin(code, role);
    if (run !== linkRun) return;
    linkView = view;
    linkDraft = "";
    allowCachedLink = true;
  } catch (error) {
    if (run !== linkRun) return;
    linkFailure = error;
    linkManual = code !== undefined;
    const reason = errorCode(error);
    if (reason === "pairing_unavailable" || reason === "pairing_incompatible") {
      linkAvailability = { available: false, reason_code: reason };
      linkAvailabilityFailure = null;
    }
    allowCachedLink = true;
  } finally {
    if (run === linkRun) { linkBusy = false; update(); scheduleLinkPoll(ctx); }
  }
}

async function cancelConnection() {
  const run = ++linkRun;
  const current = accountActions.captureIdentity();
  linkAvailabilityRun++;
  linkAvailabilityBusy = false;
  linkAvailabilityRequest = null;
  window.clearTimeout(linkTimer);
  linkBusy = true;
  update();
  try { await api.cloudLinkCancel(); }
  catch { /* The native flow is cancelled locally; an unreachable relay expires its request. */ }
  if (run !== linkRun || !current()) return;
  linkView = null;
  linkManual = false;
  linkDraft = "";
  linkFailure = null;
  linkBusy = false;
  allowCachedLink = false;
  update();
}

function connectionSection(ctx: Context, role: CloudLinkRole): HTMLElement {
  const view = linkView;
  const pending = Boolean(view && linkIsPending(view.phase));
  const feedback = connectionFeedback(view, linkFailure ?? (pending ? null : connectionAvailabilityIssue()));
  const startLabel = role === "trusted_device" ? "account.link.start_trusted" : "account.link.start_new";
  const source = qrImageSource(view?.qr_svg ?? null);
  const input = textField({ type: "text", class: "device-link-entry", value: linkDraft, autocomplete: "off", spellcheck: "false", autocapitalize: "characters", "aria-label": l("account.link.code") });
  input.addEventListener("input", () => { linkDraft = input.value; });
  const manual = linkManual ? h("form", { class: "row", onsubmit: (event: Event) => {
    event.preventDefault();
    if (input.value.trim()) void beginConnection(ctx, role, input.value.trim());
  } }, input, controlButton({ class: "btn primary", type: "submit", disabled: connectionAdmissionDisabled() }, l("account.link.join"))) : null;
  const manualAction = async () => {
    const current = accountActions.captureIdentity();
    if (pending) await cancelConnection();
    if (!current()) return;
    linkManual = true;
    linkView = null;
    linkFailure = null;
    allowCachedLink = false;
    update();
  };
  return section(l("account.link.title"), null,
    !view || view.phase === "waiting_peer" ? h("p", {}, l(role === "trusted_device" ? "account.link.intro_trusted" : "account.link.intro_new")) : null,
    view?.phase === "complete" ? h("p", {}, l("account.setup.connected")) : null,
    linkBusy ? h("p", { role: "status" }, l("account.link.working")) : feedback.status ? h("p", { role: "status", class: "muted", "aria-live": "polite" }, l(`account.link.phase.${feedback.status}`)) : null,
    !pending && linkAvailabilityBusy ? h("p", { role: "status", class: "help" }, l("account.link.checking")) : null,
    view?.account_email ? h("p", {}, l("account.link.account", () => ({ email: view.account_email! }))) : null,
    view?.peer_name ? h("p", {}, l("account.link.peer", () => ({ name: view.peer_name!, platform: view.peer_platform ?? "Pennel" }))) : null,
    source && pending ? h("img", { class: "device-link-qr", src: source, alt: l("account.link.qr") }) : null,
    view?.connection_code && pending ? [h("p", { class: "device-link-code" }, view.connection_code), h("p", { class: "help" }, l("account.link.scan"))] : null,
    view?.verification_code && pending ? [h("p", { class: "device-link-code" }, view.verification_code), role === "trusted_device" ? h("p", { class: "help" }, l("account.link.confirm")) : null] : null,
    feedback.error ? h("p", { role: "alert", class: "error" }, liveText(() => accountFailureText(feedback.error))) : null,
    manual,
    h("div", { class: "row" },
      view?.phase === "awaiting_approval" && role === "trusted_device" ? controlButton({ class: "btn primary", disabled: linkBusy, onclick: async () => {
        const run = linkRun;
        linkBusy = true; update();
        try { const next = await api.cloudLinkApprove(); if (run === linkRun) linkView = next; }
        catch (error) { if (run === linkRun) linkFailure = error; }
        finally { if (run === linkRun) { linkBusy = false; update(); scheduleLinkPoll(ctx); } }
      } }, l("account.link.approve")) : null,
      linkFailure && pending ? controlButton({ class: "btn", onclick: () => { linkFailure = null; void refreshConnection(ctx); } }, l("account.link.retry_poll")) : null,
      !pending && view?.phase !== "complete" && !linkManual ? controlButton({ class: "btn primary", disabled: connectionAdmissionDisabled(), onclick: () => void beginConnection(ctx, role) }, l(view ? "account.link.retry" : startLabel)) : null,
      !linkManual && view?.phase !== "complete" ? controlButton({ class: "btn", disabled: pending ? linkBusy : connectionAdmissionDisabled(), onclick: () => void manualAction() }, l("account.link.manual")) : null,
      !pending ? checkAvailabilityAction() : null,
      view?.phase !== "complete" && role === "new_device" ? controlButton({ class: "btn", disabled: linkBusy, onclick: async () => {
        const current = accountActions.captureIdentity();
        await cancelConnection();
        if (!current()) return;
        if (accountSignedIn) { ui.askCode = true; update(); }
        else await beginLogin(ctx);
      } }, l(accountSignedIn ? "account.link.use_recovery" : "account.setup.email")) : null,
      controlButton({ class: "btn", disabled: linkBusy, onclick: () => void cancelConnection() }, l(view?.phase === "complete" ? "account.link.done" : "common.cancel"))));
}

function slots(): [keyof Pick<Settings, "dictation_stt" | "llm" | "meeting_stt">, string][] { return [
  ["dictation_stt", t("account.slot.dictation")],
  ["llm", t("account.slot.text")],
  ["meeting_stt", t("account.slot.meetings")],
]; }

function emptySlots(settings: Settings) {
  return slots().filter(([key]) => !settings.providers.some((provider) => provider.id === settings[key].provider_id));
}

function localizedList(items: string[]): string {
  return new Intl.ListFormat(uiLocale(), { style: "long", type: "conjunction" }).format(items);
}

function cloudPrompt(ctx: Context): HTMLElement | null {
  if (!ui.offerCloud) return null;
  const empty = emptySlots(ctx.info.settings);
  if (!empty.length) return null;
  const close = () => { ui.offerCloud = false; update(); };
  const adopt = () => {
    ui.offerCloud = false;
    void ctx.mutate(async () => {
      let next = await api.info();
      if (!next.settings.providers.some((provider) => provider.kind === "pennel_cloud")) next = await api.addProvider("pennel_cloud");
      const settings = structuredClone(next.settings);
      const cloud = settings.providers.find((provider) => provider.kind === "pennel_cloud");
      const kind = next.kinds.find((item) => item.kind === "pennel_cloud");
      if (!cloud) return next;
      for (const [key] of emptySlots(settings)) {
        settings[key] = { provider_id: cloud.id, model: (key === "llm" ? kind?.default_llm_model : kind?.default_stt_model) ?? "" };
      }
      return api.saveSettings(settings);
    });
  };
  return plainSection(
    h("p", {}, l("account.cloud_offer", () => ({ tasks: localizedList(emptySlots(ctx.info.settings).map(([, label]) => label)) }))),
    h("div", { class: "row" },
      controlButton({ class: "btn primary", onclick: adopt }, l("common.yes")),
      controlButton({ class: "btn", onclick: close }, l("common.no"))));
}

function accountSection(ctx: Context, account: CloudAccount | null, error: unknown): HTMLElement {
  let signingOut = false;
  const logout = controlButton({ class: "btn" }, liveText(() => t(signingOut ? "account.logout.busy" : "account.logout.action")));
  bindLocalization(logout, () => { logout.textContent = t(signingOut ? "account.logout.busy" : "account.logout.action"); });
  logout.addEventListener("click", async () => {
    const ownsIdentity = accountActions.captureIdentity();
    const unconfirmed = recoveryNeedsConfirmation({ signedIn: true, syncOn: ctx.info.settings.cloud_sync === true, confirmed: confirmedFlag() });
    if (unconfirmed && !await confirmDialog(l("account.logout.title"),
      l("account.logout.unconfirmed"),
      l("account.logout.action"))) return;
    if (!ownsIdentity()) return;
    accountActions.invalidate();
    const currentLogout = accountActions.captureIdentity();
    signingOut = true; logout.disabled = true;
    logout.textContent = t("account.logout.busy");
    try { await api.cloudLogout(); if (currentLogout()) { resetLocalState(); await ctx.refresh(); } }
    catch (e) {
      if (!currentLogout()) return;
      signingOut = false; logout.disabled = false;
      logout.textContent = t("account.logout.action");
      ctx.report(e);
    }
  });
  return section(l("account.account_title"), null,
    account ? h("p", {}, l("account.signed_in_as", () => ({ email: account.email }))) : h("p", { class: "error", role: "alert" }, errorText(error)),
    logout);
}

function usageSection(account: CloudAccount): HTMLElement {
  const audio = h("p", {});
  const tokens = h("p", {});
  const storage = h("p", {});
  const update = () => {
    const usage = formatUsage(account);
    audio.textContent = usage.audio; tokens.textContent = usage.tokens; storage.textContent = usage.storage;
  };
  const root = section(l("account.usage_title"), null, audio, tokens, storage);
  update(); bindLocalization(root, update);
  return root;
}

function statusLine(report: SyncReport, enabled: boolean): string | null {
  if (ui.syncing) return t("account.sync.busy");
  if (!enabled) return null;
  const status = report.status;
  switch (status.kind) {
    case "pending": return t("account.sync.first_pending");
    case "synced":
      return report.last_success_ms === null ? t("account.sync.synced")
        : t("account.sync.synced_at", { time: relativeTime(report.last_success_ms, Date.now()) });
    case "offline": return t("account.sync.offline");
    case "storage_full":
    case "error": return localizeBackendText(status.message);
    default: return null;
  }
}

async function action(ctx: Context, work: (current: () => boolean) => Promise<void>) {
  const current = accountActions.begin();
  clearNotice();
  try { await work(current); }
  catch (error) { if (current()) setNoticeError(error); }
  if (current()) await ctx.refresh().catch(ctx.report);
}

function generateRecoveryCode(ctx: Context, obtain: () => Promise<string | null>, clearUnlock = false) {
  const ownsIdentity = accountActions.captureIdentity();
  return action(ctx, async (current) => {
    const code = await obtain();
    if (code && ownsIdentity()) setConfirmed(false);
    if (!code || !current()) return;
    const context = await api.cloudRecoveryCodeContext(code);
    if (!current()) return;
    showCode(code, context);
    if (clearUnlock) { ui.askCode = false; ui.lostCode = false; ui.codeDraft = ""; }
  });
}

function syncSection(ctx: Context, report: SyncReport, enabled: boolean, warning: string | null, showNotice: boolean, setup: CloudSyncSetup | null): HTMLElement {
  if (setup?.phase === "locked") ui.askCode = true;
  const state = setup ? syncSetupAction(setup) : null;
  const needsBackup = setup && syncRequiresBackup(setup, confirmedFlag());
  const prepare = () => void generateRecoveryCode(ctx, () => api.cloudSyncPrepare());
  const line = statusLine(report, enabled);
  const lineElement = line ? h("p", { class: report.brake || report.status.kind === "error" || report.status.kind === "storage_full" ? "error" : "muted", role: "status" }, line) : null;
  if (lineElement) bindLocalization(lineElement, () => { lineElement.textContent = statusLine(report, enabled) ?? ""; });
  return section(l("account.setup.title"), null,
    h("p", { class: "help" }, l("account.setup.intro")),
    h("p", { class: "help" }, l("account.sync.privacy")),
    !connectionVisible() ? connectionAvailabilityNotice() : null,
    !connectionVisible() ? checkAvailabilityAction() : null,
    ui.shownCode ? h("p", {}, l("account.setup.backup_help")) : null,
    !ui.shownCode && state === "create" ? [
      h("p", {}, l("account.setup.new_library")),
      controlButton({ class: "btn primary", onclick: prepare }, l("account.setup.create")),
    ] : null,
    !ui.shownCode && state === "unlock" ? [
      h("p", {}, l("account.setup.locked")),
      controlButton({ class: "btn primary", disabled: connectionAdmissionDisabled(), onclick: () => void beginConnection(ctx, "new_device") }, l("account.setup.connect")),
    ] : null,
    !ui.shownCode && needsBackup ? [
      h("p", { role: "status" }, l("account.setup.backup_missing")),
      controlButton({ class: "btn primary", onclick: () => void generateRecoveryCode(ctx, () => api.cloudSyncNewRecoveryCode()) }, l("account.setup.replace_backup")),
    ] : null,
    !ui.shownCode && state === "resume" && !needsBackup ? [
      h("p", { role: "status" }, l(setup?.phase === "paused" ? "account.setup.paused" : "account.setup.ready")),
      controlButton({ class: "btn primary", onclick: () => void action(ctx, () => api.cloudSyncResume()) }, l("account.setup.resume")),
    ] : null,
    lineElement,
    enabled && report.last_success_ms !== null ? h("p", { class: "help" }, l("account.setup.counts", () => ({ uploaded: report.uploaded, downloaded: report.downloaded, conflicts: report.conflicts }))) : null,
    showNotice && ui.notice ? h("p", { class: "error", role: "alert" }, liveText(noticeText)) : null,
    !setup ? controlButton({ class: "btn", onclick: () => void ctx.refresh().catch(ctx.report) }, l("common.retry")) : null,
    warning ? h("p", { class: "help" }, liveText(() => localizeBackendText(warning))) : null,
    report.brake ? controlButton({ class: "btn", onclick: async () => {
      const ok = await confirmDialog(l("account.sync.confirm_deletes_title"), l("account.sync.confirm_deletes_detail"), l("account.sync.confirm_deletes"));
      if (ok) await action(ctx, () => api.cloudSyncConfirmDeletes());
    } }, l("account.sync.confirm_deletes")) : null,
    !ui.shownCode && setup?.can_approve_devices && !needsBackup ? controlButton({ class: "btn", disabled: connectionAdmissionDisabled(), onclick: () => void beginConnection(ctx, "trusted_device") }, l("account.link.start_trusted")) : null,
    enabled ? h("div", { class: "row" },
      controlButton({ class: "btn", onclick: () => void action(ctx, () => api.cloudSyncDisable()) }, l("account.setup.pause")),
      !needsVault(report) ? controlButton({ class: "btn", disabled: ui.syncing, onclick: () => {
        const ownsIdentity = accountActions.captureIdentity();
        ui.syncing = true;
        window.clearTimeout(syncingTimer);
        syncingTimer = window.setTimeout(() => { if (ownsIdentity()) { ui.syncing = false; update(); } }, SYNC_NOW_TIMEOUT_MS);
        update();
        void api.cloudSyncNow().catch((error) => { if (ownsIdentity()) { ui.syncing = false; ctx.report(error); update(); } });
      } }, l("account.sync.now")) : null) : null);
}

const copyFeedback = new WeakMap<HTMLElement, { failure: string | null }>();

function showCopyStatus(status: HTMLElement, failure: string | null) {
  let feedback = copyFeedback.get(status);
  if (!feedback) {
    feedback = { failure };
    copyFeedback.set(status, feedback);
    const current = feedback;
    bindLocalization(status, () => { status.textContent = copyStatusText(current.failure); });
  }
  feedback.failure = failure;
  status.textContent = copyStatusText(failure);
}

async function copyRecoveryCode(code: string, status: HTMLElement) {
  try { await api.copySecretText(code); showCopyStatus(status, null); }
  catch (error) { showCopyStatus(status, errorText(error)); }
}

function printRecoveryCode(code: string) {
  const page = h("div", { id: "print-sheet" },
    h("h1", {}, l("account.recovery.print_title")),
    h("p", { class: "print-code" }, code),
    h("p", {}, l("account.recovery.store_safely")));
  document.body.append(page);
  document.body.classList.add("printing-sheet");
  const finish = () => { document.body.classList.remove("printing-sheet"); page.remove(); window.removeEventListener("afterprint", finish); };
  window.addEventListener("afterprint", finish);
  window.setTimeout(finish, 60000);
  window.print();
}

function recoverySheet(ctx: Context, code: string): HTMLElement {
  const status = h("span", { class: "muted", role: "status" });
  const context = recoveryContext;
  const ownsIdentity = accountActions.captureIdentity();
  const ownsDisplay = () => ownsIdentity() && ui.shownCode === code && recoveryContext === context;
  return section(l("account.setup.backup"), null,
    h("p", {}, l("account.setup.backup_help")),
    h("div", { class: "recovery-code", "aria-label": l("account.recovery.title") }, code.split("-").map((group) => h("span", {}, group))),
    h("div", { class: "row" },
      controlButton({ class: "btn", onclick: () => { if (ownsDisplay()) void copyRecoveryCode(code, status); } }, l("account.recovery.copy")),
      controlButton({ class: "btn primary", disabled: !context, onclick: () => {
        if (!context || !ownsDisplay()) return;
        void action(ctx, async (current) => {
          try { await api.cloudRecoveryConfirm(code, context); }
          catch (error) {
            if (current() && ownsDisplay() && ["recovery_changed", "recovery_code", "vault_locked", "not_signed_in"].includes(errorCode(error) ?? "")) { ui.shownCode = null; recoveryContext = null; }
            throw error;
          }
          if (!current() || !ownsDisplay()) return;
          recoveryContext = null; ui.shownCode = null; setConfirmed(true);
        });
      } }, l("account.recovery.saved")),
      h("button", { class: "text-button", onclick: () => { if (ownsDisplay()) printRecoveryCode(code); } }, l("account.recovery.print")),
      status));
}

function unlockVisible(report: SyncReport, enabled: boolean): boolean {
  if (ui.shownCode) return false;
  return ui.askCode || (enabled && (report.status.kind === "needs_recovery_code" || report.status.kind === "vault_reset"));
}

function unlockSection(ctx: Context, report: SyncReport, enabled: boolean): HTMLElement | null {
  const reset = enabled && report.status.kind === "vault_reset";
  if (!unlockVisible(report, enabled)) return null;
  const input = textField({ type: "text", value: ui.codeDraft, autocomplete: "off", spellcheck: "false", autocapitalize: "characters", "aria-label": l("account.recovery.title") });
  input.addEventListener("input", () => { ui.codeDraft = input.value; });
  const submit = () => void action(ctx, async (current) => {
    await api.cloudSyncUnlock(input.value);
    if (!current()) return;
    setConfirmed(true);
    ui.codeDraft = "";
    ui.askCode = false;
  });
  const form = h("form", { class: "row", onsubmit: (event: Event) => { event.preventDefault(); submit(); } },
    input, controlButton({ class: "btn primary", type: "submit" }, l("account.recovery.unlock")));
  return section(l("account.recovery.title"), null,
    reset ? h("p", {}, l("account.recovery.vault_reset")) : null,
    form,
    ui.notice ? h("p", { class: "error", role: "alert" }, liveText(noticeText)) : null,
    h("button", { class: "text-button", onclick: () => { ui.lostCode = !ui.lostCode; update(); } }, l("account.recovery.lost")),
    ui.lostCode ? [
      h("p", { class: "help" }, l("account.link.lost_help")),
      controlButton({ class: "btn danger", onclick: async () => {
        const ownsIdentity = accountActions.captureIdentity();
        const ok = await confirmDialog(l("account.recovery.reset_title"),
          l("account.recovery.reset_detail"),
          l("account.recovery.reset"));
        if (!ok || !ownsIdentity()) return;
        await generateRecoveryCode(ctx, () => api.cloudSyncReset(), true);
      } }, l("account.recovery.reset"))] : null);
}

function recoverySection(ctx: Context): HTMLElement {
  return section(l("account.recovery.title"), null,
    recoveryNeedsConfirmation({ signedIn: true, syncOn: true, confirmed: confirmedFlag() })
      ? h("p", { class: "error", role: "status" }, l("account.recovery.unconfirmed")) : null,
    controlButton({ class: "btn", onclick: async () => {
      const ownsIdentity = accountActions.captureIdentity();
      const ok = await confirmDialog(l("account.recovery.new_title"), l("account.recovery.new_detail"), l("account.recovery.new"));
      if (ok && ownsIdentity()) await generateRecoveryCode(ctx, () => api.cloudSyncNewRecoveryCode());
    } }, l("account.recovery.new")));
}

function copyField(label: UiText, value: string): HTMLElement {
  const status = h("span", { class: "muted", role: "status" });
  return h("div", {},
    h("p", { class: "help" }, label),
    h("div", { class: "row" },
      textField({ type: "text", class: "grow", readonly: true, value, "aria-label": label, spellcheck: "false" }),
      controlButton({ class: "btn", onclick: async () => {
        try { await api.copyText(value); showCopyStatus(status, null); }
        catch (error) { showCopyStatus(status, errorText(error)); }
      } }, l("common.copy")),
      status));
}

function claudeSection(ctx: Context, view: ClaudeAccessView): HTMLElement {
  const control = toggle(l("account.claude.title"), view.enabled, (checked) => void action(ctx, async () => {
    await api.setClaudeAccess(checked);
  }));
  const text = claudeLine(view.status, view.enabled);
  const line = h("p", { class: view.status.kind === "error" && view.enabled ? "error" : "muted", role: "status" }, text ?? "");
  const current = { line, enabled: view.enabled, status: view.status };
  claudeShown = current;
  bindLocalization(line, () => { line.textContent = claudeLine(current.status, current.enabled) ?? ""; });
  const steps = claudeInstructions(view.mcp_url);
  return section(l("account.claude.title"), null,
    control,
    h("p", { class: "help" }, l("account.claude.help")),
    line,
    view.enabled ? [
      copyField(l("account.claude.connector_help"), steps.connectorUrl),
      copyField(l("account.claude.code_help"), steps.command),
      h("p", { class: "help" }, l("account.claude.sign_in_help")),
    ] : null);
}

function seenText(device: CloudDevice): string {
  const time = Date.parse(device.last_seen_iso);
  return t("account.devices.last_seen", { time: Number.isNaN(time) ? device.last_seen_iso : relativeTime(time, Date.now()) });
}

function devicesSection(ctx: Context, devices: CloudDevice[]): HTMLElement {
  return section(l("account.devices.title"), null, h("div", { class: "list" }, devices.map((device) => {
    const name = textField({ type: "text", class: "device-name", value: device.name, "aria-label": l("account.devices.name") });
    name.dataset.original = device.name;
    name.addEventListener("change", () => {
      const next = name.value.trim();
      if (!next || next === device.name) { name.value = device.name; return; }
      void action(ctx, () => api.cloudRenameDevice(device.id, next));
    });
    return h("div", { class: "row device-row" },
      name,
      h("span", { class: "muted" }, liveText(() => `${device.platform} · ${seenText(device)}`)),
      device.current ? h("span", { class: "muted" }, l("account.devices.current")) : null,
      device.current ? null : controlButton({ class: "btn", onclick: async () => {
        const ok = await confirmDialog(l("account.devices.revoke_title"),
          l("account.devices.revoke_detail"),
          l("account.devices.revoke"));
        if (ok) await action(ctx, () => api.cloudRevokeDevice(device.id));
      } }, l("account.devices.revoke")));
  })));
}

function deleteSection(ctx: Context): HTMLElement {
  return section(l("account.delete.title"), null, controlButton({ class: "btn danger", onclick: async () => {
    const ownsIdentity = accountActions.captureIdentity();
    const ok = await confirmDialog(l("account.delete.confirm_title"),
      l("account.delete.detail"),
      l("account.delete.title"));
    if (!ok || !ownsIdentity()) return;
    accountActions.invalidate();
    const currentDelete = accountActions.captureIdentity();
    try { await api.cloudDeleteAccount(); if (currentDelete()) { resetLocalState(); await ctx.refresh(); } }
    catch (error) { if (currentDelete()) ctx.report(error); }
  } }, l("account.delete.title")));
}
