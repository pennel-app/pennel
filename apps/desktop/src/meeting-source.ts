import type { MeetingDetail, Segment, TranscriptMetadataStatus } from "./api";
import type { TranslationKey } from "./i18n";

export function segmentAudioAvailable(segment: Segment, tracks: MeetingDetail["audio_tracks"], status?: TranscriptMetadataStatus): boolean {
  if (status && ["stale", "unsupported", "corrupt"].includes(status) || status === "pending" && !segment.utterance_id) return false;
  if (segment.utterance_id && !segment.source_id) return false;
  const physical = (source: string) => source === "system" ? "system_audio" : source;
  return tracks.some(track => track.available && (segment.source_id
    ? track.source_id ? physical(track.source_id) === physical(segment.source_id) : track.speaker === (segment.source_id === "microphone" ? "me" : "others")
    : track.speaker === segment.speaker));
}

export function attributionNoticeKey(status: TranscriptMetadataStatus | undefined): TranslationKey | null {
  return status && status !== "valid" ? `meetings.attribution_metadata_${status}` : null;
}

export function attributionEditable(status: TranscriptMetadataStatus | undefined): boolean {
  return !status || status === "valid" || status === "absent";
}
