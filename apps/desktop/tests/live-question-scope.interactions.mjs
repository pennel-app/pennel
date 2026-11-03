const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = () => new Promise(resolve => setTimeout(resolve, 25));
async function until(check, message) {
  for (let attempt = 0; attempt < 200; attempt++) { if (check()) return; await pause(); }
  throw new Error(message);
}

export async function checkLiveQuestionSelection() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname), "Use the owned fixture server.");
  await until(() => document.querySelector(".live-workspace .live-notes"), "Workspace did not mount.");
  const fixture = window.__PENNEL_FIXTURE__;
  const root = document.querySelector(".live-workspace");
  const note = root.querySelector(".live-notes");
  root.querySelector('[data-live-tab="questions"]').click();
  const input = root.querySelector(".live-question-input");
  input.value = "Which decisions are supported by the selected sources?";
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: input.value }));
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length, "Question was not admitted.");
  const snapshot = fixture.workspace.requests().at(-1);
  assert(snapshot.selection?.scope === "selected_excerpts", "The selected-source fixture did not declare scope.");
  const metadata = root.querySelector(".live-answer-cutoff");
  await until(() => metadata.textContent.includes(`${snapshot.selection.included_sources}/${snapshot.selection.available_sources}`), "Source counts were not shown.");
  const localized = document.documentElement.lang === "nl" ? ["geselecteerde fragmenten", "Eerder gesprek ingekort"] : ["selected excerpts", "Earlier conversation shortened"];
  assert(localized.every(text => metadata.textContent.includes(text)), "Scope or shortened history was not localized.");
  assert(metadata.textContent.includes("2/8"), "Shortened history counts were not shown.");
  const frozenMetadata = metadata.textContent;
  note.value += "\nSynthetic note typed while this answer waits.";
  note.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "Synthetic note" }));
  note.setSelectionRange(2, 9, "backward");
  const expected = { text: note.value, start: note.selectionStart, end: note.selectionEnd, direction: note.selectionDirection };
  fixture.workspace.append({ start_ms: 30_000, end_ms: 38_000, speaker: "others", text: "Later finalized evidence.", failed: false, voice: null });
  fixture.workspace.complete(snapshot.request_id);
  await until(() => root.textContent.includes("Synthetic frozen answer"), "Frozen answer did not arrive.");
  assert(metadata.textContent === frozenMetadata, "A later append changed the admitted scope/cutoff.");
  assert(root.querySelector(".live-notes") === note && note.value === expected.text, "The answer replaced typed notes.");
  assert(note.selectionStart === expected.start && note.selectionEnd === expected.end && note.selectionDirection === expected.direction, "The answer changed note selection.");
  window.__PENNEL_LIVE_ACCEPTANCE__ = { selected_question_scope: { sourceCounts: true, shortenedHistory: true,
    localized: true, frozenAfterAppend: true, noteSelectionPreserved: true, syntheticOnly: true } };
  return window.__PENNEL_LIVE_ACCEPTANCE__;
}

export async function checkEnlargedQuestionAccess() {
  const root = document.querySelector(".live-workspace");
  await until(() => root.classList.contains("live-enlarged"), "The enlarged text layout did not activate.");
  const pane = root.querySelector(".live-questions-pane");
  const history = root.querySelector(".live-question-history");
  assert(history.clientHeight >= 24, "Enlarged text collapsed the answer history to an inaccessible height.");
  const metadata = root.querySelector(".live-answer-cutoff");
  metadata.scrollIntoView({ block: "nearest" });
  await new Promise(resolve => requestAnimationFrame(resolve));
  const bounds = metadata.getBoundingClientRect();
  const viewport = pane.getBoundingClientRect();
  assert(bounds.top < viewport.bottom && bounds.bottom > viewport.top, "The source disclosure cannot be reached by scrolling.");
  const stop = root.querySelector(".live-stop");
  assert(stop.getBoundingClientRect().bottom <= innerHeight, "Scrolling answers hid Stop.");
  const citation = root.querySelector(".citation");
  citation.scrollIntoView({ block: "nearest" });
  const questionScroll = pane.scrollTop;
  citation.click();
  await until(() => root.querySelector(".live-source-highlight"), "The enlarged citation did not open its source.");
  root.querySelector(".live-source-back").click();
  await new Promise(resolve => requestAnimationFrame(resolve));
  assert(pane.scrollTop === questionScroll, "Returning from a source lost the enlarged reading position.");
  metadata.scrollIntoView({ block: "nearest" });
  window.__PENNEL_LIVE_ACCEPTANCE__.selected_question_scope.enlargedAnswerReachable = true;
  window.__PENNEL_LIVE_ACCEPTANCE__.selected_question_scope.enlargedSourceReturnPreserved = true;
  return window.__PENNEL_LIVE_ACCEPTANCE__;
}
