import { api, fixtureClient, type MeetingStatus, type MeetingDetail, type MeetingFinished, type MeetingPreviewState, type Speaker, type LiveSourceAnchor, type Segment } from "./api";
import { SerialQueue, persistAll } from "./async";
import { bindLocalization, l, t, localizeBackendText, formatNumber, type TranslationKey } from "./i18n";
import { controlButton, errorText, formatClock, h, markdown, showPopupMenu, textField } from "./dom";
import { MeetingDraft } from "./meeting-draft";
import { RecordingRefresh, sameSegment, segmentKeys } from "./recording-refresh";
import { LivePaneState, LiveStatusEpoch, segmentEvidenceValue, formatSelection, type LivePane } from "./live-workspace-state";
import { frozenRecognitionLabel } from "./recording-processing";
import { meetingDate, sourceSpeakerLabel, speakerUiLabel } from "./meeting-labels";
import { VisibilityPoll } from "./window-lifecycle";
import { attributionEditable, attributionNoticeKey, segmentAudioAvailable } from "./meeting-source";
import { LiveSpeakerControl, liveSpeakerStatusKey } from "./live-speaker-state";
import { reviewedInsights, type InsightReviewHost } from "./reviewed-insights";
import { insightSourceMatches, insertionStillCurrent, utf8Range, type InsightNoteLatch, type InsightSource } from "../../../assets/contracts/meeting-insights.ts";
import type { Context } from "./main";
import { openPeoplePicker, peopleNoteControls, type PeopleHost } from "./people-picker";
import { canAcknowledgePeopleNotes, speakerLinkUtteranceIds, type PeopleSnapshot } from "./people-state";
import { conversationPanel } from "./conversation-panel";
import { meetingSummaryEditor } from "./meeting-summary-editor";
import { summaryAutomationChoice } from "./meeting-summary-automation";
import { meetingExportPreview } from "./meeting-export-preview";
import { meetingWorkspaceTools } from "./meeting-workspace-tools";
import { sourceRange, transcriptSourceBlocks } from "./source-navigation";
import { icon } from "./icons";
import { FailedRecoveryContextError, FailedTranscriptionRecovery, transcriptDisplayText } from "./failed-transcription-recovery";
import "./meeting-workspace.css";

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

export function liveMeetingWorkspace(ctx: Context, status: MeetingStatus, options: { initialSummary?: boolean; detail?: MeetingDetail } = {}): HTMLElement {
  const session = status.session_id ?? status.id!;
  let owner = workspaces.get(session);
  if (!owner) {
    owner = createWorkspace(ctx, status, options.initialSummary, options.detail);
    workspaces.set(session, owner);
  }
  return owner.bind(ctx);
}

