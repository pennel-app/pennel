const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
const frame = () => new Promise(resolve => requestAnimationFrame(resolve));

async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) {
    if (await check()) return;
    await pause();
  }
  throw new Error(message);
}

async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__?.workspace, "Use the owned synthetic workspace fixture.");
  await until(() => document.querySelector(".live-workspace .live-notes"), "The live workspace did not mount.");
  return { fixture: window.__PENNEL_FIXTURE__, root: document.querySelector(".live-workspace") };
}

function edit(input, text) {
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}

function select(root, pane) { root.querySelector(`[data-live-tab="${pane}"]`).click(); }
function panel(root, pane) { return root.querySelector(`[data-live-pane="${pane}"]`); }
function shown(node) { return !!node && !node.hidden && node.getClientRects().length > 0; }

async function ask(root, fixture, question) {
  const previous = fixture.workspace.requests().length;
  select(root, "questions");
  edit(root.querySelector(".live-question-input"), question);
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length > previous, "The question was not admitted.");
  const request = fixture.workspace.requests().at(-1);
  return { request, turn: root.querySelector(`[data-request-id="${request.request_id}"]`) };
}

function record(name, result) {
  window.__PENNEL_LIVE_ACCEPTANCE__ ??= {};
  window.__PENNEL_LIVE_ACCEPTANCE__[name] = result;
  return result;
}

export async function checkLiveCancellationAndSources() {
  const { fixture, root } = await owned();
  const note = root.querySelector(".live-notes");
  const cancelled = await ask(root, fixture, "Synthetic cancelled question");
  root.querySelector(".live-question-cancel").click();
  await until(() => root.querySelector(".live-question-cancel").hidden, "Cancellation did not release the composer.");
  assert(root.querySelector(".live-question-input").value === "Synthetic cancelled question", "Cancellation lost the question draft.");
  assert(await fixture.conversations.complete(cancelled.request.request_id, { text: "Late cancelled answer must never appear", citations: [] }) === false,
    "The durable request accepted a late cancelled completion.");
  fixture.emit("live-question-result", { snapshot: cancelled.request,
    answer: { text: "Late cancelled answer must never appear", citations: [] }, error: null });
  await frame();
  assert(!root.textContent.includes("Late cancelled answer"), "A late cancelled result entered the conversation.");
  const answer = await ask(root, fixture, "Synthetic source-linked question");
  fixture.workspace.complete(answer.request.request_id);
  await until(() => answer.turn.querySelector(".citation"), "The answer did not expose its source.");
  note.setSelectionRange(2, 12, "backward");
  note.scrollTop = 20;
  const state = { start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection, scroll: note.scrollTop };
  const history = root.querySelector(".live-question-history");
  history.scrollTop = 0;
  const historyScroll = history.scrollTop;
  answer.turn.querySelector(".citation").click();
  await until(() => root.querySelector(".live-source-highlight"), "Opening a source did not reveal the transcript passage.");
  const anchor = answer.request.anchors.find(item => item.passage.kind === "transcript");
  assert(root.querySelector(".live-source-highlight").dataset.segmentKey === anchor.segment_keys[0], "Source navigation highlighted a different transcript row.");
  root.querySelector(".live-source-back").click();
  assert(shown(panel(root, "questions")), "Returning did not restore the Questions pane.");
  assert(note.selectionStart === state.start && note.selectionEnd === state.end && note.selectionDirection === state.direction && note.scrollTop === state.scroll && history.scrollTop === historyScroll, "Source return changed editor selection or reading positions.");

  assert(typeof fixture.workspace.replaceTranscript === "function", "The changed-source fixture control is unavailable.");
  fixture.workspace.replaceTranscript([{ start_ms: 0, end_ms: 8000, speaker: "others", text: "This passage was replaced after the answer was frozen.", failed: false, voice: 1 }]);
  await until(() => root.querySelector(".live-transcript")?.textContent.includes("was replaced"), "The corrected transcript did not render.");
  answer.turn.querySelector(".citation").click();
  await until(() => shown(root.querySelector(".live-source-notice")), "Changed source did not explain its limitation.");
  assert(root.querySelector(".live-source-notice blockquote")?.textContent === anchor.source_text, "Changed-source recovery did not retain the frozen excerpt.");
  assert(!root.querySelector(".live-source-highlight"), "Changed source highlighted an unrelated current passage.");
  root.querySelector(".live-source-back").click();
  assert(root.querySelector(".live-notes") === note, "Question/source operations replaced the editor.");
  return record("cancellation_sources", { lateCancelledResultSuppressed: true, exactSource: true, returnPreserved: true, staleSourceExplained: true, syntheticOnly: true });
}

