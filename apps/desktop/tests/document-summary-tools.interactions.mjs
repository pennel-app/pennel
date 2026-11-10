import { openMeetingTools, selectMeetingPane } from "./meeting-workspace-test-controls.mjs";

const assert = (value, message) => { if (!value) throw Error(message); };
const visible = node => !!node?.getClientRects().length && !node.closest("[hidden]");
async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw Error(message);
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  await until(() => document.querySelector(".live-workspace .meeting-summary-surface"), "Open a completed meeting.");
  const root = document.querySelector(".live-workspace");
  selectMeetingPane(root, "summary");
  await until(() => !root.querySelector(".meeting-summary-editor").disabled, "The saved summary did not load.");
  return { root, fixture: window.__PENNEL_FIXTURE__, client: window.__PENNEL_PLATFORM__ };
}
function result(name, value) {
  window.__PENNEL_LIVE_ACCEPTANCE__ ??= {};
  window.__PENNEL_LIVE_ACCEPTANCE__[name] = { ...value, syntheticOnly: true };
  return value;
}

export async function checkSummaryContextAndConsent() {
  const { root, fixture, client } = await owned();
  const { t } = await import("/src/i18n.ts");
  const { flushMeetingDrafts } = await import("/src/views/meetings.ts");
  const surface = root.querySelector(".meeting-summary-surface");
  const details = surface.querySelector(".meeting-summary-details");
  const reading = surface.querySelector(".meeting-summary-rendered");
  assert(!details.open && visible(details.querySelector("summary")), "Routine processing/privacy details did not start compact and reachable.");
  assert(visible(reading) && reading.getBoundingClientRect().top < root.querySelector(".live-summary-pane").getBoundingClientRect().bottom,
    "Routine notices displaced all summary content below the reading viewport.");
  const outdated = surface.querySelector(".meeting-summary-outdated");
  if (fixture.options.summary_outdated) assert(visible(outdated) && !details.contains(outdated), "The stale-source warning was hidden inside routine details.");
  await until(() => root.querySelector('.summary-consent[data-mode="question"]'), "The unanswered summary question did not load.");
  const choice = root.querySelector(".summary-consent");
  assert(fixture.info().settings.summary_automation === "unanswered" && visible(choice)
    && !choice.querySelector("input[type=checkbox]").checked, "The unanswered automation choice is not visible and explicit.");
  const before = fixture.calls.length;
  await openMeetingTools(root, "automation");
  await until(() => visible(root.querySelector('[data-meeting-tool="automation"]')), "The automation choice did not open its focused tool.");
  const tool = root.querySelector('[data-meeting-tool="automation"]');
  assert(visible(tool.querySelector('[data-summary-automation="automatic"]')) && visible(tool.querySelector('[data-summary-automation="manual"]')),
    "Explicit automatic/manual choices are unavailable.");
  assert(tool.querySelector(".meeting-automation-explanation").textContent.includes(t("meetings.summary_automation_question", { destination: "" }).split(".")[0]),
    "The automation choice omits its privacy/processing explanation.");
  assert(!fixture.calls.slice(before).some(command => ["save_settings", "generate_meeting_summary", "generate_shareable_meeting_summary"].includes(command)),
    "Opening automation made a choice or invoked a model.");
  root.querySelector(".meeting-tool-back").click();
  assert(visible(surface) && fixture.info().settings.summary_automation === "unanswered", "Returning changed the policy or lost the summary.");

  surface.querySelector(".meeting-summary-edit").click();
  const editor = surface.querySelector(".meeting-summary-editor"), original = client.invoke.bind(client);
  client.invoke = async (command, args = {}) => {
    if (command === "save_meeting_summary") throw Error("Synthetic summary save failure.");
    return original(command, args);
  };
  try {
    editor.value = "Synthetic retained summary draft.";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
    await flushMeetingDrafts().catch(() => undefined);
    await until(() => visible(surface.querySelector(".meeting-summary-error")), "A failed summary save has no visible error.");
    assert(visible(surface.querySelector(".meeting-summary-retry")) && !details.contains(surface.querySelector(".meeting-summary-error")),
      "Critical save recovery was folded into routine details.");
    assert(editor.value === "Synthetic retained summary draft.", "The failed save lost the draft.");
  } finally { client.invoke = original; }
  await flushMeetingDrafts();
  await until(() => surface.dataset.summaryState === "saved", "The retained summary did not recover after a successful save.");
  return result("documentSummaryContext", { compactRoutineDetails: true, explicitConsent: true, visibleSaveRecovery: true });
}

