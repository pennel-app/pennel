const assert = (value, message) => { if (!value) throw new Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 150; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 30)); }
  throw new Error(message);
}
const visible = node => !!node?.getClientRects().length && !node.closest("[hidden]");
const click = node => { assert(visible(node) && !node.disabled, "The requested task action is unavailable."); node.click(); };
const edit = (node, text) => { node.focus(); node.value = text; node.dispatchEvent(new Event("input", {bubbles: true})); };

/** Run on a nonrecording owned dictation fixture; every input and result is synthetic. */
export async function checkTaskShell() {
  const fixture = window.__PENNEL_FIXTURE__;
  const client = window.__PENNEL_PLATFORM__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && fixture && client, "Use the owned desktop fixture.");
  const {api} = await import("/src/api.ts");
  const original = client.invoke.bind(client);
  const beforePad = await api.padText();
  const operations = [];
  const entries = Array.from({length: 8}, (_, index) => ({id: `shell-result-${index}`, created_at_ms: index + 1,
    app_bundle_id: null, raw: `Owned voice result ${index}`, text: `Owned voice result ${index}`, status: "done", error: null, has_audio: false}));
  client.invoke = async (command, args = {}) => {
    operations.push({command, args});
    return command === "history" ? structuredClone(entries) : original(command, args);
  };
  const section = name => document.querySelector(`.library-shortcuts [data-workspace-section="${name}"]`);
  const search = () => document.querySelector(".library-search input");
  try {
    fixture.emit("workspace-navigate", {section: "dictation", tab: "home", meeting_id: null, jump_ms: null});
    await until(() => document.querySelector(".dictation-home"), "Dictation home did not open.");
    fixture.emit("history-changed", null);
    await until(() => document.querySelectorAll("[data-recent-id]").length === 5, "Recent did not show exactly five real results.");
    assert(document.querySelectorAll(".library-shortcuts > button").length === 2, "The shell has more than two primary destinations.");
    assert(section("dictation").nextElementSibling?.classList.contains("dictation-navigation")
      && document.querySelector(".dictation-navigation").nextElementSibling === section("meetings"), "Dictation children appear beneath the wrong primary task.");
    assert(document.querySelectorAll(".library-search input").length === 1, "The shell duplicated its search field.");
    assert(!visible(document.querySelector(".library-ask")) && !visible(document.querySelector(".meeting-list")), "Dictation exposes the meeting library or meeting questions.");
    assert(visible(document.querySelector(".toolbar-recording")), "The shared toolbar lost New recording in Dictation.");
    const beforeSearch = operations.length;
    edit(search(), "voice result");
    await until(() => document.querySelectorAll("[data-history-result]").length === entries.length, "Dictation search did not search its retained results.");
    assert(!operations.slice(beforeSearch).some(item => item.command === "search_library"), "Dictation sent its query to meeting search.");
    click(document.querySelector('[data-command="toggle_sidebar"]'));
    window.dispatchEvent(new KeyboardEvent("keydown", {key: "f", metaKey: fixture.info().platform === "macos", ctrlKey: fixture.info().platform !== "macos", bubbles: true, cancelable: true}));
    await until(() => document.documentElement.dataset.sidebarVisible === "true" && document.activeElement === search(), "Find did not reveal and focus the contextual search.");
    assert(search().value === "voice result", "Find lost the current query.");
    click(section("meetings"));
    await until(() => document.querySelector('.library-sidebar[data-workspace-task="meetings"]'), "Meetings did not become the sidebar task.");
    assert(search().value === "" && visible(document.querySelector(".library-ask")), "Meeting search inherited a dictation query.");
    assert(!visible(document.querySelector('.dictation-navigation [data-workspace-section="history"]')), "History leaked into primary meeting navigation.");
    edit(search(), "planning");
    click(section("dictation"));
    await until(() => document.querySelector('.library-sidebar[data-workspace-task="dictation"]'), "Dictation did not restore its sidebar.");
    assert(search().value === "voice result", "Switching tasks lost the dictation query.");
    click(document.querySelector('[data-history-result="shell-result-3"]'));
    await until(() => document.querySelector('.history-target[data-history-id="shell-result-3"]'), "A History result lost its exact recovery selection.");
    assert(section("dictation").getAttribute("aria-current") === "page", "A legacy History route did not select Dictation.");
    click(document.querySelector('[data-dictation-section="pad"]'));
    await until(() => document.querySelector("textarea.dictation-pad:not(:disabled)"), "Write in Pennel did not open.");
    const draft = "Owned dirty pad draft across task Settings.";
    edit(document.querySelector("textarea.dictation-pad"), draft);
    click(document.querySelector(".library-settings"));
    await until(() => document.querySelectorAll("[data-settings-category]").length === 6, "Settings did not consolidate to six task categories.");
    assert(await api.padText() === draft, "Opening Settings abandoned the dirty pad draft.");
    click(document.querySelector(".settings-back"));
    await until(() => document.querySelector("textarea.dictation-pad")?.value === draft, "Settings Back did not restore Write in Pennel and its draft.");
    fixture.emit("workspace-navigate", {section: "settings", tab: "models", meeting_id: null, jump_ms: null});
    await until(() => document.querySelector('[data-settings-category="advanced"][aria-current="page"]') && document.querySelector('[data-settings-detail="models"][open]'), "The old Models link did not open Advanced Models.");
    assert(!document.querySelector(".models-settings [data-setup-task]"), "Advanced Models duplicates the task readiness overview.");
    assert(!document.querySelector("[data-inference-experimental]")?.open, "Experimental connections expanded in ordinary setup.");
    assert(!operations.some(item => ["start_meeting", "stop_meeting", "toggle_dictation", "toggle_dictation_session"].includes(item.command)), "Browsing tasks changed capture.");
    assert(!fixture.unknown.length, `Unknown fixture commands: ${fixture.unknown.join(", ")}`);
    return {primaryTasks: 2, nestedHistory: true, contextualSearch: true, shortcutRestoresSearch: true, recentLimit: 5, settingsCategories: 5, dirtyPadPreserved: true, legacyModelsLink: true};
  } finally {
    client.invoke = original;
    await api.savePadText(beforePad);
  }
}
