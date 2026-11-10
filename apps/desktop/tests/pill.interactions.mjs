const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 0) => new Promise(resolve => setTimeout(resolve, ms));

async function ready() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic pill fixture.");
  for (let attempt = 0; attempt < 100; attempt++) {
    if (!document.getElementById("pill")?.hidden && document.getElementById("label")?.textContent) break;
    await pause(20);
  }
  const pill = document.getElementById("pill");
  const status = document.getElementById("status");
  assert(pill && status && !pill.hidden, "The production pill did not become visible.");
  return { pill, status, fixture: window.__PENNEL_FIXTURE__ };
}

async function finishAnimations(pill) {
  for (const animation of pill.getAnimations({ subtree: true })) {
    if (animation.effect.getComputedTiming().iterations !== Infinity) animation.finish();
  }
  await pause(20);
}

function observeText(status) {
  const records = [];
  const observer = new MutationObserver(changes => records.push(...changes));
  observer.observe(status, { childList: true, characterData: true, subtree: true });
  return { records, observer };
}

export async function checkPillStatusAnnouncements() {
  const { pill, status, fixture } = await ready();
  fixture.emit("pill-state", { state: "listening" });
  await finishAnimations(pill);
  const { records, observer } = observeText(status);
  try {
    const firstLabel = document.getElementById("label").firstChild;
    fixture.emit("pill-state", { state: "listening" });
    await pause();
    assert(records.length === 0 && document.getElementById("label").firstChild === firstLabel,
      "Duplicate pill-state events rewrote the live status region.");
    for (const level of [0, .04, .2, .7, 1, 0]) fixture.emit("pill-level", level);
    await pause();
    assert(records.length === 0, "Audio level ticks rewrote live status text.");

    fixture.emit("pill-state", { state: "copied", paste_key: "⌘V" });
    await finishAnimations(pill);
    assert(status.textContent.includes("⌘V"), "The copy hint did not arrive in the live status region.");
    records.length = 0;
    fixture.emit("pill-state", { state: "copied", paste_key: "⌘V" });
    await pause();
    assert(records.length === 0, "A duplicate confirmation announced the same hint again.");
    fixture.emit("pill-state", { state: "copied", paste_key: "Ctrl+V" });
    await pause();
    assert(records.length > 0 && status.textContent.includes("Ctrl+V") && !status.textContent.includes("⌘V"),
      "A meaningful same-state hint change did not update the announcement.");
    await finishAnimations(pill);

    records.length = 0;
    const { getInterfaceLanguage, setInterfaceLanguage } = await import("/src/i18n.ts");
    const initialLanguage = getInterfaceLanguage();
    const previousText = status.textContent;
    setInterfaceLanguage(initialLanguage === "en" ? "nl" : "en");
    await pause();
    assert(records.length > 0 && status.textContent !== previousText, "A language change did not relabel the status.");
    await finishAnimations(pill);
    records.length = 0;
    setInterfaceLanguage(initialLanguage === "en" ? "nl" : "en");
    await pause();
    assert(records.length === 0, "An unchanged language rewrote the live status.");
    setInterfaceLanguage(initialLanguage);
    await finishAnimations(pill);
  } finally { observer.disconnect(); }
  return { duplicateAnnouncementsSuppressed: true, levelsNeverAnnounced: true, sameStateHintRelabelled: true, localeRelabelled: true, syntheticOnly: true };
}

function fade(element, direction) {
  const animation = element.getAnimations().find(animation => {
    const frames = animation.effect.getKeyframes();
    return frames.length >= 2 && Number(frames[0].opacity) === (direction === "in" ? 0 : 1)
      && Number(frames.at(-1).opacity) === (direction === "in" ? 1 : 0);
  });
  assert(animation && animation.effect.getTiming().duration === 120, `The ${direction}going status does not have its real 120 ms fade.`);
  return animation;
}

