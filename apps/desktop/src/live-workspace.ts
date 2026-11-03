import { api, listenToWindow, type MeetingStatus, type MeetingFinished, type MeetingPreviewState, type Speaker, type LiveQuestionSnapshot, type LiveQuestionResult, type LiveSourceAnchor, type MeetingAnswer, type Segment } from "./api";
import { SerialQueue, persistAll } from "./async";
import { bindLocalization, l, t, localizeBackendText, type TranslationKey } from "./i18n";
import { controlButton, errorText, formatClock, h, markdown, textField } from "./dom";
import { MeetingDraft } from "./meeting-draft";
import { RecordingRefresh, sameSegment, segmentKeys } from "./recording-refresh";
import { LivePaneState, LiveRequestState, LiveStatusEpoch, segmentEvidenceValue, formatSelection, type LivePane } from "./live-workspace-state";
import { languageModelConfigured } from "./presentation";
import { speakerUiLabel } from "./meeting-labels";
import { VisibilityPoll } from "./window-lifecycle";
import { attributionNoticeKey } from "./meeting-source";
import type { Context } from "./main";

interface Turn {
  question: string;
  requestId: string;
  snapshot?: LiveQuestionSnapshot;
  answer?: MeetingAnswer;
  error?: string;
  cancelled?: boolean;
  element: HTMLElement;
  response: HTMLElement;
  metadata: HTMLElement;
}

interface LiveWorkspace {
  root: HTMLElement;
  id: string;
  uid: string;
  session: string;
  aliases: Set<string>;
  bind(ctx: Context): HTMLElement;
  flush(): Promise<void>;
  finish(event: MeetingFinished): void;
  receive(result: LiveQuestionResult): void;
  savedExternally(text: string): void;
}

const workspaces = new Map<string, LiveWorkspace>();
let displayedSession: string | null = null;
let questionEvents: Promise<() => void> | null = null;

