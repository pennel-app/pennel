import type { MeetingDetail, Segment, TranscriptMetadataStatus } from './domain.ts';

export function transcriptKeys(segments: Segment[]): string[] {
  const identities = new Map<string, number>();
  const occurrences = new Map<string, number>();
  for (const segment of segments) if (segment.utterance_id) identities.set(segment.utterance_id, (identities.get(segment.utterance_id) ?? 0) + 1);
  return segments.map(segment => {
    if (segment.utterance_id && identities.get(segment.utterance_id) === 1) return `utterance:${segment.utterance_id}`;
    const base = `${segment.speaker}:${segment.start_ms}`;
    const occurrence = occurrences.get(base) ?? 0;
    occurrences.set(base, occurrence + 1);
    return `${base}:${occurrence}`;
  });
}

export function attributionEditable(status: TranscriptMetadataStatus | undefined): boolean {
  return !status || status === 'valid' || status === 'absent';
}

export function transcriptSpeakerLabel(segment: Segment, speakers: MeetingDetail['speakers'], t: (key: string, args?: Record<string, string | number>) => string): string {
  if (segment.confirmed_speaker_name) return segment.confirmed_speaker_name;
  if (segment.attribution) {
    if (segment.attribution.kind !== 'single') return t(`meetings.attribution_${segment.attribution.kind}`);
    if (segment.attribution.self_voice === true) return t('meetings.speaker_me');
    const voice = segment.attribution.voices.length === 1 ? segment.attribution.voices[0] : null;
    return voice === null ? t('meetings.attribution_unknown') : speakers.find(speaker => speaker.voice === voice)?.name ?? t('mobile.speakers.unknown', { number: voice });
  }
  return segment.voice === null ? t(`meetings.speaker_${segment.speaker}`) : speakers.find(speaker => speaker.voice === segment.voice)?.name ?? t('mobile.speakers.unknown', { number: segment.voice });
}

export function transcriptPlayback(id: string, segment: Segment, tracks: MeetingDetail['audio_tracks'], status?: TranscriptMetadataStatus): { method: 'meetings.play' | 'meetings.playUtterance'; payload: Record<string, string | number> } | null {
  if (status && ['stale', 'unsupported', 'corrupt'].includes(status) || status === 'pending' && !segment.utterance_id) return null;
  if (segment.utterance_id && !segment.source_id) return null;
  const physical = (source: string) => source === 'system' ? 'system_audio' : source;
  const available = tracks.some(track => track.available && (segment.source_id
    ? track.source_id ? physical(track.source_id) === physical(segment.source_id) : track.speaker === (segment.source_id === 'microphone' ? 'me' : 'others')
    : track.speaker === segment.speaker));
  if (!available) return null;
  return segment.utterance_id ? { method: 'meetings.playUtterance', payload: { id, utterance_id: segment.utterance_id } }
    : { method: 'meetings.play', payload: { id, speaker: segment.speaker, start_ms: segment.start_ms } };
}
