import { conversationRequests } from "../../../assets/contracts/meeting-conversations.ts";

const assert = (value, message) => { if (!value) throw Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 300; attempt++) { if (await check()) return; await pause(); }
  throw Error(message);
}
const visible = node => !!node?.getClientRects().length && !node.closest("[hidden]");
function edit(input, value) {
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic question fixture.");
  const questions = document.querySelector('[data-live-tab="questions"]');
  if (questions?.getAttribute("aria-selected") !== "true") questions?.click();
  await until(() => [...document.querySelectorAll(".conversation-panel")].some(panel => visible(panel) && !panel.querySelector(".conversation-input").disabled), "The question composer did not finish loading.");
  const root = [...document.querySelectorAll(".conversation-panel")].find(visible);
  const fixture = window.__PENNEL_FIXTURE__, api = window.__PENNEL_PLATFORM__;
  const scope = root.querySelector("[data-conversation-scope]").dataset.conversationScope === "library"
    ? { kind: "library" } : { kind: "meeting", meeting_uid: "qa-meeting-stable-uid" };
  return { root, fixture, api, scope, input: root.querySelector(".conversation-input"),
    snapshot: () => api.invoke("conversation_snapshot", { request: conversationRequests.snapshot(scope) }) };
}
function record(name, result) {
  window.__PENNEL_DOCUMENT_FIRST_ACCEPTANCE__ ??= {};
  window.__PENNEL_DOCUMENT_FIRST_ACCEPTANCE__[name] = { ...result, syntheticOnly: true };
  return window.__PENNEL_DOCUMENT_FIRST_ACCEPTANCE__[name];
}
function checkViewport(root) {
  const input = root.querySelector(".conversation-input").getBoundingClientRect();
  const submit = root.querySelector(".conversation-submit, .live-question-submit").getBoundingClientRect();
  const history = root.querySelector(".chat-history").getBoundingClientRect();
  const pane = root.closest(".live-pane")?.getBoundingClientRect();
  const bottom = Math.min(innerHeight - 6, pane?.bottom ?? innerHeight);
  assert(input.height >= 40 && input.bottom <= bottom + 1, `The question input is clipped: height=${input.height}, bottom=${input.bottom}, available=${bottom}.`);
  assert(submit.bottom <= bottom + 1, `The Ask action is clipped: ${submit.bottom} > ${bottom}.`);
  assert(history.height >= 64, "The composer leaves no usable conversation viewport.");
}
const settled = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function checkReachable(node, label, focus = true) {
  assert(visible(node), `${label} is unavailable.`);
  if (focus && !node.disabled) { assert(node.tabIndex >= 0, `${label} is not keyboard reachable.`); node.focus({ preventScroll: true }); }
  node.scrollIntoView({ block: "nearest", inline: "nearest" }); await settled();
  const rect = node.getBoundingClientRect(), bounds = { top: 0, left: 0, right: innerWidth, bottom: innerHeight };
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent), box = parent.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
      bounds.left = Math.max(bounds.left, box.left + parent.clientLeft);
      bounds.right = Math.min(bounds.right, box.left + parent.clientLeft + parent.clientWidth);
    }
    if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
      bounds.top = Math.max(bounds.top, box.top + parent.clientTop);
      bounds.bottom = Math.min(bounds.bottom, box.top + parent.clientTop + parent.clientHeight);
    }
  }
  assert(rect.width > 0 && rect.height > 0 && rect.top >= bounds.top - 1 && rect.bottom <= bounds.bottom + 1
    && rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1,
  `${label} is clipped after scrolling: ${JSON.stringify({ rect: { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right }, bounds })}.`);
  const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
  assert(hit && (hit === node || node.contains(hit)), `${label} is covered by another surface.`);
  if (focus && !node.disabled) assert(document.activeElement === node, `${label} could not receive keyboard focus.`);
}