function connectQuestions() {
  questionEvents ??= listenToWindow<LiveQuestionResult>("live-question-result", ({ payload }) => {
    const owner = workspaces.get(payload.snapshot.session_id);
    if (owner?.uid === payload.snapshot.meeting_uid) owner.receive(payload);
  });
  return questionEvents;
}

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
  let questionComposing = false;
  let notesTimer: number | undefined;
  let titleTimer: number | undefined;
  let preview = false;
  let previewValue = "";
  let sourceNavigation = 0;
  let following = true;
  let previousScroll = 0;
  let intendedScroll: number | null = null;
  let sourceReturn: { pane: LivePane; questionScroll: number; transcriptScroll: number; notesScroll: number; start: number; end: number; direction: "forward" | "backward" | "none"; turn: HTMLElement } | null = null;
  const panes = new LivePaneState();
  const requests = new LiveRequestState(uid, session);
  const statusEpoch = new LiveStatusEpoch();
  const notes = new MeetingDraft();
  const titleDraft = new MeetingDraft();
  const writes = new SerialQueue();
  const titleWrites = new SerialQueue();
  const refresh = new RecordingRefresh(initial);
  const turns: Turn[] = [];
  const rows = new Map<string, { element: HTMLElement; body: HTMLElement; speaker: HTMLElement; correction: HTMLElement; segment: Segment }>();
  const root = h("article", { class: "live-workspace live-recording", "data-session-id": session, "data-meeting-uid": uid });
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
  panels.get("notes")!.append(noteHeading, editor, notePreview, h("footer", { class: "live-save-controls" }, saveStatus, save), saveError);
  const follow = controlButton({ class: "btn compact live-follow", hidden: true, onclick: () => { following = true; follow.hidden = true; scrollLive(); } }, l("meetings.workspace_follow"));
  const transcriptOutput = h("div", { class: "live-transcript", tabindex: 0, "aria-label": l("meetings.workspace_transcript") });
  const transcriptEmpty = h("p", { class: "muted" }, l("meetings.empty_transcript"));
  const attributionNotice = h("p", { class: "help live-attribution-notice", hidden: true });
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
  panels.get("transcript")!.append(h("div", { class: "live-transcript-heading" }, h("p", { class: "help" }, l("meetings.workspace_cadence")), follow), attributionNotice, transcriptOutput, captionOutput);
  const history = h("div", { class: "live-question-history chat-history", tabindex: 0, "aria-label": l("meetings.workspace_questions") });
  const question = h("textarea", { class: "live-question-input", rows: 2, "aria-label": l("meetings.your_question"), placeholder: l("meetings.ask_placeholder_this") });
  const submit = controlButton({ class: "btn primary live-question-submit", onclick: () => { void ask(); } }, l("meetings.ask"));
  const cancel = controlButton({ class: "btn live-question-cancel", hidden: true, onclick: () => { void cancelQuestion(); } }, l("common.cancel"));
  const setup = h("div", { class: "meeting-setup chat-setup help", role: "status" }, h("span", {}, l("meetings.questions_setup_help")), h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_questions")));
  panels.get("questions")!.append(h("p", { class: "help live-question-scope" }, l("meetings.workspace_scope")), setup, history,
    h("div", { class: "live-question-composer" }, question, h("div", { class: "row" }, submit, cancel)), h("p", { class: "help live-source-assessment" }, l("meetings.workspace_sourcecheck_unavailable")));
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
  editor.addEventListener("compositionstart", () => { composing = true; clearTimeout(notesTimer); updateReadiness(); });
  editor.addEventListener("compositionend", () => { composing = false; notes.edit(editor.value); scheduleSave(); updateReadiness(); });
  editor.addEventListener("input", () => { notes.edit(editor.value); if (!composing) scheduleSave(); });
  editor.addEventListener("blur", () => { if (!composing) void saveNotes().catch(showSaveError); });
  title.addEventListener("input", () => { titleDraft.edit(title.value); clearTimeout(titleTimer); titleTimer = window.setTimeout(() => { void saveTitle().catch(showSaveError); }, 350); });
  title.addEventListener("blur", () => { void saveTitle().catch(showSaveError); });
  question.addEventListener("input", updateReadiness);
  question.addEventListener("compositionstart", () => { questionComposing = true; updateReadiness(); });
  question.addEventListener("compositionend", () => { questionComposing = false; updateReadiness(); });
  question.addEventListener("keydown", event => { if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void ask(); } });
  transcriptOutput.addEventListener("scroll", () => {
    const position = transcriptOutput.scrollTop;
    if (intendedScroll !== null && Math.abs(position - intendedScroll) < 2) intendedScroll = null;
    else if (position < previousScroll - 1) pauseFollowing();
    previousScroll = position;
  });
  document.addEventListener("selectionchange", () => {
    const selected = window.getSelection();
    if (selected && !selected.isCollapsed && selected.anchorNode && transcriptOutput.contains(selected.anchorNode)) pauseFollowing();
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
    receive(result) {
      if (!requests.accepts(result.snapshot.meeting_uid, result.snapshot.session_id, result.snapshot.request_id)) return;
      const turn = turns.find(value => value.requestId === result.snapshot.request_id);
      if (!turn || turn.cancelled || turn.answer || turn.error) return;
      turn.snapshot = result.snapshot;
      if (result.answer) turn.answer = result.answer;
      else turn.error = result.error ?? t("meetings.workspace_cancelled");
      requests.cancel();
      renderTurn(turn);
      updateReadiness();
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
  async function writeNotes(value: string) {
    if (!finalized && ["recording", "stopping", "error"].includes(status.phase)) await api.setMeetingNotes(value, session);
    else await api.saveMeetingNotes(owner.id, value);
  }
  function saveNotes(target = notes.value) {
    clearTimeout(notesTimer);
    if (composing) return Promise.reject(new Error(t("meetings.workspace_composing")));
    return writes.run(() => notes.persist(writeNotes, target));
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
    await persistAll([() => saveTitle(), () => saveNotes()]);
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
    const ready = languageModelConfigured(ctx.info);
    setup.hidden = ready;
    question.disabled = !ready;
    submit.disabled = !ready || !!requests.active || !question.value.trim() || composing || questionComposing;
    cancel.hidden = !requests.active;
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
  function scrollLive() { intendedScroll = Math.max(0, transcriptOutput.scrollHeight - transcriptOutput.clientHeight); transcriptOutput.scrollTop = intendedScroll; previousScroll = transcriptOutput.scrollTop; }
  function updateTranscript(segments: Segment[]) {
    const viewport = transcriptOutput.getBoundingClientRect();
    const first = [...rows.values()].find(row => row.element.getBoundingClientRect().bottom >= viewport.top);
    const offset = first ? first.element.getBoundingClientRect().top - viewport.top : 0;
    const oldPosition = transcriptOutput.scrollTop;
    const keys = segmentKeys(segments);
    const retained = new Set(keys);
    for (const [key, row] of rows) if (!retained.has(key)) { row.element.remove(); rows.delete(key); }
    transcriptEmpty.hidden = segments.length > 0;
    let before = transcriptOutput.firstElementChild === transcriptEmpty ? transcriptEmpty.nextElementSibling : transcriptOutput.firstElementChild;
    for (const [index, segment] of segments.entries()) {
      const key = keys[index];
      let row = rows.get(key);
      if (!row) {
        const body = h("div", { class: "transcript-copy" }, segment.failed ? l("meetings.transcription_failed") : segment.text);
        const speaker = h("span", { class: "speaker" }, speakerUiLabel(segment));
        const correction = h("p", { class: "help transcript-correction-review", hidden: !segment.correction_review_required }, l("meetings.attribution_correction_review"));
        const element = h("div", { class: "transcript-line", "data-segment-key": key, "data-start-ms": segment.start_ms, "data-source-id": segment.source_id ?? "" }, h("span", { class: "transcript-stamp" }, formatClock(segment.start_ms / 1000)), speaker, body, correction);
        row = { element, body, speaker, correction, segment };
        rows.set(key, row);
      } else if (!sameSegment(row.segment, segment)) {
        if (row.segment.text !== segment.text || row.segment.failed !== segment.failed) row.body.textContent = segment.failed ? t("meetings.transcription_failed") : segment.text;
        row.segment = segment;
      }
      const label = String(speakerUiLabel(segment));
      if (row.speaker.textContent !== label) row.speaker.textContent = label;
      row.speaker.title = label;
      row.correction.hidden = !segment.correction_review_required;
      row.element.dataset.sourceId = segment.source_id ?? "";
      row.element.title = segment.source_id ? t(`meetings.attribution_source_${segment.source_id}`) : "";
      if (before !== row.element) transcriptOutput.insertBefore(row.element, before);
      before = row.element.nextElementSibling;
    }
    if (following) scrollLive();
    else {
      const position = first?.element.parentElement === transcriptOutput ? oldPosition + first.element.getBoundingClientRect().top - viewport.top - offset : oldPosition;
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
  async function ask() {
    if (!question.value.trim() || requests.active || !languageModelConfigured(ctx.info) || composing || questionComposing) return;
    const text = question.value.trim();
    const target = notes.value;
    clearTimeout(notesTimer);
    const requestId = crypto.randomUUID();
    requests.begin(requestId);
    const response = h("div", { class: "live-answer", role: "status" }, l("meetings.workspace_saving_question"));
    const metadata = h("p", { class: "live-answer-cutoff help" });
    const element = h("article", { class: "chat-exchange", "data-request-id": requestId }, h("p", { class: "chat-question" }, text), metadata, response);
    const turn: Turn = { question: text, requestId, response, metadata, element };
    turns.push(turn); history.append(element);
    question.value = "";
    history.scrollTop = history.scrollHeight;
    updateReadiness();
    try {
      await writes.run(async () => {
        if (!requests.accepts(uid, session, requestId)) return;
        await notes.persist(writeNotes, target);
        if (!requests.accepts(uid, session, requestId)) return;
        await connectQuestions();
        const previous = [];
        for (const item of turns) if (item.answer && item.snapshot && !item.cancelled && (await Promise.all(item.snapshot.anchors.map(anchorMatches))).every(Boolean)) previous.push({ question: item.question, answer: item.answer.text });
        if (!requests.accepts(uid, session, requestId)) return;
        const snapshot = await api.beginLiveQuestion({ requestId, meetingUid: uid, sessionId: session, expectedNotesText: target, question: text, history: previous });
        if (!requests.accepts(uid, session, requestId)) {
          await api.cancelLiveQuestion(requestId, uid, session);
          return;
        }
        turn.snapshot = snapshot;
        if (!turn.answer && !turn.error) { response.replaceChildren(h("progress", { "aria-label": l("meetings.writing_answer") })); updateTurnMetadata(turn); }
      });
    } catch (error) {
      if (!requests.accepts(uid, session, requestId)) return;
      turn.error = notes.error ? t("meetings.workspace_question_save_error") : errorText(error);
      requests.cancel();
      if (!question.value) question.value = text;
      renderTurn(turn);
    } finally { updateReadiness(); }
  }
  async function cancelQuestion() {
    const requestId = requests.active;
    if (!requestId) return;
    requests.cancel();
    const turn = turns.find(value => value.requestId === requestId);
    if (turn) { turn.cancelled = true; turn.error = t("meetings.workspace_cancelled"); if (!question.value) question.value = turn.question; renderTurn(turn); }
    updateReadiness();
    await api.cancelLiveQuestion(requestId, uid, session).catch(error => { warning.textContent = errorText(error); warning.hidden = false; });
  }
  function updateTurnMetadata(turn: Turn) {
    if (!turn.snapshot) return;
    turn.metadata.textContent = t("meetings.workspace_cutoff", { time: formatClock(turn.snapshot.included_through_ms / 1000) })
      + (turn.snapshot.pending_transcription ? ` · ${t("meetings.workspace_pending", { count: turn.snapshot.pending_transcription })}` : "");
    bindLocalization(turn.metadata, () => updateTurnMetadataText(turn));
  }
  function updateTurnMetadataText(turn: Turn) {
    if (turn.snapshot) turn.metadata.textContent = t("meetings.workspace_cutoff", { time: formatClock(turn.snapshot.included_through_ms / 1000) })
      + (turn.snapshot.pending_transcription ? ` · ${t("meetings.workspace_pending", { count: turn.snapshot.pending_transcription })}` : "");
  }
  function renderTurn(turn: Turn) {
    const position = history.scrollTop;
    if (turn.answer) {
      const citations = h("div", { class: "citations" });
      for (const citation of turn.answer.citations) {
        const anchor = turn.snapshot?.anchors[citation.number - 1];
        const place = citation.passage.kind === "transcript" && citation.passage.start_ms !== null ? formatClock(citation.passage.start_ms / 1000) : t("meetings.source_notes");
        citations.append(h("button", { class: "text-button citation", onclick: () => { void openSource(turn, anchor); } }, `[${citation.number}] ${place}`));
      }
      turn.response.replaceChildren(markdown(turn.answer.text), citations);
    } else {
      turn.response.replaceChildren(h("p", { class: "error" }, localizeBackendText(turn.error ?? "")), h("button", { class: "text-button", onclick: () => { if (!requests.active) { question.value = turn.question; updateReadiness(); question.focus(); } } }, l("common.retry")));
    }
    updateTurnMetadata(turn);
    history.scrollTop = position;
  }
  async function openSource(turn: Turn, anchor?: LiveSourceAnchor) {
    const navigation = ++sourceNavigation;
    sourceReturn = { pane: panes.selected, questionScroll: history.scrollTop, transcriptScroll: transcriptOutput.scrollTop, notesScroll: editor.scrollTop, start: editor.selectionStart, end: editor.selectionEnd, direction: editor.selectionDirection, turn: turn.element };
    sourceBack.hidden = false;
    sourceNotice.hidden = true;
    root.querySelectorAll(".live-source-highlight").forEach(node => node.classList.remove("live-source-highlight"));
    const originalSource = anchor ? currentSourceStamp(anchor) : "";
    const matches = !!anchor && await anchorMatches(anchor);
    if (navigation !== sourceNavigation) return;
    if (!anchor || !matches || originalSource !== currentSourceStamp(anchor)) {
      sourceNotice.replaceChildren(h("p", { class: "help" }, l("meetings.workspace_source_changed")), h("blockquote", {}, anchor?.source_text ?? turn.answer?.citations[0]?.passage.text ?? ""));
      sourceNotice.hidden = false;
      return;
    }
    if (anchor.passage.kind === "notes") {
      selectPane("notes", true); showPreview(false); editor.focus({ preventScroll: true }); editor.setSelectionRange(anchor.notes_start!, anchor.notes_end!);
    } else {
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
  }
  updateStatus(initial);
  updateSaving();
  updatePanes();
  void connectQuestions().catch(error => { warning.textContent = errorText(error); warning.hidden = false; });
  return owner;
}
