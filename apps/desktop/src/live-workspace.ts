import { api, type MeetingStatus, type MeetingFinished, type MeetingPreviewState, type Speaker, type LiveSourceAnchor, type Segment } from "./api";
import { SerialQueue, persistAll } from "./async";
import { bindLocalization, l, t, localizeBackendText, type TranslationKey } from "./i18n";
import { controlButton, errorText, formatClock, h, markdown, textField } from "./dom";
import { MeetingDraft } from "./meeting-draft";
import { RecordingRefresh, sameSegment, segmentKeys } from "./recording-refresh";
import { LivePaneState, LiveStatusEpoch, segmentEvidenceValue, formatSelection, type LivePane } from "./live-workspace-state";
import { speakerUiLabel } from "./meeting-labels";
import { VisibilityPoll } from "./window-lifecycle";
import { attributionEditable, attributionNoticeKey } from "./meeting-source";
import { LiveSpeakerControl, liveSpeakerStatusKey } from "./live-speaker-state";
import { reviewedInsights, type InsightReviewHost } from "./reviewed-insights";
import { insightSourceMatches, insertionStillCurrent, utf8Range, type InsightNoteLatch, type InsightSource } from "../../../assets/contracts/meeting-insights.ts";
import type { Context } from "./main";
import { openPeoplePicker, peopleNoteControls, type PeopleHost } from "./people-picker";
import { canAcknowledgePeopleNotes, speakerLinkUtteranceIds, type PeopleSnapshot } from "./people-state";
import { conversationPanel } from "./conversation-panel";

interface LiveWorkspace {
  root: HTMLElement;
  id: string;
  uid: string;
  session: string;
  aliases: Set<string>;
  bind(ctx: Context): HTMLElement;
  flush(): Promise<void>;
  finish(event: MeetingFinished): void;
  savedExternally(text: string): void;
}

const workspaces = new Map<string, LiveWorkspace>();
let displayedSession: string | null = null;
export function getRetainedMeetingWorkspace(id?: string): HTMLElement | null {
  return [...workspaces.values()].find(owner => id ? owner.id === id || owner.aliases.has(id) : owner.session === displayedSession)?.root ?? null;
}

export function retainedMeetingWorkspace(ctx: Context, id: string): HTMLElement | null {
  const owner = [...workspaces.values()].find(owner => owner.id === id || owner.aliases.has(id));
  return owner?.bind(ctx) ?? null;
}

export function displayedLiveWorkspaceSession() { return displayedSession; }
export function finishLiveWorkspace(event: MeetingFinished) { workspaces.get(event.session_id)?.finish(event); }
export function acknowledgeRetainedNotes(id: string, text: string) { [...workspaces.values()].find(owner => owner.id === id || owner.aliases.has(id))?.savedExternally(text); }
export async function flushLiveWorkspaces() { await persistAll([...workspaces.values()].map(owner => () => owner.flush())); }

export function liveMeetingWorkspace(ctx: Context, status: MeetingStatus): HTMLElement {
  const session = status.session_id ?? status.id!;
  let owner = workspaces.get(session);
  if (!owner) {
    owner = createWorkspace(ctx, status);
    workspaces.set(session, owner);
  }
  return owner.bind(ctx);
}

