const assert = (condition, message) => { if (!condition) throw Error(message); };
const visible = node => Boolean(node?.getClientRects().length && !node.closest("[hidden]"));
async function until(check, message) {
  for (let attempt = 0; attempt < 180; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 30));
  }
  throw Error(message);
}
const click = node => { assert(visible(node) && !node.disabled, "The requested review action is unavailable."); node.click(); };
const edit = (node, value) => { node.focus(); node.value = value; node.dispatchEvent(new Event("input", { bubbles: true })); };
async function owned() {
  const fixture = window.__PENNEL_FIXTURE__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && fixture?.review, "Use an isolated owner-review fixture.");
  return { fixture, api: (await import("/src/api.ts")).api };
}

export async function checkOwnerReviewNavigation() {
  const { fixture, api } = await owned();
  await until(() => document.querySelector(".dictation-home"), "Dictation did not open.");
  const before = await api.history();
  click(document.querySelector('[data-dictation-section="pad"]'));
  await until(() => document.querySelector("textarea.dictation-pad:not(:disabled)"), "The editable pad did not open.");
  const pad = document.querySelector("textarea.dictation-pad"), draft = "Owner review draft retained through synthetic capture.";
  edit(pad, draft);
  fixture.review.dictationDelay(10_000);
  click(document.querySelector("main .btn.primary"));
  await until(() => pad.disabled && !document.querySelector("main .btn.primary").disabled, "Synthetic dictation did not begin listening.");
  click(document.querySelector("main .btn.primary"));
  await until(() => fixture.review.pendingDictations() === 1, "Dictation did not enter its pending completion.");
  click(document.querySelector('.library-shortcuts [data-workspace-section="meetings"]'));
  assert(pad.isConnected && pad.disabled, "Navigating abandoned an owned dictation that was still processing.");
  fixture.review.completeDictation();
  await until(() => !pad.disabled && pad.value.includes(draft) && pad.value !== draft, "Completed synthetic dictation did not return to the owned pad.");
  assert((await api.history()).length === before.length + 1, "A completed pad dictation was not retained exactly once.");
  const completed = pad.value;
  click(document.querySelector('.library-shortcuts [data-workspace-section="meetings"]'));
  await until(() => document.querySelector('.library-sidebar[data-workspace-task="meetings"]'), "Meetings did not open.");
  edit(document.querySelector(".library-search input"), "Studio Noord");
  await until(() => document.querySelectorAll(".meeting-row").length === 1, "Meeting search did not filter the seeded client conversation.");
  assert(document.querySelector(".meeting-row").textContent.includes("Studio Noord"), "Meeting search returned an unrelated result.");
  click(document.querySelector('.library-shortcuts [data-workspace-section="dictation"]'));
  await until(() => document.querySelector("textarea.dictation-pad")?.value === completed, "The pad text did not survive contextual search and return.");
  assert(!fixture.unknown.length, "Owner navigation used an unsupported fixture command.");
  return { syntheticOnly: true, completedOnce: true, processingOwnership: true, contextualSearch: true, padRetained: true };
}

