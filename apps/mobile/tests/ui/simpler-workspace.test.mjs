import assert from 'node:assert/strict';
import test from 'node:test';
import { completedRecording, nativeOwnsRecordingDraft } from '../../src/recording-finish.ts';
import { dictationProcessingRoutes, processingEndpoint, recentResults, taskReadiness } from '../../src/task-readiness.ts';
import { simplerCopy } from '../../src/simpler-copy.ts';
import { translate } from '../../src/localization.ts';

test('Stop retains the workspace through finishing and failed saving, then opens only this completed recording', () => {
  const intent = { id: 'recording-one', previousFinishedId: 'older-finished' };
  const meeting = { phase: 'finishing', id: 'recording-one', last_finished_id: 'older-finished', retained_id: null, error: null };
  assert.equal(completedRecording(meeting, intent), null);
  assert.equal(completedRecording({ ...meeting, phase: 'idle' }, intent), null);
  assert.equal(completedRecording({ ...meeting, phase: 'idle', last_finished_id: 'new-finished', retained_id: 'recording-one' }, intent), null);
  assert.equal(completedRecording({ ...meeting, phase: 'idle', last_finished_id: 'new-finished', error: 'Storage failed' }, intent), null);
  assert.equal(completedRecording({ ...meeting, phase: 'idle', last_finished_id: 'new-finished' }, intent), 'new-finished');
  assert.equal(completedRecording({ ...meeting, phase: 'idle', last_finished_id: 'new-finished' }, null), null);
  assert.equal(completedRecording({ ...meeting, phase: 'idle', id: 'another-meeting', last_finished_id: 'new-finished' }, intent), null);
});

test('task readiness distinguishes chosen models from usable saved credentials and Cloud sign-in', () => {
  const snapshot = { ready: true, settings: { dictation_stt: { provider_id: 'owned', model: 'speech' }, meeting_stt: { provider_id: 'owned', model: 'speech' }, llm: { provider_id: 'owned', model: 'text' }, providers: [{ id: 'owned', name: 'My service', kind: 'open_ai' }] }, kinds: [{ kind: 'open_ai', stt: true, llm: true, requires_key: true, availability: { status: 'available' } }], keys: { owned: false }, cloud: { signed_in: false } };
  assert.equal(taskReadiness(snapshot, 'dictation_stt').ready, false);
  assert.equal(taskReadiness({ ...snapshot, keys: { owned: true } }, 'dictation_stt').ready, true);
  assert.equal(taskReadiness({ ...snapshot, keys: { owned: true }, kinds: [{ ...snapshot.kinds[0], llm: false }] }, 'llm').ready, false);
  const cloud = { ...snapshot, settings: { ...snapshot.settings, providers: [{ id: 'owned', kind: 'pennel_cloud' }] }, kinds: [{ ...snapshot.kinds[0], kind: 'pennel_cloud', requires_key: false }] };
  assert.equal(taskReadiness(cloud, 'meeting_stt').ready, false);
  assert.equal(taskReadiness({ ...cloud, cloud: { signed_in: true } }, 'meeting_stt').ready, true);
});

test('minimizing a finishing or retained recording requires the same native-owned draft', () => {
  const meeting = { id: 'owned', phase: 'finishing', retained_id: null, notes: 'Acknowledged', pending_notes: 'Captured at Stop' };
  assert.equal(nativeOwnsRecordingDraft(meeting, 'owned', 'Captured at Stop'), true);
  assert.equal(nativeOwnsRecordingDraft(meeting, 'owned', 'Typed after Stop'), false);
  assert.equal(nativeOwnsRecordingDraft(meeting, 'different-meeting', 'Captured at Stop'), false);
  assert.equal(nativeOwnsRecordingDraft({ ...meeting, id: null, phase: 'idle', retained_id: 'owned' }, 'owned', 'Captured at Stop'), true);
  assert.equal(nativeOwnsRecordingDraft({ ...meeting, phase: 'recording' }, 'owned', 'Captured at Stop'), false);
});

test('recent results preserve native order and all history keeps every retained result', () => {
  const entries = Array.from({ length: 9 }, (_, index) => ({ id: index + 1, text: `Result ${index + 1}` }));
  assert.deepEqual(recentResults(entries, false).map(entry => entry.id), [1, 2, 3, 4, 5]);
  assert.equal(recentResults(entries, true).length, 9); assert.equal(entries.length, 9);
});

test('processing disclosure shows the selected endpoint without embedded credentials or query secrets', () => {
  assert.equal(processingEndpoint('https://user:secret@example.invalid/v1?key=secret#private'), 'https://example.invalid/v1');
  assert.equal(processingEndpoint('http://127.0.0.1:8080/v1'), 'http://127.0.0.1:8080/v1');
  assert.equal(processingEndpoint('file:///private/path'), null); assert.equal(processingEndpoint('not a URL'), null);
});