export async function checkPillCrossfade() {
  const { pill, status, fixture } = await ready();
  fixture.emit("pill-state", { state: "listening" });
  await finishAnimations(pill);
  const oldText = status.textContent;
  fixture.emit("pill-state", { state: "working" });
  const outgoing = document.querySelector(".pill-status-outgoing");
  assert(outgoing?.textContent === oldText && outgoing !== status && outgoing.isConnected,
    "Changing states discarded the outgoing content instead of retaining a visual layer.");
  assert(outgoing.getAttribute("aria-hidden") === "true" && !outgoing.hasAttribute("role") && !outgoing.hasAttribute("aria-live")
    && !outgoing.id && !outgoing.querySelector("[id], [role], [aria-live]"), "The outgoing visual layer duplicated live-region semantics or IDs.");
  assert(document.querySelectorAll('[role="status"]').length === 1 && document.querySelectorAll("[aria-live]").length === 1
    && document.getElementById("status") === status, "A state switch replaced or duplicated the one live status region.");
  const entering = fade(status, "in");
  const leaving = fade(outgoing, "out");
  for (const animation of [entering, leaving]) { animation.pause(); animation.currentTime = 60; }
  const opacities = [status, outgoing].map(element => Number(getComputedStyle(element).opacity));
  assert(opacities.every(opacity => opacity > 0 && opacity < 1) && Math.abs(opacities[0] + opacities[1] - 1) < .02,
    "Incoming and outgoing content did not overlap as complementary fades.");
  fixture.emit("pill-state", { state: "working" });
  assert(document.querySelector(".pill-status-outgoing") === outgoing && status.getAnimations().includes(entering),
    "A duplicate state restarted an active crossfade.");
  const intermediateText = status.textContent;
  fixture.emit("pill-state", { state: "inserted", app: "Mail" });
  const replacement = document.querySelector(".pill-status-outgoing");
  assert(!outgoing.isConnected && leaving.playState === "idle" && entering.playState === "idle"
    && replacement !== outgoing && replacement.textContent === intermediateText && document.querySelectorAll(".pill-status-outgoing").length === 1,
    "An interrupted switch retained an obsolete outgoing layer or animation.");
  fade(status, "in"); fade(replacement, "out");
  await finishAnimations(pill);
  assert(!document.querySelector(".pill-status-outgoing"), "The outgoing layer survived its completed fade.");

  fixture.emit("pill-state", { state: "copied", paste_key: "⌘V" });
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "copied", paste_key: "Ctrl+V" });
  assert(document.querySelector(".pill-status-outgoing")?.textContent.includes("⌘V") && status.textContent.includes("Ctrl+V"),
    "A same-state hint update did not crossfade its previous content.");
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "problem", title: "No connection", detail: "Your words are kept in History" });
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "problem", title: "No connection", detail: "Original text pasted" });
  assert(document.querySelector(".pill-status-outgoing") && !document.getElementById("detail").hidden,
    "A meaningful same-state recovery detail did not crossfade.");
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "working" });
  const bounds = pill.getBoundingClientRect();
  assert([...document.querySelector(".pill-status-outgoing").children].filter(element => !element.hidden).every(element => {
    const child = element.getBoundingClientRect();
    return child.left >= bounds.left && child.top >= bounds.top && child.right <= bounds.right && child.bottom <= bounds.bottom;
  }), "The outgoing recovery text escaped the pill when changing to a shorter state.");
  fixture.emit("pill-state", { state: "hidden" });
  assert(!document.querySelector(".pill-status-outgoing"), "Hiding did not cancel the outgoing status animation.");
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "listening" });
  await finishAnimations(pill);
  return { trueCrossfade: true, durationMs: 120, oneLiveRegion: true, outgoingAriaHidden: true, interruptedAnimationsCancelled: true, sameStateContentCrossfades: true, boundedOutgoingContent: true, syntheticOnly: true };
}

export async function checkPillPreviewLifecycle() {
  const { pill, fixture } = await ready();
  const preview = document.getElementById("preview");
  const text = document.getElementById("preview-text");
  fixture.emit("pill-preview", null);
  fixture.emit("pill-preview", { session_id: "pill-first", revision: 0, text: "" });
  fixture.emit("pill-preview", { session_id: "pill-first", revision: 1, text: "Latest synthetic words" });
  fixture.emit("pill-state", { state: "working" });
  assert(!preview.hidden && text.textContent === "Latest synthetic words", "Working discarded the current preview.");
  fixture.emit("pill-preview", { session_id: "pill-first", revision: 1, text: "Older synthetic words" });
  assert(text.textContent === "Latest synthetic words", "A stale revision replaced the preview.");
  fixture.emit("pill-preview", { session_id: "pill-next", revision: 0, text: "" });
  fixture.emit("pill-preview", { session_id: "pill-first", revision: 99, text: "Late old session" });
  assert(preview.hidden && text.textContent === "", "A new session retained old words or admitted a superseded session.");
  fixture.emit("pill-preview", { session_id: "pill-next", revision: 1, text: "<b>Literal synthetic words</b>" });
  assert(text.textContent === "<b>Literal synthetic words</b>" && !text.querySelector("*"), "Preview markup was interpreted.");
  fixture.emit("pill-state", { state: "hidden" });
  fixture.emit("pill-preview", { session_id: "pill-next", revision: 2, text: "Late hidden words" });
  fixture.emit("pill-state", { state: "listening" });
  assert(preview.hidden && text.textContent === "", "Late text reappeared after hiding and a new capture.");
  await finishAnimations(pill);
  return { workingRetainsPreview: true, staleRevisionAndSessionRejected: true, hideClearsOwnership: true, previewIsPlainText: true, syntheticOnly: true };
}