export async function checkOwnerReviewMeeting() {
  const { fixture, api } = await owned();
  const begin = async title => {
    click(document.querySelector(".library-record"));
    await until(() => document.querySelector(".live-workspace .live-notes") && visible(document.querySelector(".live-stop")), "The synthetic meeting did not start in the notes workspace.");
    const heading = document.querySelector(".live-title"); edit(heading, title); heading.blur();
    await until(async () => (await api.meetingStatus()).title === title, "The running meeting title was not saved.");
    return api.meetingStatus();
  };
  await until(() => document.querySelector(".library-record"), "The meeting task did not open.");
  const first = await begin("Synthetic review meeting one");
  const note = "Synthetic private note retained after Stop.";
  const notes = document.querySelector(".live-notes");
  edit(notes, note); notes.blur();
  await until(async () => (await api.meetingStatus()).notes === note, "The synthetic live note was not saved.");
  click(document.querySelector(".live-stop"));
  await until(async () => !(await api.meetingStatus()).recording && !visible(document.querySelector(".live-stop")), "Stop did not finish synthetic capture.");
  assert((await api.meetingDetail(first.id)).notes === note, "Stopping lost the meeting note.");
  const second = await begin("Synthetic review meeting two");
  assert(second.id !== first.id && second.session_id !== first.session_id, "The next meeting reused the previous recording identity.");
  assert((await api.meetingDetail(first.id)).notes === note, "Starting a second meeting overwrote the first note.");
  click(document.querySelector(".live-stop"));
  await until(async () => !(await api.meetingStatus()).recording, "The second capture did not stop.");
  const row = () => document.querySelector(`.meeting-row[data-id="${CSS.escape(first.id)}"] .meeting-select`);
  await until(() => row(), "The first captured meeting was not retained in the library.");
  click(row());
  await until(() => document.querySelector(".live-notes")?.value === note, "Reopening the first meeting lost the original note.");
  const prepared = await api.generateShareableMeetingSummary(first.id, "algemeen");
  const output = await api.previewMeetingExport(first.id, "shareable", true);
  const { t } = await import("/src/i18n.ts");
  const appended = output.markdown.slice(prepared.markdown.length);
  assert(appended.includes(`${t("meetings.speaker_number", { number: "1" })} (${t("meetings.attribution_source_system_audio")})`)
    && !output.markdown.includes(note), "Shareable output lost localized canonical labels or included the private note.");
  assert(prepared.provenance === "transcript_only_v1" && !fixture.unknown.length, "The synthetic shared output lost its provenance or used an unknown command.");
  return { syntheticOnly: true, successiveMeetings: true, stoppedNotesRetained: true, reopenPreservesNotes: true, canonicalLabels: true, privateNotesExcluded: true };
}

export async function showShareableTranscriptForCapture() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned export fixture.");
  await (await import("/tests/simpler-meeting-workspace.interactions.mjs")).checkScopedCopyPreview();
  const preview = document.querySelector(".meeting-export-preview");
  click(preview.querySelector('[data-export-scope="shareable"]'));
  const generate = preview.querySelector(".meeting-export-generate");
  await until(() => !generate.disabled, "The shareable output action did not settle.");
  if (visible(generate)) click(generate);
  await until(() => !preview.querySelector('input[type="checkbox"]').disabled && preview.querySelector(".meeting-export-copy").value.trim(), "Shareable output did not prepare.");
  click(preview.querySelector('input[type="checkbox"]'));
  await until(() => !preview.querySelector('input[type="checkbox"]').disabled && preview.querySelector(".meeting-export-copy").value.includes("## Transcript"), "The exact preview did not include its transcript.");
  assert(!preview.querySelector(".meeting-export-copy").value.includes("PRIVATE_SYNTHETIC"), "Private content appeared in the shareable transcript capture.");
  return { syntheticOnly: true, scope: "shareable", includeTranscript: true };
}

export async function checkOwnerReviewReloadAndReset() {
  await owned();
  const frames = [], profile = `${new URL(location.href).searchParams.get("profile")}-target`;
  const open = async (onboarding = false) => {
    const url = new URL(location.href);
    url.searchParams.set("qa", onboarding ? "owner-review-setup-new" : "owner-review-navigation");
    url.searchParams.set("window", onboarding ? "onboarding" : "workspace"); url.searchParams.set("profile", profile);
    const frame = document.createElement("iframe"); frame.hidden = true; frame.src = url.href;
    frames.push(frame); document.body.append(frame);
    await until(() => frame.contentWindow?.__PENNEL_FIXTURE__?.review && frame.contentDocument?.querySelector("#app")?.textContent.trim(), "A separate review context did not load.");
    return { client: frame.contentWindow.__PENNEL_PLATFORM__, fixture: frame.contentWindow.__PENNEL_FIXTURE__ };
  };
  try {
    const a = await open(), id = a.fixture.options.meeting_id, model = "whisper-large-v3-turbo";
    await a.client.invoke("delete_model", { id: model });
    await a.client.invoke("download_model", { id: model });
    await until(async () => (await a.client.invoke("model_state", { id: model })).kind === "downloaded", "The controlled model download did not complete.");
    const before = await a.client.invoke("meeting_summary", { id });
    await a.client.invoke("save_meeting_summary", { id, expectedRevision: before.revision, expectedSourceRevision: before.source_revision, markdown: "Synthetic revised summary." });
    await a.client.invoke("save_pad_text", { text: "Synthetic text from the old review context." });
    const b = await open();
    assert((await b.client.invoke("model_state", { id: model })).kind === "downloaded", "Reloading lost a completed model download.");
    assert((await b.client.invoke("meeting_summary_history", { id }))[0]?.markdown === before.markdown, "Reloading lost a recoverable summary revision.");
    a.fixture.review.dictationDelay(60_000);
    await a.client.invoke("toggle_dictation"); await a.client.invoke("toggle_dictation");
    b.fixture.review.resetProfile();
    assert(a.fixture.review.completeDictation() === 0, "A reset profile accepted an older context's dictation completion.");
    let rejected = false;
    try { await a.client.invoke("save_pad_text", { text: "An obsolete late write." }); } catch { rejected = true; }
    assert(rejected, "An older review context could write after reset.");
    const fresh = await open(true);
    assert((await fresh.client.invoke("model_state", { id: model })).kind === "not_downloaded", "Reset retained completed synthetic model state.");
    assert(!(await fresh.client.invoke("pad_text")).includes("old review context"), "Late work recreated the cleared review profile.");
    assert((await fresh.client.invoke("meeting_summary_history", { id })).length === 0, "Reset retained synthetic summary history.");
    return { syntheticOnly: true, downloadSurvivesReload: true, summaryHistorySurvivesReload: true, crossContextResetFenced: true };
  } finally { frames.forEach(frame => frame.remove()); }
}

