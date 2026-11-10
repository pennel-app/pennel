import assert from 'node:assert/strict';
import test from 'node:test';
import { attributionEditable, transcriptKeys, transcriptSpeakerLabel, transcriptPlayback, transcriptAttributionProvisional } from '../../src/transcript-state.ts';
import { translate } from '../../src/localization.ts';
import { meetingDetail } from '../../src/normalize.ts';

const utterance = { start_ms: 500, end_ms: 1200, utterance_id: 'u-one', source_id: 'room_microphone', source_fingerprint: 'a'.repeat(64),
  speaker: 'others', voice: 2, failed: false, text: 'The exact spoken words.', attribution: { kind: 'single', voices: [2] }, attribution_revision: 1 };

test('mobile transcript keys remain stable across attribution changes and preserve duplicate legacy rows', () => {
  assert.deepEqual(transcriptKeys([utterance]), ['utterance:u-one']);
  assert.deepEqual(transcriptKeys([{ ...utterance, speaker: 'me', voice: null, attribution_revision: 2 }]), ['utterance:u-one']);
  assert.equal(new Set(transcriptKeys([{ ...utterance, utterance_id: null }, { ...utterance, utterance_id: null }])).size, 2);
  assert.equal(new Set(transcriptKeys([utterance, utterance])).size, 2);
});

test('mobile labels preserve unknown, mixed and overlapping attribution independently of physical microphone', () => {
  for (const language of ['en', 'nl']) {
    const t = (key, args) => translate(language, key, args);
    const label = kind => transcriptSpeakerLabel({ ...utterance, speaker: 'me', attribution: { kind, voices: kind === 'unknown' ? [] : [1, 2] } }, [], t);
    assert.equal(label('unknown'), t('meetings.attribution_unknown'));
    assert.equal(label('mixed'), t('meetings.attribution_mixed'));
    assert.equal(label('overlap'), t('meetings.attribution_overlap'));
    assert.equal(transcriptSpeakerLabel(utterance, [{ voice: 2, name: 'Noor' }], t), 'Noor');
    assert.equal(transcriptSpeakerLabel({ ...utterance, attribution: { kind: 'unknown', voices: [] }, confirmed_speaker_name: 'Samenvatting {number}' }, [], t), 'Samenvatting {number}');
    const self = { ...utterance, source_id: 'system_audio', speaker: 'others', attribution: { kind: 'single', voices: [], self_voice: true } };
    assert.equal(transcriptSpeakerLabel(self, [], t), t('meetings.speaker_me'));
    assert.equal(transcriptSpeakerLabel({ ...self, confirmed_speaker_name: 'Noor' }, [], t), 'Noor');
    assert.equal(transcriptSpeakerLabel({ ...self, attribution: { kind: 'unknown', voices: [], self_voice: true } }, [], t), t('meetings.attribution_unknown'));
  }
});

test('mobile automatic names remain Teams estimates and explicit names remain literal in both languages', () => {
  const automatic = { ...utterance, source_id: 'system_audio', automatic_speaker_name: 'Sam {number}', attribution: { kind: 'single', voices: [2], provisional: false } };
  const names = [{ voice: 2, name: 'Sam {number}' }];
  for (const [language, expected] of [['en', 'Sam {number} (Teams estimate)'], ['nl', 'Sam {number} (Teams-inschatting)']]) {
    const t = (key, args) => translate(language, key, args);
    assert.equal(transcriptSpeakerLabel(automatic, names, t), expected);
    assert.equal(transcriptSpeakerLabel(automatic, [], t), expected);
    assert.equal(transcriptSpeakerLabel({ ...automatic, confirmed_speaker_name: 'Manual {name}' }, names, t), 'Manual {name}');
    assert.equal(transcriptSpeakerLabel({ ...automatic, automatic_speaker_name: null }, names, t), 'Sam {number}');
    for (const kind of ['unknown', 'mixed', 'overlap']) {
      assert.equal(transcriptSpeakerLabel({ ...automatic, attribution: { kind, voices: [] } }, names, t), t(`meetings.attribution_${kind}`));
    }
    assert.equal(transcriptSpeakerLabel({ ...automatic, attribution: { kind: 'single', voices: [], self_voice: true } }, names, t), t('meetings.speaker_me'));
    assert.equal(transcriptSpeakerLabel({ ...automatic, attribution: { ...automatic.attribution, provisional: true } }, names, t), 'Sam {number}');
    const normalized = meetingDetail({ info: { id: 'meeting' }, transcript: [automatic], speakers: names });
    assert.equal(normalized.transcript[0].automatic_speaker_name, 'Sam {number}');
    assert.equal(normalized.transcript[0].text, utterance.text);
  }
});

test('mobile durable playback requests a core source resolution rather than the mutable speaker track', () => {
  const detail = { audio_tracks: [{ speaker: 'others', source_id: 'room_microphone', available: true }] };
  assert.deepEqual(transcriptPlayback('meeting', { ...utterance, speaker: 'me' }, detail.audio_tracks),
    { method: 'meetings.playUtterance', payload: { id: 'meeting', utterance_id: 'u-one' } });
  assert.equal(transcriptPlayback('meeting', { ...utterance, source_id: null }, detail.audio_tracks), null);
  assert.equal(transcriptPlayback('meeting', utterance, detail.audio_tracks, 'unsupported'), null);
  assert.deepEqual(transcriptPlayback('meeting', { ...utterance, utterance_id: null, source_id: null }, detail.audio_tracks),
    { method: 'meetings.play', payload: { id: 'meeting', speaker: 'others', start_ms: 500 } });
});

test('normalization retains richer metadata and exact text while keeping old meetings readable', () => {
  const normalized = meetingDetail({ info: { id: 'meeting' }, meeting_uid: 'stable-uid', metadata_status: 'valid', revision: 3, transcript: [utterance] });
  assert.equal(normalized.transcript[0].utterance_id, 'u-one');
  assert.equal(normalized.transcript[0].source_fingerprint, 'a'.repeat(64));
  assert.equal(normalized.transcript[0].text, utterance.text);
  assert.equal(normalized.metadata_status, 'valid');
  assert.deepEqual(meetingDetail({ info: { id: 'legacy' } }).transcript, []);
  for (const status of [undefined, 'valid', 'absent']) assert.equal(attributionEditable(status), true);
  for (const status of ['pending', 'stale', 'unsupported', 'corrupt']) assert.equal(attributionEditable(status), false);
});

test('persisted live estimates have a qualitative review marker until final or manually confirmed', () => {
  const provisional = { ...utterance, attribution: { ...utterance.attribution, provisional: true } };
  assert.equal(transcriptAttributionProvisional(utterance), false);
  assert.equal(transcriptAttributionProvisional(provisional), true);
  assert.equal(transcriptAttributionProvisional({ ...provisional, attribution: { ...provisional.attribution, provisional: false } }), false);
  assert.equal(transcriptAttributionProvisional({ ...provisional, confirmed_speaker_name: 'Confirmed person' }), false);
  for (const language of ['en', 'nl']) assert.notEqual(translate(language, 'meetings.live_speakers_provisional_tag'), 'meetings.live_speakers_provisional_tag');
});
