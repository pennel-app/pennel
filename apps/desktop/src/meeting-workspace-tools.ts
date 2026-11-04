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

export function meetingWorkspaceTools(initial: Context, host: {
  id(): string;
  finished(): boolean;
  flush(): Promise<void>;
  refresh(): Promise<void>;
  template(): string;
  setTemplate(value: string): void;
  speakers: HTMLElement;
  people: HTMLElement;
  processingChanged?(status: MeetingRetranscriptionStatus | null): void;
}) {
  let ctx = initial, detail: MeetingDetail | null = null, setup: SpeakerSetup | null = null;
  let busy = false, voices: MeetingVoice[] = [], templates: MeetingTemplate[] = [], binding = 0;
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
  const recognize = controlButton({ class: "btn", onclick: () => { void run(async () => {
    const result = await api.recognizeSpeakers(host.id()); status.textContent = recognitionSummary(result); status.hidden = false;
    await host.refresh(); await refresh();
  }); } }, l("meetings.recognize_speakers"));
  const reprocess = controlButton({ class: "btn", onclick: () => { void run(async () => {
    await host.flush();
    const fresh = await api.meetingDetail(host.id());
    if (!attributionEditable(fresh.metadata_status)) {
      detail = fresh; update(); return;
    }
    const control = retranscriptionControl(fresh.retranscription_plan, fresh.retranscription_status, !host.finished(), false);
    if (control.disabled) { detail = fresh; update(); return; }
    if (!await confirmDialog(l("meetings.retranscription_confirm_title"), l("meetings.retranscription_confirm", {
      recognition: frozenRecognitionLabel(fresh.retranscription_plan.recognition),
    }), l("meetings.retranscribe_audio"))) return;
    await api.retranscribeMeetingAudio(host.id()); await host.refresh(); await refresh();
  }); } }, l("meetings.retranscribe_audio"));
  const speakerSetup = h("button", { class: "text-button", onclick: () => ctx.navigate("meeting-settings") }, l("meetings.live_speakers_setup"));
  const transcriptionSetup = h("button", { class: "text-button", onclick: () => ctx.openSetup("meeting_stt") }, l("meetings.recording_settings"));
  const stopAudio = controlButton({ class: "btn compact", onclick: () => { void api.stopAudio().catch(report); } }, l("meetings.stop_audio"));
  const folder = controlButton({ class: "btn", onclick: () => { if (detail) void api.openPath(detail.dir).catch(report); } },
    ctx.info.platform === "macos" ? l("meetings.show_finder") : l("meetings.open_folder"));
  const attendees = controlButton({ class: "btn", onclick: () => { void run(async () => {
    if (!detail) return;
    const value = await askText(l("meetings.attendees"), detail.attendees.join(", "), l("meetings.attendees_help"));
    if (value === null) return;
    await api.setMeetingAttendees(host.id(), value.split(",").map(name => name.trim()).filter(Boolean)); await refresh();
  }); } }, l("meetings.attendees_action"));
  const conflicts = h("div", { class: "meeting-conflict-actions" });
  files.append(folder, attendees);
  const root = h("section", { class: "meeting-workspace-tools" }, h("h2", {}, l("meetings.workspace_tools")), status,
    h("section", { "data-meeting-tool": "people" }, h("h3", {}, l("meetings.workspace_people")), host.people),
    h("section", { "data-meeting-tool": "speakers" }, h("h3", {}, l("meetings.workspace_speaker_controls")), host.speakers,
      recognizeReason, h("div", { class: "row" }, recognize, speakerSetup), voiceList),
    h("section", { "data-meeting-tool": "template" }, h("h3", {}, l("meetings.summary_template_change")),
      h("p", { class: "help" }, l("meetings.tools_templates_help")), template),
    h("section", { "data-meeting-tool": "automation" }, automation.root),
    h("section", { "data-meeting-tool": "transcription" }, h("h3", {}, l("meetings.transcript")), reprocessPlan, reprocessReason, reprocessProgress,
      h("div", { class: "row" }, reprocess, transcriptionSetup, stopAudio)),
    h("section", { "data-meeting-tool": "files" }, h("h3", {}, l("meetings.workspace_file_actions")), files, conflicts));

  function report(failure: unknown, reveal = false) {
    status.textContent = errorText(failure); status.hidden = false; status.classList.add("error");
    if (reveal && status.getClientRects().length) status.scrollIntoView({ block: "nearest" });
  }
  async function run(work: () => Promise<void>) {
    if (busy) return; busy = true; status.hidden = false; status.classList.remove("error"); status.textContent = t("meetings.working"); update();
    try { await work(); if (status.textContent === t("meetings.working")) status.hidden = true; }
    catch (failure) { report(failure, true); }
    finally { busy = false; update(); }
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
    if (host.finished()) {
      const [saved, speaker, loadedVoices] = await Promise.allSettled([api.meetingDetail(host.id()), api.speakerSetup(), api.meetingVoices(host.id())]);
      if (saved.status === "fulfilled") detail = saved.value; else report(saved.reason);
      if (speaker.status === "fulfilled") setup = speaker.value;
      if (loadedVoices.status === "fulfilled") voices = loadedVoices.value;
      voiceList.replaceChildren(...voices.filter(voice => voice.voice != null).map(voice => h("div", { class: "meeting-voice-row row" },
        h("span", { class: "grow" }, voice.name || t("meetings.voice_name_label", { name: String(voice.voice) })),
        controlButton({ class: "btn compact", onclick: () => { void renameVoice(voice, false); } }, l("people.rename")),
        controlButton({ class: "btn compact", disabled: !voice.can_remember, title: voice.can_remember ? "" : t("meetings.speaker_recognition_unavailable"),
          onclick: () => { void renameVoice(voice, true); } }, l("people.remember")))));
      if (ctx.info.settings.cloud_sync) {
        const names = await api.meetingConflictFiles(host.id());
        conflicts.replaceChildren(...names.map(name => h("div", { class: "meeting-conflict row" }, h("span", { class: "help grow" }, name),
          controlButton({ class: "btn danger compact", onclick: async () => {
            if (!await confirmDialog(l("meetings.delete_conflict_title"), l("meetings.file_will_delete", { name }), l("common.delete"))) return;
            try { await api.deleteConflictCopy(host.id(), name); await refresh(); } catch (failure) { report(failure); }
          } }, l("common.delete")))));
        if (names.length) conflicts.prepend(h("p", { class: "help" }, l("meetings.tools_conflicts")));
      }
    }
    if (!templates.length) {
      try {
        templates = await api.meetingTemplates();
        if (templates.length) {
          if (!templates.some(value => value.id === host.template())) host.setTemplate(templates[0].id);
          templateChoice = select(templates.map(value => [value.id, templateUiLabel(value)]), host.template(), host.setTemplate, "menu");
          templateChoice.setAttribute("aria-label", t("meetings.template")); template.replaceChildren(templateChoice);
        }
      } catch (failure) { report(failure); }
    }
    update();
  }
  function progressText(status: MeetingRetranscriptionStatus) {
    const percent = retranscriptionPercent(status.progress);
    const track = t(status.progress.speaker === "me" ? "meetings.retranscription_microphone" : "meetings.retranscription_computer");
    return status.progress.phase === "transcribing" && percent !== null ? t("meetings.retranscription_progress", { track, percent })
      : status.progress.phase === "speakers" ? t("meetings.recognizing_speakers")
      : status.progress.phase === "saving" ? t("meetings.retranscription_saving") : t("meetings.retranscription_decoding");
  }
  async function renameVoice(voice: MeetingVoice, remember: boolean) {
    await run(async () => {
      const name = await askText(l(remember ? "people.remember" : "people.rename"), voice.name ?? "",
        l(remember ? "meetings.voice_remember_help" : "meetings.voice_name_help"));
      if (name === null || remember && !name) return;
      if (remember) await api.rememberSpeaker(host.id(), voice.voice, name); else await api.renameSpeaker(host.id(), voice.voice, name);
      await host.refresh(); await refresh();
    });
  }
  bindLocalization(root, update); update();
  return { root, refresh, bind(next: Context) {
    ctx = next; const mine = ++binding; automation.bind(next); next.onInfoChange(update);
    const subscription = listen<{ id: string; status: MeetingRetranscriptionStatus }>("meeting-retranscription-changed", event => {
      if (mine !== binding || event.payload.id !== host.id()) return;
      if (detail) detail.retranscription_status = event.payload.status;
      host.processingChanged?.(event.payload.status); update();
      if (!event.payload.status.running) void host.refresh().then(refresh).catch(report);
    });
    next.onDispose(() => { if (mine === binding) ++binding; void subscription.then(unsubscribe => unsubscribe()).catch(report); });
    update();
  }, reveal(section?: string) {
    void refresh().catch(report);
    if (section) requestAnimationFrame(() => root.querySelector<HTMLElement>(`[data-meeting-tool="${section}"]`)?.scrollIntoView({ block: "start" }));
  } };
}
