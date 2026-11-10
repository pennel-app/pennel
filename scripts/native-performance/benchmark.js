(() => {
  const scenario = new URL(location.href).searchParams.get("scenario") || "library";
  const client = window.__PENNEL_PLATFORM__;
  const fixture = window.__PENNEL_FIXTURE__;
  if (!client || !fixture || location.hostname !== "127.0.0.1") throw new Error("Owned synthetic platform required");
  const operations = [];
  const errors = [];
  const actualInvoke = client.invoke.bind(client);
  let revision = 1;
  let changedTitle = false;
  const segments = Array.from({ length: 2400 }, (_, index) => ({
    start_ms: index * 3000, end_ms: index * 3000 + 3000, speaker: index % 2 ? "me" : "others",
    text: `Synthetic segment ${index + 1}. Review the planning and confirm the next meeting with the team.`,
    failed: false, voice: index % 2 ? null : 1,
  }));
  const meetings = Array.from({ length: 1000 }, (_, index) => ({
    id: `2026-10-24 0900 Synthetic ${String(index + 1).padStart(4, "0")}`,
    title: `Synthetic meeting ${String(index + 1).padStart(4, "0")}`,
    start_iso: "2026-10-24T09:00:00+02:00", duration_min: 120, has_summary: true,
  }));
  const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const phase = name => window.webkit.messageHandlers.benchmark.postMessage({ type: "phase", phase: name });
  const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const until = async (check, message, timeout = 10000) => {
    const start = performance.now();
    while (!check()) {
      if (performance.now() - start > timeout) throw new Error(message);
      await sleep(10);
    }
  };
  const stats = values => {
    const sorted = [...values].sort((a, b) => a - b);
    return { count: sorted.length, median_ms: sorted[Math.floor(sorted.length / 2)] || 0,
      p95_ms: sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * .95))] || 0,
      max_ms: sorted.at(-1) || 0, over_50_ms: sorted.filter(value => value > 50).length };
  };
  const commandSummary = from => {
    const result = {};
    for (const operation of operations.slice(from)) {
      const row = result[operation.command] ||= { count: 0, response_bytes: 0 };
      row.count++; row.response_bytes += operation.bytes;
    }
    return result;
  };
  client.invoke = async (command, args = {}) => {
    // Optional command delays make a navigation waterfall reproducible, independent of disk caching.
    const delay = { meeting_detail: 20, meeting_voices: 40, speaker_setup: 40, meeting_templates: 40, mark_lines: 40 }[command] || 0;
    if (delay) await sleep(delay);
    let value = await actualInvoke(command, args);
    if (command === "meetings") value = structuredClone(meetings).map((meeting, index) => ({ ...meeting, title: index === 100 && changedTitle ? "Synthetic changed title" : meeting.title }));
    if (command === "meeting_detail") value = { ...value, info: meetings.find(meeting => meeting.id === args.id) || value.info, transcript: structuredClone(segments) };
    if (command === "meeting_status") value = { ...value, transcript_revision: revision, transcript: scenario === "recording" ? structuredClone(segments) : [] };
    if (command === "meeting_runtime_status") {
      const { notes, transcript, ...metadata } = value;
      value = { ...metadata, transcript_revision: revision };
    }
    operations.push({ command, bytes: new TextEncoder().encode(JSON.stringify(value) ?? "null").length });
    return value;
  };
  addEventListener("error", event => errors.push(String(event.message)));
  addEventListener("unhandledrejection", event => errors.push(String(event.reason)));
  const start = performance.now();
  async function run() {
    await until(() => window.__PENNEL_FIXTURE_READY__, "Frontend did not become ready");
    await until(() => document.querySelectorAll(".meeting-row").length === 1000, "Large synthetic library did not appear");
    await paint();
    const metrics = { scenario, startup_ready_ms: performance.now() - start, library_rows: 1000, transcript_segments: 2400,
      fixture_delays_ms: { meeting_detail: 20, meeting_voices: 40, speaker_setup: 40, meeting_templates: 40, mark_lines: 40 },
      measurements: {}, errors };
    if (scenario === "library") {
      const contentReady = [], feedback = [];
      for (let index = 0; index < 6; index++) {
        const target = meetings[index + 1];
        const button = document.querySelector(`.meeting-row[data-id="${target.id}"] .meeting-select`);
        const before = performance.now();
        button.click();
        await paint();
        const acknowledged = button.hasAttribute("aria-current") || button.closest(".meeting-row").classList.contains("selected") || document.querySelector("#content[aria-busy=true]");
        if (!acknowledged) await until(() => document.querySelector(".live-workspace .live-title")?.value === target.title, "Navigation never acknowledged its target");
        feedback.push(performance.now() - before);
        await until(() => document.querySelector(".live-workspace .live-title")?.value === target.title, "Meeting did not finish opening");
        await paint();
        contentReady.push(performance.now() - before);
      }
      metrics.measurements.navigation_feedback = stats(feedback);
      metrics.measurements.navigation_content_ready = stats(contentReady);
      await sleep(150);
      const transcriptTabStart = performance.now();
      document.querySelector('.live-workspace [data-live-tab="transcript"]').click();
      metrics.measurements.transcript_tab_synchronous_ms = performance.now() - transcriptTabStart;
      await until(() => document.querySelectorAll(".live-workspace .transcript-line").length === segments.length, "Long transcript did not render");
      await paint();
      metrics.measurements.transcript_tab_to_paint_ms = performance.now() - transcriptTabStart;
      const panel = document.querySelector(".live-transcript");
      const gaps = [];
      let previous;
      for (let index = 0; index < 120; index++) {
        await new Promise(resolve => requestAnimationFrame(timestamp => { if (previous !== undefined) gaps.push(timestamp - previous); previous = timestamp; resolve(); }));
        panel.scrollTop = (panel.scrollHeight - panel.clientHeight) * index / 119;
      }
      metrics.measurements.scripted_scroll_frame_gaps = stats(gaps);
      document.querySelector('.live-workspace .meeting-companion-close').click();
      const stableRow = document.querySelectorAll(".meeting-row")[1];
      const beforeLibraryUpdate = document.querySelector(".live-workspace");
      changedTitle = true;
      fixture.emit("meeting-updated", meetings[100].id);
      await until(() => [...document.querySelectorAll(".meeting-title")].some(node => node.textContent === "Synthetic changed title"), "Changed library metadata did not appear");
      await sleep(250);
      await paint();
      metrics.measurements.unchanged_library_row_retained = stableRow === document.querySelectorAll(".meeting-row")[1];
      metrics.measurements.unrelated_library_update_keeps_document = beforeLibraryUpdate === document.querySelector(".live-workspace");
      const previousArticle = document.querySelector(".live-workspace"), detailReads = operations.filter(row => row.command === "meeting_detail").length;
      fixture.emit("meeting-updated", meetings[6].id);
      const overlappingTabStart = performance.now();
      document.querySelector('.live-workspace [data-live-tab="transcript"]').click();
      metrics.measurements.transcript_during_refresh_synchronous_ms = performance.now() - overlappingTabStart;
      await until(() => operations.filter(row => row.command === "meeting_detail").length > detailReads, "Selected meeting content-change notification did not refresh the document");
      metrics.measurements.selected_library_update_keeps_document = document.querySelector(".live-workspace") === previousArticle;
      await until(() => document.querySelectorAll(".live-workspace .transcript-line").length === segments.length, "Long transcript did not survive a concurrent content refresh");
      await paint();
      metrics.measurements.transcript_during_refresh_to_paint_ms = performance.now() - overlappingTabStart;
      document.querySelector('.live-workspace .meeting-companion-close').click();
      const input = document.querySelector(".live-workspace .live-notes");
      const typing = [];
      input.focus();
      for (let index = 0; index < 20; index++) {
        const before = performance.now();
        input.applyUserChange(input.value + " synthetic");
        await paint();
        typing.push(performance.now() - before);
      }
      metrics.measurements.synthetic_typing_to_paint = stats(typing);
      document.querySelector(".library-settings").click();
      await until(() => document.querySelector(".settings-workspace"), "Settings failed to open");
      await sleep(250);
      const idleStart = operations.length;
      phase("settings-idle");
      await sleep(6500);
      phase("active");
      metrics.measurements.settings_idle_commands_6_5s = commandSummary(idleStart);
    } else {
      const disclosure = document.querySelector(".live-transcript-disclosure");
      if (disclosure) disclosure.open = true;
      await until(() => document.querySelectorAll(".live-recording .transcript-line").length === segments.length, "Live transcript did not render");
      await paint();
      const stableRow = document.querySelector(".live-recording .transcript-line");
      const idleStart = operations.length;
      phase("recording-idle");
      await sleep(3500);
      phase("active");
      metrics.measurements.recording_idle_commands_3_5s = commandSummary(idleStart);
      segments.push({ ...segments.at(-1), start_ms: 7200000, end_ms: 7203000, text: "A new synthetic segment." });
      revision++;
      await until(() => document.querySelectorAll(".live-recording .transcript-line").length === segments.length, "New live segment did not appear");
      metrics.measurements.unchanged_transcript_row_retained = stableRow === document.querySelector(".live-recording .transcript-line");
      const notes = document.querySelector(".live-notes");
      notes.focus(); notes.applyUserChange(notes.value + " synthetic note");
      await paint();
      metrics.measurements.live_note_focus_retained = document.activeElement === notes.contentDOM;
    }
    metrics.commands = commandSummary(0);
    metrics.unknown_commands = fixture.unknown;
    metrics.elapsed_ms = performance.now() - start;
    return metrics;
  }
  addEventListener("load", () => {
    run().then(result => window.webkit.messageHandlers.benchmark.postMessage(result), error => {
      window.webkit.messageHandlers.benchmark.postMessage({ scenario, error: String(error.stack || error), errors, commands: commandSummary(0), unknown_commands: fixture.unknown });
    });
  });
})();
