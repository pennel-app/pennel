import type { MeetingDetail, MeetingTemplate, Segment } from "./api";
import { formatDateTime, formatNumber, l, liveText, t, type TranslationKey, type UiText } from "./i18n.ts";

export function meetingDate(date: string): string {
  const parsed = new Date(date);
  return Number.isNaN(parsed.valueOf()) ? date : formatDateTime(parsed, { dateStyle: "medium", timeStyle: "short" });
}

export function dayGroup(date: string, now = new Date()): string {
  const day = new Date(date);
  const today = new Date(now); today.setHours(0, 0, 0, 0);
  const week = new Date(today); week.setDate(week.getDate() - 7);
  if (day >= today) return t("meetings.group_today");
  return day >= week ? t("meetings.group_week") : t("meetings.group_earlier");
}

export function speakerLabel(segment: Segment, detail?: Pick<MeetingDetail, "speakers">): string {
  if (segment.confirmed_speaker_name) return segment.confirmed_speaker_name;
  if (segment.attribution) {
    if (segment.attribution.kind !== "single") return t(`meetings.attribution_${segment.attribution.kind}`);
    if (segment.attribution.self_voice === true) return t("meetings.speaker_me");
    const voice = segment.attribution.voices.length === 1 ? segment.attribution.voices[0] : null;
    if (voice === null) return t("meetings.attribution_unknown");
    return detail?.speakers.find((name) => name.voice === voice)?.name ?? t("meetings.speaker_number", { number: formatNumber(voice) });
  }
  if (segment.speaker === "me") return t("meetings.speaker_me");
  if (segment.voice === null) return t("meetings.speaker_others");
  return detail?.speakers.find((name) => name.voice === segment.voice)?.name ?? t("meetings.speaker_number", { number: formatNumber(segment.voice) });
}

export function speakerUiLabel(segment: Segment, detail?: Pick<MeetingDetail, "speakers">): UiText {
  if (segment.confirmed_speaker_name) return segment.confirmed_speaker_name;
  if (segment.attribution) return liveText(() => speakerLabel(segment, detail));
  if (segment.speaker === "me") return l("meetings.speaker_me");
  if (segment.voice === null) return l("meetings.speaker_others");
  return detail?.speakers.find((name) => name.voice === segment.voice)?.name
    ?? l("meetings.speaker_number", () => ({ number: formatNumber(segment.voice!) }));
}

/** Cited passage text uses the core's stable speaker prefixes, independently of interface copy. */
export function sourceSpeakerLabel(segment: Segment, detail?: Pick<MeetingDetail, "speakers">): string {
  if (segment.speaker === "me") return "Ik";
  if (segment.voice === null) return "Anderen";
  return detail?.speakers.find((name) => name.voice === segment.voice)?.name ?? `Anderen ${segment.voice}`;
}

const builtinTemplates: Record<string, TranslationKey> = {
  algemeen: "meetings.template.general",
  "een-op-een": "meetings.template.one_to_one",
  klantgesprek: "meetings.template.client",
  standup: "meetings.template.standup",
};

export function templateLabel(template: MeetingTemplate): string {
  const key = builtinTemplates[template.id];
  return template.builtin && !template.customized && key ? t(key) : template.name;
}

export function templateUiLabel(template: MeetingTemplate): UiText {
  const key = builtinTemplates[template.id];
  return template.builtin && !template.customized && key ? l(key) : template.name;
}