export async function checkFocusedToolsReturn() {
  const { root, fixture, client } = await owned();
  const { flushMeetingDrafts } = await import("/src/views/meetings.ts");
  selectMeetingPane(root, "notes");
  const notes = root.querySelector(".live-notes"), pane = root.querySelector(".meeting-document");
  const text = Array.from({ length: 80 }, (_, index) => `Owned synthetic note ${index}.`).join("\n");
  notes.applyUserChange(text);
  await flushMeetingDrafts();
  notes.focus(); notes.setSelectionRange(12, 22, "backward"); notes.scrollTop = 65; pane.scrollTop = 20;
  const retained = { start: notes.selectionStart, end: notes.selectionEnd, direction: notes.selectionDirection, scroll: notes.scrollTop, paneScroll: pane.scrollTop };
  const before = fixture.calls.length;
  await openMeetingTools(root, "template");
  const tools = root.querySelector(".meeting-workspace-tools");
  await until(() => tools.querySelector(".meeting-template-choice button"), "The focused template chooser did not load.");
  assert([...tools.querySelectorAll("[data-meeting-tool]")].filter(visible).length === 1 && tools.dataset.activeTool === "template",
    "The selected template opened unrelated tool sections.");
  await until(() => document.activeElement === tools.querySelector(".meeting-tool-heading"), "Keyboard focus did not enter the selected tool.");
  assert(!fixture.calls.slice(before).some(command => ["speaker_setup", "meeting_voices"].includes(command)), "Opening templates loaded unrelated speaker setup.");
  tools.querySelector(".meeting-tool-back").click();
  assert(visible(notes) && document.activeElement === notes.contentDOM && notes.value === text, "Returning from a tool lost or replaced the document.");
  assert(notes.selectionStart === retained.start && notes.selectionEnd === retained.end && notes.selectionDirection === retained.direction
    && notes.scrollTop === retained.scroll && pane.scrollTop === retained.paneScroll, "Returning from a tool lost writing selection or reading position.");
  for (const section of ["speakers", "files", "automation"]) {
    await openMeetingTools(root, section);
    assert(tools.dataset.activeTool === section && [...tools.querySelectorAll("[data-meeting-tool]")].filter(visible).length === 1,
      `Opening ${section} exposed unrelated tool sections.`);
    tools.querySelector(".meeting-tool-back").click();
    assert(visible(notes) && notes.value === text, `Returning from ${section} lost the writing document.`);
  }
  await checkToolStatusOwnership(root, client);
  return result("documentFocusedTools", { oneSelectedTask: true, clearReturn: true, retainedSelection: true, noUnrelatedSpeakerLoad: true, readAndActionOwnership: true, actionSettlementAfterSettings: true });
}

