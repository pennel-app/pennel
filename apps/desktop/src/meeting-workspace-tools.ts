import { api, listen, type MeetingDetail, type MeetingRetranscriptionStatus, type MeetingTemplate, type MeetingVoice, type SpeakerSetup } from "./api";
import type { Context } from "./main";
import { bindLocalization, l, localizeBackendText, t } from "./i18n";
import { askText, confirmDialog, controlButton, errorText, h, select } from "./dom";
import { frozenRecognitionLabel } from "./recording-processing";
import { retranscriptionControl, retranscriptionPercent } from "./meeting-retranscription";
import { attributionEditable, attributionNoticeKey } from "./meeting-source";
import { templateUiLabel } from "./meeting-labels";
import { recognitionSummary } from "./speaker-state";
import { summaryAutomationChoice } from "./meeting-summary-automation";

const toolLabels = {
  people: "meetings.workspace_people", speakers: "meetings.workspace_speaker_controls", template: "meetings.summary_template_change",
  automation: "meetings.summary_automation_change", transcription: "meetings.transcript", files: "meetings.workspace_file_actions",
} as const;
type MeetingTool = keyof typeof toolLabels;
type ToolOwner = { section: MeetingTool; id: string; binding: number };
type ToolStatus = { text: string; error: boolean; source: "read" | "action"; working?: boolean };

