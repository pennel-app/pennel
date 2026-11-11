import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";

/** Owned synthetic provider/storage behavior; this does not open capture devices. */
export async function checkFailedRecovery() {
  const fixture = window.__PENNEL_FIXTURE__;
  if (!/^(localhost|127\.0\.0\.1)$/.test(location.hostname) || !fixture?.options.failed_recovery) throw Error("Owned failed-recovery fixture required");
  const { t } = await import("../src/i18n.ts");
  const api = window.__PENNEL_PLATFORM__;
  const assert = (value, message) => { if (!value) throw Error(message); };
  const until = async (check, message) => {
    for (let attempt = 0; attempt < 160; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
    throw Error(message);
  };
  const root = () => document.querySelector(".live-workspace");
  const notice = () => root()?.querySelector(".meeting-failed-recovery");
  const retry = () => notice()?.querySelector("button");
  const retryCalls = () => fixture.calls.filter(command => command === "retranscribe_meeting").length;
  await until(() => notice()?.getClientRects().length && retry() && !retry().disabled, "Reopened failed transcript has no usable recovery notice");
  assert(notice().textContent.includes(t("meetings.failed_recovery_title")), "Missing localized incomplete-transcript guidance");
  assert(retryCalls() === 0, "Opening a saved meeting automatically retried speech");
  selectMeetingPane(root(), "transcript");
  assert(notice().getClientRects().length, "The Transcript companion lost recovery guidance");
  selectMeetingPane(root(), "notes");
  const editor = root().querySelector(".live-notes");
  const edited = editor.value + "\nOwned private recovery note.";
  fixture.workspace.failSaves(true);
  editor.applyUserChange(edited);
  retry().click();
  await until(() => notice().querySelector(".error")?.textContent.includes("Synthetic notes storage"), "Note-save failure did not stay visible");
  assert(retryCalls() === 0 && editor.value === edited, "A failed note save started speech or lost the draft");
  fixture.workspace.failSaves(false);
  fixture.recovery.mode("zero"); retry().click();
  await until(() => notice()?.dataset.recoveryState === "none", "Zero recovered was presented as completion");
  assert(notice().textContent.includes(t("meetings.failed_recovery_outcome_none")), "Zero-progress recovery has no useful message");
  fixture.recovery.mode("partial_error"); fixture.recovery.delay(250);
  const before = retryCalls(); retry().click(); retry().click();
  await until(() => fixture.recovery.pending() === 1, "The explicit partial recovery did not start");
  const latest = edited + "\nTyped while recovery is running.";
  editor.applyUserChange(latest);
  await until(() => notice()?.dataset.recoveryState === "partial" && !retry().disabled, "Partial failure did not refresh remaining-only recovery");
  assert(retryCalls() === before + 1, "Double click submitted duplicate failed-chunk recovery");
  assert(notice().querySelector(".error").textContent.includes("Synthetic speech"), "Partial provider error was hidden");
  assert(editor.value === latest, "Refreshing partial progress replaced the user's newer note draft");
  assert((await api.invoke("meeting_detail", { id: fixture.options.meeting_id })).failed_recovery.failed_count === 1, "Successful prefix did not remain committed");
  fixture.recovery.mode("success"); fixture.recovery.delay(0); retry().click();
  await until(() => notice()?.dataset.recoveryState === "complete", "Remaining-only retry did not complete");
  const final = await api.invoke("meeting_detail", { id: fixture.options.meeting_id });
  assert(final.failed_recovery.failed_count === 0 && final.notes === latest, "Recovery lost notes or retained failed rows");
  assert(!fixture.calls.some(command => ["retranscribe_meeting_audio", "summarize_meeting"].includes(command)), "Failed-only retry invoked full retranscription or summaries");
  assert(editor.value === latest, "Completed recovery lost reading/editing context");
  return { reopenedNotice: true, noAutomaticRetry: true, noteSaveGuard: true, zeroTruthful: true, partialError: true, duplicateGuard: true, notesPreserved: true, remainingOnly: true };
}

async function recoveryFixture() {
  const fixture = window.__PENNEL_FIXTURE__;
  if (!/^(localhost|127\.0\.0\.1)$/.test(location.hostname) || !fixture?.options.failed_recovery) throw Error("Owned failed-recovery fixture required");
  const { t, getInterfaceLanguage } = await import("../src/i18n.ts");
  const assert = (value, message) => { if (!value) throw Error(message); };
  const until = async (check, message) => {
    for (let attempt = 0; attempt < 160; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
    throw Error(message);
  };
  await until(() => document.querySelector(".meeting-failed-recovery")?.getClientRects().length, "Missing saved-meeting recovery notice");
  const notice = document.querySelector(".meeting-failed-recovery");
  return { fixture, notice, retry: notice.querySelector("button"), assert, until, t, language: getInterfaceLanguage() };
}

export async function checkFailedRecoveryEligibilityCopy() {
  const { fixture, notice, retry, assert, t, language } = await recoveryFixture();
  const detail = await window.__PENNEL_PLATFORM__.invoke("meeting_detail", { id: fixture.options.meeting_id });
  const plan = detail.failed_recovery;
  assert(plan.failed_count === 2 && plan.reason_code === "no_retained_audio", "The missing-audio fixture lost its authoritative failure plan");
  const partial = fixture.options.failed_recovery_retryable === 1;
  assert(plan.retryable_count === (partial ? 1 : 0) && retry.disabled === !partial, "Copy testing changed authoritative recovery eligibility");
  const expected = partial ? language === "nl"
    ? "Voor sommige delen is er geen bijpassende herstelaudio meer. Probeer de beschikbare delen opnieuw, of kies Opname opnieuw transcriberen als de volledige opname beschikbaar is."
    : "Some passages have no matching retry audio. Retry the available passages, or use Retranscribe recording if a full recording is available."
    : t("meetings.failed_recovery_no_audio");
  const description = notice.querySelector(".meeting-failed-recovery-copy > .help").textContent;
  assert(description === [t("meetings.failed_recovery_count", { count: 2 }), t("meetings.failed_recovery_available", { retryable: partial ? 1 : 0, count: 2 }), expected].join(" "), `Recovery copy contradicts ${partial ? "partial" : "unavailable"} eligibility: ${description}`);
  assert(!fixture.calls.some(command => ["retranscribe_meeting", "retranscribe_meeting_audio"].includes(command)), "Presenting eligibility started speech processing");
  return { partialEligibility: partial, retryableCount: plan.retryable_count, unavailableCopyPreserved: !partial, noAutomaticRetry: true };
}

export async function checkFailedRecoveryOwnedErrors() {
  const { fixture, notice, retry, assert, until, language } = await recoveryFixture();
  const diagnostics = [
    ["The recovery audio changed during recognition.", "De herstelaudio is tijdens de spraakherkenning gewijzigd."],
    ["Some recovery audio cannot be linked to its original transcript. Keep the files and review their sources before retrying recognition.", "Een deel van de herstelaudio kan niet aan het oorspronkelijke transcript worden gekoppeld. Bewaar de bestanden en controleer de bronnen voordat je de spraakherkenning opnieuw probeert."],
    ["Restore the transcript's source metadata before retrying recognition.", "Herstel de broninformatie van het transcript voordat je de spraakherkenning opnieuw probeert."],
    ["Provider response: The recovery audio changed before recognition. {message} <audio>", "Provider response: The recovery audio changed before recognition. {message} <audio>"],
    ["The recovery audio changed before recognition.", "De herstelaudio is gewijzigd voordat de spraakherkenning begon."],
  ];
  for (const [english, dutch] of diagnostics) {
    fixture.recovery.error(english);
    await until(() => !retry.disabled, "A prior failed attempt left recovery blocked");
    retry.click();
    const expected = language === "nl" ? dutch : english;
    await until(() => !retry.disabled && !notice.querySelector(".error").hidden, "A raw recovery error was hidden");
    assert(notice.querySelector(".error").textContent === expected, `Recovery error did not use ${language}: ${notice.querySelector(".error").textContent}`);
  }
  const detail = await window.__PENNEL_PLATFORM__.invoke("meeting_detail", { id: fixture.options.meeting_id });
  assert(detail.failed_recovery.failed_count === 2 && notice.querySelector('[role="status"]').hidden, "An integrity error falsely completed recovery");
  assert(!fixture.calls.some(command => ["retranscribe_meeting_audio", "summarize_meeting"].includes(command)), "Failed-only recovery invoked a separate processing action");
  return { ownedErrorsLocalized: 4, providerResponseLiteral: true, failuresPreserved: true };
}
