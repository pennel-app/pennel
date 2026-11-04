import { bindLocalization, l, t, formatNumber, localizeBackendText, type UiText } from "../i18n.ts";
import { meetingDate as dateLabel, speakerLabel, speakerUiLabel, sourceSpeakerLabel, templateUiLabel } from "../meeting-labels.ts";
import { api, fixtureClient, listen, type MeetingDetail, type MeetingFinished, type MeetingStatus, type MeetingVoice, type Segment, type SpeakerRecognition, type MeetingRetranscriptionStatus, type MeetingTemplate, type SpeakerSetup } from "../api";
import { SerialQueue, orFallback, persistAll } from "../async";
import { controlButton, textField, askText, confirmDialog, errorText, formatClock, h, markdown, menuButton, select } from "../dom";
import { recognitionSummary } from "../speaker-state";
import { MeetingDraft } from "../meeting-draft";
import type { Context } from "../main";
import { icon } from "../icons";
import { languageModelConfigured } from "../presentation";
import { VisibilityPoll } from "../window-lifecycle";
import { sourceBlocks, sourceRange, transcriptSourceBlocks } from "../source-navigation";
import { processingSummary } from "../processing-state";
import { frozenRecognitionLabel, meetingProcessingSummary, processingLabel, retainedMeetingFailure } from "../recording-processing";
import { retranscriptionControl, retranscriptionPercent } from "../meeting-retranscription";
import { summaryControl } from "../voice-flow-state";
import { segmentKeys, sameSegment } from "../recording-refresh";
import { attributionEditable, attributionNoticeKey, segmentAudioAvailable } from "../meeting-source";
import { reviewedInsights, flushReviewedInsights } from "../reviewed-insights";
import { insightSourceMatches, insertionStillCurrent, utf8Range, type InsightNoteLatch, type InsightSource } from "../../../../assets/contracts/meeting-insights.ts";
import { liveMeetingWorkspace, retainedMeetingWorkspace, getRetainedMeetingWorkspace, acknowledgeRetainedNotes, flushLiveWorkspaces, finishLiveWorkspace, displayedLiveWorkspaceSession } from "../live-workspace";
import { openPeoplePicker, peopleNoteControls } from "../people-picker";
import { canAcknowledgePeopleNotes, speakerLinkUtteranceIds, type PeopleSnapshot } from "../people-state";
import { conversationPanel, flushConversations, restoreConversationPanel } from "../conversation-panel";
import type { ConversationScope } from "../../../../assets/contracts/meeting-conversations";

const notesQueue = new SerialQueue();
const drafts = new Map<string, MeetingDraft>();
const liveDraft = new MeetingDraft();
const liveTitleDraft = new MeetingDraft();
const titleQueue = new SerialQueue();
let liveMeetingId: string | null = null;
const sessionMeetingIds = new Map<string, string>();
type MeetingTab = "notes" | "transcript" | "ask" | "insights";
const detailState = new Map<string, { tab: MeetingTab; template: string; working: boolean; message: string; recognition?: SpeakerRecognition;
  scrollTop: Record<MeetingTab, number>; ownNotesOpen?: boolean; noteSelection?: { start: number; end: number; direction: "forward" | "backward" | "none" } }>();
let answerReturn: { scope: string | null; conversationScope: ConversationScope; conversationId: string; messageId: string; sourceMeetingId: string } | null = null;
let panelNumber = 0;

function schedule(ctx: Context, interval: number, work: () => Promise<void>) {
  const poll = new VisibilityPoll(interval, work, ctx.report);
  poll.setVisible(ctx.visible, false);
  ctx.onVisibilityChange((visible) => poll.setVisible(visible));
  ctx.onDispose(() => poll.dispose());
}

function draftFor(id: string, notes: string) {
  if (liveMeetingId === id && liveDraft.dirty) return liveDraft;
  let draft = drafts.get(id);
  if (!draft) { draft = new MeetingDraft(); drafts.set(id, draft); }
  if (!draft.dirty) draft.sync(id, notes);
  return draft;
}

async function saveDraft(draft: MeetingDraft, meetingId: string) {
  const receipt = draft.insightReceipt;
  await draft.persist((value) => notesQueue.run(async () => { await api.saveMeetingNotes(meetingId, value, receipt?.text === value ? receipt.attempt : undefined); acknowledgeRetainedNotes(meetingId, value); }));
}

async function flushLiveTitle() {
  const sessionId = liveTitleDraft.sessionId;
  if (!sessionId) return;
  await liveTitleDraft.persist((title) => titleQueue.run(() => api.setMeetingTitle(title, sessionId)));
}

export async function flushMeetingDrafts(includeLiveWorkspaces = true) {
  await persistAll([
    flushReviewedInsights,
    flushConversations,
    ...includeLiveWorkspaces ? [flushLiveWorkspaces] : [],
    async () => { await titleQueue.drain(); await flushLiveTitle(); },
    async () => {
      await notesQueue.drain();
      await persistAll([
        ...[...drafts].map(([id, draft]) => () => saveDraft(draft, id)),
        async () => { if (liveMeetingId) await saveDraft(liveDraft, liveMeetingId); },
      ]);
      await notesQueue.drain();
    },
  ]);
}

export function displayedMeetingSession(): string | null {
  return displayedLiveWorkspaceSession() ?? liveDraft.sessionId;
}

export function finishMeetingDrafts(event: MeetingFinished) {
  finishLiveWorkspace(event);
  if (!event.id) return;
  const oldId = sessionMeetingIds.get(event.session_id);
  if (oldId && oldId !== event.id && drafts.has(oldId)) {
    drafts.set(event.id, drafts.get(oldId)!);
    drafts.delete(oldId);
  }
  if (oldId && oldId !== event.id) {
    const reading = detailState.get(oldId);
    if (reading) { detailState.set(event.id, reading); detailState.delete(oldId); }
    if (answerReturn?.scope === oldId) answerReturn.scope = event.id;
    if (answerReturn?.sourceMeetingId === oldId) answerReturn.sourceMeetingId = event.id;
  }
  sessionMeetingIds.set(event.session_id, event.id);
  if (liveDraft.sessionId === event.session_id) liveMeetingId = event.id;
}