export async function checkPillMotion({ reduced = false } = {}) {
  const { pill, fixture } = await ready();
  assert(matchMedia("(prefers-reduced-motion: reduce)").matches === reduced, "Set the real reduced-motion media feature for this case.");
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "hidden" });
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "listening" });
  const arrival = pill.getAnimations().find(animation => animation.effect.target === pill);
  assert(arrival, "Arrival has no short fade.");
  const frames = arrival.effect.getKeyframes();
  const movement = frames.some(frame => frame.transform && frame.transform !== "none");
  const start = new DOMMatrixReadOnly(frames[0].transform || "none");
  assert(Number(frames[0].opacity) === 0 && Number(frames.at(-1).opacity) === 1, "Arrival lost its opacity fade.");
  assert(reduced ? !movement && arrival.effect.getTiming().duration <= 140 : start.m42 === 8 && arrival.effect.getTiming().duration === 220,
    "Arrival does not preserve 220 ms / 8 px, or reduced motion kept movement or a long fade.");
  await finishAnimations(pill);
  fixture.emit("pill-state", { state: "working" });
  const spinner = document.getElementById("indicator").getAnimations();
  assert(reduced ? spinner.length === 0 : spinner.some(animation => animation.effect.getTiming().iterations === Infinity),
    "The working indicator did not respect system reduced motion.");
  fixture.emit("pill-state", { state: "hidden" });
  const departure = pill.getAnimations().find(animation => animation.effect.target === pill);
  assert(departure && departure.effect.getTiming().duration === (reduced ? 120 : 140), "Departure lost the expected short fade timing.");
  assert(!reduced || departure.effect.getKeyframes().every(frame => !frame.transform || frame.transform === "none"),
    "Reduced-motion departure retained movement.");
  await finishAnimations(pill);
  assert(pill.hidden, "The finished departure did not hide the pill.");
  fixture.emit("pill-state", { state: "listening" });
  await finishAnimations(pill);
  return { reduced, arrivalDurationMs: reduced ? 120 : 220, arrivalOffsetPx: reduced ? 0 : 8, departureDurationMs: reduced ? 120 : 140, opacityRetained: true, syntheticOnly: true };
}

export async function checkPillModelSwitch() {
  const { pill, fixture } = await ready();
  const label = document.getElementById("label"), chip = document.getElementById("model-chip");
  const { t } = await import('/src/i18n.ts');
  const model = name => ({ choice: { provider_id: name, model: "synthetic-model" }, label: name });
  fixture.emit("pill-state", { state: "listening" });
  fixture.emit("pill-preview", { session_id: "model-capture", revision: 0, text: "" });
  const first = { session_id: "model-capture", generation: 0, model: model("First model"), available_models: 2, enabled: true, switched: false };
  fixture.emit("pill-model", first);
  assert(!chip.hidden && chip.textContent === "First model" && !chip.matches('button,[role="button"]'), "The keyboard-only chip did not show the actual configured model.");
  const second = { ...first, generation: 1, model: model("Second model"), switched: true };
  fixture.emit("pill-model", second);
  assert(label.textContent === t("dictation.pill.model_switched", { model: "Second model" }) && chip.hidden, "The selected model was not confirmed.");
  await pause(500);
  fixture.emit("pill-model", second);
  fixture.emit("pill-model", { ...first, model: model("Obsolete model") });
  await pause(450);
  assert(label.textContent === t("dictation.pill.listening") && !chip.hidden && chip.textContent === "Second model", "Duplicates or old events extended or replaced the confirmation.");
  fixture.emit("pill-model", { ...second, generation: 2, model: model("Third model") });
  fixture.emit("pill-state", { state: "working" });
  await pause(950);
  assert(label.textContent === t("dictation.pill.working"), "An old model timer replaced the working state.");
  fixture.emit("pill-state", { state: "hidden" });
  fixture.emit("pill-state", { state: "listening" });
  fixture.emit("pill-preview", { session_id: "next-model-capture", revision: 0, text: "" });
  fixture.emit("pill-model", { ...second, generation: 99 });
  assert(chip.hidden, "An old capture supplied a new capture's model.");
  fixture.emit("pill-model", { ...first, session_id: "next-model-capture", enabled: false });
  assert(chip.hidden, "The actual disabled setting did not hide the chip.");
  await finishAnimations(pill);
  return { actualModel: true, keyboardOnly: true, confirmationMs: 900, duplicateTimerSuppressed: true, oldCapturesRejected: true, workingStatePreserved: true, syntheticOnly: true };
}

