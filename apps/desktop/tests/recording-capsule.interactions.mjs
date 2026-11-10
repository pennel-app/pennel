const assert = (value, message) => { if (!value) throw Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 200; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
  throw Error(message);
}
const visible = node => !!node?.getClientRects().length && !node.closest("[hidden]");
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));

/** Exercises the real shell against owned, synthetic capture and local events only. */
export async function checkRecordingCapsule() {
  const fixture = window.__PENNEL_FIXTURE__, client = window.__PENNEL_PLATFORM__;
  assert(location.hostname === "127.0.0.1" && fixture && client, "Use the owned local fixture.");
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => visible(document.querySelector(".toolbar-recording")) && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "The idle toolbar did not become ready.");
  const original = client.invoke.bind(client), calls = [];
  let releaseStart, releaseStop, blockStart = false;
  client.invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === "start_meeting") {
      if (blockStart) throw Error("Synthetic microphone startup failed.");
      if (releaseStart === undefined) await new Promise(resolve => { releaseStart = resolve; });
    }
    if (command === "stop_meeting" && releaseStop === undefined) await new Promise(resolve => { releaseStop = resolve; });
    return original(command, args);
  };
  const record = document.querySelector(".toolbar-recording"), capsule = record.closest(".toolbar-capture");
  const startCount = () => calls.filter(call => call.command === "start_meeting").length;
  try {
    if (fixture.options.recording_start_error) {
      blockStart = true;
      record.click();
      await until(() => document.querySelector(".recording-view input") && !document.querySelector("#content").inert, "Failed immediate start did not open recovery.");
      const fallback = document.querySelector(".recording-view");
      assert(fallback.textContent.includes("Synthetic microphone startup failed."), "Startup failure lost its reason.");
      assert([...fallback.querySelectorAll("button")].some(button => button.textContent === t("meetings.recording_settings")), "Startup recovery lost recording settings.");
      assert([...fallback.querySelectorAll("button")].some(button => button.textContent === t("dictation.home.finish_setup")), "Startup recovery lost task setup.");
      assert(visible([...fallback.querySelectorAll("button")].find(button => button.textContent === t("onboarding.open_settings"))), "Blocked microphone startup lost its repair action.");
      const repair = [...fallback.querySelectorAll("button")].find(button => button.textContent === t("onboarding.open_settings"));
      repair.click();
      await until(() => fixture.info().capabilities.microphone.status === "available", "Microphone repair did not refresh access.");
      const setupBefore = JSON.stringify(fixture.info().settings.meeting_stt);
      [...fallback.querySelectorAll("button")].find(button => button.textContent === t("dictation.home.finish_setup")).click();
      await until(() => document.querySelector(".task-setup-dialog [data-setup-cancel]"), "Startup recovery did not open task setup.");
      document.querySelector(".task-setup-dialog [data-setup-cancel]").click();
      await until(() => !document.querySelector(".task-setup-dialog"), "Task setup did not return to startup recovery.");
      assert(JSON.stringify(fixture.info().settings.meeting_stt) === setupBefore, "Opening recovery silently changed the speech setup.");
      fallback.querySelector("input").value = "Synthetic retry title";
      blockStart = false; releaseStart = () => {};
      fallback.querySelector(".btn.primary").click();
    } else {
      record.click(); record.click();
      fixture.emit("desktop-command", "new_recording");
      await until(() => releaseStart, "The toolbar did not request capture immediately.");
      assert(startCount() === 1 && !document.querySelector(".recording-view input"), "New recording duplicated capture or opened the old form before capture.");
      releaseStart();
    }
    await until(() => capsule.classList.contains("is-recording") && document.querySelector(".live-notes") && !document.querySelector("#content").inert, "Capture did not reveal its persistent capsule.");
    const status = await api.meetingRuntimeStatus(), session = status.session_id;
    assert(record === document.querySelector(".toolbar-recording") && document.querySelectorAll(".library-record").length === 1, "The shell replaced or duplicated its one recording action.");
    assert(record.textContent.includes(t("meetings.recording_clock", { time: "12:34" })), "The capsule lost the localized recording clock.");
    assert(visible(capsule.querySelector(".toolbar-capture-stop")), "The distinct Stop action is missing.");
    assert(document.querySelectorAll(`.meeting-row[data-id="${CSS.escape(status.id)}"]`).length === 1 && document.querySelector(".meeting-row-live"), "Now duplicated or omitted the active meeting row.");
    assert(document.querySelector(".meeting-day").textContent === t("meetings.now"), "The live row is still in a date group.");
    const row = document.querySelector(".meeting-row-live"), open = row.querySelector(".meeting-select"); open.focus();
    fixture.emit("meeting-level", { session_id: session, level: .8 }); await settle();
    const heights = () => [...capsule.querySelectorAll(".recording-levels i")].map(bar => bar.style.height).join();
    const liveHeight = heights();
    assert(liveHeight.includes("18.2px"), "Current-session live audio did not move the bars.");
    fixture.emit("meeting-level", { session_id: "previous-session", level: 1 });
    fixture.emit("meeting-level", { session_id: session, level: NaN }); await settle();
    assert(heights() === liveHeight && document.activeElement === open && open.closest(".meeting-row") === row, "Stale levels moved the bars or replaced the focused row.");
    const starts = startCount(); record.click();
    await until(() => window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready && !record.disabled, "Return did not finish navigating.");
    assert(startCount() === starts, "Return started another capture.");
    for (const section of ["dictation", "settings", "meetings"]) {
      fixture.emit("desktop-command", section);
      await until(() => document.querySelector(".toolbar-section").textContent === t(section === "settings" ? "window.settings" : section === "dictation" ? "window.dictation" : "navigation.meetings") && !document.querySelector("#content").inert && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, `Navigation to ${section} did not finish.`);
      assert(visible(capsule) && visible(capsule.querySelector(".toolbar-capture-stop")), `The capsule disappeared in ${section}.`);
    }
    const toggle = document.querySelector('[data-command="toggle_sidebar"]'); toggle.click(); await until(() => getComputedStyle(toggle).backgroundColor !== "rgba(0, 0, 0, 0)", "The collapsed toggle never reached its filled state.");
    assert(toggle.getAttribute("aria-expanded") === "false" && getComputedStyle(toggle).backgroundColor !== "rgba(0, 0, 0, 0)" && visible(capsule), `Collapsed navigation lost its filled toggle or recording capsule: ${JSON.stringify({expanded:toggle.getAttribute("aria-expanded"),background:getComputedStyle(toggle).backgroundColor,capsuleVisible:visible(capsule),connected:capsule.isConnected})}`);
    record.focus();
    record.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    assert(document.activeElement === capsule.querySelector(".toolbar-capture-stop"), "Toolbar keyboard navigation did not reach Stop.");
    if (fixture.options.reduced_motion) assert([...capsule.querySelectorAll(".recording-levels i"), capsule].every(node => getComputedStyle(node).transitionDuration.split(",").every(value => parseFloat(value) === 0)), "Reduced motion retained waveform or growing-capsule transitions.");
    const stop = capsule.querySelector(".toolbar-capture-stop"); stop.click();
    const duplicateStop = api.stopMeeting(session);
    await until(() => releaseStop, "Stop did not reach capture.");
    assert(calls.filter(call => call.command === "stop_meeting").length === 1, "Stop was invoked more than once for the current session.");
    fixture.emit("meeting-level", { session_id: session, level: 1 }); await settle();
    assert(heights() === "3px,3px,3px,3px,3px", "Stop did not immediately clear or close the bars.");
    releaseStop(); await duplicateStop;
    await until(() => !capsule.classList.contains("is-recording") && !record.disabled && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "Stop did not shrink the control back to New recording.");
    assert(document.activeElement === record, "Stop left keyboard focus on a hidden control.");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "n", metaKey: fixture.info().platform === "macos", ctrlKey: fixture.info().platform !== "macos", bubbles: true, cancelable: true }));
    await until(() => capsule.classList.contains("is-recording") && !document.querySelector("#content").inert && !record.disabled && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "New recording shortcut did not immediately start the next capture.");
    const next = await api.meetingRuntimeStatus(); assert(next.session_id !== session, "The next recording reused the old session.");
    fixture.emit("meeting-level", { session_id: session, level: 1 }); await settle();
    assert(heights() === "3px,3px,3px,3px,3px", "Previous-capture levels reached the next recording.");
    fixture.emit("meeting-level", { session_id: next.session_id, level: .7 });
    if (fixture.options.recording_enlarged) {
      const sizes = [...document.querySelectorAll(".workspace-toolbar button,.workspace-toolbar .library-live-title,.workspace-toolbar .toolbar-section")].map(node => [node, parseFloat(getComputedStyle(node).fontSize)]);
      for (const [node, size] of sizes) node.style.fontSize = `${size * 1.5}px`;
    }
    await new Promise(resolve => setTimeout(resolve, 260));
    assert(document.documentElement.scrollWidth <= innerWidth, "Recording controls overflow the viewport.");
    const bounds = capsule.getBoundingClientRect(); assert(bounds.left >= 0 && bounds.right <= innerWidth && bounds.bottom <= document.querySelector(".workspace-toolbar").getBoundingClientRect().bottom, "Capsule controls are clipped.");
    return { immediateStart: true, singleStart: true, recovery: !!fixture.options.recording_start_error, persistentAcrossTasks: true, singleStop: true, staleLevelsRejected: true, keyboard: true, reducedMotion: !!fixture.options.reduced_motion, syntheticOnly: true };
  } finally { client.invoke = original; }
}