export async function checkLiveAfterStopAndNextMeeting() {
  const { fixture, root } = await owned();
  const oldStatus = await window.__PENNEL_PLATFORM__.invoke("meeting_status");
  const original = await ask(root, fixture, "Keep this frozen answer with the original meeting");
  root.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording(), "Capture was not stopped during generation.");
  await until(() => root.dataset.recording === "false", "The stopped workspace did not show processing/review.");
  assert(typeof fixture.workspace.restart === "function", "The next-meeting fixture control is unavailable.");
  const next = fixture.workspace.restart();
  fixture.emit("workspace-navigate", { section: "recording", tab: null, meeting_id: next.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace")?.dataset.sessionId === next.session_id, "The next meeting did not mount independently.");
  const nextRoot = document.querySelector(".live-workspace");
  assert(nextRoot !== root && nextRoot.querySelector(".live-notes").value === next.notes, "The next meeting reused previous notes or workspace identity.");
  await fixture.workspace.complete(original.request.request_id);
  await until(async () => {
    const state = await window.__PENNEL_PLATFORM__.invoke("conversation_snapshot", { request: { scope: { kind: "meeting", meeting_uid: oldStatus.meeting_uid }, conversation_id: original.request.conversation_id } });
    return state.messages.some(message => message.request_id === original.request.request_id && message.role === "assistant" && message.status === "completed");
  }, "The original answer was not stored after Stop.");
  assert(!nextRoot.textContent.includes("Synthetic frozen answer") && !nextRoot.textContent.includes(original.request.request_id), "The old result entered the next meeting.");
  fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: oldStatus.id, jump_ms: null });
  await until(() => document.querySelector(".live-workspace") === root, "Returning to the old meeting did not restore its workspace.");
  await until(() => original.turn.textContent.includes("Synthetic frozen answer"), "Returning to the old meeting did not restore its completed answer.");
  assert(root.querySelector(`[data-request-id="${original.request.request_id}"]`) === original.turn, "Returning lost the original conversation.");
  return record("stop_next_meeting", { stoppedDuringGeneration: true, originalAnswerCompleted: true, nextMeetingIsolated: true, conversationRetained: true, syntheticOnly: true });
}

export async function checkLiveResponsiveFollow() {
  const { fixture, root } = await owned();
  const content = root.querySelector(".live-workspace-panes");
  const originalWidth = content.style.width;
  const note = root.querySelector(".live-notes");
  const output = root.querySelector(".live-transcript");
  try {
    content.style.width = "790px";
    await until(() => !root.classList.contains("live-wide"), "The narrow workspace did not use four tabs.");
    select(root, "questions");
    content.style.width = "810px";
    await until(() => root.classList.contains("live-wide"), "The wide workspace did not expose both panes.");
    assert(shown(panel(root, "notes")) && shown(panel(root, "questions")), "The wide transition lost the deliberately selected companion.");
    const separator = root.querySelector(".live-pane-separator");
    const ratio = Number(separator.getAttribute("aria-valuenow"));
    separator.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true, cancelable: true }));
    assert(Number(separator.getAttribute("aria-valuenow")) === ratio - 5, "The pane divider cannot be resized by keyboard.");
    content.style.width = "790px";
    await until(() => !root.classList.contains("live-wide"), "The workspace did not return to tabs.");
    assert(shown(panel(root, "questions")), "The narrow transition forgot the selected Questions tab.");
    select(root, "notes");
    content.style.width = "810px";
    await until(() => root.classList.contains("live-wide"), "The second wide transition failed.");
    assert(shown(panel(root, "questions")), "Selecting Notes discarded the companion choice.");
    content.style.width = "790px";
    await until(() => !root.classList.contains("live-wide"), "The second narrow transition failed.");
    assert(shown(panel(root, "notes")) && root.querySelector(".live-notes") === note, "The deliberate Notes selection or editor identity was lost.");

    select(root, "transcript");
    for (let index = 0; index < 80; index++) fixture.workspace.append({ start_ms: 30_000 + index * 15_000, end_ms: 45_000 + index * 15_000,
      speaker: index % 2 ? "me" : "others", text: `Synthetic reading passage ${index}.`, failed: false, voice: null });
    await until(() => output.textContent.includes("Synthetic reading passage 79."), "The transcript fixture did not render.");
    await frame();
    output.scrollTop = Math.max(0, output.scrollHeight - output.clientHeight - 150);
    output.dispatchEvent(new Event("scroll"));
    const row = [...output.querySelectorAll(".transcript-line")].find(node => node.getBoundingClientRect().bottom > output.getBoundingClientRect().top);
    const text = row.querySelector(".transcript-copy").firstChild;
    const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, Math.min(8, text.length));
    const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
    document.dispatchEvent(new Event("selectionchange"));
    const selected = selection.toString();
    const scroll = output.scrollTop;
    const rowOffset = row.getBoundingClientRect().top - output.getBoundingClientRect().top;
    fixture.workspace.append({ start_ms: 1_250_000, end_ms: 1_265_000, speaker: "others", text: "A new finalized row arrives while the reader selects earlier text.", failed: false, voice: null });
    await until(() => output.textContent.includes("A new finalized row arrives"), "The new transcript row did not render.");
    assert(selection.toString() === selected && output.scrollTop === scroll && Math.abs(row.getBoundingClientRect().top - output.getBoundingClientRect().top - rowOffset) < 2, "Transcript refresh moved selected reading text.");
    assert(shown(root.querySelector(".live-follow")), "Scrollback/selection did not offer Follow live.");
    selection.removeAllRanges();
    root.querySelector(".live-follow").click();
    await frame();
    assert(Math.abs(output.scrollHeight - output.clientHeight - output.scrollTop) < 2, "Follow live did not return to the transcript end.");
    fixture.emit("workspace-visibility-changed", { visible: false });
    await pause(700);
    fixture.workspace.append({ start_ms: 1_280_000, end_ms: 1_295_000, speaker: "me", text: "Synthetic transcript committed while hidden.", failed: false, voice: null });
    await pause(700);
    fixture.emit("workspace-visibility-changed", { visible: true });
    await until(() => output.textContent.includes("committed while hidden"), "Reopening did not refresh the retained workspace.");
    assert(document.querySelector(".live-notes") === note, "Hide/reopen replaced the editor.");
    return record("responsive_follow", { paneChoiceRetained: true, keyboardDivider: true, selectionAnchored: true, followLive: true, hideReopenRetained: true, syntheticOnly: true });
  } finally { content.style.width = originalWidth; getSelection()?.removeAllRanges(); }
}