export async function checkPillNativeShortcutHints() {
  const { fixture } = await ready();
  fixture.emit('pill-state', { state: 'hands_free', cancel_shortcut: { keycaps: ['Ctrl', 'X'], text: 'Ctrl+X' } });
  const hint = document.getElementById('hint');
  assert(hint.textContent.includes('Ctrl+X') && !hint.textContent.includes('Esc'), 'Cancel hint ignored its actual native label.');
  fixture.emit('pill-state', { state: 'hands_free' });
  assert(!hint.textContent.includes('Ctrl+X'), 'An unavailable Cancel hint remained visible.');
  fixture.emit('pill-preview', { session_id: 'live-shortcuts', revision: 0, text: '' });
  const next = { session_id: 'live-shortcuts', generation: 0, model: { choice: { provider_id: 'local', model: 'synthetic' }, label: 'Actual model' }, available_models: 2, enabled: true, switched: false, next_shortcut: { keycaps: ['Ctrl', 'M'], text: 'Ctrl+M' } };
  fixture.emit('pill-model', next);
  assert(document.getElementById('model-chip').title.includes('Ctrl+M'), 'The live next-model label was missing.');
  fixture.emit('pill-model', { ...next, next_shortcut: null });
  assert(!document.getElementById('model-chip').title.includes('Ctrl+M'), 'Cached idle shortcut metadata overrode unavailable live readiness.');
  return { actualCancelLabel: true, unavailableHintsHidden: true, actualNextModelLabel: true, syntheticOnly: true };
}

export async function checkConfiguredCancelHint() {
  const result = await checkPillNativeShortcutHints();
  const { fixture } = await ready();
  const hint = document.getElementById('hint');
  fixture.emit('pill-state', { state: 'hands_free', cancel_shortcut: { keycaps: ['Ctrl', 'X'], text: 'Ctrl+X' } });
  assert(hint.textContent.includes('Ctrl+X'), 'The actual Cancel label did not appear before processing.');
  fixture.emit('pill-state', { state: 'working' });
  assert(!hint.textContent.includes('Ctrl+X'), 'Processing retained a listening-only Cancel hint.');
  fixture.emit('pill-state', { state: 'hands_free', cancel_shortcut: { keycaps: ['Ctrl', 'X'], text: 'Ctrl+X' } });
  return { ...result, processingHintCleared: true };
}

export async function checkPillPreferenceUpdates() {
  await ready();
  const { api } = await import('/src/api.ts');
  const client = window.__PENNEL_PLATFORM__, original = client.invoke;
  const initial = await api.info();
  const pending = [];
  client.invoke = (command, args) => command === 'app_info' ? new Promise(resolve => pending.push(resolve)) : original(command, args);
  try {
    window.__PENNEL_FIXTURE__.emit('settings-changed', null);
    window.__PENNEL_FIXTURE__.emit('settings-changed', null);
    await pause();
    assert(pending.length === 2, 'Preference refresh requests did not start.');
    pending[1]({ ...initial, settings: { ...initial.settings, pill_colour: 'light', pill_size: 'large', pill_position: 'top_centre' } });
    await pause();
    pending[0]({ ...initial, settings: { ...initial.settings, pill_colour: 'ink', pill_size: 'compact', pill_position: 'bottom_centre' } });
    await pause();
    assert(document.documentElement.dataset.pillColour === 'light' && document.documentElement.dataset.pillSize === 'large' && document.documentElement.dataset.pillPosition === 'top_centre', 'An older settings response overwrote newer pill preferences.');
  } finally { client.invoke = original; }
  return { stalePreferencesRejected: true, syntheticOnly: true };
}