function editNotes(ctx: Context, draft: MeetingDraft, id: string, rows = 8) {
  const input = h("textarea", { rows, placeholder: l("meetings.notes_placeholder"), "aria-label": l("meetings.your_notes") });
  const saveError = h("p", { class: "error insight-note-save-error", hidden: true, role: "status" }, l("insights.error_notes_not_saved"));
  input.value = draft.value;
  const draftOwner = draft.sessionId;
  let timer: number | undefined;
  let composing = false, revision = 0, selectionRevision = 0, selection = "";
  const observeSelection = () => { const next = `${input.selectionStart}:${input.selectionEnd}`; if (selection !== next) { selection = next; ++selectionRevision; } };
  input.addEventListener("select", observeSelection);
  input.addEventListener("compositionstart", () => { composing = true; ++revision; window.clearTimeout(timer); });
  input.addEventListener("compositionend", () => { composing = false; ++revision; draft.edit(input.value); timer = window.setTimeout(() => { void flush().catch(ctx.report); }, 600); });
  input.addEventListener("input", () => {
    ++revision;
    draft.edit(input.value);
    window.clearTimeout(timer);
    if (!composing) timer = window.setTimeout(() => { void flush().catch(ctx.report); }, 600);
  });
  const flush = async () => { window.clearTimeout(timer); if (composing) throw new Error(t("meetings.workspace_composing")); await saveDraft(draft, id); saveError.hidden = true; };
  ctx.beforeLeave(flush);
  ctx.onFlush(flush);
  input.addEventListener("blur", () => { void flush().catch(ctx.report); });
  ctx.onDispose(() => { void flush().catch(ctx.report); });
  ctx.onDispose(draft.subscribe(() => {
    if (draft.sessionId !== draftOwner || composing || input.value === draft.value) return;
    const start = input.selectionStart, end = input.selectionEnd, direction = input.selectionDirection, scroll = input.scrollTop;
    input.value = draft.value; input.setSelectionRange(start, end, direction); input.scrollTop = scroll;
  }));
  const current = (): InsightNoteLatch => {
    observeSelection();
    return { text: draft.value, revision, selection_start: input.selectionStart, selection_end: input.selectionEnd, selection_revision: selectionRevision, composing };
  };
  const people = peopleNoteControls({ id: () => id, platform: ctx.info.platform, valid: () => input.isConnected && !input.disabled,
    report: ctx.report, notes: { current, flush, saved: (result, before) => {
      if (result.notes === undefined) return;
      if (!canAcknowledgePeopleNotes(before, current(), result.notes, draftOwner, draft.sessionId)) { ctx.report(new Error(t("people.selection_changed"))); return; }
      ++revision; draft.sync(id, result.notes); input.value = result.notes; acknowledgeRetainedNotes(id, result.notes);
      input.setSelectionRange(result.selection_start ?? before.selection_start, result.selection_end ?? before.selection_end);
      saveError.hidden = true;
    } } });
  return { input, people, error: saveError, flush, current, stage: async (value: string, attempt: string, before: InsightNoteLatch): Promise<boolean> => {
    if (composing || !draft.stageInsightInsertion(value, attempt, before.text)) return false;
    ++revision; input.value = value; input.focus({ preventScroll: true }); input.setSelectionRange(before.text.length, value.length);
    try { await flush(); return true; } catch { saveError.hidden = false; return false; }
  } };
}

async function nameVoice(id: string, voice: number, suggested: string, remembering = false): Promise<boolean> {
  const name = await askText(l(remembering ? "people.remember" : "people.rename"), suggested,
    l(remembering ? "meetings.voice_remember_help" : "meetings.voice_name_help"));
  if (name === null || remembering && !name) return false;
  if (remembering) await api.rememberSpeaker(id, voice, name);
  else await api.renameSpeaker(id, voice, name);
  return true;
}