async function fingerprint(segment: Segment): Promise<string> {
  const value = JSON.stringify(segmentEvidenceValue(segment));
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

function createWorkspace(initialContext: Context, initial: MeetingStatus, initialSummary = false, initialDetail?: MeetingDetail): LiveWorkspace {
  let ctx = initialContext;
  const session = initial.session_id ?? initial.id!;
  const uid = initial.meeting_uid ?? initial.id!;
  let status = initial;
  let finalized = !initial.busy && initial.phase === "idle";
  let reviewDetail: MeetingDetail | null = initialDetail ?? null;
  let selectedTemplate = ctx.info.settings.default_template;
  let lastReviewRefresh = -Infinity;
  const readingPositions = new Map<HTMLElement, { top: number; left: number }>();
  let suspendedReading: Map<HTMLElement, { top: number; left: number }> | null = null;
  let suspendedSelections: { input: HTMLTextAreaElement; start: number; end: number; direction: "forward" | "backward" | "none" }[] | null = null;
  let restoringReading = false;
  let binding = 0;
  let recoveryBindingActive = false;
  let poll: VisibilityPoll | null = null;
  let composing = false;
  let notesTimer: number | undefined;
  let titleTimer: number | undefined;
  let preview = false;
  let previewValue = "";
  let sourceNavigation = 0;
  let questionSourceNavigation = 0;
  let following = initial.recording;
  let previousScroll = 0;
  let intendedScroll: number | null = null;
  let pendingTranscript: Segment[] | null = null;
  let sourceReturn: { view: Pick<LivePaneState, "selected" | "primary" | "companion" | "companionOpen">; questionScroll: number; questionPaneScroll: number; transcriptScroll: number; notesScroll: number; documentScroll: number;
    start: number; end: number; direction: "forward" | "backward" | "none"; summaryStart: number; summaryEnd: number; summaryDirection: "forward" | "backward" | "none"; summaryScroll: number; summaryEditing: boolean;
    turn: HTMLElement; restore?: () => void; label?: TranslationKey } | null = null;
  let insights: ReturnType<typeof reviewedInsights> | null = null;
  let inputRevision = 0, selectionRevision = 0, interactionRevision = 0;
  let observedSelection = "";
  let insertionReceipt: { text: string; attempt: string } | null = null;
  const panes = new LivePaneState();
  if (finalized && initialSummary) panes.select("summary");
  const fixturePane = fixtureClient()?.fixture?.meeting_tab;
  if (["transcript", "ask", "insights", "summary"].includes(String(fixturePane))) panes.select(fixturePane === "ask" ? "questions" : fixturePane === "insights" ? "suggestions" : fixturePane as LivePane);
  const statusEpoch = new LiveStatusEpoch();
  const notes = new MeetingDraft();
  const titleDraft = new MeetingDraft();
  const writes = new SerialQueue();
  const titleWrites = new SerialQueue();
  const refresh = new RecordingRefresh(initial);
  const liveSpeakers = new LiveSpeakerControl(session, initial.live_speaker_attribution);
  let peopleSnapshot: PeopleSnapshot | null = null;
  const rows = new Map<string, { element: HTMLElement; body: HTMLElement; speaker: HTMLElement; stamp: HTMLButtonElement; provisional: HTMLElement; correction: HTMLElement; segment: Segment }>();
  const root = h("article", { class: "live-workspace live-recording meeting-notes-first", "data-session-id": session, "data-meeting-uid": uid });
  root.addEventListener("scroll", event => {
    const target = event.target;
    if (!restoringReading && root.isConnected && target instanceof HTMLElement && target.getClientRects().length && target.clientHeight) {
      readingPositions.set(target, { top: target.scrollTop, left: target.scrollLeft });
    }
  }, true);
  root.addEventListener("click", event => {
    // Conversation source resolution is asynchronous before it reaches this workspace.
    if (event.target instanceof Element && event.target.closest(".live-questions-pane .citation")) questionSourceNavigation = sourceNavigation;
  }, true);
  for (const event of ["pointerdown", "keydown", "input"]) root.addEventListener(event, () => { ++interactionRevision; }, true);
  const title = textField({ class: "live-title", "aria-label": l("meetings.meeting_title_label") });
  const clock = h("span", { class: "live-clock" });
  const phase = h("span", { class: "live-phase", role: "status" });
  const stop = controlButton({ class: "btn live-stop", onclick: () => { void stopCapture(); } }, l("common.stop_recording"));
  const copyExport = controlButton({ class: "btn primary meeting-export-action", hidden: true, onclick: () => openExport("shareable") }, l("meetings.share"));
  const headerActions = h("div", { class: "meeting-header-actions" }, copyExport);
  const date = h("span", { class: "meeting-header-date" });
  const duration = h("span", { class: "meeting-header-duration" });
  const people = h("div", { class: "meeting-header-people", "aria-label": l("meetings.workspace_people") });
  let peopleSignature = "";
  const metadata = h("div", { class: "meeting-header-metadata help", hidden: true }, date, duration, people);
  const recordingControls = h("div", { class: "row recording-controls" }, h("span", { class: "status-dot", "aria-hidden": "true" }), phase, clock, h("span", { class: "grow" }), stop);
  const band = h("header", { class: "live-recording-band" }, h("div", { class: "meeting-heading-row" }, title, headerActions), metadata, recordingControls);
  const warning = h("p", { class: "live-warning help", role: "status", hidden: true });
  const recovery = h("div", { class: "live-recovery", hidden: true });
  const failedRecoveryHeading = h("strong");
  const failedRecoveryDescription = h("p", { class: "help" });
  const failedRecoveryOutcome = h("p", { class: "help", role: "status", hidden: true });
  const failedRecoveryError = h("p", { class: "error", role: "alert", hidden: true });
  const failedRecoveryModel = new FailedTranscriptionRecovery<MeetingDetail>({
    scope: () => recoveryBindingActive && root.isConnected && finalized ? `${uid}:${binding}` : null,
    id: () => owner.id, meetingUid: uid,
    blocked: () => !finalized || status.busy || !!reviewDetail?.retranscription_status?.running || composing,
    flush, read: id => api.meetingDetail(id), retry: id => api.retranscribeMeeting(id),
    accept: detail => { statusEpoch.invalidate(); applyReview(detail); void summary.refresh().catch(showSaveError); }, changed: renderFailedRecovery,
  });
  const failedRecoveryRetry = controlButton({ class: "btn compact", onclick: () => {
    // Slower background reads must not restore the counts captured before an explicit retry.
    statusEpoch.invalidate(); void failedRecoveryModel.retry();
  } }, l("meetings.failed_recovery_retry"));
  const failedRecoveryNotice = h("section", { class: "meeting-failed-recovery", hidden: true, "aria-label": l("meetings.failed_recovery_title") },
    h("div", { class: "meeting-failed-recovery-copy" }, failedRecoveryHeading, failedRecoveryDescription, failedRecoveryOutcome, failedRecoveryError), failedRecoveryRetry);
  const sourceBack = h("button", { class: "text-button live-source-back", hidden: true, onclick: () => returnToAnswer() }, l("meetings.workspace_back_answer"));
  const sourceNotice = h("div", { class: "live-source-notice", hidden: true });
  const writingTabs = h("div", { class: "meeting-writing-tabs", role: "tablist", "aria-label": l("meetings.workspace_notes") });
  const companionTabs = h("div", { class: "meeting-companion-tabs", role: "tablist", "aria-label": l("meetings.workspace_tools") });
  const closeCompanion = h("button", { class: "btn compact meeting-companion-close", hidden: true, title: l("meetings.workspace_close_companion"), "aria-label": l("meetings.workspace_close_companion"), onclick: () => returnToDocument() }, icon("close"));
  const more = h("button", { class: "btn compact meeting-workspace-more", "aria-haspopup": "menu", onclick: () => {
    showPopupMenu(more, [
      { value: "format", label: l("meetings.workspace_formatting"), run: () => { selectPane("notes"); noteHeading.hidden = !noteHeading.hidden; } },
      { value: "people", label: l("meetings.workspace_people"), run: () => openTools("people") },
      { value: "speakers", label: l("meetings.workspace_speaker_controls"), run: () => openTools("speakers") },
      { value: "template", label: l("meetings.summary_template_change"), run: () => openTools("template") },
      { value: "automation", label: l("meetings.summary_automation_change"), run: () => openTools("automation") },
      { value: "suggestions", label: l("meetings.workspace_suggestions"), run: () => { selectPane("suggestions"); if (finalized) void insights?.refresh(); } },
      { value: "transcription", label: l("meetings.retranscribe_audio"), run: () => openTools("transcription") },
      { value: "files", label: l("meetings.workspace_file_actions"), run: () => openTools("files") },
      { value: "shareable", label: l("meetings.export_default_action"), separatorBefore: true, disabled: !finalized, title: !finalized ? l("meetings.tools_no_recording") : undefined, run: () => openExport("shareable") },
      { value: "personal", label: l("meetings.export_personal_action"), disabled: !finalized, title: !finalized ? l("meetings.tools_no_recording") : undefined, run: () => openExport("personal") },
    ], { report: showSaveError });
  } }, l("meetings.workspace_more"), icon("more"));
  headerActions.append(more);
  const tabList = h("nav", { class: "meeting-workspace-nav" }, writingTabs, h("span", { class: "grow" }), companionTabs, closeCompanion);
  const paneNames: [LivePane, TranslationKey][] = [["notes", "meetings.your_notes"], ["summary", "meetings.workspace_summary"], ["transcript", "meetings.workspace_transcript"], ["questions", "meetings.workspace_questions"], ["suggestions", "meetings.workspace_suggestions"], ["tools", "meetings.workspace_tools"]];
  const tabButtons = new Map<LivePane, HTMLButtonElement>();
  const panels = new Map<LivePane, HTMLElement>();
  for (const [pane, key] of paneNames) {
    const button = h("button", { class: "tab", role: "tab", id: `live-${session}-${pane}-tab`, "data-live-tab": pane, "aria-controls": `live-${session}-${pane}`, onclick: () => {
      if (pane === "notes" || pane === "summary") selectPane(pane);
      else { ++sourceNavigation; panes.toggleCompanion(pane); updatePanes(); }
    } }, l(key));
    button.addEventListener("keydown", event => {
      const available = paneNames.filter(([name]) => !tabButtons.get(name)?.hidden && (pane === "notes" || pane === "summary" ? name === "notes" || name === "summary" : name === "transcript" || name === "questions")).map(([name]) => name);
      const index = available.indexOf(pane);
      const next = event.key === "ArrowRight" ? available[(index + 1) % available.length] : event.key === "ArrowLeft" ? available[(index - 1 + available.length) % available.length] : event.key === "Home" ? available[0] : event.key === "End" ? available.at(-1) : null;
      if (next) { event.preventDefault(); selectPane(next); tabButtons.get(next)?.focus(); tabButtons.get(next)?.scrollIntoView({ block: "nearest", inline: "nearest" }); }
    });
    tabButtons.set(pane, button);
    (pane === "notes" || pane === "summary" ? writingTabs : companionTabs).append(button);
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
  for (const [format, key, text] of [["heading", "meetings.workspace_heading", "H"], ["bold", "meetings.workspace_bold", "B"], ["list", "meetings.workspace_list", null], ["task", "meetings.workspace_task", null]] as const) {
    const button = h("button", { class: "btn compact", title: l(key), "aria-label": l(key), "data-live-format": format }, text ?? l(key));
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
  const noteHeading = h("div", { class: "live-note-controls", hidden: true }, h("div", { class: "live-preview-toggle" }, sourceToggle, previewToggle), formatting);
  const peopleControls = peopleNoteControls(peopleHost());
  const notesTitle = h("h2", { id: `live-${session}-notes-heading` }, l("meetings.your_notes"));
  const notesSectionHeading = h("div", { class: "meeting-notes-heading", hidden: true }, notesTitle, h("p", { class: "help" }, l("meetings.notes_private_share")));
  panels.get("notes")!.append(notesSectionHeading, noteHeading, editor, notePreview, h("footer", { class: "live-save-controls" }, saveStatus, save), saveError);
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
  panels.get("transcript")!.append(h("div", { class: "live-transcript-heading" }, follow), attributionNotice, transcriptOutput, captionOutput);
  const questions = conversationPanel(ctx, {
    scope: { kind: "meeting", meeting_uid: uid, meeting_id: initial.id! }, live: true,
    canAsk: () => !composing,
    prepare: async () => {
      if (composing) throw new Error(t("meetings.workspace_composing"));
      const target = notes.value, externalSession = finalized ? undefined : session;
      clearTimeout(notesTimer);
      await writes.run(() => notes.persist(writeNotes, target));
      await summary.flush();
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(target));
      const expected_notes_revision = [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
      return { expected_notes_revision, ...externalSession ? { session_id: externalSession } : {} };
    },
    source: async (source, origin, _message, restore) => {
      if (questionSourceNavigation !== sourceNavigation) return true;
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
  const summary = meetingSummaryEditor(ctx, { id: () => owner.id, finished: () => finalized, flushNotes: async () => { await saveNotes(); await saveTitle(); }, template: () => selectedTemplate,
    processing: () => !!reviewDetail?.retranscription_status?.running,
    canReview: () => !composing && !notes.dirty && notes.status !== "saving" && !titleDraft.dirty && titleDraft.status !== "saving" });
  panels.get("summary")!.append(summary.root);
  const automation = summaryAutomationChoice(ctx, { compact: true, configure: () => openTools("automation") });
  const automationNudge = h("div", { class: "meeting-automation-nudge", hidden: !finalized }, automation.root);
  const tools = meetingWorkspaceTools(ctx, { id: () => owner.id, finished: () => finalized, flush, refresh: loadReview,
    template: () => selectedTemplate, setTemplate: value => { selectedTemplate = value; }, speakers: speakerControls, people: peopleControls, close: returnToDocument,
    processingChanged: value => { if (reviewDetail) reviewDetail.retranscription_status = value; summary.readiness(); } });
  const exportPreview = meetingExportPreview(ctx, { id: () => owner.id, template: () => selectedTemplate, flush });
  panels.get("tools")!.append(tools.root, exportPreview.root);
  const separator = h("div", { class: "live-pane-separator", role: "separator", tabindex: 0, "aria-orientation": "vertical", "aria-label": l("meetings.workspace_split"), "aria-valuemin": 35, "aria-valuemax": 75 });
  const transcriptProvenance = h("p", { class: "help meeting-transcript-provenance", hidden: true });
  const documentSurface = h("div", { class: "meeting-document", "aria-label": l("meetings.document_label"), tabindex: -1 }, panels.get("summary")!, panels.get("notes")!, transcriptProvenance);
  let lastDocumentFocus: HTMLElement | null = null;
  let documentReturn: { scroll: number; focus: HTMLElement | null } | null = null;
  documentSurface.addEventListener("focusin", event => { if (event.target instanceof HTMLElement) lastDocumentFocus = event.target; });
  const rememberDocument = () => { if (!documentSurface.hidden) documentReturn = { scroll: documentSurface.scrollTop, focus: lastDocumentFocus }; };
  const summaryHeading = summary.root.querySelector("h2")!; summaryHeading.id = `live-${session}-summary-heading`;
  const content = h("div", { class: "live-workspace-panes" }, documentSurface, separator, panels.get("transcript")!, panels.get("questions")!, panels.get("suggestions")!, panels.get("tools")!);
  root.append(band, warning, recovery, failedRecoveryNotice, sourceBack, sourceNotice, tabList, automationNudge, content);
  let resizeFrame = 0;
  const resize = new ResizeObserver(entries => {
    const width = entries[0]?.contentRect.width ?? 0;
    cancelAnimationFrame(resizeFrame);
    if (!root.isConnected || width <= 0) return;
    resizeFrame = requestAnimationFrame(() => { if (root.isConnected) { panes.resize(width); updatePanes(); } });
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
    if (!root.isConnected || restoringReading || !transcriptOutput.getClientRects().length) return;
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
      recoveryBindingActive = true;
      statusEpoch.invalidate();
      ++sourceNavigation;
      poll?.dispose();
      poll = new VisibilityPoll(500, pollStatus, error => { warning.textContent = errorText(error); warning.hidden = false; });
      poll.setVisible(ctx.visible);
      next.onVisibilityChange(visible => { if (mine === binding) poll?.setVisible(visible); });
      next.onDispose(() => { if (mine === binding) { recoveryBindingActive = false; suspendReading(); ++sourceNavigation; statusEpoch.invalidate(); poll?.dispose(); poll = null; } });
      next.onInfoChange(() => { if (mine === binding) updateReadiness(); });
      next.onFlush(flush);
      questions.bind(next);
      summary.bind(next); automation.bind(next); tools.bind(next); exportPreview.bind(next);
      if (finalized && insights) insights.bind(ctx, insightHost());
      const peopleId = owner.id;
      void api.meetingPeople(peopleId).then(snapshot => {
        if (mine === binding && owner.id === peopleId) { peopleSnapshot = snapshot; updatePeopleLinks(); }
      }).catch(() => undefined);
      updateReadiness();
      if (finalized) void loadReview().catch(showSaveError);
      requestAnimationFrame(() => { if (mine === binding) { panes.resize(content.getBoundingClientRect().width); updatePanes(); void openRequestedSource(); } });
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
    const keepNotesVisible = finalized && root.dataset.documentLayout !== "continuous" && document.activeElement === editor;
    const previousNotesScroll = editor.scrollTop;
    panes.setFinished(finalized);
    root.dataset.documentLayout = finalized ? "continuous" : "notes";
    root.classList.toggle("meeting-finished-document", finalized);
    documentSurface.hidden = !panes.shown("notes") && !panes.shown("summary");
    writingTabs.hidden = finalized;
    notesSectionHeading.hidden = !finalized;
    root.classList.toggle("live-wide", panes.wide);
    root.classList.toggle("live-companion-open", panes.companionOpen);
    automationNudge.hidden = !finalized || panes.companionOpen;
    root.classList.toggle("live-enlarged", parseFloat(getComputedStyle(tabList).fontSize) >= 20);
    content.style.setProperty("--live-notes-ratio", String(panes.ratio / 100));
    root.style.setProperty("--live-notes-ratio", String(panes.ratio / 100));
    separator.hidden = !panes.wide || !panes.companionOpen;
    closeCompanion.hidden = !panes.companionOpen;
    separator.setAttribute("aria-valuenow", String(Math.round(panes.ratio)));
    for (const [pane, panel] of panels) {
      panel.hidden = !panes.shown(pane);
      const button = tabButtons.get(pane)!;
      const writing = pane === "notes" || pane === "summary";
      if (writing) { panel.setAttribute("role", finalized ? "region" : "tabpanel"); panel.setAttribute("aria-labelledby", finalized ? pane === "notes" ? notesTitle.id : summaryHeading.id : button.id); }
      button.hidden = pane === "tools" || pane === "suggestions" || finalized && writing || pane === "summary" && !finalized;
      const selected = pane === "notes" || pane === "summary" ? pane === panes.primary && (panes.wide || pane === panes.selected)
        : panes.companionOpen && pane === panes.companion && (panes.wide || pane === panes.selected);
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected || pane === panes.primary ? 0 : -1;
    }
    sizeNotes();
    if (keepNotesVisible) documentSurface.scrollTop += editor.getBoundingClientRect().top - documentSurface.getBoundingClientRect().top + previousNotesScroll;
    restoreReading();
  }
  function suspendReading() {
    if (!root.isConnected) return;
    for (const target of [documentSurface, ...panels.values(), editor, notePreview, transcriptOutput, history, summary.editor]) {
      if (target.getClientRects().length && target.clientHeight) readingPositions.set(target, { top: target.scrollTop, left: target.scrollLeft });
    }
    for (const target of readingPositions.keys()) if (!root.contains(target)) readingPositions.delete(target);
    suspendedReading = new Map(readingPositions);
    suspendedSelections = [editor, summary.editor].map(input => ({ input, start: input.selectionStart, end: input.selectionEnd, direction: input.selectionDirection }));
  }
  function restoreReading() {
    if (!root.isConnected || !suspendedReading || !content.getBoundingClientRect().width) return;
    restoringReading = true;
    try {
      for (const selection of suspendedSelections ?? []) selection.input.setSelectionRange(selection.start, selection.end, selection.direction);
      suspendedSelections = null;
      for (const [target, position] of suspendedReading) {
        if (!target.getClientRects().length || !target.clientHeight) continue;
        target.scrollTop = position.top; target.scrollLeft = position.left;
        if (target === transcriptOutput) { previousScroll = target.scrollTop; intendedScroll = target.scrollTop; }
        suspendedReading.delete(target);
      }
      if (!suspendedReading.size) suspendedReading = null;
    } finally { restoringReading = false; }
  }
  function sizeNotes() {
    if (!finalized) { editor.style.height = ""; return; }
    const top = documentSurface.scrollTop;
    editor.style.height = "auto";
    editor.style.height = `${Math.max(180, editor.scrollHeight)}px`;
    documentSurface.scrollTop = top;
  }
  function scrollDocumentSection(pane: "notes" | "summary") {
    const section = panels.get(pane)!;
    documentSurface.scrollTop += section.getBoundingClientRect().top - documentSurface.getBoundingClientRect().top;
  }
  function selectPane(pane: LivePane, openingSource = false) {
    if (!openingSource) ++sourceNavigation;
    panes.select(pane); updatePanes();
    if (finalized && (pane === "notes" || pane === "summary")) scrollDocumentSection(pane);
  }
  function openTools(section?: string) { rememberDocument(); tools.root.hidden = false; exportPreview.root.hidden = true; selectPane("tools"); tools.reveal(section); }
  function returnToDocument() {
    ++sourceNavigation;
    panes.closeCompanion(); updatePanes();
    if (finalized && documentReturn) {
      const retained = documentReturn; documentReturn = null;
      documentSurface.scrollTop = retained.scroll;
      (retained.focus?.isConnected ? retained.focus : documentSurface).focus({ preventScroll: true });
      return;
    }
    if (panes.primary === "notes" && !preview) editor.focus({ preventScroll: true });
    else if (panes.primary === "summary" && summary.editing()) summary.editor.focus({ preventScroll: true });
    else if (finalized) documentSurface.focus({ preventScroll: true });
    else tabButtons.get(panes.primary)?.focus({ preventScroll: true });
  }
  function openExport(scope: "shareable" | "personal") { if (!finalized) return; rememberDocument(); tools.root.hidden = true; exportPreview.open(scope); selectPane("tools"); }
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
  function scheduleSave() { sizeNotes(); clearTimeout(notesTimer); notesTimer = window.setTimeout(() => { void saveNotes().catch(showSaveError); }, 600); }
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
    await persistAll([() => saveTitle(), () => saveNotes(), questions.flush, summary.flush]);
  }
  function showSaveError(error: unknown) { saveError.textContent = errorText(error); saveError.hidden = false; updateSaving(); }
  function updateSaving() {
    const state = notes.status === "saving" || titleDraft.status === "saving" ? "saving" : notes.dirty || titleDraft.dirty ? "unsaved" : "saved";
    saveStatus.dataset.saveState = state;
    saveStatus.textContent = state === "saved" && !notes.acknowledgedRevision && !titleDraft.acknowledgedRevision ? ""
      : t(state === "saving" ? "meetings.workspace_saving" : state === "unsaved" ? "meetings.workspace_unsaved" : "meetings.workspace_saved");
    save.disabled = state === "saving" || !notes.dirty && !titleDraft.dirty || composing;
    const failure = notes.error ?? titleDraft.error;
    if (failure) { saveError.textContent = localizeBackendText(failure); saveError.hidden = false; }
    else if (!notes.dirty && !titleDraft.dirty) saveError.hidden = true;
    summary.readiness();
  }
  function updateReadiness() {
    questions.readiness();
    updateSaving();
    renderFailedRecovery();
  }
  function renderFailedRecovery() {
    const control = failedRecoveryModel.control;
    failedRecoveryNotice.hidden = !finalized || !control.visible && !failedRecoveryModel.error && !failedRecoveryModel.outcome;
    failedRecoveryNotice.setAttribute("aria-busy", String(failedRecoveryModel.busy));
    failedRecoveryNotice.dataset.recoveryState = failedRecoveryModel.busy ? "working" : failedRecoveryModel.outcome ?? (control.canRetry ? "retryable" : "unavailable");
    failedRecoveryHeading.textContent = t(control.incomplete ? "meetings.failed_recovery_title" : failedRecoveryModel.outcome === "complete" ? "meetings.failed_recovery_complete" : "meetings.failed_recovery_unknown");
    const count = control.failedCount;
    const counts = count === null || count === 0 ? "" : t("meetings.failed_recovery_count", { count });
    const available = control.retryableCount === null || count === null || count === 0 ? "" : t("meetings.failed_recovery_available", { retryable: control.retryableCount, count });
    const partiallyAvailable = count !== null && control.retryableCount !== null && control.retryableCount > 0 && control.retryableCount < count;
    const reasons: Record<string, TranslationKey> = {
      metadata_unavailable: "meetings.failed_recovery_metadata", unverified_provenance: "meetings.failed_recovery_unverified",
      audio_changed: "meetings.failed_recovery_audio_changed", unreadable_audio: "meetings.failed_recovery_unreadable",
      no_retained_audio: partiallyAvailable ? "meetings.failed_recovery_some_no_audio" : "meetings.failed_recovery_no_audio",
      status_unavailable: "meetings.failed_recovery_unknown_help",
    };
    const reason = control.reasonCode ? t(reasons[control.reasonCode] ?? "meetings.failed_recovery_unknown_help") : "";
    failedRecoveryDescription.textContent = [counts, available, reason].filter(Boolean).join(" ");
    failedRecoveryDescription.hidden = !failedRecoveryDescription.textContent;
    failedRecoveryRetry.disabled = !control.canRetry;
    title.disabled = failedRecoveryModel.busy || !finalized && !["recording", "stopping"].includes(status.phase);
    failedRecoveryRetry.hidden = !control.visible && !failedRecoveryModel.busy;
    failedRecoveryRetry.textContent = t(failedRecoveryModel.busy ? "meetings.failed_recovery_working" : "meetings.failed_recovery_retry");
    failedRecoveryOutcome.hidden = !failedRecoveryModel.outcome;
    if (failedRecoveryModel.outcome) failedRecoveryOutcome.textContent = t(`meetings.failed_recovery_outcome_${failedRecoveryModel.outcome}`);
    failedRecoveryError.hidden = !failedRecoveryModel.error;
    if (failedRecoveryModel.error) failedRecoveryError.textContent = failedRecoveryModel.error instanceof FailedRecoveryContextError
      ? t("meetings.failed_recovery_meeting_changed") : localizeBackendText(errorText(failedRecoveryModel.error));
  }
  function updateStatus(next: MeetingStatus) {
    status = next;
    if (next.id) { owner.id = next.id; owner.aliases.add(next.id); }
    clock.textContent = formatClock(next.elapsed_secs);
    phase.textContent = t(next.recording ? "meetings.workspace_recording" : next.busy ? "meetings.workspace_processing" : "meetings.workspace_ready");
    root.dataset.recording = String(next.recording);
    root.dataset.meetingId = owner.id;
    if (!next.recording) follow.hidden = true;
    stop.disabled = !next.recording || next.phase !== "recording";
    stop.hidden = !next.busy && finalized;
    copyExport.hidden = !finalized;
    recordingControls.hidden = finalized;
    updateHeader();
    automationNudge.hidden = !finalized;
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
    renderFailedRecovery();
    if (finalized && (!insights || insights.model.id !== owner.id)) {
      insights = reviewedInsights(ctx, insightHost()); panels.get("suggestions")!.replaceChildren(insights.root);
    }
    if (sourceReturn?.label) sourceBack.textContent = t(sourceReturn.label);
    updatePanes();
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
        ++inputRevision; notes.sync(session, result.notes); editor.value = result.notes; selectPane("notes"); showPreview(false);
        editor.focus({ preventScroll: true });
        editor.setSelectionRange(result.selection_start ?? before.selection_start, result.selection_end ?? before.selection_end);
        updateSaving();
      } } };
  }
  function updateHeader() {
    metadata.hidden = !finalized;
    date.textContent = reviewDetail?.info?.start_iso ? meetingDate(reviewDetail.info.start_iso) : "";
    duration.textContent = reviewDetail?.info ? t("meetings.duration_minutes", { count: formatNumber(reviewDetail.info.duration_min) }) : "";
    const names = peopleSnapshot?.people.length ? peopleSnapshot.people.map(person => person.display_name) : reviewDetail?.attendees ?? [];
    const signature = JSON.stringify(names);
    if (signature !== peopleSignature) {
      peopleSignature = signature;
      people.replaceChildren(...names.map(name => controlButton({ class: "meeting-person-chip", onclick: () => openTools("people") }, name)));
    }
    const recognition = initial.session_id && initial.recognition && !reviewDetail?.retranscription_status ? initial.recognition : null;
    transcriptProvenance.hidden = !finalized || !recognition || !status.transcript.some(segment => !segment.failed && segment.text.trim());
    transcriptProvenance.textContent = recognition ? t("meetings.transcript_provenance", { destination: frozenRecognitionLabel(recognition) }) : "";
  }
  function updatePeopleLinks() {
    updateHeader();
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
  function pauseFollowing() { following = false; follow.hidden = !status.recording; }
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
        && row.body.textContent !== transcriptDisplayText(segment)) return true;
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
        const body = h("div", { class: "transcript-copy" }, transcriptDisplayText(segment));
        bindLocalization(body, () => {
          const text = transcriptDisplayText(rows.get(key)?.segment ?? segment);
          if (body.textContent !== text) body.textContent = text;
        });
        const stamp = h("button", { class: "transcript-stamp text-button", onclick: () => {
          const current = rows.get(key)?.segment;
          if (!current || !finalized) return;
          const work = current.utterance_id ? api.playMeetingUtteranceAudio(owner.id, current.utterance_id) : api.playMeetingAudio(owner.id, current.speaker, current.start_ms);
          void work.catch(showSaveError);
        } }, formatClock(segment.start_ms / 1000));
        const speaker = h("button", { class: "speaker people-speaker-action", type: "button", onclick: () => {
          const current = rows.get(key)?.segment;
          if (!current?.utterance_id) { showSaveError(new Error(t("people.no_stable_speaker"))); return; }
          const ids = speakerLinkUtteranceIds(current, status.transcript);
          if (!ids.length) { showSaveError(new Error(t("people.no_stable_speaker"))); return; }
          void openPeoplePicker(peopleHost(), { kind: "speaker", utteranceIds: ids }).catch(showSaveError);
        } }, speakerUiLabel(segment, reviewDetail ?? undefined));
        const provisional = h("span", { class: "live-speaker-provisional", hidden: true, title: l("meetings.live_speakers_provisional_help") }, l("meetings.live_speakers_provisional_tag"));
        const correction = h("p", { class: "help transcript-correction-review", hidden: !segment.correction_review_required }, l("meetings.attribution_correction_review"));
        const linked = h("span", { class: "people-link-state", hidden: true });
        const element = h("div", { class: "transcript-line", "data-segment-key": key, "data-start-ms": segment.start_ms, "data-source-id": segment.source_id ?? "" }, stamp, h("div", { class: "live-transcript-speaker" }, speaker, provisional, linked), body, correction);
        row = { element, body, speaker, stamp, provisional, correction, segment };
        rows.set(key, row);
      } else if (!sameSegment(row.segment, segment)) {
        if (row.segment.text !== segment.text || row.segment.failed !== segment.failed) row.body.textContent = transcriptDisplayText(segment);
        row.segment = segment;
      }
      const label = String(speakerUiLabel(segment, reviewDetail ?? undefined));
      row.element.dataset.transcriptIndex = String(index);
      const canPlay = finalized && !!reviewDetail && segmentAudioAvailable(segment, reviewDetail.audio_tracks, reviewDetail.metadata_status);
      row.stamp.disabled = !canPlay || ctx.info.capabilities.audio_playback.status !== "available";
      row.stamp.title = !finalized ? t("meetings.tools_no_recording") : canPlay ? ctx.info.capabilities.audio_playback.reason ?? t("meetings.play_from_here") : t("meetings.passage_audio_unavailable");
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
    if (questions.model.busy && questions.root.offsetParent === null) await questions.model.refresh();
    if (finalized) {
      if (performance.now() - lastReviewRefresh > 3000) { lastReviewRefresh = performance.now(); await loadReview(); }
      return;
    }
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
    applyReview(detail);
    await summary.refresh();
  }
  function applyReview(detail: MeetingDetail) {
    reviewDetail = detail;
    finalized = true;
    if (!composing && document.activeElement !== editor && !notes.dirty && notes.status !== "saving") {
      notes.sync(session, detail.notes); if (editor.value !== notes.value) editor.value = notes.value;
    }
    if (document.activeElement !== title && !titleDraft.dirty && titleDraft.status !== "saving") {
      titleDraft.sync(session, detail.info?.title ?? status.title ?? ""); if (title.value !== titleDraft.value) title.value = titleDraft.value;
    }
    updateStatus({ ...status, id: owner.id, busy: false, recording: false, phase: "idle", pending: 0, notes: detail.notes, transcript: detail.transcript,
      metadata_status: detail.metadata_status, title: detail.info?.title ?? status.title, transcription_warning: null });
    failedRecoveryModel.observe(detail);
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
    if (anchor.passage.kind === "summary") return !!sourceRange(summary.editor.value, anchor.source_text);
    const keys = segmentKeys(status.transcript);
    for (const [index, key] of anchor.segment_keys.entries()) {
      const at = keys.indexOf(key);
      if (at < 0 || !anchor.segment_fingerprints[index] || await fingerprint(status.transcript[at]) !== anchor.segment_fingerprints[index]) return false;
    }
    return anchor.segment_keys.length > 0;
  }
  async function openSource(turn: HTMLElement, anchor?: LiveSourceAnchor, frozenText?: string, restore?: () => void, label: TranslationKey = "meetings.workspace_back_answer") {
    const navigation = ++sourceNavigation;
    sourceReturn = { view: { selected: panes.selected, primary: panes.primary, companion: panes.companion, companionOpen: panes.companionOpen },
      questionScroll: history.scrollTop, questionPaneScroll: panels.get("questions")!.scrollTop, transcriptScroll: transcriptOutput.scrollTop, notesScroll: editor.scrollTop, documentScroll: documentSurface.scrollTop, start: editor.selectionStart, end: editor.selectionEnd, direction: editor.selectionDirection,
      summaryStart: summary.editor.selectionStart, summaryEnd: summary.editor.selectionEnd, summaryDirection: summary.editor.selectionDirection, summaryScroll: summary.editor.scrollTop,
      summaryEditing: summary.editing(),
      turn, restore, label };
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
    } else if (anchor.passage.kind === "summary") {
      selectPane("summary", true); summary.focusSource(anchor.source_text);
    } else {
      if (pendingTranscript) { clearTranscriptSelection(); flushPendingTranscript(); }
      pauseFollowing(); selectPane("transcript", true);
      const row = rows.get(anchor.segment_keys[0]);
      row?.element.classList.add("live-source-highlight"); row?.element.scrollIntoView({ block: "center", behavior: "instant" });
    }
  }
  function currentSourceStamp(anchor: LiveSourceAnchor) {
    if (anchor.passage.kind === "notes") return JSON.stringify([anchor.notes_start, anchor.notes_end, editor.value.slice(anchor.notes_start ?? 0, anchor.notes_end ?? 0)]);
    if (anchor.passage.kind === "summary") return summary.editor.value;
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
    Object.assign(panes, retained.view); updatePanes();
    history.scrollTop = retained.questionScroll;
    transcriptOutput.scrollTop = retained.transcriptScroll;
    editor.setSelectionRange(retained.start, retained.end, retained.direction);
    editor.scrollTop = retained.notesScroll;
    summary.editor.setSelectionRange(retained.summaryStart, retained.summaryEnd, retained.summaryDirection);
    summary.editor.scrollTop = retained.summaryScroll;
    summary.restorePresentation(retained.summaryEditing);
    documentSurface.scrollTop = retained.documentScroll;
    sourceBack.hidden = true; sourceNotice.hidden = true;
    panels.get("questions")!.scrollTop = retained.questionPaneScroll;
    root.querySelectorAll(".live-source-highlight").forEach(node => node.classList.remove("live-source-highlight"));
    retained.turn.setAttribute("tabindex", "-1"); retained.turn.focus({ preventScroll: true });
    if (retained.label === "meetings.workspace_back_answer" && retained.turn.isConnected) {
      const bounds = retained.turn.getBoundingClientRect(), historyBounds = history.getBoundingClientRect(), paneBounds = panels.get("questions")!.getBoundingClientRect();
      const visible = bounds.bottom > Math.max(historyBounds.top, paneBounds.top) && bounds.top < Math.min(historyBounds.bottom, paneBounds.bottom);
      if (!visible) retained.turn.scrollIntoView({ block: "center", inline: "nearest" });
    }
    // A retained answer already has its exact rows and reading position; selecting it again would recenter it.
    if (retained.restore && (retained.label !== "meetings.workspace_back_answer" || !retained.turn.isConnected)) requestAnimationFrame(retained.restore);
  }
  async function openRequestedSource() {
    const navigation = ++sourceNavigation;
    const source = ctx.sourceTarget;
    if (!source && ctx.jumpMs === undefined) return;
    if (finalized) await loadReview();
    if (!root.isConnected || navigation !== sourceNavigation) return;
    let matched = false;
    if (source?.kind === "notes") {
      const range = sourceRange(editor.value, source.text);
      if (range) { selectPane("notes", true); showPreview(false); editor.focus({ preventScroll: true }); editor.setSelectionRange(range.start, range.end); matched = true; }
    } else if (source?.kind === "summary") {
      selectPane("summary", true); matched = summary.focusSource(source.text);
    } else {
      const candidates = status.transcript.map((segment, index) => ({ ...segment, index, text: `${sourceSpeakerLabel(segment, reviewDetail ?? undefined)}: ${segment.text.trim()}` })).filter(segment => !segment.failed);
      const stale = source?.meeting_uid && source.meeting_uid !== uid || source?.utterance_ids && status.metadata_status && status.metadata_status !== "valid";
      const range = source && !stale ? transcriptSourceBlocks(candidates, source) : null;
      const target = ctx.jumpMs === undefined ? null : status.transcript.find(segment => segment.start_ms <= ctx.jumpMs! && segment.end_ms > ctx.jumpMs!)
        ?? status.transcript.find(segment => segment.start_ms >= ctx.jumpMs!);
      const indices = range ? candidates.slice(range.first, range.last + 1).map(segment => segment.index) : !source && target ? [status.transcript.indexOf(target)] : [];
      if (indices.length) {
        pauseFollowing(); selectPane("transcript", true);
        const elements = indices.flatMap(index => { const row = root.querySelector<HTMLElement>(`[data-transcript-index="${index}"]`); return row ? [row] : []; });
        elements.forEach(element => element.classList.add("live-source-highlight"));
        elements[0]?.scrollIntoView({ block: "center" }); matched = elements.length > 0;
      }
    }
    if (source && !matched) { sourceNotice.replaceChildren(h("p", { class: "help" }, l("meetings.source_changed")), h("blockquote", {}, source.text)); sourceNotice.hidden = false; }
  }
  if (initialDetail) failedRecoveryModel.observe(initialDetail);
  updateStatus(initial);
  updateSaving();
  updatePanes();
  return owner;
}