export async function checkOwnerReviewPractice() {
  const { api } = await owned(), { t } = await import("/src/i18n.ts");
  await until(() => document.querySelector(".dictation-home-practice:not(:disabled)"), "The primary practice action is not ready.");
  const pad = await api.padText(), originalOpen = window.open;
  let opened, frame, reloaded;
  window.open = (url, name) => { opened = { url, name }; return { focus() {} }; };
  try {
    click(document.querySelector(".dictation-home-practice"));
    await until(() => opened, "Try it here did not open an owned practice route.");
    const target = new URL(opened.url);
    assert(target.origin === location.origin && target.searchParams.get("qa") === "owner-review-practice"
      && target.searchParams.get("profile") === new URL(location.href).searchParams.get("profile"), "Practice left the local review or lost its shared profile.");
    frame = document.createElement("iframe"); frame.src = target.href;
    frame.style.cssText = "position:fixed;left:-10000px;top:0;width:980px;height:640px;border:0";
    document.body.append(frame);
    const doc = () => frame.contentDocument;
    try { await until(() => doc()?.querySelector("textarea.practice-text"), "The requested practice window did not render."); }
    catch (error) { throw Error(`${error.message} Child heading: ${doc()?.querySelector("h1")?.textContent ?? "not mounted"}`); }
    const newerPad = `${pad}\nParent edit made after opening practice.`;
    await api.savePadText(newerPad);
    const button = () => [...doc().querySelectorAll("button")].find(node => node.textContent === t("onboarding.start_dictation"));
    await until(() => button(), "The local practice window has no start action.");
    const action = button(); click(action);
    await until(() => action.textContent === t("onboarding.stop_dictation"), "Practice did not begin synthetic listening.");
    click(action);
    await until(() => doc().querySelector("textarea.practice-text")?.value.trim() && !doc().querySelector("textarea.practice-text").disabled, "Practice did not complete into its own editable field.");
    assert(await api.padText() === newerPad, "Owned practice overwrote the parent's saved pad.");
    assert(!frame.contentWindow.__PENNEL_FIXTURE__.calls.includes("copy_text"), "Practice fell back to the clipboard.");
    const reopen = new URL(location.href); reopen.searchParams.set("qa", "owner-review-navigation");
    reloaded = document.createElement("iframe"); reloaded.src = reopen.href; reloaded.style.cssText = frame.style.cssText; document.body.append(reloaded);
    await until(() => reloaded.contentWindow?.__PENNEL_PLATFORM__, "The parent profile could not reopen.");
    assert(await reloaded.contentWindow.__PENNEL_PLATFORM__.invoke("pad_text") === newerPad, "A practice checkpoint replaced newer parent work on reload.");
    return { syntheticOnly: true, primaryActionWorks: true, localOwnedWindow: true, editableResult: true, parentDraftPreserved: true };
  } finally { window.open = originalOpen; frame?.remove(); reloaded?.remove(); }
}