async function fingerprint(segment: Segment): Promise<string> {
  const value = JSON.stringify(segmentEvidenceValue(segment));
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

function createWorkspace(initialContext: Context, initial: MeetingStatus): LiveWorkspace {
  let ctx = initialContext;
  const session = initial.session_id ?? initial.id!;
  const uid = initial.meeting_uid ?? initial.id!;
  let status = initial;
  let finalized = false;
  let binding = 0;
  let poll: VisibilityPoll | null = null;
  let composing = false;
  let notesTimer: number | undefined;
  let titleTimer: number | undefined;
  let preview = false;
  let previewValue = "";
  let sourceNavigation = 0;
  let following = true;
  let previousScroll = 0;
  let intendedScroll: number | null = null;
  let pendingTranscript: Segment[] | null = null;
  let sourceReturn: { pane: LivePane; questionScroll: number; transcriptScroll: number; notesScroll: number; start: number; end: number; direction: "forward" | "backward" | "none"; turn: HTMLElement; restore?: () => void; label?: TranslationKey } | null = null;
  let insights: ReturnType<typeof reviewedInsights> | null = null;
  let inputRevision = 0, selectionRevision = 0, interactionRevision = 0;
  let observedSelection = "";
  let insertionReceipt: { text: string; attempt: string } | null = null;
  const panes = new LivePaneState();
  const statusEpoch = new LiveStatusEpoch();
  const notes = new MeetingDraft();
  const titleDraft = new MeetingDraft();
  const writes = new SerialQueue();
  const titleWrites = new SerialQueue();
  const refresh = new RecordingRefresh(initial);
  const liveSpeakers = new LiveSpeakerControl(session, initial.live_speaker_attribution);
  let peopleSnapshot: PeopleSnapshot | null = null;
  const rows = new Map<string, { element: HTMLElement; body: HTMLElement; speaker: HTMLElement; provisional: HTMLElement; correction: HTMLElement; segment: Segment }>();
  const root = h("article", { class: "live-workspace live-recording", "data-session-id": session, "data-meeting-uid": uid });
  for (const event of ["pointerdown", "keydown", "input"]) root.addEventListener(event, () => { ++interactionRevision; }, true);
  const title = textField({ class: "live-title", "aria-label": l("meetings.meeting_title_label") });
  const clock = h("span", { class: "live-clock" });
  const phase = h("span", { class: "live-phase", role: "status" });
  const stop = controlButton({ class: "btn live-stop", onclick: () => { void stopCapture(); } }, l("common.stop_recording"));
  const review = controlButton({ class: "btn live-review", hidden: true, onclick: () => { void flush().then(() => ctx.navigate("review", owner.id)).catch(showSaveError); } }, l("meetings.workspace_review_export"));
  const band = h("header", { class: "live-recording-band" }, title,
    h("div", { class: "row recording-controls" }, h("span", { class: "status-dot", "aria-hidden": "true" }), phase, clock, h("span", { class: "grow" }), stop, review));
  const warning = h("p", { class: "live-warning help", role: "status", hidden: true });
  const recovery = h("div", { class: "live-recovery", hidden: true });
  const sourceBack = h("button", { class: "text-button live-source-back", hidden: true, onclick: () => returnToAnswer() }, l("meetings.workspace_back_answer"));
  const sourceNotice = h("div", { class: "live-source-notice", hidden: true });
  const tabList = h("div", { class: "live-tabs tabs", role: "tablist", "aria-label": l("meetings.workspace_notes") });
  const paneNames: [LivePane, TranslationKey][] = [["notes", "meetings.workspace_notes"], ["transcript", "meetings.workspace_transcript"], ["questions", "meetings.workspace_questions"], ["suggestions", "meetings.workspace_suggestions"]];
  const tabButtons = new Map<LivePane, HTMLButtonElement>();
  const panels = new Map<LivePane, HTMLElement>();
  for (const [pane, key] of paneNames) {
    const button = h("button", { class: "tab", role: "tab", id: `live-${session}-${pane}-tab`, "data-live-tab": pane, "aria-controls": `live-${session}-${pane}`, onclick: () => selectPane(pane) }, l(key));
    button.addEventListener("keydown", event => {
      const available = paneNames.filter(([name]) => !panes.wide || name !== "notes").map(([name]) => name);
      const index = available.indexOf(pane);
      const next = event.key === "ArrowRight" ? available[(index + 1) % available.length] : event.key === "ArrowLeft" ? available[(index - 1 + available.length) % available.length] : event.key === "Home" ? available[0] : event.key === "End" ? available.at(-1) : null;
      if (next) { event.preventDefault(); selectPane(next); tabButtons.get(next)?.focus(); tabButtons.get(next)?.scrollIntoView({ block: "nearest", inline: "nearest" }); }
    });
    tabButtons.set(pane, button);
    tabList.append(button);
    panels.set(pane, h("section", { class: `live-pane live-${pane}-pane`, id: `live-${session}-${pane}`, "data-live-pane": pane, role: "tabpanel", "aria-labelledby": button.id }));
  }
  const editor = h("textarea", { class: "live-notes", placeholder: l("meetings.live_notes_placeholder"), "aria-label": l("meetings.your_notes"), spellcheck: "true" });
  const notePreview = h("div", { class: "live-note-preview markdown", hidden: true, tabindex: 0 });
  const saveStatus = h("span", { class: "live-save-status", role: "status", "aria-live": "polite" });
  const save = controlButton({ class: "btn compact live-save", onclick: () => { void flush().catch(showSaveError); } }, l("meetings.workspace_save"));
  const saveError = h("p", { class: "error live-save-error", hidden: true, role: "status" });
  const sourceToggle = h("button", { class: "btn compact", "aria-pressed": "true", onclick: () => showPreview(false) }, l("meetings.workspace_source"));
  const previewToggle = h("button", { class: "btn compact", "aria-pressed": "false", onclick: () => showPreview(true) }, l("meetings.workspace_preview"));
  const formatting = h("div", { class: "live-formatting", role: "toolbar", "aria-label": l("meetings.workspace_source") });
  for (const [format, key, text] of [["heading", "meetings.workspace_heading", "H"], ["bold", "meetings.workspace_bold", "B"], ["list", "meetings.workspace_list", "•"], ["task", "meetings.workspace_task", "☐"]] as const) {
    const button = h("button", { class: "btn compact", title: l(key), "aria-label": l(key), "data-live-format": format }, text);
    button.addEventListener("pointerdown", event => event.preventDefault());
    button.addEventListener("click", () => {
      if (composing) return;
      const direction = editor.selectionDirection;
      const scroll = editor.scrollTop;
      const changed = formatSelection(editor.value, editor.selectionStart, editor.selectionEnd, format);
      editor.value = changed.value;
      notes.edit(changed.value);
      showPreview(false);
      editor.focus({ preventScroll: true });
      editor.setSelectionRange(changed.start, changed.end, direction);
      editor.scrollTop = scroll;
      scheduleSave();
    });
    formatting.append(button);
  }
  const noteHeading = h("div", { class: "live-note-controls" }, h("div", { class: "live-preview-toggle" }, sourceToggle, previewToggle), formatting);
  const peopleControls = peopleNoteControls(peopleHost());
  formatting.append(peopleControls);
  panels.get("notes")!.append(noteHeading, editor, notePreview, h("footer", { class: "live-save-controls" }, saveStatus, save), saveError);
  const follow = controlButton({ class: "btn compact live-follow", hidden: true, onclick: resumeFollowing }, l("meetings.workspace_follow"));
  const transcriptOutput = h("div", { class: "live-transcript", tabindex: 0, "aria-label": l("meetings.workspace_transcript") });
  const transcriptEmpty = h("p", { class: "muted" }, l("meetings.empty_transcript"));
  const attributionNotice = h("p", { class: "help live-attribution-notice", hidden: true });
  const speakerToggle = h("input", { type: "checkbox", role: "switch", "data-live-speaker-toggle": "true", checked: false, disabled: true });
  const speakerState = h("p", { class: "help live-speaker-state", role: "status" });
  const speakerDestination = h("summary", { class: "help live-speaker-destination" });
  const speakerModels = h("p", { class: "help live-speaker-models" });
  const speakerSetup = h("button", { class: "text-button", hidden: true, onclick: () => ctx.navigate("meeting-settings") }, l("meetings.live_speakers_setup"));
  const speakerControls = h("section", { class: "live-speaker-controls", tabindex: 0, "aria-label": l("meetings.live_speakers_title") },
    h("label", { class: "checkbox" }, speakerToggle, l("meetings.live_speakers_title")), speakerState,
    h("details", { class: "live-speaker-details" }, speakerDestination, speakerModels, h("p", { class: "help" }, l("meetings.live_speakers_help")),
      h("p", { class: "help" }, l("meetings.workspace_cadence")), speakerSetup));
  speakerToggle.addEventListener("change", () => { void changeLiveSpeakers(speakerToggle.checked); });
  transcriptOutput.append(transcriptEmpty);
  const captionOutput = h("div", { class: "live-captions", hidden: true, role: "region", tabindex: 0, "aria-label": l("meetings.captions_title") });
  const captionRows = new Map<Speaker, { element: HTMLElement; state: HTMLElement; body: HTMLElement }>();
  for (const source of ["me", "others"] as const) {
    const state = h("span", { class: "live-caption-state" });
    const body = h("div", { class: "live-caption-copy", "data-preview-source": source });
    const element = h("div", { class: "live-caption", "data-caption-source": source },
      h("div", { class: "live-caption-heading" }, h("span", {}, l(source === "me" ? "meetings.captions_microphone" : "meetings.captions_system")),
        h("span", { class: "live-caption-interim" }, l("meetings.captions_interim"))), state, body);
    captionRows.set(source, { element, state, body });
    captionOutput.append(element);
  }
  panels.get("transcript")!.append(h("div", { class: "live-transcript-heading" }, follow), speakerControls, attributionNotice, transcriptOutput, captionOutput);
  const questions = conversationPanel(ctx, {
    scope: { kind: "meeting", meeting_uid: uid, meeting_id: initial.id! }, live: true,
    canAsk: () => !composing,
    prepare: async () => {
      if (composing) throw new Error(t("meetings.workspace_composing"));
      const target = notes.value, externalSession = finalized ? undefined : session;
      clearTimeout(notesTimer);
      await writes.run(() => notes.persist(writeNotes, target));
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(target));
      const expected_notes_revision = [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
      return { expected_notes_revision, ...externalSession ? { session_id: externalSession } : {} };
    },
    source: async (source, origin, _message, restore) => {
      await openSource(origin, source.status === "current" ? source.anchor ?? undefined : undefined,
        source.anchor?.source_text ?? source.passage.text, restore);
      return true;
    },
  });
  const history = questions.history;
  panels.get("questions")!.append(h("details", { class: "conversation-context-help" }, h("summary", {}, l("conversations.context")),
    h("p", { class: "help live-question-scope" }, l("meetings.workspace_scope")),
    h("p", { class: "help live-source-assessment" }, l("meetings.workspace_sourcecheck_unavailable"))), questions.root);
  panels.get("suggestions")!.append(h("h2", {}, l("meetings.workspace_suggestions")), h("p", { class: "help" }, l("meetings.workspace_suggestions_unavailable")));
  const separator = h("div", { class: "live-pane-separator", role: "separator", tabindex: 0, "aria-orientation": "vertical", "aria-label": l("meetings.workspace_split"), "aria-valuemin": 35, "aria-valuemax": 75 });
  const content = h("div", { class: "live-workspace-panes" }, panels.get("notes")!, separator, panels.get("transcript")!, panels.get("questions")!, panels.get("suggestions")!);
  root.append(band, warning, recovery, sourceBack, sourceNotice, tabList, content);
  let resizeFrame = 0;
  const resize = new ResizeObserver(entries => {
    const width = entries[0]?.contentRect.width ?? 0;
    cancelAnimationFrame(resizeFrame);
    resizeFrame = requestAnimationFrame(() => { panes.resize(width); updatePanes(); });
  });
  resize.observe(content);
  separator.addEventListener("keydown", event => {
    const next = event.key === "ArrowLeft" ? panes.ratio - 5 : event.key === "ArrowRight" ? panes.ratio + 5 : event.key === "Home" ? 35 : event.key === "End" ? 75 : null;
    if (next !== null) { event.preventDefault(); panes.ratio = Math.max(35, Math.min(75, next)); updatePanes(); }
  });
  separator.addEventListener("pointerdown", event => {
    separator.setPointerCapture(event.pointerId);
    const move = (event: PointerEvent) => { const bounds = content.getBoundingClientRect(); panes.ratio = Math.max(35, Math.min(75, (event.clientX - bounds.left) / bounds.width * 100)); updatePanes(); };
    const end = () => { separator.removeEventListener("pointermove", move); separator.removeEventListener("pointerup", end); separator.removeEventListener("pointercancel", end); };
    separator.addEventListener("pointermove", move);
    separator.addEventListener("pointerup", end);
    separator.addEventListener("pointercancel", end);
  });

  notes.sync(session, initial.notes);
  titleDraft.sync(session, initial.title ?? t("meetings.default_title"));
  editor.value = notes.value;
  title.value = titleDraft.value;
  notes.subscribe(updateSaving);
  titleDraft.subscribe(updateSaving);
  editor.addEventListener("compositionstart", () => { composing = true; ++inputRevision; clearTimeout(notesTimer); updateReadiness(); });
  editor.addEventListener("compositionend", () => { composing = false; ++inputRevision; notes.edit(editor.value); scheduleSave(); updateReadiness(); });
  editor.addEventListener("input", () => { ++inputRevision; if (insertionReceipt && insertionReceipt.text !== editor.value) insertionReceipt = null; notes.edit(editor.value); if (!composing) scheduleSave(); });
  editor.addEventListener("select", () => { const selection = `${editor.selectionStart}:${editor.selectionEnd}`; if (selection !== observedSelection) { observedSelection = selection; ++selectionRevision; } });
  editor.addEventListener("blur", () => { if (!composing) void saveNotes().catch(showSaveError); });
  title.addEventListener("input", () => { titleDraft.edit(title.value); clearTimeout(titleTimer); titleTimer = window.setTimeout(() => { void saveTitle().catch(showSaveError); }, 350); });
  title.addEventListener("blur", () => { void saveTitle().catch(showSaveError); });
  transcriptOutput.addEventListener("scroll", () => {
    const position = transcriptOutput.scrollTop;
    if (intendedScroll !== null && Math.abs(position - intendedScroll) < 2) intendedScroll = null;
    else if (position < previousScroll - 1) pauseFollowing();
    previousScroll = position;
  });
  document.addEventListener("selectionchange", () => {
    if (transcriptSelection().length) pauseFollowing();
    flushPendingTranscript();
  });
  bindLocalization(root, () => { updateStatus(status); updateSaving(); });

  const owner: LiveWorkspace = {
    root, id: initial.id!, uid, session, aliases: new Set([initial.id!]),
    bind(next) {
      ctx = next;
      displayedSession = session;
      const mine = ++binding;
      statusEpoch.invalidate();
      ++sourceNavigation;
      poll?.dispose();
      poll = new VisibilityPoll(500, pollStatus, error => { warning.textContent = errorText(error); warning.hidden = false; });
      poll.setVisible(ctx.visible);
      next.onVisibilityChange(visible => { if (mine === binding) poll?.setVisible(visible); });
      next.onDispose(() => { if (mine === binding) { ++sourceNavigation; statusEpoch.invalidate(); poll?.dispose(); poll = null; } });
      next.onInfoChange(() => { if (mine === binding) updateReadiness(); });
      next.onFlush(flush);
      questions.bind(next);
      if (finalized && insights) insights.bind(ctx, insightHost());
      const peopleId = owner.id;
      void api.meetingPeople(peopleId).then(snapshot => {
        if (mine === binding && owner.id === peopleId) { peopleSnapshot = snapshot; updatePeopleLinks(); }
      }).catch(() => undefined);
      updateReadiness();
      if (finalized) void loadReview().catch(showSaveError);
      requestAnimationFrame(() => { if (mine === binding) { panes.resize(content.getBoundingClientRect().width); updatePanes(); } });
      return root;
    },
    flush,
    finish(event) {
      if (event.session_id !== session) return;
      statusEpoch.invalidate();
      if (event.id) { owner.id = event.id; owner.aliases.add(event.id); }
      finalized = !!event.id;
      updateStatus({ ...status, id: owner.id, busy: false, recording: false, phase: event.error && !event.id ? "error" : "idle", error: event.error, persistence_error: event.error, pending: 0 });
      if (event.flush_warning) { saveError.textContent = localizeBackendText(event.flush_warning); saveError.hidden = false; }
      if (event.id) void loadReview().catch(showSaveError);
    },
    savedExternally(text) {
      if (notes.dirty || composing || notes.status === "saving") return;
      const start = editor.selectionStart, end = editor.selectionEnd, direction = editor.selectionDirection, scroll = editor.scrollTop;
      notes.sync(session, text);
      if (editor.value !== text) { editor.value = text; editor.setSelectionRange(start, end, direction); editor.scrollTop = scroll; }
    },
  };

  function updatePanes() {
    root.classList.toggle("live-wide", panes.wide);
    root.classList.toggle("live-enlarged", parseFloat(getComputedStyle(tabList).fontSize) >= 20);
    content.style.setProperty("--live-notes-ratio", String(panes.ratio / 100));
    root.style.setProperty("--live-notes-ratio", String(panes.ratio / 100));
    separator.hidden = !panes.wide;
    separator.setAttribute("aria-valuenow", String(Math.round(panes.ratio)));
    for (const [pane, panel] of panels) {
      panel.hidden = !panes.shown(pane);
      const button = tabButtons.get(pane)!;
      button.hidden = panes.wide && pane === "notes";
      const selected = panes.wide ? pane === panes.companion : pane === panes.selected;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
  }
  function selectPane(pane: LivePane, openingSource = false) { if (!openingSource) ++sourceNavigation; panes.select(pane); updatePanes(); }
  function showPreview(value: boolean) {
    if (composing) return;
    preview = value;
    if (value && previewValue !== notes.value) { notePreview.replaceChildren(markdown(notes.value)); previewValue = notes.value; }
    editor.hidden = value;
    notePreview.hidden = !value;
    formatting.hidden = value;
    sourceToggle.setAttribute("aria-pressed", String(!value));
    previewToggle.setAttribute("aria-pressed", String(value));
  }
  function scheduleSave() { clearTimeout(notesTimer); notesTimer = window.setTimeout(() => { void saveNotes().catch(showSaveError); }, 600); }
  async function writeNotes(value: string, capturedReceipt = insertionReceipt?.text === value ? insertionReceipt.attempt : undefined) {
    if (!finalized && ["recording", "stopping", "error"].includes(status.phase)) await api.setMeetingNotes(value, session);
    else {
      const receipt = capturedReceipt;
      await api.saveMeetingNotes(owner.id, value, receipt);
      if (receipt && insertionReceipt?.attempt === receipt) insertionReceipt = null;
    }
  }
  function saveNotes(target = notes.value) {
    clearTimeout(notesTimer);
    if (composing) return Promise.reject(new Error(t("meetings.workspace_composing")));
    const receipt = insertionReceipt?.text === target ? insertionReceipt.attempt : undefined;
    return writes.run(() => notes.persist(value => writeNotes(value, receipt), target));
  }
  function saveTitle() {
    clearTimeout(titleTimer);
    if (!titleDraft.dirty) return Promise.resolve();
    return titleWrites.run(() => titleDraft.persist(value => finalized ? writes.run(async () => {
      const previousId = owner.id;
      owner.id = await api.renameMeeting(owner.id, value);
      owner.aliases.add(owner.id);
      status = { ...status, id: owner.id, title: value };
      ctx.meetingRenamed?.(previousId, owner.id);
    }) : api.setMeetingTitle(value, session)));
  }
  async function flush() {
    clearTimeout(notesTimer); clearTimeout(titleTimer);
    await persistAll([() => saveTitle(), () => saveNotes(), questions.flush]);
  }
  function showSaveError(error: unknown) { saveError.textContent = errorText(error); saveError.hidden = false; updateSaving(); }
  function updateSaving() {
    const state = notes.status === "saving" || titleDraft.status === "saving" ? "saving" : notes.dirty || titleDraft.dirty ? "unsaved" : "saved";
    saveStatus.dataset.saveState = state;
    saveStatus.textContent = t(state === "saving" ? "meetings.workspace_saving" : state === "unsaved" ? "meetings.workspace_unsaved" : "meetings.workspace_saved");
    save.disabled = state === "saving" || !notes.dirty && !titleDraft.dirty || composing;
    const failure = notes.error ?? titleDraft.error;
    if (failure) { saveError.textContent = localizeBackendText(failure); saveError.hidden = false; }
    else if (!notes.dirty && !titleDraft.dirty) saveError.hidden = true;
  }
  function updateReadiness() {
    questions.readiness();
    updateSaving();
  }
  function updateStatus(next: MeetingStatus) {
    status = next;
    if (next.id) { owner.id = next.id; owner.aliases.add(next.id); }
    clock.textContent = formatClock(next.elapsed_secs);
    phase.textContent = t(next.recording ? "meetings.workspace_recording" : next.busy ? "meetings.workspace_processing" : "meetings.workspace_review");
    root.dataset.recording = String(next.recording);
    stop.disabled = !next.recording || next.phase !== "recording";
    stop.hidden = !next.busy && finalized;
    review.hidden = !finalized;
    title.disabled = !finalized && !["recording", "stopping"].includes(next.phase);
    const messages = [next.transcription_warning, next.audio_warning, next.error].filter((value): value is string => !!value);
    warning.textContent = messages.map(localizeBackendText).join(" ");
    warning.hidden = !warning.textContent;
    recovery.hidden = !next.can_retry_persistence;
    if (next.can_retry_persistence && !recovery.childElementCount) recovery.append(h("p", { class: "error" }, localizeBackendText(next.persistence_error ?? next.terminal_error ?? "")), controlButton({ class: "btn", onclick: async () => { try { await flush(); await api.retryMeetingPersistence(session); } catch (error) { showSaveError(error); } } }, l("common.retry")));
    if (preview && !composing && previewValue !== notes.value) { const position = notePreview.scrollTop; notePreview.replaceChildren(markdown(notes.value)); notePreview.scrollTop = position; previewValue = notes.value; }
    updateTranscript(next.transcript);
    const metadataNotice = attributionNoticeKey(next.metadata_status);
    attributionNotice.textContent = metadataNotice ? t(metadataNotice) : "";
    attributionNotice.hidden = !metadataNotice;
    updateCaptions(next);
    if (!next.recording && next.phase !== "starting") liveSpeakers.stop();
    else liveSpeakers.observe(session, next.live_speaker_attribution);
    updateLiveSpeakers();
    if (finalized && (!insights || insights.model.id !== owner.id)) {
      insights = reviewedInsights(ctx, insightHost()); panels.get("suggestions")!.replaceChildren(insights.root);
    }
    if (sourceReturn?.label) sourceBack.textContent = t(sourceReturn.label);
  }
  function updateLiveSpeakers() {
    const value = liveSpeakers.value;
    speakerToggle.checked = liveSpeakers.checked;
    speakerToggle.disabled = !liveSpeakers.canChange(status.recording);
    speakerControls.dataset.liveSpeakerState = !status.recording && status.phase !== "starting" ? "stopped" : value?.state ?? "unavailable";
    speakerControls.setAttribute("aria-busy", String(liveSpeakers.pending));
    const stateKey = liveSpeakers.pending ? "meetings.live_speakers_updating" : !status.recording && status.phase !== "starting"
      ? "meetings.live_speakers_stopped" : liveSpeakerStatusKey(value);
    const stateText = liveSpeakers.error ?? t(stateKey);
    if (speakerState.textContent !== stateText) speakerState.textContent = stateText;
    speakerState.hidden = !liveSpeakers.pending && !liveSpeakers.error && !!value?.available && ["off", "running"].includes(value.state);
    speakerState.classList.toggle("error", !!liveSpeakers.error || value?.state === "failed");
    const destination = t(value?.destination === "local" ? "meetings.live_speakers_destination_local_short" : "meetings.live_speakers_destination_unavailable");
    const readiness = t(value?.available ? "meetings.live_speakers_models_ready_short" : value?.reason_code === "models_missing"
      ? "meetings.live_speakers_models_missing_short" : "meetings.live_speakers_models_unknown");
    const destinationText = `${destination} · ${readiness}`;
    if (speakerDestination.textContent !== destinationText) speakerDestination.textContent = destinationText;
    const modelText = value?.model ? t(value.available ? "meetings.live_speakers_models_ready" : "meetings.live_speakers_models_selected", { model: value.model }) : t("meetings.live_speakers_models_unknown");
    if (speakerModels.textContent !== modelText) speakerModels.textContent = modelText;
    speakerSetup.hidden = !["models_missing", "local_required", "configuration_changed"].includes(value?.reason_code ?? "");
  }
  async function changeLiveSpeakers(enabled: boolean) {
    const request = liveSpeakers.begin(status.recording);
    updateLiveSpeakers();
    if (request === null) return;
    try { liveSpeakers.complete(request, await api.setLiveSpeakerAttribution(session, enabled)); }
    catch (error) { liveSpeakers.fail(request, errorText(error)); }
    updateLiveSpeakers();
  }
  function currentNotes(): InsightNoteLatch {
    const selection = `${editor.selectionStart}:${editor.selectionEnd}`;
    if (selection !== observedSelection) { observedSelection = selection; ++selectionRevision; }
    return { text: notes.value, revision: inputRevision, selection_start: editor.selectionStart, selection_end: editor.selectionEnd,
      selection_revision: selectionRevision, interaction_revision: interactionRevision, composing };
  }
  function peopleHost(): PeopleHost {
    return { id: () => owner.id, sessionId: () => finalized ? undefined : session, platform: ctx.info.platform,
      valid: () => root.isConnected, report: showSaveError,
      changed: snapshot => { peopleSnapshot = snapshot; updatePeopleLinks(); },
      notes: { current: currentNotes, flush: async () => { await saveNotes(); await writes.drain(); }, saved: (result, before) => {
        if (result.notes === undefined) return;
        if (!canAcknowledgePeopleNotes(before, currentNotes(), result.notes, session, notes.sessionId)) { showSaveError(new Error(t("people.selection_changed"))); return; }
        ++inputRevision; notes.sync(session, result.notes); editor.value = result.notes; showPreview(false);
        editor.setSelectionRange(result.selection_start ?? before.selection_start, result.selection_end ?? before.selection_end);
        updateSaving();
      } } };
  }
  function updatePeopleLinks() {
    for (const row of rows.values()) {
      const linked = row.element.querySelector<HTMLElement>(".people-link-state");
      if (!linked) continue;
      const link = peopleSnapshot?.speaker_links.find(link => !!row.segment.utterance_id && link.utterance_ids.includes(row.segment.utterance_id));
      linked.hidden = !link; linked.textContent = link ? t(link.state === "needs_review" ? "people.review" : "people.linked") : "";
    }
  }
  function insightHost(): InsightReviewHost {
    return { id: owner.id, meetingUid: uid, notes: {
      flush: async () => { try { await saveNotes(); return true; } catch { return false; } }, current: currentNotes,
      stage: async (value, attempt, before) => {
        if (!finalized || !insertionStillCurrent(before, currentNotes())) return false;
        insertionReceipt = { text: value, attempt }; ++inputRevision; notes.edit(value); editor.value = value;
        selectPane("notes"); showPreview(false); editor.focus({ preventScroll: true }); editor.setSelectionRange(before.text.length, value.length);
        try { await saveNotes(value); return true; } catch (error) { showSaveError(error); return false; }
      },
    }, openSource: openInsightSource };
  }
  async function openInsightSource(source: InsightSource, origin: HTMLElement, restore: () => void): Promise<boolean> {
    const navigation = ++sourceNavigation;
    const detail = await api.meetingDetail(owner.id);
    if (navigation !== sourceNavigation || !finalized) return true;
    const segment = detail.transcript.find(segment => segment.utterance_id === source.utterance_id);
    if (!insightSourceMatches(source, segment, detail.meeting_uid ?? uid)) return false;
    updateStatus({ ...status, transcript: detail.transcript, metadata_status: detail.metadata_status });
    const anchor: LiveSourceAnchor = { passage: { meeting_id: owner.id, meeting_uid: uid, title: status.title ?? "", start_iso: "", kind: "transcript",
      start_ms: source.start_ms, text: source.quote, snippet: source.quote }, segment_keys: [`utterance:${source.utterance_id}`],
      segment_fingerprints: [source.source_fingerprint], notes_start: null, notes_end: null, source_text: source.quote };
    const opening = openSource(origin, anchor, source.quote, restore, "insights.back");
    const sourceEpoch = sourceNavigation; await opening;
    if (sourceNavigation !== sourceEpoch) return true;
    const row = rows.get(`utterance:${source.utterance_id}`);
    const range = segment && utf8Range(segment.text, source.start_byte, source.end_byte);
    if (row?.body.firstChild instanceof Text && range) {
      const selected = document.createRange(); selected.setStart(row.body.firstChild, range.start); selected.setEnd(row.body.firstChild, range.end);
      window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(selected);
    }
    return true;
  }
  function updateCaptions(next: MeetingStatus) {
    const snapshot = next.preview;
    captionOutput.hidden = !next.recording || !snapshot || snapshot.transcript_revision !== next.transcript_revision;
    const reasons: Record<MeetingPreviewState, TranslationKey> = {
      disabled: "meetings.captions_disabled", warming: "meetings.captions_warming", ready: "meetings.captions_listening",
      busy: "meetings.captions_busy", failed: "meetings.captions_failed", unsupported: "meetings.captions_unsupported", stopped: "meetings.captions_stopped",
    };
    for (const [source, row] of captionRows) {
      const track = snapshot?.tracks.find(track => track.source === source);
      row.element.hidden = !track || source === "others" && !next.system_audio;
      const text = !captionOutput.hidden && track && !["stopped", "disabled", "unsupported", "failed"].includes(track.state) ? track.text : "";
      if (row.body.textContent !== text) row.body.textContent = text;
      const reason = track && (track.state !== "ready" || !text) ? t(reasons[track.state]) : "";
      if (row.state.textContent !== reason) row.state.textContent = reason;
      row.state.hidden = !reason;
      row.element.dataset.previewState = track?.state ?? "stopped";
    }
  }
  function pauseFollowing() { following = false; follow.hidden = false; }
  function transcriptSelection(): Range[] {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed) return [];
    return Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index))
      .filter(range => !range.collapsed && range.intersectsNode(transcriptOutput));
  }
  function clearTranscriptSelection() {
    if (transcriptSelection().length) window.getSelection()?.removeAllRanges();
  }
  function resumeFollowing() {
    clearTranscriptSelection();
    following = true; follow.hidden = true;
    flushPendingTranscript();
    scrollLive();
  }
  function flushPendingTranscript() {
    if (pendingTranscript) updateTranscript(pendingTranscript);
  }
  function changesSelectedTranscript(segments: Segment[], keys: string[]): boolean {
    const selected = transcriptSelection();
    if (!selected.length) return false;
    const next = new Map(keys.map((key, index) => [key, segments[index]]));
    for (const [key, row] of rows) {
      if (!selected.some(range => range.intersectsNode(row.element))) continue;
      const segment = next.get(key);
      if (!segment) return true;
      if (selected.some(range => range.intersectsNode(row.body))
        && row.body.textContent !== (segment.failed ? t("meetings.transcription_failed") : segment.text)) return true;
      if (selected.some(range => range.intersectsNode(row.speaker)) && row.speaker.textContent !== String(speakerUiLabel(segment))) return true;
      if (selected.some(range => range.intersectsNode(row.provisional))
        && row.provisional.hidden !== (segment.attribution?.provisional !== true || !!segment.confirmed_speaker_name)) return true;
      if (selected.some(range => range.intersectsNode(row.correction)) && row.correction.hidden !== !segment.correction_review_required) return true;
    }
    const retained = new Set(keys);
    const currentOrder = [...transcriptOutput.querySelectorAll<HTMLElement>(".transcript-line")]
      .map(row => row.dataset.segmentKey!).filter(key => retained.has(key));
    const nextOrder = keys.filter(key => rows.has(key));
    return currentOrder.some((key, index) => nextOrder[index] !== key);
  }
  function scrollLive() { intendedScroll = Math.max(0, transcriptOutput.scrollHeight - transcriptOutput.clientHeight); transcriptOutput.scrollTop = intendedScroll; previousScroll = transcriptOutput.scrollTop; }
  function updateTranscript(segments: Segment[]) {
    const keys = segmentKeys(segments);
    // Retain the user's actual text nodes rather than guessing a Range across new speaker turns.
    if (changesSelectedTranscript(segments, keys)) { pendingTranscript = segments; return; }
    pendingTranscript = null;
    const viewport = transcriptOutput.getBoundingClientRect();
    const first = [...rows.values()].find(row => row.element.getBoundingClientRect().bottom >= viewport.top);
    const offset = first ? first.element.getBoundingClientRect().top - viewport.top : 0;
    const oldPosition = transcriptOutput.scrollTop;
    const retained = new Set(keys);
    for (const [key, row] of rows) if (!retained.has(key)) { row.element.remove(); rows.delete(key); }
    transcriptEmpty.hidden = segments.length > 0;
    let before = transcriptOutput.firstElementChild === transcriptEmpty ? transcriptEmpty.nextElementSibling : transcriptOutput.firstElementChild;
    for (const [index, segment] of segments.entries()) {
      const key = keys[index];
      let row = rows.get(key);
      if (!row) {
        const body = h("div", { class: "transcript-copy" }, segment.failed ? l("meetings.transcription_failed") : segment.text);
        const speaker = h("button", { class: "speaker people-speaker-action", type: "button", onclick: () => {
          const current = rows.get(key)?.segment;
          if (!current?.utterance_id) { showSaveError(new Error(t("people.no_stable_speaker"))); return; }
          const ids = speakerLinkUtteranceIds(current, status.transcript);
          if (!ids.length) { showSaveError(new Error(t("people.no_stable_speaker"))); return; }
          void openPeoplePicker(peopleHost(), { kind: "speaker", utteranceIds: ids }).catch(showSaveError);
        } }, speakerUiLabel(segment));
        const provisional = h("span", { class: "live-speaker-provisional", hidden: true, title: l("meetings.live_speakers_provisional_help") }, l("meetings.live_speakers_provisional_tag"));
        const correction = h("p", { class: "help transcript-correction-review", hidden: !segment.correction_review_required }, l("meetings.attribution_correction_review"));
        const linked = h("span", { class: "people-link-state", hidden: true });
        const element = h("div", { class: "transcript-line", "data-segment-key": key, "data-start-ms": segment.start_ms, "data-source-id": segment.source_id ?? "" }, h("span", { class: "transcript-stamp" }, formatClock(segment.start_ms / 1000)), h("div", { class: "live-transcript-speaker" }, speaker, provisional, linked), body, correction);
        row = { element, body, speaker, provisional, correction, segment };
        rows.set(key, row);
      } else if (!sameSegment(row.segment, segment)) {
        if (row.segment.text !== segment.text || row.segment.failed !== segment.failed) row.body.textContent = segment.failed ? t("meetings.transcription_failed") : segment.text;
        row.segment = segment;
      }
      const label = String(speakerUiLabel(segment));
      if (row.speaker.textContent !== label) row.speaker.textContent = label;
      const metadataReason = !attributionEditable(status.metadata_status) ? attributionNoticeKey(status.metadata_status) : null;
      row.speaker.title = metadataReason ? t(metadataReason) : label;
      (row.speaker as HTMLButtonElement).disabled = !attributionEditable(status.metadata_status) || !segment.utterance_id || !!segment.failed || !!segment.attribution && segment.attribution.kind !== "single";
      row.speaker.setAttribute("aria-label", t("meetings.voice_name_label", { name: label }));
      row.provisional.hidden = segment.attribution?.provisional !== true || !!segment.confirmed_speaker_name;
      row.correction.hidden = !segment.correction_review_required;
      row.element.dataset.sourceId = segment.source_id ?? "";
      row.element.title = segment.source_id ? t(`meetings.attribution_source_${segment.source_id}`) : "";
      if (before !== row.element) transcriptOutput.insertBefore(row.element, before);
      before = row.element.nextElementSibling;
    }
    updatePeopleLinks();
    if (following) scrollLive();
    else {
      const delta = first?.element.parentElement === transcriptOutput ? first.element.getBoundingClientRect().top - viewport.top - offset : null;
      // Layout may already have moved scrollTop through native scroll anchoring.
      const position = delta === null ? oldPosition : transcriptOutput.scrollTop + delta;
      intendedScroll = position;
      transcriptOutput.scrollTop = position;
    }
  }
  async function pollStatus() {
    if (finalized) return;
    const epoch = statusEpoch.capture();
    const runtime = await api.meetingRuntimeStatus();
    if (!statusEpoch.accepts(epoch) || finalized) return;
    if (runtime.session_id !== session) {
      if (!runtime.busy && owner.id) await loadReview();
      return;
    }
    const next = await refresh.refresh(runtime, () => api.meetingStatus());
    if (!statusEpoch.accepts(epoch) || finalized) return;
    if (next) updateStatus(next);
  }
  async function loadReview() {
    const epoch = statusEpoch.capture();
    const id = owner.id;
    const detail = await api.meetingDetail(id);
    if (!statusEpoch.accepts(epoch) || owner.id !== id) return;
    finalized = true;
    updateStatus({ ...status, id: owner.id, busy: false, recording: false, phase: "idle", pending: 0, notes: detail.notes, transcript: detail.transcript,
      metadata_status: detail.metadata_status, title: detail.info?.title ?? status.title });
  }
  async function stopCapture() {
    statusEpoch.invalidate();
    const epoch = statusEpoch.capture();
    stop.disabled = true;
    try {
      await api.stopMeeting(session);
      if (!statusEpoch.accepts(epoch) || finalized) return;
      statusEpoch.invalidate();
      updateStatus({ ...status, recording: false, phase: "stopping", busy: true });
    } catch (error) { if (statusEpoch.accepts(epoch) && !finalized) { warning.textContent = errorText(error); warning.hidden = false; stop.disabled = status.phase !== "recording"; } }
  }
  async function anchorMatches(anchor: LiveSourceAnchor) {
    if (anchor.passage.kind === "notes") return anchor.notes_start !== null && anchor.notes_end !== null && editor.value.slice(anchor.notes_start, anchor.notes_end) === anchor.source_text;
    const keys = segmentKeys(status.transcript);
    for (const [index, key] of anchor.segment_keys.entries()) {
      const at = keys.indexOf(key);
      if (at < 0 || !anchor.segment_fingerprints[index] || await fingerprint(status.transcript[at]) !== anchor.segment_fingerprints[index]) return false;
    }
    return anchor.segment_keys.length > 0;
  }
  async function openSource(turn: HTMLElement, anchor?: LiveSourceAnchor, frozenText?: string, restore?: () => void, label: TranslationKey = "meetings.workspace_back_answer") {
    const navigation = ++sourceNavigation;
    sourceReturn = { pane: panes.selected, questionScroll: history.scrollTop, transcriptScroll: transcriptOutput.scrollTop, notesScroll: editor.scrollTop, start: editor.selectionStart, end: editor.selectionEnd, direction: editor.selectionDirection, turn, restore, label };
    sourceBack.textContent = t(label);
    sourceBack.hidden = false;
    sourceNotice.hidden = true;
    root.querySelectorAll(".live-source-highlight").forEach(node => node.classList.remove("live-source-highlight"));
    const originalSource = anchor ? currentSourceStamp(anchor) : "";
    const matches = !!anchor && await anchorMatches(anchor);
    if (navigation !== sourceNavigation) return;
    if (!anchor || !matches || originalSource !== currentSourceStamp(anchor)) {
      sourceNotice.replaceChildren(h("p", { class: "help" }, l("meetings.workspace_source_changed")), h("blockquote", {}, anchor?.source_text ?? frozenText ?? ""));
      history.append(sourceNotice);
      sourceNotice.hidden = false;
      sourceNotice.scrollIntoView({ block: "nearest" });
      return;
    }
    if (anchor.passage.kind === "notes") {
      selectPane("notes", true); showPreview(false); editor.focus({ preventScroll: true }); editor.setSelectionRange(anchor.notes_start!, anchor.notes_end!);
    } else {
      if (pendingTranscript) { clearTranscriptSelection(); flushPendingTranscript(); }
      pauseFollowing(); selectPane("transcript", true);
      const row = rows.get(anchor.segment_keys[0]);
      row?.element.classList.add("live-source-highlight"); row?.element.scrollIntoView({ block: "center", behavior: "instant" });
    }
  }
  function currentSourceStamp(anchor: LiveSourceAnchor) {
    if (anchor.passage.kind === "notes") return JSON.stringify([anchor.notes_start, anchor.notes_end, editor.value.slice(anchor.notes_start ?? 0, anchor.notes_end ?? 0)]);
    const keys = segmentKeys(status.transcript);
    return JSON.stringify(anchor.segment_keys.map(key => {
      const segment = status.transcript[keys.indexOf(key)];
      return segment ? segmentEvidenceValue(segment) : null;
    }));
  }
  function returnToAnswer() {
    ++sourceNavigation;
    if (!sourceReturn) return;
    const retained = sourceReturn; sourceReturn = null;
    selectPane(retained.pane);
    history.scrollTop = retained.questionScroll;
    transcriptOutput.scrollTop = retained.transcriptScroll;
    editor.setSelectionRange(retained.start, retained.end, retained.direction);
    editor.scrollTop = retained.notesScroll;
    sourceBack.hidden = true; sourceNotice.hidden = true;
    root.querySelectorAll(".live-source-highlight").forEach(node => node.classList.remove("live-source-highlight"));
    retained.turn.setAttribute("tabindex", "-1"); retained.turn.focus({ preventScroll: true });
    if (retained.restore) requestAnimationFrame(retained.restore);
  }
  updateStatus(initial);
  updateSaving();
  updatePanes();
  return owner;
}
