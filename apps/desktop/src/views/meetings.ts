import { bindLocalization, l, localizeBackendText } from "../i18n.ts";
import { speakerLabel } from "../meeting-labels.ts";
import { api, type MeetingFinished, type MeetingStatus } from "../api";
import { persistAll } from "../async";
import { controlButton, textField, errorText, formatClock, h } from "../dom";
import type { Context } from "../main";
import { icon } from "../icons";
import { VisibilityPoll } from "../window-lifecycle";
import { processingSummary } from "../processing-state";
import { meetingProcessingSummary, processingLabel, retainedMeetingFailure } from "../recording-processing";
import { flushReviewedInsights } from "../reviewed-insights";
import { liveMeetingWorkspace, retainedMeetingWorkspace, flushLiveWorkspaces, finishLiveWorkspace, displayedLiveWorkspaceSession } from "../live-workspace";
import { conversationPanel, flushConversations, restoreConversationPanel } from "../conversation-panel";
import type { ConversationScope } from "../../../../assets/contracts/meeting-conversations";

let answerReturn: { scope: string | null; conversationScope: ConversationScope; conversationId: string; messageId: string; sourceMeetingId: string } | null = null;

export async function flushMeetingDrafts(includeLiveWorkspaces = true) {
  await persistAll([flushReviewedInsights, flushConversations, ...includeLiveWorkspaces ? [flushLiveWorkspaces] : []]);
}

export function displayedMeetingSession(): string | null { return displayedLiveWorkspaceSession(); }
export function finishMeetingDrafts(event: MeetingFinished) { finishLiveWorkspace(event); }

function schedule(ctx: Context, interval: number, work: () => Promise<void>) {
  const poll = new VisibilityPoll(interval, work, ctx.report);
  poll.setVisible(ctx.visible, false);
  ctx.onVisibilityChange((visible) => poll.setVisible(visible));
  ctx.onDispose(() => poll.dispose());
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
    try { return await meetingDetail(ctx, ctx.param); }
    catch (error) { return h("div", { class: "empty-state" }, h("p", { class: "error" }, errorText(error)), controlButton({ class: "btn", onclick: () => ctx.navigate("meetings") }, l("meetings.back_meetings"))); }
  }
  const meetings = await api.meetings("");
  if (meetings.length) return h("div", { class: "empty-state library-placeholder" }, h("p", { class: "muted" }, l("meetings.choose_or_record")));
  return h("div", { class: "empty-state meeting-library-empty" }, icon("notebook"), h("h2", {}, l("meetings.empty_meetings")),
    h("p", { class: "muted" }, l("meetings.empty_meetings_help")),
    controlButton({ class: "btn primary", onclick: () => ctx.navigate("live") }, l("meetings.first_recording")));
}

async function meetingDetail(ctx: Context, id: string): Promise<HTMLElement> {
  const retained = retainedMeetingWorkspace(ctx, id);
  if (retained) return withAnswerReturn(ctx, retained, id);
  const [detail, runtime] = await Promise.all([api.meetingDetail(id), api.meetingRuntimeStatus()]);
  if (!detail.info) return h("div", { class: "empty-state muted" }, l("meetings.not_found"));
  const active = runtime.busy && runtime.id === id;
  const status: MeetingStatus = { ...runtime,
    meeting_uid: detail.meeting_uid ?? id, metadata_status: detail.metadata_status,
    notes: detail.notes, transcript: detail.transcript, id, title: detail.info.title,
    ...active ? {} : { session_id: null, busy: false, recording: false, phase: "idle", pending: 0, error: null,
      elapsed_secs: detail.info.duration_min * 60, can_retry_persistence: false, persistence_error: null, terminal_error: null,
      transcription_warning: null, audio_warning: null, preview: null, live_speaker_attribution: null },
  };
  return withAnswerReturn(ctx, liveMeetingWorkspace(ctx, status, { initialSummary: !active && !!detail.summary?.trim(), detail }), id);
}

function withAnswerReturn(ctx: Context, root: HTMLElement, id: string) {
  root.querySelector(".meeting-source-return")?.remove();
  if (answerReturn?.sourceMeetingId !== id) return root;
  const back = h("button", { class: "text-button source-back meeting-source-return", onclick: () => {
    const origin = answerReturn; answerReturn = null;
    if (!origin) return;
    void restoreConversationPanel(ctx, origin.conversationScope, origin.conversationId, origin.messageId).then(currentId => {
      if (origin.scope) ctx.navigate("meetings", currentId ?? origin.scope);
      else ctx.navigate("chat");
    }).catch(ctx.report);
  } }, l("meetings.back_answer"));
  root.prepend(back); return root;
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
    const status = h("p", { class: "error", role: "status" }, ctx.recordingStartError ? localizeBackendText(ctx.recordingStartError) : initial.error ? localizeBackendText(initial.error) : "");
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
      try { if (ctx.startRecording) await ctx.startRecording(title.value); else { await api.startMeeting(title.value); ctx.rerender(); } }
      catch (error) { status.textContent = errorText(error); start.disabled = false; await ctx.refresh().catch(ctx.report); }
    });
    return h("div", { class: "recording-view" }, h("h1", {}, l("meetings.new_meeting")),
      h("p", { class: "muted" }, l("meetings.recording_tracks_help")),
      processing.element, h("div", { class: "row" }, title, start), status, microphoneAccess, recoveryNotice(ctx, initial), audioReason,
      h("button", { class: "text-button", onclick: () => api.showWindow("settings", "recording") }, l("meetings.recording_settings")),
      controlButton({ class: "btn", onclick: () => ctx.openSetup("meeting_stt") }, l("dictation.home.finish_setup")));
  }
  if (!initial.id) {
    schedule(ctx, 500, async () => { const status = await api.meetingRuntimeStatus(); if (status.id || !status.busy) ctx.rerender(); });
    return h("div", { class: "recording-view" }, h("h1", {}, l("meetings.starting")), meetingProcessingNotice(ctx, () => initial, true).element, h("progress", { "aria-label": l("meetings.starting_label") }));
  }
  return liveMeetingWorkspace(ctx, initial);
}