export function meetingWorkspaceTools(initial: Context, host: {
  id(): string;
  finished(): boolean;
  flush(): Promise<void>;
  refresh(): Promise<void>;
  template(): string;
  setTemplate(value: string): void;
  speakers: HTMLElement;
  people: HTMLElement;
  close(): void;
  processingChanged?(status: MeetingRetranscriptionStatus | null): void;
}) {
  let ctx = initial, detail: MeetingDetail | null = null, setup: SpeakerSetup | null = null;
  let busy = false, voices: MeetingVoice[] = [], templates: MeetingTemplate[] = [], binding = 0;
  let activeTool: MeetingTool = "template", readRevision = 0;
  const messages = new Map<MeetingTool, ToolStatus>();
  let templateChoice: ReturnType<typeof select> | null = null;
  const status = h("p", { class: "help meeting-tools-status", role: "status", hidden: true });
  const template = h("div", { class: "meeting-template-choice" });
  const voiceList = h("div", { class: "meeting-voice-list" });
  const recognizeReason = h("p", { class: "help" });
  const reprocessReason = h("p", { class: "help" });
  const reprocessProgress = h("p", { class: "help meeting-message meeting-retranscription-status", role: "status", hidden: true });
  const reprocessPlan = h("p", { class: "help meeting-retranscription-plan" });
  const files = h("div", { class: "meeting-file-actions row" });
  const automation = summaryAutomationChoice(ctx);
  const recognize = controlButton({ class: "btn", onclick: () => { void run(async owner => {
    const result = await api.recognizeSpeakers(owner.id);
    setStatus(owner, { text: recognitionSummary(result), error: false, source: "action" });
    if (!owns(owner)) return;
    await host.refresh(); await refresh();
  }); } }, l("meetings.recognize_speakers"));
  const reprocess = controlButton({ class: "btn", onclick: () => { void run(async owner => {
    await host.flush();
    if (!owns(owner)) return;
    const fresh = await api.meetingDetail(owner.id);
    if (!owns(owner)) return;
    if (!attributionEditable(fresh.metadata_status)) {
      detail = fresh; update(); return;
    }
    const control = retranscriptionControl(fresh.retranscription_plan, fresh.retranscription_status, !host.finished(), false);
    if (control.disabled) { detail = fresh; update(); return; }
    if (!await confirmDialog(l("meetings.retranscription_confirm_title"), l("meetings.retranscription_confirm", {
      recognition: frozenRecognitionLabel(fresh.retranscription_plan.recognition),
    }), l("meetings.retranscribe_audio"))) return;
    if (!owns(owner)) return;
    await api.retranscribeMeetingAudio(owner.id); await host.refresh(); await refresh();
  }); } }, l("meetings.retranscribe_audio"));
  const speakerSetup = h("button", { class: "text-button", onclick: () => ctx.navigate("meeting-settings") }, l("meetings.live_speakers_setup"));
  const transcriptionSetup = h("button", { class: "text-button", onclick: () => ctx.openSetup("meeting_stt") }, l("meetings.recording_settings"));
  const stopAudio = controlButton({ class: "btn compact", onclick: () => { void run(async () => { await api.stopAudio(); }); } }, l("meetings.stop_audio"));
  const folder = controlButton({ class: "btn", onclick: () => { const path = detail?.dir; if (path) void run(async () => { await api.openPath(path); }); } },
    ctx.info.platform === "macos" ? l("meetings.show_finder") : l("meetings.open_folder"));
  const attendees = controlButton({ class: "btn", onclick: () => { void run(async owner => {
    if (!detail) return;
    const value = await askText(l("meetings.attendees"), detail.attendees.join(", "), l("meetings.attendees_help"));
    if (value === null || !owns(owner)) return;
    await api.setMeetingAttendees(owner.id, value.split(",").map(name => name.trim()).filter(Boolean)); await refresh();
  }); } }, l("meetings.attendees_action"));
  const conflicts = h("div", { class: "meeting-conflict-actions" });
  files.append(folder, attendees);
  const heading = h("h2", { class: "meeting-tool-heading", tabindex: -1 }, l(toolLabels[activeTool]));
  const back = h("button", { class: "text-button meeting-tool-back", onclick: host.close }, l("meetings.tools_return_document"));
  const root = h("section", { class: "meeting-workspace-tools" }, back, heading, status,
    h("section", { "data-meeting-tool": "people" }, h("h3", {}, l("meetings.workspace_people")), host.people),
    h("section", { "data-meeting-tool": "speakers" }, h("h3", {}, l("meetings.workspace_speaker_controls")), host.speakers,
      recognizeReason, h("div", { class: "row" }, recognize, speakerSetup), voiceList),
    h("section", { "data-meeting-tool": "template" }, h("h3", {}, l("meetings.summary_template_change")),
      h("p", { class: "help" }, l("meetings.tools_templates_help")), template),
    h("section", { "data-meeting-tool": "automation" }, automation.root),
    h("section", { "data-meeting-tool": "transcription" }, h("h3", {}, l("meetings.transcript")), reprocessPlan, reprocessReason, reprocessProgress,
      h("div", { class: "row" }, reprocess, transcriptionSetup, stopAudio)),
    h("section", { "data-meeting-tool": "files" }, h("h3", {}, l("meetings.workspace_file_actions")), files, conflicts));
  const sections = [...root.querySelectorAll<HTMLElement>("[data-meeting-tool]")];
  for (const section of sections) {
    const title = section.firstElementChild;
    if (title instanceof HTMLElement && title.tagName === "H3") title.hidden = true;
  }

  function owner(section: MeetingTool = activeTool): ToolOwner { return { section, id: host.id(), binding }; }
  function owns(value: ToolOwner) { return value.id === host.id() && value.binding === binding; }
  function renderStatus() {
    const message = messages.get(activeTool);
    status.textContent = message?.working ? t("meetings.working") : message?.text ?? "";
    status.hidden = !message; status.classList.toggle("error", message?.error ?? false);
  }
  function setStatus(value: ToolOwner, message: ToolStatus | null) {
    if (!owns(value)) return;
    if (message) messages.set(value.section, message); else messages.delete(value.section);
    renderStatus();
  }
  function report(failure: unknown, value: ToolOwner, reveal = false) {
    setStatus(value, { text: errorText(failure), error: true, source: "action" });
    if (owns(value) && value.section === activeTool && reveal && status.getClientRects().length) status.scrollIntoView({ block: "nearest" });
  }
  async function run(work: (value: ToolOwner) => Promise<void>) {
    if (busy) return;
    const value = owner(); busy = true;
    const pending: ToolStatus = { text: "", error: false, source: "action", working: true };
    setStatus(value, pending); update();
    try { await work(value); }
    catch (failure) { report(failure, value, true); }
    finally {
      if (messages.get(value.section) === pending) messages.delete(value.section);
      busy = false; update();
    }
  }
  function speakerReason() {
    if (!host.finished()) return t("meetings.tools_no_recording");
    const metadata = detail && !attributionEditable(detail.metadata_status) && attributionNoticeKey(detail.metadata_status);
    if (metadata) return t(metadata);
    const capability = ctx.info.capabilities.diarization;
    if (capability.status !== "available") return capability.reason ? localizeBackendText(capability.reason) : t("meetings.speaker_recognition_unavailable");
    if (!detail?.audio_tracks.some(track => track.available)) return t("meetings.no_saved_audio");
    if (!detail.transcript.some(segment => !segment.failed)) return t("meetings.empty_transcript");
    if (!setup?.ready) return t("meetings.download_speaker_models");
    return "";
  }
  function update() {
    root.dataset.activeTool = activeTool;
    heading.textContent = t(toolLabels[activeTool]);
    for (const section of sections) section.hidden = section.dataset.meetingTool !== activeTool;
    renderStatus();
    const reason = speakerReason(); recognizeReason.textContent = reason; recognizeReason.hidden = !reason;
    recognize.disabled = busy || !!reason || !!detail?.retranscription_status?.running; recognize.title = reason;
    speakerSetup.hidden = !reason;
    const control = detail ? retranscriptionControl(detail.retranscription_plan, detail.retranscription_status, !host.finished(), busy) : null;
    const metadata = detail && !attributionEditable(detail.metadata_status) && attributionNoticeKey(detail.metadata_status);
    const reasonText = !host.finished() ? t("meetings.tools_no_recording") : metadata ? t(metadata) : control?.reason ? localizeBackendText(control.reason) : "";
    reprocessReason.textContent = reasonText; reprocessReason.hidden = !reasonText;
    reprocess.disabled = !control || control.disabled || !!metadata; reprocess.title = reasonText;
    const progress = detail?.retranscription_status;
    reprocessProgress.textContent = progress?.running ? progressText(progress) : progress?.error ? localizeBackendText(progress.error)
      : progress?.warning ? localizeBackendText(progress.warning) : progress?.progress.phase === "completed" ? t("meetings.retranscription_done") : "";
    reprocessProgress.hidden = !reprocessProgress.textContent;
    reprocessProgress.classList.toggle("error", !!progress?.error);
    reprocessPlan.textContent = detail ? frozenRecognitionLabel(detail.retranscription_plan.recognition) : t("meetings.tools_no_recording");
    if (templateChoice) templateChoice.disabled = busy || !!progress?.running;
    stopAudio.disabled = !host.finished() || ctx.info.capabilities.audio_playback.status !== "available" || !detail?.audio_tracks.some(track => track.available);
    stopAudio.title = ctx.info.capabilities.audio_playback.reason ?? (!detail?.audio_tracks.some(track => track.available) ? t("meetings.no_saved_audio") : "");
    folder.disabled = !detail; attendees.disabled = !detail || busy;
    automation.refresh();
  }
  async function refresh() {
    const value = owner(), revision = ++readRevision;
    const current = () => owns(value) && value.section === activeTool && revision === readRevision;
    const selected = value.section, target = value.id;
    try {
      if (host.finished() && ["speakers", "transcription", "files"].includes(selected)) {
        const [saved, speaker, loadedVoices] = await Promise.allSettled([api.meetingDetail(target),
          selected === "speakers" ? api.speakerSetup() : Promise.resolve(null),
          selected === "speakers" ? api.meetingVoices(target) : Promise.resolve(null)]);
        if (!current()) return;
        if (saved.status === "fulfilled") detail = saved.value; else throw saved.reason;
        if (speaker.status === "rejected") throw speaker.reason;
        if (loadedVoices.status === "rejected") throw loadedVoices.reason;
        if (speaker.status === "fulfilled" && speaker.value) setup = speaker.value;
        if (loadedVoices.status === "fulfilled" && loadedVoices.value) voices = loadedVoices.value;
        if (selected === "speakers") voiceList.replaceChildren(...voices.filter(voice => voice.voice != null).map(voice => h("div", { class: "meeting-voice-row row" },
          h("span", { class: "grow" }, voice.name || t("meetings.voice_name_label", { name: String(voice.voice) })),
          controlButton({ class: "btn compact", onclick: () => { void renameVoice(voice, false); } }, l("people.rename")),
          controlButton({ class: "btn compact", disabled: !voice.can_remember, title: voice.can_remember ? "" : t("meetings.speaker_recognition_unavailable"),
            onclick: () => { void renameVoice(voice, true); } }, l("people.remember")))));
        if (selected === "files" && ctx.info.settings.cloud_sync) {
          const names = await api.meetingConflictFiles(target);
          if (!current()) return;
          conflicts.replaceChildren(...names.map(name => h("div", { class: "meeting-conflict row" }, h("span", { class: "help grow" }, name),
            controlButton({ class: "btn danger compact", onclick: () => { void run(async action => {
              if (!await confirmDialog(l("meetings.delete_conflict_title"), l("meetings.file_will_delete", { name }), l("common.delete")) || !owns(action)) return;
              await api.deleteConflictCopy(action.id, name); await refresh();
            }); } }, l("common.delete")))));
          if (names.length) conflicts.prepend(h("p", { class: "help" }, l("meetings.tools_conflicts")));
        }
      }
      if (selected === "template" && !templates.length) {
        const loaded = await api.meetingTemplates();
        if (!current()) return;
        templates = loaded;
        if (templates.length) {
          if (!templates.some(value => value.id === host.template())) host.setTemplate(templates[0].id);
          templateChoice = select(templates.map(value => [value.id, templateUiLabel(value)]), host.template(), host.setTemplate, "menu");
          templateChoice.setAttribute("aria-label", t("meetings.template")); template.replaceChildren(templateChoice);
        }
      }
      if (current() && messages.get(selected)?.source === "read") setStatus(value, null);
    } catch (failure) {
      const previous = messages.get(selected);
      if (current() && !(previous?.source === "action" && previous.error)) setStatus(value, { text: errorText(failure), error: true, source: "read" });
    } finally { if (current()) update(); }
  }
  function progressText(status: MeetingRetranscriptionStatus) {
    const percent = retranscriptionPercent(status.progress);
    const track = t(status.progress.speaker === "me" ? "meetings.retranscription_microphone" : "meetings.retranscription_computer");
    return status.progress.phase === "transcribing" && percent !== null ? t("meetings.retranscription_progress", { track, percent })
      : status.progress.phase === "speakers" ? t("meetings.recognizing_speakers")
      : status.progress.phase === "saving" ? t("meetings.retranscription_saving") : t("meetings.retranscription_decoding");
  }
  async function renameVoice(voice: MeetingVoice, remember: boolean) {
    await run(async owner => {
      const name = await askText(l(remember ? "people.remember" : "people.rename"), voice.name ?? "",
        l(remember ? "meetings.voice_remember_help" : "meetings.voice_name_help"));
      if (name === null || remember && !name || !owns(owner)) return;
      if (remember) await api.rememberSpeaker(owner.id, voice.voice, name); else await api.renameSpeaker(owner.id, voice.voice, name);
      await host.refresh(); await refresh();
    });
  }
  bindLocalization(root, update); update();
  return { root, refresh, bind(next: Context) {
    ctx = next; const mine = ++binding; automation.bind(next); next.onInfoChange(update);
    ++readRevision;
    const subscription = listen<{ id: string; status: MeetingRetranscriptionStatus }>("meeting-retranscription-changed", event => {
      if (mine !== binding || event.payload.id !== host.id()) return;
      if (detail) detail.retranscription_status = event.payload.status;
      host.processingChanged?.(event.payload.status); update();
      if (!event.payload.status.running) {
        const value = owner("transcription");
        void host.refresh().then(() => { if (owns(value)) return refresh(); }).catch(failure => report(failure, value));
      }
    });
    const value = owner("transcription");
    next.onDispose(() => { if (mine === binding) { ++binding; ++readRevision; } void subscription.then(unsubscribe => unsubscribe()).catch(failure => report(failure, value)); });
    update();
  }, reveal(section?: string) {
    activeTool = section && Object.hasOwn(toolLabels, section) ? section as MeetingTool : "template";
    update();
    void refresh();
    const focusOwner = document.activeElement;
    requestAnimationFrame(() => {
      if (!root.getClientRects().length || root.closest("[hidden]")) return;
      const focused = document.activeElement;
      if (focused !== focusOwner && focused instanceof HTMLElement && (root.contains(focused) || focused.matches("textarea, input, [contenteditable=\"true\"]"))) return;
      const pane = root.closest<HTMLElement>(".live-pane");
      if (pane) pane.scrollTop = 0;
      heading.focus({ preventScroll: true });
    });
  } };
}