export async function checkPillDemoPrivacy() {
  for (let attempt = 0; attempt < 100 && !document.getElementById('label')?.textContent; attempt++) await pause();
  const fixture = window.__PENNEL_FIXTURE__;
  assert(fixture && !fixture.calls.some(command => /dictation_state|toggle_dictation|copy_|append_dictation/.test(command)), 'The display-only preview fetched or changed capture text.');
  assert(document.getElementById('pill').dataset.state === 'preview' && document.getElementById('preview').hidden, 'The demo pretended to be a recording or displayed recognition text.');
  return { previewOnly: true, noCaptureSnapshot: true, noTextOrClipboardMutation: true, syntheticOnly: true };
}

export async function checkTinyPillRecovery() {
  const { pill } = await ready();
  assert(getComputedStyle(document.getElementById('preview')).display === 'none', 'A display too short for captions must keep space for the recovery message.');
  const rect = pill.getBoundingClientRect();
  assert(rect.top >= 0 && rect.bottom <= innerHeight, 'The recovery pill is clipped in a tiny work area.');
  assert(!document.getElementById('detail').hidden && document.getElementById('detail').textContent.trim(), 'The recovery detail disappeared in a tiny work area.');
  return { recoveryVisible: true, captionSpaceBounded: true, syntheticOnly: true };
}

export async function checkPillBackToBackModels() {
  const { fixture } = await ready();
  const chip = document.getElementById('model-chip');
  const metadata = (session_id, generation, label) => ({ session_id, generation, model: { choice: { provider_id: label, model: 'synthetic' }, label }, available_models: 2, enabled: true, switched: false });
  fixture.emit('pill-preview', { session_id: 'first-model-capture', revision: 0, text: '' });
  fixture.emit('pill-model', metadata('first-model-capture', 2, 'First capture model'));
  fixture.emit('pill-state', { state: 'working' });
  fixture.emit('pill-state', { state: 'listening' });
  fixture.emit('pill-preview', { session_id: 'second-model-capture', revision: 0, text: '' });
  assert(chip.hidden, 'A new preview retained the old capture model while waiting for its metadata.');
  fixture.emit('pill-model', metadata('second-model-capture', 0, 'Second capture model'));
  assert(!chip.hidden && chip.textContent.includes('Second capture model'), 'The new capture generation-zero model was missing.');
  fixture.emit('pill-model', metadata('first-model-capture', 99, 'Obsolete capture model'));
  assert(chip.textContent.includes('Second capture model'), 'An old capture replaced the new model.');
  return { backToBackCapture: true, generationZeroAccepted: true, oldModelCleared: true, lateCaptureRejected: true, syntheticOnly: true };
}

export async function checkPillFollowingAppearance() {
  const { pill, fixture } = await ready();
  assert(fixture.options.app_appearance_opposite_system, 'Use the declared opposite-appearance fixture.');
  const { api } = await import('/src/api.ts');
  const info = await api.info();
  const expected = info.settings.appearance === 'dark' ? 'rgb(42, 39, 35)' : 'rgb(252, 251, 248)';
  assert(getComputedStyle(pill).backgroundColor === expected, 'The following pill ignored Pennel appearance in favour of the opposite OS query.');
  await api.saveSettings({ ...info.settings, appearance: 'system' });
  const systemColour = matchMedia('(prefers-color-scheme: dark)').matches ? 'rgb(42, 39, 35)' : 'rgb(252, 251, 248)';
  for (let i = 0; i < 100 && getComputedStyle(pill).backgroundColor !== systemColour; i++) await pause();
  assert(getComputedStyle(pill).backgroundColor === systemColour, 'The pill did not resume OS-following appearance.');
  await api.saveSettings(info.settings);
  for (let i = 0; i < 100 && getComputedStyle(pill).backgroundColor !== expected; i++) await pause();
  assert(getComputedStyle(pill).backgroundColor === expected, 'The following pill did not return to Pennel appearance.');
  return { forcedAppOppositeSystem: true, sharedAppearanceHierarchy: true, syntheticOnly: true };
}