export async function checkLiveQuestionSaveFailure() {
  const { fixture, root } = await owned();
  const note = root.querySelector(".live-notes");
  const text = "Synthetic draft that must be saved before the question is sent.";
  fixture.workspace.failSaves(true);
  edit(note, text);
  select(root, "questions");
  const question = root.querySelector(".live-question-input");
  edit(question, "Do not send this question with incomplete note context");
  const begins = fixture.calls.filter(command => command === "begin_live_question").length;
  root.querySelector(".live-question-submit").click();
  await until(() => root.querySelector(".live-answer .error"), "Failed note storage did not explain that the question was not sent.");
  assert(fixture.workspace.requests().length === 0 && fixture.calls.filter(command => command === "begin_live_question").length === begins, "A failed note save still admitted a model request.");
  assert(question.value === "Do not send this question with incomplete note context" && note.value === text, "Failed submission discarded question or note text.");
  assert(root.querySelector(".live-save-status").dataset.saveState === "unsaved", "The failed note write was displayed as saved.");
  fixture.workspace.failSaves(false);
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length === 1, "Retry did not save notes and admit the retained question.");
  const request = fixture.workspace.requests()[0];
  assert(fixture.workspace.notes() === text && request.anchors.some(anchor => anchor.passage.kind === "notes" && anchor.source_text === text), "Retried question did not use the acknowledged notes.");
  fixture.workspace.complete(request.request_id);
  await until(() => root.textContent.includes("Synthetic frozen answer"), "Retried question did not complete.");
  assert(root.querySelector(".live-notes") === note, "Retry replaced the notes editor.");
  return record("question_save_failure", { providerNotCalledOnFailedSave: true, draftsRetained: true, acknowledgedRetryContext: true, syntheticOnly: true });
}