function transcript(ctx: Context, initial: Segment[], detail?: MeetingDetail, id?: string, initialVoices: MeetingVoice[] | null = []) {
  const element = h("div", { class: "transcript" });
  let voices = initialVoices;
  let people: PeopleSnapshot | null = null;
  let disposed = false;
  ctx.onDispose(() => { disposed = true; });
  const rows = new Map<string, { segment: Segment; element: HTMLElement; update: (segment: Segment, index: number, speakersChanged: boolean) => void }>();
  const empty = h("p", { class: "muted" }, l("meetings.empty_transcript"));
  const create = (initialSegment: Segment, index: number) => {
    let segment = initialSegment;
    const play = ctx.info.capabilities.audio_playback;
    const available = () => !!detail && segmentAudioAvailable(segment, detail.audio_tracks, detail.metadata_status);
    const stamp = h("button", { class: "transcript-stamp text-button", disabled: !id || !available() || play.status !== "available",
      title: available() ? play.reason ? localizeBackendText(play.reason) : l("meetings.play_from_here") : l("meetings.passage_audio_unavailable") }, formatClock(segment.start_ms / 1000));
    if (play.reason) bindLocalization(stamp, () => { stamp.title = available() ? localizeBackendText(play.reason!) : t("meetings.passage_audio_unavailable"); });
    if (id) stamp.addEventListener("click", () => {
      const work = segment.utterance_id ? api.playMeetingUtteranceAudio(id, segment.utterance_id) : api.playMeetingAudio(id, segment.speaker, segment.start_ms);
      void work.catch(ctx.report);
    });
    const makeSpeaker = () => {
      const speakerColor = segment.attribution && segment.attribution.kind !== "single" ? "var(--foreground)" : segment.speaker === "me" ? "var(--accent)" : segment.voice === null ? "var(--foreground)" : `var(--voice-${(segment.voice + 3) % 4 + 1})`;
      let speaker: HTMLElement = h("span", { class: "speaker", style: `color:${speakerColor}` }, speakerUiLabel(segment, detail));
      const canRename = segment.voice !== null && segment.speaker === "others" && voices !== null;
      if (id && (segment.utterance_id || canRename) && (!segment.attribution || segment.attribution.kind === "single") && attributionEditable(detail?.metadata_status)) {
        const currentName = segment.confirmed_speaker_name ?? detail?.speakers.find((item) => item.voice === segment.voice)?.name ?? "";
        const voice = voices?.find((item) => item.voice === segment.voice);
        const offerRemember = canRename && !!voice?.can_remember;
        const picker = select([
          ["current", speakerUiLabel(segment, detail)], ...segment.utterance_id ? [["person", l("people.link_title")] as [string, UiText]] : [],
          ...canRename ? [["rename", l("people.rename")] as [string, UiText]] : [],
          ...offerRemember ? [["remember", l("people.remember")] as [string, UiText]] : [],
        ], "current", (value) => {
          void (async () => {
            try {
              if (value === "person") {
                const ids = speakerLinkUtteranceIds(segment, detail?.transcript ?? [segment]);
                if (!ids.length) throw new Error(t("people.no_stable_speaker"));
                await openPeoplePicker({ id: () => id, platform: ctx.info.platform, valid: () => !disposed, report: ctx.report,
                  changed: snapshot => { people = snapshot; updateLinkedStates(); } }, { kind: "speaker", utteranceIds: ids });
              } else if (value !== "current" && await nameVoice(id, segment.voice!, currentName, value === "remember")) ctx.rerender();
              picker.value = "current";
            } catch (error) { picker.value = "current"; ctx.report(error); }
          })();
        }, "menu");
        picker.classList.add("speaker", "speaker-picker");
        picker.style.color = speakerColor;
        const relabelPicker = () => picker.setAttribute("aria-label", t("meetings.voice_name_label", { name: speakerLabel(segment, detail) }));
        relabelPicker();
        bindLocalization(picker, relabelPicker);
        speaker = picker;
      }
      return speaker;
    };
    let speaker = makeSpeaker();
    const body = h("div", { class: "transcript-copy" });
    const drawBody = () => {
      body.replaceChildren(segment.failed ? h("span", { class: "muted" }, l("meetings.transcription_failed")) : document.createTextNode(segment.text));
      if (segment.failed && id) {
        const retry = h("button", { class: "text-button", onclick: async () => {
          retry.disabled = true;
          try { await api.retranscribeMeeting(id); ctx.rerender(); }
          catch (error) { ctx.report(error); retry.disabled = false; }
        } }, l("common.retry"));
        body.append(retry);
      }
    };
    drawBody();
    const provisional = h("span", { class: "live-speaker-provisional", hidden: segment.attribution?.provisional !== true || !!segment.confirmed_speaker_name,
      title: l("meetings.live_speakers_provisional_help") }, l("meetings.live_speakers_provisional_tag"));
    const correction = h("p", { class: "help transcript-correction-review", hidden: !segment.correction_review_required }, l("meetings.attribution_correction_review"));
    const linked = h("span", { class: "people-link-state", hidden: true });
    const updateLink = () => {
      const link = people?.speaker_links.find(link => !!segment.utterance_id && link.utterance_ids.includes(segment.utterance_id));
      linked.hidden = !link; linked.textContent = link ? t(link.state === "needs_review" ? "people.review" : "people.linked") : "";
    };
    bindLocalization(linked, updateLink);
    const row = h("div", { class: "transcript-line", "data-start-ms": segment.start_ms, "data-transcript-index": index,
      "data-source-id": segment.source_id ?? "", title: segment.source_id ? l(`meetings.attribution_source_${segment.source_id}`) : "" },
      stamp, h("div", { class: "live-transcript-speaker" }, speaker, provisional, linked), body, correction);
    updateLink();
    return { segment, element: row, update: (next: Segment, nextIndex: number, speakersChanged: boolean) => {
      const bodyChanged = next.text !== segment.text || next.failed !== segment.failed;
      const speakerChanged = speakersChanged || next.voice !== segment.voice || next.speaker !== segment.speaker
        || next.confirmed_speaker_name !== segment.confirmed_speaker_name
        || JSON.stringify(next.attribution ?? null) !== JSON.stringify(segment.attribution ?? null);
      if (next.start_ms !== segment.start_ms) { stamp.textContent = formatClock(next.start_ms / 1000); row.dataset.startMs = String(next.start_ms); }
      segment = next;
      stamp.disabled = !id || !available() || play.status !== "available";
      stamp.title = available() ? play.reason ? localizeBackendText(play.reason) : t("meetings.play_from_here") : t("meetings.passage_audio_unavailable");
      correction.hidden = !segment.correction_review_required;
      provisional.hidden = segment.attribution?.provisional !== true || !!segment.confirmed_speaker_name;
      row.dataset.sourceId = segment.source_id ?? "";
      row.title = segment.source_id ? t(`meetings.attribution_source_${segment.source_id}`) : "";
      if (bodyChanged) drawBody();
      updateLink();
      if (speakerChanged) { const replacement = makeSpeaker(); speaker.replaceWith(replacement); speaker = replacement; }
      row.dataset.transcriptIndex = String(nextIndex);
    } };
  };
  function updateLinkedStates() {
    for (const row of rows.values()) {
      const linked = row.element.querySelector<HTMLElement>(".people-link-state");
      if (!linked) continue;
      const link = people?.speaker_links.find(link => !!row.segment.utterance_id && link.utterance_ids.includes(row.segment.utterance_id));
      linked.hidden = !link; linked.textContent = link ? t(link.state === "needs_review" ? "people.review" : "people.linked") : "";
    }
  }
  if (id) void api.meetingPeople(id).then(snapshot => { if (!disposed) { people = snapshot; updateLinkedStates(); } }).catch(() => undefined);
  const update = (segments: Segment[], nextVoices = voices) => {
    const speakersChanged = nextVoices !== voices;
    voices = nextVoices;
    const keys = segmentKeys(segments);
    const retained = new Set(keys);
    for (const [key, row] of rows) if (!retained.has(key)) { row.element.remove(); rows.delete(key); }
    if (segments.length) empty.remove();
    else if (!empty.isConnected || empty.parentNode !== element) element.append(empty);
    let before = element.firstElementChild;
    const appended = document.createDocumentFragment();
    for (const [index, segment] of segments.entries()) {
      const key = keys[index];
      let row = rows.get(key);
      if (!row) { row = create(segment, index); rows.set(key, row); }
      else if (speakersChanged || !sameSegment(row.segment, segment) || row.element.dataset.transcriptIndex !== String(index)) {
        row.update(segment, index, speakersChanged); row.segment = segment;
      }
      if (!before && !row.element.parentNode) appended.append(row.element);
      else {
        if (before !== row.element) element.insertBefore(row.element, before);
        before = row.element.nextElementSibling;
      }
    }
    element.append(appended);
  };
  update(initial);
  return { element, update };
}

function recoveryNotice(ctx: Context, status: MeetingStatus, latest = () => status, includeTranscription = true): HTMLElement | null {
  const primary = status.persistence_error ?? status.terminal_error ?? status.error;
  let actionError: string | null = null;
  const warnings = () => actionError ? [localizeBackendText(actionError)] : [...new Set([primary, includeTranscription ? status.transcription_warning : null].filter((message): message is string => !!message).map(localizeBackendText))];
  if (!warnings().length && !status.can_retry_persistence) return null;
  const message = h("p", { class: "error", role: status.terminal_error ? "alert" : "status" }, warnings().join(" "));
  bindLocalization(message, () => { message.textContent = warnings().join(" "); });
  const actions = h("div", { class: "row" });
  if (status.can_retry_persistence && status.session_id) {
    const retry = controlButton({ class: "btn", onclick: async () => {
      retry.disabled = true;
      try { await flushMeetingDrafts(); await api.retryMeetingPersistence(status.session_id!); ctx.rerender(); }
      catch (error) { actionError = errorText(error); message.textContent = localizeBackendText(actionError); retry.disabled = false; }
    } }, l("meetings.retry_save"));
    actions.append(retry);
  }
  if (status.transcript.some((segment) => !segment.failed && segment.text.trim())) {
    const copied = h("span", { class: "help", role: "status" });
    actions.append(controlButton({ class: "btn", onclick: async () => {
      try {
        await api.copyText(latest().transcript.filter((segment) => !segment.failed).map((segment) => `${formatClock(segment.start_ms / 1000)} ${speakerLabel(segment)}: ${segment.text}`).join("\n"));
        copied.replaceChildren(h("span", {}, l("meetings.copied_transcript")));
      } catch (error) { actionError = errorText(error); message.textContent = localizeBackendText(actionError); }
    } }, l("meetings.copy_transcript")), copied);
  }
  return h("div", { class: "meeting-recovery-notice" }, message, actions);
}

function meetingProcessingNotice(ctx: Context, latest: () => MeetingStatus | undefined = () => undefined, active = false) {
  const element = h("p", { class: "help processing-path" });
  const refresh = () => {
    const label = processingLabel(meetingProcessingSummary(processingSummary(ctx.info.settings, "meeting"), latest()?.recognition, active));
    if (element.textContent !== label) element.textContent = label;
  };
  ctx.onInfoChange(refresh);
  bindLocalization(element, refresh);
  refresh();
  return { element, refresh };
}