async function checkToolStatusOwnership(root, client) {
  const original = client.invoke.bind(client);
  let rejectSpeaker, speakerSettled = false, failRead = true, failAction = true;
  let holdAction = false, pendingAction, actionSettled = false, pathCalls = 0;
  client.invoke = async (command, args = {}) => {
    if (command === "speaker_setup" && failRead) {
      return new Promise((_, reject) => { rejectSpeaker = reject; }).finally(() => { speakerSettled = true; });
    }
    if (command === "open_path") {
      ++pathCalls;
      if (holdAction) return new Promise((resolve, reject) => { pendingAction = { resolve, reject }; }).finally(() => { actionSettled = true; });
      if (failAction) throw Error("Synthetic file action failure.");
      return null;
    }
    return original(command, args);
  };
  const tools = root.querySelector(".meeting-workspace-tools"), status = tools.querySelector(".meeting-tools-status");
  try {
    await openMeetingTools(root, "speakers");
    await until(() => rejectSpeaker, "The owned speaker read was not held.");
    await openMeetingTools(root, "template");
    rejectSpeaker(Error("Synthetic late speaker read failure."));
    await until(() => speakerSettled, "The held speaker read did not settle.");
    await new Promise(requestAnimationFrame);
    assert(tools.dataset.activeTool === "template" && !visible(status), "A late speaker read polluted the focused Template tool.");
    failRead = false;
    await openMeetingTools(root, "speakers");
    await new Promise(requestAnimationFrame);
    assert(!visible(status), "A successful speaker retry retained an obsolete read error.");

    await openMeetingTools(root, "files");
    const folder = tools.querySelector(".meeting-file-actions button");
    await until(() => !folder.disabled, "The owned file action did not become available.");
    folder.click();
    await until(() => visible(status) && status.textContent.includes("Synthetic file action failure."), "The originating action failure was hidden.");
    await openMeetingTools(root, "template");
    assert(!visible(status), "A Files action failure appeared in the Template tool.");
    await openMeetingTools(root, "files");
    await new Promise(requestAnimationFrame);
    assert(visible(status) && status.textContent.includes("Synthetic file action failure."), "Reading the tool erased its unresolved action failure.");
    failAction = false; folder.click();
    await until(() => !visible(status), "A successful explicit action retry retained its old failure.");
    for (const outcome of ["resolve", "reject"]) {
      holdAction = true; pendingAction = undefined; actionSettled = false;
      folder.click();
      await until(() => pendingAction && visible(status), "The owned file action did not remain pending.");
      document.querySelector(".library-settings").click();
      await until(() => document.querySelector(".settings-back"), "Settings did not open while a tool action was pending.");
      document.querySelector(".settings-back").click();
      await until(() => document.querySelector(".live-workspace") === root && root.isConnected, "Settings did not return to the same owned meeting.");
      pendingAction[outcome](outcome === "reject" ? Error("Synthetic late file action failure.") : null);
      await until(() => actionSettled && !visible(status), "A settled action left Working visible after the Settings detour.");
      holdAction = false;
      const beforeRetry = pathCalls;
      folder.click();
      await until(() => pathCalls > beforeRetry && !visible(status), "The completed old action kept a new tool action blocked.");
    }
  } finally {
    failRead = false;
    if (!speakerSettled) rejectSpeaker?.(Error("Synthetic held read cleanup."));
    pendingAction?.resolve(null);
    client.invoke = original;
    tools.querySelector(".meeting-tool-back").click();
  }
}

export async function checkLongSummaryCitationReturn() {
  const { root, fixture } = await owned();
  const { flushMeetingDrafts } = await import("/src/views/meetings.ts");
  const summary = root.querySelector(".meeting-summary-surface"), editor = summary.querySelector(".meeting-summary-editor");
  summary.querySelector(".meeting-summary-edit").click();
  const text = "## Long synthetic summary\n\n" + Array.from({ length: 100 }, (_, index) => `Paragraph ${index}: a distinct synthetic summary passage whose rendered reading position must survive citation inspection.`).join("\n\n");
  editor.value = text; editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
  await flushMeetingDrafts(); summary.querySelector(".meeting-summary-edit").click();
  await until(() => editor.hidden, "The saved long summary did not return to reading mode.");
  selectMeetingPane(root, "questions");
  const input = root.querySelector(".live-question-input"); input.value = "Which summary passage supports this answer?"; input.dispatchEvent(new Event("input", { bubbles: true }));
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length, "The summary question was not admitted.");
  const request = fixture.workspace.requests().at(-1), index = request.anchors.findIndex(anchor => anchor.passage.kind === "summary");
  assert(index >= 0, "The owned question fixture did not capture the saved summary source.");
  await fixture.conversations.complete(request.request_id, { text: `A saved summary passage [${index + 1}]`, citations: [{ number: index + 1, passage: request.anchors[index].passage }] });
  await until(() => root.querySelector(".citation"), "The real summary citation did not render.");
  const document = root.querySelector(".meeting-document"); document.scrollTop = Math.min(1000, document.scrollHeight - document.clientHeight);
  const reading = document.scrollTop; assert(reading > 500, "The summary needs a deep real reading position.");
  root.querySelector(".citation").click();
  await until(() => !editor.hidden && editor.value.slice(editor.selectionStart, editor.selectionEnd) === request.anchors[index].source_text, "The citation did not inspect the exact saved summary passage.");
  root.querySelector(".live-source-back").click();
  assert(editor.hidden && document.scrollTop === reading, `Citation return lost long-summary reading position: ${document.scrollTop} instead of ${reading}.`);
  assert(editor.value === text && !root.querySelector(".live-questions-pane").hidden, "Citation return lost summary content or its answer context.");
  return result("longSummaryCitationReturn", { realSummaryCitation: true, renderedPresentationRestored: true, deepDocumentScrollRestored: true, exactSourceAndContentRetained: true });
}