export async function checkRecordingStartBarrier() {
  const fixture = window.__PENNEL_FIXTURE__;
  const { api } = await import("/src/api.ts");
  const { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector('[data-dictation-section="pad"]') && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "The pad navigation did not become ready.");
  document.querySelector('[data-dictation-section="pad"]').click();
  await until(() => document.querySelector("textarea.dictation-pad:not(:disabled)"), "The owned pad did not open.");
  const pad = document.querySelector("textarea.dictation-pad");
  pad.value = "Synthetic draft preserved before immediate recording."; pad.dispatchEvent(new Event("input", { bubbles: true }));
  document.querySelector("main .btn.primary").click();
  await until(() => pad.disabled && !document.querySelector("main .btn.primary").disabled, "The pad did not start listening.");
  const starts = fixture.calls.filter(command => command === "start_meeting").length;
  document.querySelector(".toolbar-recording").click();
  await until(() => document.querySelector(".pad-status").textContent.includes(t("dictation.pad.finish_before_leaving")) && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "Recording bypassed or failed to explain the owned-session barrier.");
  assert(pad.isConnected && pad.disabled && fixture.calls.filter(command => command === "start_meeting").length === starts, "New recording abandoned the owned session or started capture through its barrier.");
  [...document.querySelectorAll("main button")].find(button => button.textContent === t("common.cancel")).click();
  await until(() => !pad.disabled, "Cancelling owned dictation did not restore the saved draft.");
  document.querySelector(".toolbar-recording").click();
  await until(() => document.querySelector(".live-notes") && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, "Recording did not start after the session was explicitly cancelled.");
  assert((await api.padText()).includes("Synthetic draft preserved"), "Starting capture lost the durable pad draft.");
  return { ownedSessionBarrier: true, explicitCancelUnblocksStart: true, draftRetained: true, syntheticOnly: true };
}