function chat(ctx: Context, meetingId: string | null, meetingUid?: string, prepare?: () => Promise<{ expected_notes_revision?: string }>) {
  const panel = conversationPanel(ctx, {
    scope: meetingId ? { kind: "meeting", meeting_uid: meetingUid ?? meetingId, meeting_id: meetingId } : { kind: "library" },
    prepare,
    source: (source, _origin, message) => {
      if (source.status !== "current" || !source.current_meeting_id) return false;
      answerReturn = { scope: meetingId, conversationScope: panel.model.scope, conversationId: message.conversation_id, messageId: message.id, sourceMeetingId: source.current_meeting_id };
      ctx.navigate("meetings", source.current_meeting_id, source.passage.kind === "transcript" ? source.passage.start_ms ?? undefined : undefined,
        { kind: source.passage.kind, start_ms: source.passage.start_ms, text: source.passage.text,
          ...(source.passage.meeting_uid ? { meeting_uid: source.passage.meeting_uid } : {}),
          ...(source.passage.utterance_ids != null ? { utterance_ids: source.passage.utterance_ids, utterance_fingerprints: source.passage.utterance_fingerprints } : {}) });
      return true;
    },
  });
  return panel.root;
}

export async function meetingsView(ctx: Context): Promise<HTMLElement> {
  if (ctx.param === "__chat__") return h("div", { class: "meeting-selected" }, chat(ctx, null));
  if (ctx.param) {
    const retained = ctx.review ? null : retainedMeetingWorkspace(ctx, ctx.param);
    if (retained) return retained;
    try { return h("div", { class: "meeting-selected" }, await meetingDetail(ctx, ctx.param)); }
    catch (error) { return h("div", { class: "empty-state" }, h("p", { class: "error" }, errorText(error)), controlButton({ class: "btn", onclick: () => ctx.navigate("meetings") }, l("meetings.back_meetings"))); }
  }
  const meetings = await api.meetings("");
  if (meetings.length) return h("div", { class: "empty-state library-placeholder" }, h("p", { class: "muted" }, l("meetings.choose_or_record")));
  return h("div", { class: "empty-state meeting-library-empty" }, icon("notebook"), h("h2", {}, l("meetings.empty_meetings")),
    h("p", { class: "muted" }, l("meetings.empty_meetings_help")),
    controlButton({ class: "btn primary", onclick: () => ctx.navigate("live") }, l("meetings.first_recording")));
}

