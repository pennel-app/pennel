const assert = (value, message) => { if (!value) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 150; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 40));
  }
  throw new Error(message);
}
function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__ && window.__PENNEL_PLATFORM__, "Transcript checks require the owned synthetic fixture server.");
  return window.__PENNEL_FIXTURE__;
}

export async function checkTranscriptTabStability() {
  const fixture = owned();
  await until(() => document.querySelector('.meeting-detail [data-tab="transcript"]'), "A saved meeting must be open.");
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  client.invoke = async (command, args) => {
    const value = await original(command, args);
    if (command !== "meeting_detail") return value;
    return { ...value, transcript: Array.from({ length: 50 }, (_, index) => value.transcript.map(segment => ({
      ...segment, start_ms: segment.start_ms + index * 30_000, end_ms: segment.end_ms + index * 30_000,
    }))).flat() };
  };
  try {
    const oldArticle = document.querySelector(".meeting-detail");
    fixture.emit("workspace-navigate", { section: "meetings", tab: null, meeting_id: fixture.options.meeting_id, jump_ms: null });
    await until(() => document.querySelector(".meeting-detail") !== oldArticle, "The long owned transcript did not mount.");
    document.querySelector('.meeting-detail [data-tab="transcript"]').click();
    await until(() => document.querySelectorAll(".meeting-panel .transcript-line").length > 100, "The long transcript did not render.");
    await settle();
    const panel = document.querySelector(".meeting-panel");
    const content = panel.querySelector(".transcript");
    const rows = [...content.children];
    panel.scrollTop = Math.min(600, panel.scrollHeight - panel.clientHeight);
    await settle();
    const position = panel.scrollTop;
    assert(position > 0, "The fixture needs a scrollable transcript.");
    document.querySelector('.meeting-detail [data-tab="notes"]').click();
    await settle();
    document.querySelector('.meeting-detail [data-tab="transcript"]').click();
    await settle();
    assert(panel.querySelector(".transcript") === content, "Returning to Transcript rebuilt its content root.");
    assert(rows.every((row, index) => content.children[index] === row), "Returning to Transcript rebuilt rows.");
    assert(Math.abs(panel.scrollTop - position) <= 1, "Returning to Transcript lost the reading position.");
    return { rowsRetained: rows.length, readingPositionPreserved: true };
  } finally { client.invoke = original; }
}

export async function checkLiveTranscriptStability() {
  const fixture = owned();
  await until(() => document.querySelector(".live-transcript .transcript-line"), "Use the active recording fixture.");
  const { api } = await import("/src/api.ts");
  const initial = await api.meetingStatus();
  assert(initial.busy && initial.transcript.length > 1, "The synthetic recording must have transcript text.");
  const client = window.__PENNEL_PLATFORM__;
  const original = client.invoke.bind(client);
  let revision = initial.transcript_revision;
  let segments = initial.transcript;
  let reads = 0;
  let releaseRead;
  let delayRead = false;
  client.invoke = async (command, args) => {
    const value = await original(command, args);
    if (command === "meeting_runtime_status") return { ...value, transcript_revision: revision };
    if (command !== "meeting_status") return value;
    reads++;
    const snapshot = { ...value, transcript_revision: revision, transcript: segments.map(segment => ({ ...segment })) };
    if (delayRead) await new Promise(resolve => { releaseRead = resolve; });
    return snapshot;
  };
  try {
    document.querySelector('[data-live-tab="transcript"]').click();
    await settle();
    const output = document.querySelector(".live-transcript");
    const content = output;
    const currentRows = () => [...content.querySelectorAll(".transcript-line")];
    const rows = currentRows();
    const selectedText = rows[0].querySelector(".transcript-copy").firstChild;
    const range = document.createRange();
    range.setStart(selectedText, 0);
    range.setEnd(selectedText, Math.min(8, selectedText.textContent.length));
    getSelection().removeAllRanges(); getSelection().addRange(range);
    const selected = getSelection().toString();
    await new Promise(resolve => setTimeout(resolve, 2200));
    assert(reads === 0, "Unchanged recording metadata fetched the full transcript.");
    assert(getSelection().toString() === selected && getSelection().anchorNode === selectedText, "Clock updates replaced the selected transcript text.");

    segments = [...segments, ...Array.from({ length: 40 }, (_, index) => ({ ...segments[0], start_ms: 100_000 + index * 2000, end_ms: 101_000 + index * 2000, text: `Owned synthetic segment ${index}` }))];
    revision++;
    await until(() => currentRows().length === segments.length, "New transcript segments did not arrive.");
    assert(reads === 1, "A single revision caused duplicate full transcript requests.");
    assert(rows.every((row, index) => currentRows()[index] === row), "Appending transcript text replaced existing rows.");
    assert(getSelection().toString() === selected && getSelection().anchorNode === selectedText, "Appending transcript text lost the existing selection.");
    assert(output.scrollTop === 0 && !document.querySelector(".live-follow").hidden, "Selecting transcript text did not pause automatic following.");
    getSelection().removeAllRanges();
    document.querySelector(".live-follow").click();
    assert(output.scrollTop + output.clientHeight >= output.scrollHeight - 1, "Follow live did not resume at the transcript end.");

    output.scrollTop = 0;
    output.dispatchEvent(new Event("scroll"));
    getSelection().addRange(range);
    segments = [...segments, { ...segments[0], start_ms: 200_000, end_ms: 201_000, text: "A final owned synthetic segment" }];
    revision++;
    await until(() => currentRows().length === segments.length, "A later transcript segment did not arrive.");
    assert(output.scrollTop === 0, "A new transcript segment pulled the user away from their reading position.");
    assert(getSelection().toString() === selected, "A later transcript update lost the selected text.");

    delayRead = true; revision++;
    await until(() => releaseRead, "The controlled late transcript read did not begin.");
    document.querySelector(".library-settings").click();
    await until(() => document.querySelector(".settings-workspace"), "Settings did not open during the controlled read.");
    releaseRead();
    await settle();
    assert(document.querySelector(".settings-workspace") && !document.querySelector(".live-transcript"), "A disposed recording view accepted a late transcript read.");
    assert(!fixture.unknown.length, "Transcript updates used an unsupported fixture command.");
    return { unchangedFullReads: 0, rowIdentityPreserved: true, textSelectionPreserved: true, readingPositionPreserved: true, lateReadDiscarded: true };
  } finally { releaseRead?.(); client.invoke = original; getSelection().removeAllRanges(); }
}