export async function checkQuestionStarters() {
  const { root, fixture, input, snapshot } = await owned();
  const { getInterfaceLanguage, setInterfaceLanguage, t } = await import("/src/i18n.ts");
  const { flushConversations } = await import("/src/conversation-panel.ts");
  const initialLanguage = getInterfaceLanguage(), contentLanguage = fixture.info().settings.language;
  const scope = root.querySelector(".conversation-scope"), starters = root.querySelector(".conversation-starters");
  const beginCount = () => fixture.calls.filter(command => command === "begin_conversation_question").length;
  const before = beginCount();
  const question = contentLanguage?.toLowerCase().split("-")[0] === "nl" ? "Wat zijn de volgende stappen?" : "What are the next steps?";
  const staleStarter = root.querySelector('[data-question-starter="decisions"]');
  assert(visible(scope) && visible(starters) && starters.querySelectorAll("button").length === 3, "An empty conversation lacks its scope or question starters.");
  assert(input.getAttribute("aria-describedby") === scope.id, "The question input has no accessible scope description.");
  try {
    root.querySelector('[data-question-starter="actions"]').click();
    assert(input.value === question && document.activeElement === input, "A starter did not prepare and focus an editable question.");
    assert(!visible(starters), "Starters remain available over a prepared draft.");
    staleStarter.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    assert(input.value === question, "An old starter replaced a question draft.");
    await flushConversations();
    assert((await snapshot()).conversation.draft === question, "The prepared question did not use the durable draft flow.");
    input.setSelectionRange(2, 8, "backward");
    setInterfaceLanguage(initialLanguage === "en" ? "nl" : "en");
    assert(root.querySelector(".conversation-input") === input && input.value === question
      && input.selectionStart === 2 && input.selectionEnd === 8 && input.selectionDirection === "backward", "Relabeling changed prepared content, input identity or selection.");
    assert(scope.textContent === t(scope.dataset.conversationScope === "library" ? "conversations.scope_library" : "conversations.scope_meeting"), "Scope labels did not follow interface language.");
    edit(input, "");
    assert(visible(starters), "Clearing an empty conversation did not restore its starters.");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    assert(!visible(starters), "Starters can replace an active composition.");
    staleStarter.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    assert(input.value === "", "A starter wrote into an active composition.");
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    root.querySelector('[data-question-starter="actions"]').click();
    assert(input.value === question, "Changing interface language changed the canonical question language.");
    assert(beginCount() === before && fixture.info().settings.language === contentLanguage, "Preparing a question sent it or changed content language.");
    edit(input, ""); await flushConversations();
    checkViewport(root);
    return record("questionStarters", { draftOnly: true, durableDraft: true, localeIndependentContent: true, selectionPreserved: true,
      compositionPreserved: true, scope: scope.dataset.conversationScope });
  } finally { setInterfaceLanguage(initialLanguage); }
}

export async function checkQuestionScopeAfterAnswer() {
  const { root, fixture, input } = await owned();
  const scope = root.querySelector(".conversation-scope"), starters = root.querySelector(".conversation-starters");
  const beginCount = () => fixture.calls.filter(command => command === "begin_conversation_question").length;
  const before = beginCount();
  root.querySelector('[data-question-starter="decisions"]').click();
  const question = input.value;
  assert(question && beginCount() === before, "A question starter invoked inference before explicit submission.");
  const submit = root.querySelector(".conversation-submit, .live-question-submit");
  await until(() => !submit.disabled, "The configured synthetic fixture cannot submit its question.");
  submit.click();
  await until(() => root.querySelector('.conversation-assistant[data-message-status="completed"]'), "The explicitly submitted question did not complete.");
  assert(visible(scope) && !visible(starters), "A populated conversation lost its scope or reopened its empty-state starters.");
  assert(root.querySelector(".chat-question").textContent === question && beginCount() === before + 1, "Submission changed the prepared question or ran it more than once.");
  assert(root.querySelector(".citation"), "The answer no longer exposes its supporting source.");
  checkViewport(root);
  return record("questionScopeAfterAnswer", { explicitSingleSubmission: true, persistentScope: true, citationsRetained: true });
}

export async function checkEnlargedQuestionStarters() {
  await checkQuestionStarters();
  const { root, fixture, input } = await owned();
  const sizes = [...document.querySelectorAll("#app *")].filter(node => node instanceof HTMLElement)
    .map(node => [node, parseFloat(getComputedStyle(node).fontSize)]);
  for (const [node, size] of sizes) { node.style.fontSize = `${size * 2}px`; node.style.lineHeight = "1.4"; }
  await settled();
  const workspace = root.closest(".live-workspace");
  if (workspace) await until(() => workspace.classList.contains("live-enlarged"), "The enlarged meeting layout did not activate.");
  const before = fixture.calls.filter(command => command === "begin_conversation_question").length;
  const starter = root.querySelector('[data-question-starter="actions"]');
  await checkReachable(starter, "Question starter"); starter.click();
  assert(input.value, "The enlarged starter did not prepare a question.");
  await checkReachable(input, "Question composer");
  await checkReachable(root.querySelector(".conversation-scope"), "Question scope", false);
  await checkReachable(root.querySelector(".conversation-submit, .live-question-submit"), "Ask action");
  assert(fixture.calls.filter(command => command === "begin_conversation_question").length === before, "Enlarged question preparation sent content automatically.");
  return record("enlargedQuestionStarters", { enlargedText: true, composerReachable: true, actionsReachable: true,
    scope: root.querySelector(".conversation-scope").dataset.conversationScope, noAutomaticSubmission: true });
}