async function meetingDetail(ctx: Context, id: string): Promise<HTMLElement> {
  const retained = ctx.review ? null : retainedMeetingWorkspace(ctx, id);
  if (retained) return retained;
  let disposed = false;
  ctx.onDispose(() => { disposed = true; });
  const reportOptional = (error: unknown) => { if (!disposed) ctx.report(error); };
  let pendingRetranscription: MeetingRetranscriptionStatus | null | undefined;
  let receiveRetranscription: ((status: MeetingRetranscriptionStatus | null) => void) | undefined;
  const subscription = listen<{ id: string; status: MeetingRetranscriptionStatus }>("meeting-retranscription-changed", (event) => {
    if (event.payload.id !== id) return;
    if (receiveRetranscription) receiveRetranscription(event.payload.status);
    else pendingRetranscription = event.payload.status;
  });
  ctx.onDispose(() => { void subscription.then((unsubscribe) => unsubscribe()).catch(reportOptional); });
  const [detail, runtimeMetadata] = await Promise.all([
    subscription.then(() => api.meetingDetail(id), (error) => { reportOptional(error); return api.meetingDetail(id); }), api.meetingRuntimeStatus(),
  ]);
  if (!detail.info) return h("div", { class: "empty-state muted" }, l("meetings.not_found"));
  const meetingInfo = detail.info;
  const runtime: MeetingStatus = { ...runtimeMetadata, notes: detail.notes, transcript: detail.transcript };
  const voicesReady = orFallback(() => api.meetingVoices(id), [], reportOptional);
  const speakerSetupReady = orFallback(() => api.speakerSetup(), null, reportOptional);
  const templatesReady = orFallback(() => api.meetingTemplates(), [], reportOptional);
  const markedLines = detail.summary ? orFallback(() => api.markLines(detail.summary!, detail.notes), [], reportOptional) : Promise.resolve([]);
  const conflictsReady = ctx.info.settings.cloud_sync === true ? orFallback(() => api.meetingConflictFiles(id), [], reportOptional) : Promise.resolve([]);
  const active = runtime.busy && runtime.id === id;
  let state = detailState.get(id);
  const firstVisit = !state;
  if (!state) { state = { tab: "notes", template: ctx.info.settings.default_template, working: false, message: "", scrollTop: { notes: 0, transcript: 0, ask: 0, insights: 0 } }; detailState.set(id, state); }
  const fixtureTab = fixtureClient()?.fixture?.meeting_tab;
  if (firstVisit && ["notes", "transcript", "ask", "insights"].includes(String(fixtureTab))) state.tab = fixtureTab as MeetingTab;
  const current = state;
  const playback = ctx.info.capabilities.audio_playback;
  const audioAvailable = detail.audio_tracks.some((track) => track.available);
  const audioReason = () => !audioAvailable ? t("meetings.no_saved_audio") : playback.status !== "available" ? playback.reason ? localizeBackendText(playback.reason) : t("meetings.audio_unavailable") : null;
  const metadataReason = () => {
    const key = attributionNoticeKey(detail.metadata_status);
    return !attributionEditable(detail.metadata_status) && key ? t(key) : null;
  };
  let voices: MeetingVoice[] | null = null;
  let speakerSetup: SpeakerSetup | null = null;
  let speakersLoaded = false;
  let templatesLoaded = false;
  let templates: MeetingTemplate[] = [];
  const remembered = h("p", { class: "help remembered-voices", hidden: true });
  let transcriptContent: ReturnType<typeof transcript> | null = null;
  let transcriptControls: HTMLElement | null = null;
  if (ctx.sourceTarget) current.tab = ctx.sourceTarget.kind === "transcript" ? "transcript" : "notes";
  else if (ctx.jumpMs !== undefined) current.tab = "transcript";
  const notesDraft = draftFor(id, detail.notes);
  const notes = editNotes(ctx, notesDraft, id);
  if (current.noteSelection) notes.input.setSelectionRange(current.noteSelection.start, current.noteSelection.end, current.noteSelection.direction);
  const rememberNoteSelection = () => { current.noteSelection = { start: notes.input.selectionStart, end: notes.input.selectionEnd, direction: notes.input.selectionDirection }; };
  notes.input.addEventListener("select", rememberNoteSelection);
  notes.input.addEventListener("blur", rememberNoteSelection);
  notes.input.disabled = active;
  if (active) {
    notes.input.title = t("meetings.edit_recording_notes");
    bindLocalization(notes.input, () => { notes.input.title = t("meetings.edit_recording_notes"); });
  }
  const enhanced = h("div", { class: "enhanced-notes" });
  const summaryBlocks: { text: string; element: HTMLElement }[] = [];
  if (detail.summary) {
    const lines = detail.summary.split(/\r?\n/);
    if (lines.at(-1) === "") lines.pop();
    for (const text of lines) {
      const rendered = markdown(text);
      rendered.classList.add("generated-line");
      enhanced.append(rendered);
      summaryBlocks.push({ text, element: rendered });
    }
    void markedLines.then((marked) => {
      if (disposed) return;
      marked.forEach((line, index) => {
        const block = summaryBlocks[index];
        if (!block || block.text !== line.text) return;
        block.element.classList.toggle("own-line", line.mine);
        block.element.classList.toggle("generated-line", !line.mine);
      });
    });
  }
  const panelId = `meeting-panel-${++panelNumber}`;
  let retranscriptionStatus = detail.retranscription_status;
  let ownRetranscription = false;
  const reprocess = controlButton({ class: "btn" }, l("meetings.retranscribe_audio"));
  const reprocessReason = h("p", { class: "help meeting-control-reason" });
  const refreshRetranscription = () => {
    const control = retranscriptionControl(detail.retranscription_plan, retranscriptionStatus, runtime.busy, current.working);
    const reason = metadataReason() ?? (control.reason ? localizeBackendText(control.reason) : null);
    reprocess.disabled = control.disabled || !!metadataReason();
    reprocess.title = reason ?? "";
    reprocessReason.textContent = reason ?? "";
    reprocessReason.hidden = !reason || !!metadataReason();
    notes.input.disabled = active || !!retranscriptionStatus?.running;
  };
  const tabs = h("div", { class: "segmented detail-tabs", role: "tablist", "aria-label": l("meetings.default_title") });
  const panel = h("section", { id: panelId, class: "meeting-panel", role: "tabpanel", tabindex: 0 });
  let renderedTab = current.tab;
  let drawGeneration = 0;
  let questions: HTMLElement | null = null;
  let insights: ReturnType<typeof reviewedInsights> | null = null;
  let sourceNavigation = 0, interactionRevision = 0;
  let restoreInsightSource: (() => void) | null = null;
  const insightBack = h("button", { class: "text-button insight-source-back", hidden: true, onclick: () => {
    const restore = restoreInsightSource; restoreInsightSource = null; insightBack.hidden = true; chooseTab("insights"); if (restore) requestAnimationFrame(restore);
  } }, l("insights.back"));
  const rememberPosition = () => { current.scrollTop[renderedTab] = panel.scrollTop; };
  panel.addEventListener("scroll", rememberPosition);
  ctx.beforeLeave(async () => { rememberPosition(); rememberNoteSelection(); });
  ctx.onDispose(rememberPosition);
  const chooseTab = (tab: MeetingTab) => { ++sourceNavigation; rememberPosition(); current.tab = tab; draw(); };
  const draw = () => {
    const generation = ++drawGeneration;
    renderedTab = current.tab;
    const scrollTop = current.scrollTop[renderedTab];
    requestAnimationFrame(() => { if (panel.isConnected && generation === drawGeneration) panel.scrollTop = scrollTop; });
    footer.hidden = current.tab === "ask" || current.tab === "insights";
    refreshSummaryControls();
    diarize.hidden = current.tab !== "transcript";
    diarizeReason.hidden = current.tab !== "transcript" || !speakerReason();
    tabs.querySelectorAll("button").forEach((button) => {
      button.setAttribute("aria-selected", String(button.dataset.tab === current.tab));
      button.tabIndex = button.dataset.tab === current.tab ? 0 : -1;
    });
    panel.setAttribute("aria-labelledby", `${panelId}-${current.tab}`);
    if (current.tab === "ask") { questions ??= chat(ctx, id, detail.meeting_uid, async () => {
      await notes.flush();
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(notes.current().text));
      return { expected_notes_revision: [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("") };
    }); panel.replaceChildren(questions); return; }
    if (current.tab === "insights") {
      if (active) { panel.replaceChildren(h("p", { class: "help" }, l("insights.finished_only"))); return; }
      insights ??= reviewedInsights(ctx, { id, meetingUid: detail.meeting_uid ?? id, notes: {
        flush: async () => { try { await notes.flush(); return true; } catch { return false; } },
        current: () => ({ ...notes.current(), interaction_revision: interactionRevision }),
        stage: async (value, attempt, before) => {
          if (!insertionStillCurrent(before, { ...notes.current(), interaction_revision: interactionRevision })) return false;
          chooseTab("notes"); const disclosure = result.querySelector<HTMLDetailsElement>(".own-notes"); if (disclosure) disclosure.open = true;
          return notes.stage(value, attempt, before);
        },
      }, openSource: async (source: InsightSource, origin, restore) => {
        const navigation = ++sourceNavigation; const fresh = await api.meetingDetail(id);
        if (navigation !== sourceNavigation || !result.isConnected) return true;
        const sourceIndex = fresh.transcript.findIndex(segment => segment.utterance_id === source.utterance_id);
        if (!insightSourceMatches(source, fresh.transcript[sourceIndex], fresh.meeting_uid ?? id)) return false;
        detail.transcript = fresh.transcript; detail.speakers = fresh.speakers; detail.metadata_status = fresh.metadata_status;
        transcriptContent?.update(fresh.transcript, voices); chooseTab("transcript");
        const row = result.querySelector<HTMLElement>(`[data-transcript-index="${sourceIndex}"]`);
        row?.classList.add("source-highlight"); row?.scrollIntoView({ block: "center" });
        const text = row?.querySelector(".transcript-copy")?.firstChild, range = utf8Range(fresh.transcript[sourceIndex].text, source.start_byte, source.end_byte);
        if (text instanceof Text && range) { const selected = document.createRange(); selected.setStart(text, range.start); selected.setEnd(text, range.end); window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(selected); }
        restoreInsightSource = () => { restore(); origin.focus({ preventScroll: true }); }; insightBack.hidden = false; return true;
      } });
      panel.replaceChildren(insights.root); void insights.refresh(); return;
    }
    if (current.tab === "transcript") {
      if (!transcriptControls) {
        const playbackReason = h("span", { class: audioReason() ? "help" : "grow" }, audioReason());
        bindLocalization(playbackReason, () => { playbackReason.textContent = audioReason(); });
        const stopAudio = h("button", { class: "text-button", disabled: !!audioReason(), title: audioReason(), onclick: () => api.stopAudio() }, l("meetings.stop_audio"));
        bindLocalization(stopAudio, () => { stopAudio.title = audioReason() ?? ""; });
        transcriptControls = h("div", { class: "transcript-tools" }, playbackReason, stopAudio, reprocess);
      }
      transcriptContent ??= transcript(ctx, detail.transcript, detail, id, voices);
      refreshRetranscription();
      const metadataNotice = attributionNoticeKey(detail.metadata_status);
      panel.replaceChildren(transcriptControls, reprocessReason, remembered,
        ...(metadataNotice ? [h("p", { class: "help attribution-notice" }, l(metadataNotice))] : []), transcriptContent.element);
      return;
    }
    panel.replaceChildren(detail.summary ? enhanced : detail.notes.trim() ? h("div", { class: "markdown" }, detail.notes) : h("p", { class: "muted" }, l("meetings.empty_summary")));
    const ownNotes = h("details", { class: "own-notes", open: current.ownNotesOpen ?? !detail.summary }, h("summary", {}, l("meetings.your_notes")), notes.people, notes.input, notes.error);
    ownNotes.addEventListener("toggle", () => { if (ownNotes.isConnected) current.ownNotesOpen = ownNotes.open; });
    panel.append(ownNotes);
  };
  for (const [tab, label] of [["notes", l("meetings.notes")], ["transcript", l("meetings.transcript")], ["ask", l("meetings.questions")], ["insights", l("insights.title")]] as const) {
    const button = h("button", { id: `${panelId}-${tab}`, role: "tab", "aria-controls": panelId, "data-tab": tab, onclick: () => { chooseTab(tab); } }, label);
    button.addEventListener("keydown", (event) => {
      const order = ["notes", "transcript", "ask", "insights"] as const;
      const index = order.indexOf(current.tab);
      const next = event.key === "ArrowRight" ? (index + 1) % order.length : event.key === "ArrowLeft" ? (index + order.length - 1) % order.length
        : event.key === "Home" ? 0 : event.key === "End" ? order.length - 1 : -1;
      if (next < 0) return;
      event.preventDefault(); chooseTab(order[next]);
      tabs.querySelector<HTMLButtonElement>(`[data-tab="${current.tab}"]`)?.focus();
    });
    tabs.append(button);
  }
  let sourceChanged = false;
  let recognizing = false;
  const progressText = (status: MeetingRetranscriptionStatus) => {
    const percent = retranscriptionPercent(status.progress);
    const track = t(status.progress.speaker === "me" ? "meetings.retranscription_microphone" : "meetings.retranscription_computer");
    if (status.progress.phase === "transcribing" && percent !== null) return t("meetings.retranscription_progress", { track, percent });
    if (status.progress.phase === "speakers") return t("meetings.recognizing_speakers");
    if (status.progress.phase === "saving") return t("meetings.retranscription_saving");
    return t("meetings.retranscription_decoding");
  };
  const messageText = () => sourceChanged ? t("meetings.source_changed")
    : retranscriptionStatus?.running ? progressText(retranscriptionStatus)
    : current.working ? t(recognizing ? "meetings.recognizing_speakers" : "meetings.working")
    : current.message ? localizeBackendText(current.message)
    : current.recognition ? recognitionSummary(current.recognition)
    : retranscriptionStatus?.error ? localizeBackendText(retranscriptionStatus.error)
    : retranscriptionStatus?.warning ? localizeBackendText(retranscriptionStatus.warning)
    : retranscriptionStatus?.progress.phase === "completed" ? t("meetings.retranscription_done") : "";
  const message = h("p", { class: "meeting-message help", role: "status" }, messageText());
  bindLocalization(message, () => { message.textContent = messageText(); });
  const regenerate = controlButton({ class: "btn" });
  const setupReason = h("span", { class: "meeting-setup-reason" });
  const setupAction = h("button", { class: "text-button", onclick: async () => {
    try { await notes.flush(); rememberPosition(); ctx.openSetup("llm"); }
    catch (error) { ctx.report(error); }
  } }, l("meetings.set_up_summaries"));
  const summarySetup = h("div", { class: "meeting-setup meeting-summary-setup help", role: "status" }, setupReason, setupAction);
  const refreshSummaryControls = () => {
    const control = summaryControl(languageModelConfigured(ctx.info), runtime.busy, current.working || !!retranscriptionStatus?.running);
    const reason = control.reason === "setup" ? t("meetings.summary_setup_help") : control.reason === "recording" ? t("meetings.wait_recording") : "";
    regenerate.disabled = control.disabled || !templatesLoaded;
    regenerate.title = reason;
    regenerate.textContent = t(current.working ? "dictation.working" : detail.summary ? "meetings.regenerate" : "meetings.summarize");
    setupReason.textContent = reason;
    summarySetup.hidden = !reason || current.tab === "ask";
    setupAction.hidden = !control.canSetUp;
  };
  ctx.onInfoChange(refreshSummaryControls);
  bindLocalization(summarySetup, refreshSummaryControls);
  refreshSummaryControls();
  regenerate.addEventListener("click", async () => {
    if (!templatesLoaded || summaryControl(languageModelConfigured(ctx.info), runtime.busy, current.working || !!retranscriptionStatus?.running).disabled) return;
    recognizing = false; current.recognition = undefined; current.working = true; refreshSummaryControls(); diarize.disabled = true; message.textContent = t("dictation.working");
    try { await notes.flush(); await api.summarizeMeeting(id, current.template); current.message = ""; }
    catch (error) { current.message = errorText(error); }
    current.working = false; rememberPosition(); ctx.rerender();
  });
  const templateSlot = h("span");
  let selected = select([[current.template, l("common.loading")]], current.template, (template) => { current.template = template; }, "menu");
  const configureTemplate = () => {
    selected.setAttribute("aria-label", t("meetings.template"));
    selected.disabled = !templatesLoaded || runtime.busy || !!retranscriptionStatus?.running || current.working;
  };
  configureTemplate();
  bindLocalization(templateSlot, configureTemplate);
  templateSlot.append(selected);
  const copied = h("span", { class: "help", role: "status" });
  const copy = async (formatted: boolean, withTranscript = false) => {
    const exported = await api.exportMeeting(id, withTranscript);
    await api.copyText(exported.markdown, formatted ? exported.html : null);
    copied.replaceChildren(h("span", {}, withTranscript ? l("meetings.copied_with_transcript") : l("meetings.copied")));
  };
  const rich = ctx.info.capabilities.rich_clipboard;
  const shareAction = (work: () => Promise<unknown>) => () => work().catch(ctx.report);
  const share = menuButton(l("meetings.share"), [
    { label: l("meetings.copy_formatted"), disabled: rich.status !== "available", title: rich.reason ? localizeBackendText(rich.reason) : null, run: shareAction(() => copy(true)) },
    { label: l("meetings.copy_markdown"), run: shareAction(() => copy(false)) },
    { label: l("meetings.copy_with_transcript"), run: shareAction(() => copy(rich.status === "available", true)) },
    { label: l("meetings.attendees_action"), run: shareAction(async () => {
      const value = await askText(l("meetings.attendees"), detail.attendees.join(", "), l("meetings.attendees_help"));
      if (value === null) return;
      await api.setMeetingAttendees(id, value.split(",").map((name) => name.trim()).filter(Boolean)); ctx.rerender();
    }) },
    { label: l("meetings.email"), run: shareAction(() => api.shareMeeting(id, false, "mail")) },
    { label: l("meetings.share_action"), disabled: ctx.info.platform !== "macos",
      title: ctx.info.platform === "macos" ? l("meetings.open_share_menu") : l("meetings.share_unavailable"),
      run: shareAction(() => api.shareMeeting(id, false, "share")) },
  ], "share-menu");
  const diarization = ctx.info.capabilities.diarization;
  const cloudRoute = ctx.info.settings.speaker_recognition === "cloud";
  const speakerReason = () => metadataReason() ?? (retranscriptionStatus?.running ? t("meetings.working") : runtime.busy || meetingInfo.duration_min <= 0 ? t("meetings.finish_first")
    : diarization.status !== "available" ? diarization.reason ? localizeBackendText(diarization.reason) : t("meetings.speaker_recognition_unavailable")
    : !detail.audio_tracks.some((track) => track.available) ? t("meetings.no_saved_audio")
    : !detail.transcript.some((segment) => !segment.failed) ? t("meetings.empty_transcript")
    : !speakersLoaded ? t("common.loading")
    : !speakerSetup?.ready ? cloudRoute ? t("backend.sign_in_to_pennel_cloud_under_settings_account_first") : t("meetings.download_speaker_models")
    : null);
  const canDiarize = !speakerReason();
  const diarize = controlButton({ class: "btn", disabled: !canDiarize || current.working,
    title: speakerReason() ?? (cloudRoute ? l("meetings.speaker_cloud_help") : l("meetings.speaker_local_help")) }, l("meetings.recognize_speakers"));
  diarize.addEventListener("click", async () => {
    if (current.working || speakerReason()) return;
    recognizing = true; current.recognition = undefined; current.working = true; refreshSummaryControls(); diarize.disabled = true; message.textContent = t("meetings.recognizing_speakers");
    try { current.recognition = await api.recognizeSpeakers(id); current.message = recognitionSummary(current.recognition); }
    catch (error) { current.message = errorText(error); }
    current.working = false; ctx.rerender();
  });
  const diarizeReason = h("p", { class: "help meeting-control-reason", hidden: true }, speakerReason());
  bindLocalization(diarizeReason, () => { diarizeReason.textContent = speakerReason(); });
  bindLocalization(diarize, () => { diarize.title = speakerReason() ?? t(cloudRoute ? "meetings.speaker_cloud_help" : "meetings.speaker_local_help"); });
  reprocess.addEventListener("click", async () => {
    if (metadataReason() || retranscriptionControl(detail.retranscription_plan, retranscriptionStatus, runtime.busy, current.working).disabled) return;
    try {
      await notes.flush();
      const fresh = await api.meetingDetail(id);
      detail.retranscription_plan = fresh.retranscription_plan;
      retranscriptionStatus = fresh.retranscription_status;
      if (retranscriptionControl(detail.retranscription_plan, retranscriptionStatus, runtime.busy, current.working).disabled) { refreshRetranscription(); return; }
      if (!await confirmDialog(l("meetings.retranscription_confirm_title"), l("meetings.retranscription_confirm", { recognition: frozenRecognitionLabel(detail.retranscription_plan.recognition) }), l("meetings.retranscribe_audio"))) return;
      ownRetranscription = true; current.working = true; current.message = ""; current.recognition = undefined;
      refreshRetranscription(); refreshSummaryControls(); diarize.disabled = true;
      await api.retranscribeMeetingAudio(id);
    } catch (error) { current.message = errorText(error); message.textContent = messageText(); }
    finally {
      if (ownRetranscription) {
        ownRetranscription = false; current.working = false; rememberPosition(); ctx.rerender();
      }
    }
  });
  const applyRetranscriptionStatus = (status: MeetingRetranscriptionStatus | null) => {
    if (disposed) return;
    const previous = retranscriptionStatus;
    retranscriptionStatus = status;
    detail.retranscription_status = status;
    refreshRetranscription(); refreshSummaryControls();
    diarize.disabled = !!speakerReason() || current.working;
    configureTemplate();
    message.textContent = messageText();
    if (!status?.running && status && previous !== status && !ownRetranscription) { rememberPosition(); ctx.rerender(); }
  };
  receiveRetranscription = applyRetranscriptionStatus;
  if (pendingRetranscription !== undefined) applyRetranscriptionStatus(pendingRetranscription);
  const refreshRetranscriptionPlan = async () => {
    if (disposed) return;
    const fresh = await api.meetingDetail(id);
    if (disposed) return;
    detail.retranscription_plan = fresh.retranscription_plan;
    if (JSON.stringify(fresh.retranscription_status) !== JSON.stringify(retranscriptionStatus)) applyRetranscriptionStatus(fresh.retranscription_status);
    else refreshRetranscription();
  };
  bindLocalization(reprocess, refreshRetranscription);
  ctx.onInfoChange(() => { void refreshRetranscriptionPlan().catch(ctx.report); });
  ctx.onFocusChange((focused) => { if (focused) void refreshRetranscriptionPlan().catch(ctx.report); });
  refreshRetranscription();
  const footer = h("footer", { class: "meeting-footer" }, h("div", { class: "row" }, templateSlot, regenerate, h("span", { class: "grow" }), copied, diarize, share,
    controlButton({ class: "btn", onclick: () => api.openPath(detail.dir) }, ctx.info.platform === "macos" ? l("meetings.show_finder") : l("meetings.open_folder"))), diarizeReason);
  const banners = h("div", { hidden: true });
  void conflictsReady.then((conflicts) => {
    if (disposed) return;
    banners.hidden = !conflicts.length;
    banners.replaceChildren(...conflicts.map((name) => h("div", { class: "conflict-banner", role: "status" }, icon("warning"),
    h("span", {}, l("meetings.conflict_notice")),
    controlButton({ class: "btn", onclick: () => { void api.openPath(detail.dir).catch(ctx.report); } }, l("meetings.view")),
    controlButton({ class: "btn danger", onclick: async () => {
      if (!await confirmDialog(l("meetings.delete_conflict_title"), l("meetings.file_will_delete", { name }), l("common.delete"))) return;
      try { await api.deleteConflictCopy(id, name); } catch (error) { ctx.report(error); }
      ctx.rerender();
    } }, l("common.delete")))));
  });
  const metadataText = () => [dateLabel(meetingInfo.start_iso), meetingInfo.duration_min ? t("meetings.duration_minutes", { count: formatNumber(meetingInfo.duration_min) }) : "", detail.attendees.join(", ")].filter(Boolean).join(" · ");
  const metadata = h("p", { class: "help" }, metadataText());
  bindLocalization(metadata, () => { metadata.textContent = metadataText(); });
  const result = h("article", { class: "meeting-detail" },
    ctx.review && getRetainedMeetingWorkspace(id) ? h("button", { class: "text-button source-back meeting-workspace-return", onclick: () => ctx.navigate("meetings", id) }, l("meetings.workspace_return")) : null,
    answerReturn?.sourceMeetingId === id ? h("button", { class: "text-button source-back meeting-source-return", onclick: () => {
      const origin = answerReturn;
      answerReturn = null;
      if (!origin) return;
      void restoreConversationPanel(ctx, origin.conversationScope, origin.conversationId, origin.messageId).then(currentId => {
        if (origin.scope) {
          const id = currentId ?? origin.scope;
          const state = detailState.get(id) ?? detailState.get(origin.scope);
          if (state) { state.tab = "ask"; detailState.set(id, state); }
          ctx.navigate("meetings", id);
        } else ctx.navigate("chat");
      }).catch(ctx.report);
    } }, l("meetings.back_answer")) : null,
    h("header", { class: "meeting-header" }, h("div", { class: "grow" }, h("h1", {}, meetingInfo.title),
      metadata), tabs),
    runtime.id === id && runtime.recognition ? meetingProcessingNotice(ctx, () => runtime, true).element : null,
    banners, runtime.id === id ? recoveryNotice(ctx, runtime) : null,
    detail.summary_outdated ? h("p", { class: "help", role: "status" }, l("meetings.summary_outdated")) : null,
    summarySetup, message, insightBack, panel, footer);
  for (const event of ["pointerdown", "keydown", "input"]) result.addEventListener(event, () => { ++interactionRevision; }, true);
  void voicesReady.then((loaded) => {
    if (disposed) return;
    voices = loaded;
    const names = loaded.filter((voice) => voice.remembered && voice.name).map((voice) => voice.name);
    remembered.hidden = !names.length;
    remembered.replaceChildren(...(names.length ? [h("span", {}, l("meetings.remembered_voices", { names: names.join(", ") }))] : []));
    transcriptContent?.update(detail.transcript, voices);
  });
  void speakerSetupReady.then((loaded) => {
    if (disposed) return;
    speakerSetup = loaded; speakersLoaded = true;
    diarize.disabled = !!speakerReason() || current.working;
    diarizeReason.textContent = speakerReason();
    diarizeReason.hidden = current.tab !== "transcript" || !speakerReason();
    diarize.title = speakerReason() ?? t(cloudRoute ? "meetings.speaker_cloud_help" : "meetings.speaker_local_help");
  });
  void templatesReady.then((loaded) => {
    if (disposed) return;
    templates = loaded; templatesLoaded = !!loaded.length;
    if (!templates.some((template) => template.id === current.template)) current.template = templates[0]?.id ?? current.template;
    if (templatesLoaded) {
      selected = select(templates.map((template) => [template.id, templateUiLabel(template)]), current.template, (template) => { current.template = template; }, "menu");
      templateSlot.replaceChildren(selected);
    }
    configureTemplate(); refreshSummaryControls();
  });
  draw();
  if (ctx.sourceTarget) {
    const source = ctx.sourceTarget;
    requestAnimationFrame(() => {
      if (!result.isConnected) return;
      let matched = false;
      const highlight = (elements: HTMLElement[]) => {
        if (!elements.length) return;
        elements.forEach((element) => element.classList.add("source-highlight"));
        elements[0].tabIndex = -1;
        elements[0].focus({ preventScroll: true });
        elements[0].scrollIntoView({ block: "center" });
        matched = true;
      };
      if (source.kind === "notes") {
        const range = sourceRange(notes.input.value, source.text);
        const disclosure = result.querySelector<HTMLDetailsElement>(".own-notes");
        if (disclosure) disclosure.open = true;
        if (range) {
          notes.input.classList.add("source-highlight");
          notes.input.focus({ preventScroll: true });
          notes.input.setSelectionRange(range.start, range.end);
          notes.input.scrollIntoView({ block: "center" });
          matched = true;
        }
      } else if (source.kind === "summary") {
        const range = sourceBlocks(summaryBlocks.map((block) => block.text), source.text);
        if (range) highlight(summaryBlocks.slice(range.first, range.last + 1).map((block) => block.element));
      } else {
        const candidates = detail.transcript.map((segment, index) => ({ ...segment, index, text: `${sourceSpeakerLabel(segment, detail)}: ${segment.text.trim()}` })).filter((segment) => !segment.failed);
        const stale = source.meeting_uid && source.meeting_uid !== detail.meeting_uid
          || source.utterance_ids && detail.metadata_status && detail.metadata_status !== "valid";
        const range = stale ? null : transcriptSourceBlocks(candidates, source);
        if (range) highlight(candidates.slice(range.first, range.last + 1).flatMap((segment) => {
          const element = result.querySelector<HTMLElement>(`[data-transcript-index="${segment.index}"]`);
          return element ? [element] : [];
        }));
      }
      if (!matched) { sourceChanged = true; message.textContent = messageText(); }
    });
  } else if (ctx.jumpMs !== undefined) {
    const jump = ctx.jumpMs;
    requestAnimationFrame(() => {
      const target = detail.transcript.find((segment) => segment.start_ms <= jump && segment.end_ms > jump)
        ?? detail.transcript.find((segment) => segment.start_ms >= jump);
      const line = target ? result.querySelector<HTMLElement>(`[data-start-ms="${target.start_ms}"]`) : null;
      line?.classList.add("highlight"); line?.scrollIntoView({ block: "center" });
    });
  }
  return result;
}