const processingSnapshot = () => ({
  ready: true, platform: 'ios',
  settings: { mode: 'clean', dictation_stt: { provider_id: 'speech', model: 'technical-speech-id' }, llm: { provider_id: 'cleanup', model: 'technical-text-id' }, providers: [
    { id: 'speech', kind: 'apple_speech', name: 'Apple dictation', base_url: '' },
    { id: 'cleanup', kind: 'custom', name: 'My text service', base_url: 'https://text.example.invalid/v1' },
  ] },
  kinds: [
    { kind: 'apple_speech', stt: true, llm: false, requires_key: false, availability: { status: 'available' } },
    { kind: 'apple_intelligence', stt: false, llm: true, requires_key: false, availability: { status: 'available' } },
    { kind: 'custom', stt: true, llm: true, requires_key: true, availability: { status: 'available' } },
    { kind: 'pennel_cloud', stt: true, llm: true, requires_key: false, availability: { status: 'available' } },
  ], keys: { speech: true, cleanup: true }, cloud: { signed_in: true },
});
const routes = (snapshot, language = 'en') => dictationProcessingRoutes(snapshot, (key, values) => translate(language, key, values));

test('Dictation distinguishes local Apple audio from remote cleanup without exposing model IDs', () => {
  const snapshot = processingSnapshot();
  assert.deepEqual(routes(snapshot), {
    audio: 'Audio: On this iPhone · Apple Dictation',
    text: 'Text processing: My text service · https://text.example.invalid/v1',
  });
  assert.deepEqual(routes(snapshot, 'nl'), {
    audio: 'Audio: Op deze iPhone · Apple-dicteren',
    text: 'Tekstverwerking: My text service · https://text.example.invalid/v1',
  });
  assert.equal(JSON.stringify(routes(snapshot)).includes('technical-'), false);
});

test('remote audio and on-device Apple cleanup remain separate destinations', () => {
  const snapshot = processingSnapshot();
  snapshot.settings.providers = [
    { id: 'speech', kind: 'custom', name: 'My speech service', base_url: 'https://speech.example.invalid/v1' },
    { id: 'cleanup', kind: 'apple_intelligence', name: 'Apple Intelligence', base_url: 'https://ignored.example.invalid' },
  ];
  assert.deepEqual(routes(snapshot), {
    audio: 'Audio: My speech service · https://speech.example.invalid/v1',
    text: 'Text processing: On this iPhone · Apple Intelligence',
  });
});

test('Fast explicitly skips cleanup even when a remote text service is configured', () => {
  const snapshot = processingSnapshot(); snapshot.settings.mode = 'fast'; snapshot.keys.cleanup = false;
  assert.equal(routes(snapshot).text, 'Text processing: No cleanup · Fast mode');
  assert.equal(routes(snapshot, 'nl').text, 'Tekstverwerking: Geen opkuis · Snelle modus');
  assert.equal(routes(snapshot).text.includes('text.example.invalid'), false);
});

test('route disclosure strips endpoint credentials and distinguishes missing keys from usable configuration', () => {
  const snapshot = processingSnapshot();
  snapshot.settings.providers[1].base_url = 'https://fixture-user:fixture-password@text.example.invalid/v1?token=fixture-token#private';
  snapshot.keys.cleanup = false;
  const missing = routes(snapshot);
  assert.equal(missing.text, 'Text processing: My text service · https://text.example.invalid/v1 · needs setup');
  assert.equal(JSON.stringify(missing).includes('fixture-'), false);
  assert.equal(routes(snapshot, 'nl').text, 'Tekstverwerking: My text service · https://text.example.invalid/v1 · instellen nodig');
  snapshot.keys.cleanup = true;
  assert.equal(routes(snapshot).text, 'Text processing: My text service · https://text.example.invalid/v1');
});

test('managed Cloud disclosure names its actual service instead of a saved URL the core ignores', () => {
  const snapshot = processingSnapshot();
  snapshot.settings.providers[1] = { id: 'cleanup', kind: 'pennel_cloud', name: 'My Cloud account', base_url: 'https://ignored.example.invalid/v1' };
  snapshot.cloud.signed_in = false;
  assert.equal(routes(snapshot).text, 'Text processing: Pennel Cloud · needs setup');
  snapshot.cloud.signed_in = true;
  assert.equal(routes(snapshot).text, 'Text processing: Pennel Cloud');
  snapshot.settings.llm = { provider_id: '', model: '' };
  assert.equal(routes(snapshot).text, 'Text processing: Not configured');
});

test('simpler workflow copy has equal Dutch and English controls and preserves interpolation fields', () => {
  assert.deepEqual(Object.keys(simplerCopy.en).sort(), Object.keys(simplerCopy.nl).sort());
  for (const key of Object.keys(simplerCopy.en)) {
    const fields = value => [...value.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();
    assert.deepEqual(fields(simplerCopy.en[key]), fields(simplerCopy.nl[key]), key);
    assert.ok(simplerCopy.en[key].trim()); assert.ok(simplerCopy.nl[key].trim());
  }
});