export async function checkLiveMissingTextSetup() {
  const { fixture, root } = await owned();
  const note = root.querySelector(".live-notes");
  const text = "Synthetic unsaved meeting draft during text setup.";
  fixture.workspace.failSaves(true);
  edit(note, text);
  note.setSelectionRange(4, 18, "backward");
  select(root, "questions");
  assert(root.querySelector(".live-question-input").disabled && root.querySelector(".live-question-submit").disabled, "Questions did not explain the missing text configuration.");
  assert(!note.disabled && !root.querySelector(".live-stop").disabled, "Missing text configuration blocked notes or capture control.");
  const setup = root.querySelector(".chat-setup button");
  setup.click();
  await until(() => document.querySelector('dialog[open][data-setup-task="llm"]'), "Question setup did not open beside its meeting.");
  let dialog = document.querySelector('dialog[open][data-setup-task="llm"]');
  await until(() => shown(dialog.querySelector("[data-capture-stop]")), "Stop was unreachable inside the modal setup flow.");
  assert(fixture.workspace.recording() && root.querySelector(".live-notes") === note && note.value === text, "Setup changed capture or discarded the originating draft.");
  dialog.querySelector("[data-setup-cancel]").click();
  await until(() => !document.querySelector("dialog[open]"), "Setup cancellation did not return to the same meeting.");
  assert(document.querySelector(".live-workspace") === root && note.value === text && note.selectionStart === 4 && note.selectionEnd === 18, "Setup return replaced the editor, draft or selection.");
  setup.click();
  await until(() => document.querySelector('dialog[open][data-setup-task="llm"]'), "Question setup did not reopen.");
  dialog = document.querySelector('dialog[open][data-setup-task="llm"]');
  await until(() => shown(dialog.querySelector("[data-capture-stop]")), "The reopened setup lost Stop.");
  dialog.querySelector("[data-capture-stop]").click();
  await until(() => !fixture.workspace.recording(), "Stop in setup waited on the failed note write.");
  dialog.querySelector("[data-setup-cancel]").click();
  await until(() => !document.querySelector("dialog[open]"), "Setup remained open after capture stopped.");
  assert(document.querySelector(".live-workspace") === root && note.value === text, "Stop in setup discarded the retained draft.");
  fixture.workspace.failSaves(false);
  await import("/src/views/meetings.ts").then(module => module.flushMeetingDrafts());
  return record("missing_text_setup", { questionsOnlyBlocked: true, sameMeetingOnReturn: true, stopReachableInSetup: true, stopDoesNotWaitForSave: true, syntheticOnly: true });
}

function distribution(values) {
  const ordered = [...values].sort((left, right) => left - right);
  return { samples: ordered.length, median_ms: ordered[Math.floor(ordered.length / 2)] ?? 0,
    p95_ms: ordered[Math.min(ordered.length - 1, Math.floor(ordered.length * .95))] ?? 0, max_ms: ordered.at(-1) ?? 0 };
}

export async function measureLiveLongSession() {
  const { fixture, root } = await owned();
  const note = root.querySelector(".live-notes");
  const output = root.querySelector(".live-transcript");
  const commits = [];
  const input = [];
  select(root, "transcript");
  for (let batch = 0; batch < 5; batch++) {
    const started = performance.now();
    for (let offset = 0; offset < 48; offset++) {
      const index = batch * 48 + offset;
      fixture.workspace.append({ start_ms: index * 15_000, end_ms: (index + 1) * 15_000, speaker: index % 2 ? "me" : "others",
        text: `Synthetic sixty-minute fixture passage ${index}. The team confirms the current planning.`, failed: false, voice: null });
    }
    const last = batch * 48 + 47;
    await until(() => output.textContent.includes(`fixture passage ${last}.`), "The sixty-minute fixture did not render its committed batch.");
    commits.push(performance.now() - started);
    select(root, "notes");
    for (let index = 0; index < 10; index++) {
      const inputStarted = performance.now();
      edit(note, `# Synthetic notes\n\nConcurrent typing ${batch * 10 + index}.`);
      await frame();
      input.push(performance.now() - inputStarted);
    }
    select(root, "transcript");
  }
  select(root, "notes");
  const saveStarted = performance.now();
  edit(note, "# Synthetic final notes\n\nConfirmed at the end of a sixty-minute transcript fixture.");
  root.querySelector(".live-save").click();
  await until(() => fixture.workspace.notes() === note.value && root.querySelector(".live-save-status").dataset.saveState === "saved", "The final notes were not acknowledged.");
  const saveMs = performance.now() - saveStarted;
  const answerStarted = performance.now();
  const answer = await ask(root, fixture, "What does this synthetic meeting confirm?");
  fixture.workspace.complete(answer.request.request_id);
  await until(() => answer.turn.textContent.includes("Synthetic frozen answer"), "The synthetic answer did not complete.");
  const answerMs = performance.now() - answerStarted;
  const stopStarted = performance.now();
  root.querySelector(".live-stop").click();
  await until(() => !fixture.workspace.recording(), "The synthetic capture state did not stop.");
  return record("sixty_minute_fixture", { transcript_duration_ms: 3_600_000, finalized_rows_added: 240,
    finalized_commit_to_render: distribution(commits), input_to_next_frame: distribution(input), save_acknowledgement_ms: saveMs,
    synthetic_answer_ms: answerMs, stop_to_fixture_capture_off_ms: performance.now() - stopStarted,
    actual_audio_capture: false, actual_provider_generation: false, accelerated_fixture: true, syntheticOnly: true });
}
