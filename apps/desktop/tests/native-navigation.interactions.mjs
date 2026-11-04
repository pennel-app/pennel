const assert = (value, message) => { if (!value) throw new Error(message); };
import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
const pause = (ms = 20) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 200; attempt++) { if (check()) return; await pause(); }
  throw new Error(typeof message === "function" ? message() : message);
}

/** Exercises the production navigation with delayed synthetic IPC, without native/user data. */
export async function checkResponsiveNavigation() {
  const fixture = window.__PENNEL_FIXTURE__;
  const client = window.__PENNEL_PLATFORM__;
  assert(location.hostname === "127.0.0.1" && fixture && client, "Use the owned fixture server.");
  await until(() => document.querySelectorAll(".meeting-row").length >= 3 && document.querySelector(".live-notes"), "The meeting library did not finish mounting.");
  const original = client.invoke.bind(client);
  let releaseOptional;
  const optional = new Promise(resolve => { releaseOptional = resolve; });
  let releaseOld;
  const old = new Promise(resolve => { releaseOld = resolve; });
  let releaseSave;
  const save = new Promise(resolve => { releaseSave = resolve; });
  let slowId;
  let slowStarted = false;
  let saving = false;
  let optionalBlocked = true;
  let saveBlocked = false;
  client.invoke = async (command, args = {}) => {
    if (command === "meeting_detail" && args.id === slowId) { slowStarted = true; await old; }
    if (command === "save_meeting_notes" && saveBlocked) { saving = true; await save; }
    const value = await original(command, args);
    if (optionalBlocked && ["meeting_templates", "meeting_voices", "speaker_setup", "mark_lines"].includes(command)) await optional;
    return value;
  };
  const rows = () => [...document.querySelectorAll(".meeting-row")];
  const title = () => document.querySelector(".live-workspace .live-title")?.value;
  try {
    const beforeUpdate = document.querySelector(".live-workspace");
    fixture.emit("meeting-updated", "qa-unrelated-meeting");
    await pause(100);
    assert(document.querySelector(".live-workspace") === beforeUpdate, "Updating another meeting replaced the open document.");
    const first = rows()[0];
    const firstTitle = first.querySelector(".meeting-title").textContent;
    first.querySelector(".meeting-select").click();
    assert(first.classList.contains("selected") && document.querySelector("#content").getAttribute("aria-busy") === "true", "Navigation did not acknowledge the destination immediately.");
    await until(() => title() === firstTitle && !document.querySelector("#content").inert, "Optional meeting controls blocked the primary content.");
    optionalBlocked = false; releaseOptional();

    slowId = rows()[1].dataset.id;
    rows()[1].querySelector(".meeting-select").click();
    await until(() => slowStarted, "The delayed navigation was never requested.");
    const latest = rows()[2];
    const latestTitle = latest.querySelector(".meeting-title").textContent;
    latest.querySelector(".meeting-select").click();
    await until(() => title() === latestTitle && !document.querySelector("#content").inert, "A slow request blocked the newest navigation.");
    const mounted = document.querySelector(".live-workspace");
    releaseOld(); await pause(100);
    assert(document.querySelector(".live-workspace") === mounted && title() === latestTitle, "A stale meeting response replaced the selected destination.");

    const note = document.querySelector(".live-notes");
    assert(note, "The meeting note editor disappeared.");
    selectMeetingPane(mounted, "notes");
    saveBlocked = true;
    const draft = "Owned note retained through delayed durable persistence.";
    note.value = draft; note.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector(".library-settings").click();
    await until(() => saving, "Leaving did not flush the note draft.");
    assert(document.querySelector(".live-workspace") === mounted && document.querySelector("#content").inert, "Navigation left the draft before persistence completed.");
    saveBlocked = false; releaseSave();
    await until(() => document.querySelector(".settings-back") && !document.querySelector("#content").inert, "Settings did not open after the saved draft.");
    document.querySelector(".settings-back").click();
    await until(() => document.querySelector(".live-notes")?.value === draft, () => `Returning from Settings lost the saved draft: ${JSON.stringify({title: document.querySelector('.live-title')?.value, notes: document.querySelector('.live-notes')?.value, state: document.querySelector('.live-workspace')?.dataset, saved: fixture.calls.filter(command => command === 'save_meeting_notes').length})}`);
    assert(!document.querySelector("#content").inert, "Navigation stayed locked after returning.");
  } finally {
    releaseOptional(); releaseOld(); releaseSave();
    client.invoke = original;
  }
}