export async function meetingView(ctx: Context): Promise<HTMLElement> {
  const initial = await api.meetingStatus();
  if (retainedMeetingFailure(initial) && initial.id && initial.session_id) return liveMeetingWorkspace(ctx, initial);
  if (retainedMeetingFailure(initial)) {
    return h("div", { class: "recording-view retained-recording" }, h("h1", {}, initial.title ?? l("meetings.save_recording")),
      h("p", { class: "help" }, l("meetings.recording_stopped", { time: formatClock(initial.elapsed_secs) })),
      h("p", { class: "muted" }, l("meetings.save_before_new")),
      meetingProcessingNotice(ctx, () => initial, true).element, recoveryNotice(ctx, initial));
  }
  if (!initial.busy && ctx.param) return meetingDetail(ctx, ctx.param);
  if (!initial.busy) {
    const title = textField({ type: "text", placeholder: l("meetings.meeting_title_placeholder"), class: "grow", "aria-label": l("meetings.meeting_title_label") });
    const status = h("p", { class: "error", role: "status" }, initial.error ? localizeBackendText(initial.error) : "");
    const start = controlButton({ class: "btn primary" }, l("common.start_recording"));
    const retained = !!initial.recognition && !!(initial.persistence_error || initial.terminal_error);
    const processing = meetingProcessingNotice(ctx, () => retained ? initial : undefined, retained);
    const audioReason = h("p", { class: "help" });
    const refreshAudioReason = () => {
      audioReason.textContent = ctx.info.capabilities.system_audio.reason ? localizeBackendText(ctx.info.capabilities.system_audio.reason) : "";
      audioReason.hidden = !audioReason.textContent;
    };
    const microphoneAccess = controlButton({ class: "btn", onclick: async () => {
      try { await api.requestPermission("microphone"); await ctx.refresh(); } catch (error) { ctx.report(error); }
    } }, l("onboarding.open_settings"));
    const refreshMicrophoneAccess = () => {
      microphoneAccess.hidden = ctx.info.capabilities.microphone.status !== "permission_required";
    };
    ctx.onInfoChange(refreshAudioReason);
    bindLocalization(audioReason, refreshAudioReason);
    ctx.onInfoChange(refreshMicrophoneAccess);
    refreshAudioReason();
    refreshMicrophoneAccess();
    start.addEventListener("click", async () => {
      start.disabled = true;
      try { await api.startMeeting(title.value); ctx.rerender(); }
      catch (error) { status.textContent = errorText(error); start.disabled = false; await ctx.refresh().catch(ctx.report); }
    });
    return h("div", { class: "recording-view" }, h("h1", {}, l("meetings.new_meeting")),
      h("p", { class: "muted" }, l("meetings.recording_tracks_help")),
      processing.element, h("div", { class: "row" }, title, start), status, microphoneAccess, recoveryNotice(ctx, initial), audioReason,
      h("button", { class: "text-button", onclick: () => api.showWindow("settings", "recording") }, l("meetings.recording_settings")));
  }
  if (!initial.id) {
    schedule(ctx, 500, async () => { const status = await api.meetingRuntimeStatus(); if (status.id || !status.busy) ctx.rerender(); });
    return h("div", { class: "recording-view" }, h("h1", {}, l("meetings.starting")), meetingProcessingNotice(ctx, () => initial, true).element, h("progress", { "aria-label": l("meetings.starting_label") }));
  }
  return liveMeetingWorkspace(ctx, initial);
}
