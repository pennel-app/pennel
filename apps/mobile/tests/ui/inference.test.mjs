import assert from 'node:assert/strict';
import test from 'node:test';
import { inferenceConfiguration, inferenceConfigIdentity, inferenceCopy, inferenceDraft, inferenceProviderEligible, inferenceProtocolAllowed, inferenceSelection, InferenceVerificationController, selectInferenceProvider } from '../../src/inference.ts';

const config = (extra = {}) => ({ provider_id: 'one', protocol: 'open_router_decisions', model: 'typesafe/jev-1.14', expected_model: null, upstream: 'typesafe', max_input_bytes: 16000, timeout_secs: 30, ...extra });
const protocols = [
  ...['open_router_decisions', 'open_router_system_one', 'type_safe', 'perplexity', 'mercury', 'local_decider'].map((protocol) => ({
    protocol, decision: true, reranking: false, requires_key: protocol !== 'local_decider', managed: false,
    allowed_provider_kinds: protocol.startsWith('open_router_') ? ['open_router', 'custom'] : ['custom'],
  })),
  { protocol: 'tei', decision: false, reranking: true, requires_key: false, managed: false, allowed_provider_kinds: ['custom'] },
  { protocol: 'pennel_cloud', decision: true, reranking: true, requires_key: false, managed: true, allowed_provider_kinds: ['pennel_cloud'] },
];
const snapshot = () => ({ platform: 'ios', settings: { decision: config(), reranking: null, providers: [{ id: 'one', kind: 'open_router', name: 'Provider', base_url: 'https://openrouter.ai/api/v1' }], dictation_stt: { provider_id: 'speech', model: 'unchanged' } }, keys: { one: true }, inference_key_revisions: { one: 1 } });
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('optional defaults do not select a provider or model', () => {
  const draft = inferenceDraft('decision');
  assert.equal(draft.provider_id, ''); assert.equal(draft.model, '');
  assert.equal(draft.protocol, 'open_router_decisions'); assert.equal(inferenceConfiguration(draft, 'decision', protocols), null);
  assert.equal(inferenceDraft('reranking').protocol, 'tei');
});

test('atomic configuration trims identities and requires a bounded explicit OpenRouter route', () => {
  const draft = inferenceDraft('decision', config());
  assert.deepEqual(inferenceConfiguration({ ...draft, model: '  chosen  ', expected_model: '   ' }, 'decision', protocols), config({ model: 'chosen' }));
  for (const change of [{ upstream: '' }, { provider_id: '' }, { model: '  ' }, { max_input_bytes: '' }, { max_input_bytes: '65537' }, { max_input_bytes: '1.5' }, { timeout_secs: '91' }, { timeout_secs: '0' }, { protocol: 'tei' }]) {
    assert.equal(inferenceConfiguration({ ...draft, ...change }, 'decision', protocols), null, JSON.stringify(change));
  }
  assert.ok(inferenceConfiguration(inferenceDraft('reranking', config({ protocol: 'tei', upstream: null })), 'reranking', protocols));
});

test('saved-configuration identity ignores JSON member order while retaining every route/budget field', () => {
  const first = config();
  const reversed = Object.fromEntries(Object.entries(first).reverse());
  assert.equal(inferenceConfigIdentity(first), inferenceConfigIdentity(reversed));
  for (const changed of [{ protocol: 'mercury' }, { expected_model: 'pinned' }, { max_input_bytes: 1000 }, { timeout_secs: 10 }, { upstream: 'different' }]) {
    assert.notEqual(inferenceConfigIdentity(first), inferenceConfigIdentity({ ...first, ...changed }));
  }
});

test('selection requires a supported explicit configuration and keeps STT untouched', () => {
  const state = snapshot(), original = structuredClone(state.settings.dictation_stt);
  assert.ok(inferenceSelection(state, 'decision', protocols));
  assert.equal(inferenceSelection(state, 'reranking', protocols), null);
  state.keys.one = false; assert.equal(inferenceSelection(state, 'decision', protocols), null);
  state.keys.one = true; state.platform = 'android'; assert.equal(inferenceSelection(state, 'decision', protocols), null);
  state.platform = 'ios'; state.settings.providers[0].kind = 'apple_speech'; assert.equal(inferenceSelection(state, 'decision', protocols), null);
  assert.deepEqual(state.settings.dictation_stt, original);
});

test('provider URL and credential revisions invalidate the selected connection fingerprint', () => {
  const state = snapshot(); const first = inferenceSelection(state, 'decision', protocols);
  state.inference_key_revisions.one += 1;
  assert.notEqual(inferenceSelection(state, 'decision', protocols).fingerprint, first.fingerprint);
  const second = inferenceSelection(state, 'decision', protocols);
  state.settings.providers[0].base_url = 'https://eu.openrouter.ai/api/v1';
  assert.notEqual(inferenceSelection(state, 'decision', protocols).fingerprint, second.fingerprint);
});

test('OpenRouter accounts and known OpenRouter hosts cannot bypass fixed routing through a direct API format', () => {
  const account = snapshot().settings.providers[0];
  assert.equal(inferenceProviderEligible(account, 'decision', protocols), true);
  assert.equal(inferenceProviderEligible(account, 'reranking', protocols), false);
  for (const protocol of ['type_safe', 'perplexity', 'mercury', 'local_decider', 'tei']) assert.equal(inferenceProtocolAllowed(account, 'decision', protocol, protocols), false);
  assert.equal(inferenceProtocolAllowed(account, 'decision', 'open_router_decisions', protocols), true);
  assert.equal(inferenceProtocolAllowed(account, 'decision', 'open_router_system_one', protocols), true);
  for (const base_url of ['https://openrouter.ai/api/v1', 'https://eu.openrouter.ai/api/v1', 'https://US.OPENROUTER.AI./api/v1']) {
    const custom = { ...account, kind: 'custom', base_url };
    assert.equal(inferenceProviderEligible(custom, 'reranking', protocols), false);
    assert.equal(inferenceProtocolAllowed(custom, 'decision', 'mercury', protocols), false);
    assert.equal(inferenceProtocolAllowed(custom, 'decision', 'open_router_decisions', protocols), true);
  }
  const server = { ...account, kind: 'custom', base_url: 'https://example.invalid/v1' };
  assert.equal(inferenceProtocolAllowed(server, 'decision', 'mercury', protocols), true);
  assert.equal(inferenceProtocolAllowed(server, 'decision', 'open_router_decisions', protocols), true);
  assert.equal(inferenceProtocolAllowed(server, 'reranking', 'tei', protocols), true);
  const state = snapshot(); state.settings.decision.protocol = 'mercury';
  assert.equal(inferenceSelection(state, 'decision', protocols), null);
});

test('choosing an OpenRouter provider changes only the unsaved draft to a compatible API format', () => {
  const original = inferenceDraft('decision', config({ protocol: 'mercury', provider_id: 'custom' }));
  const next = selectInferenceProvider(original, snapshot().settings.providers[0]);
  assert.equal(next.protocol, 'open_router_decisions'); assert.equal(next.provider_id, 'one');
  assert.equal(original.protocol, 'mercury'); assert.equal(original.provider_id, 'custom');
  assert.equal(next.model, original.model);
});

test('verification sends only capability and opaque request identity through the native command', async () => {
  const calls = [];
  const client = { invoke: async (method, payload) => { calls.push([method, payload]); return { verified: true, message: 'Verified', identity: null }; } };
  const verifier = new InferenceVerificationController(client, () => 'opaque-one');
  verifier.configure(inferenceSelection(snapshot(), 'decision', protocols));
  await verifier.verify();
  assert.deepEqual(calls, [['inference.verify', { capability: 'decision', request_id: 'opaque-one' }]]);
  assert.equal(verifier.getSnapshot().result.verified, true);
});

test('reconfiguration cancels native work and a late result cannot verify the replacement', async () => {
  const operation = deferred(), calls = [];
  const client = { invoke: (method, payload) => { calls.push([method, payload]); return method === 'inference.verify' ? operation.promise : Promise.resolve(true); } };
  const verifier = new InferenceVerificationController(client, () => 'opaque-two');
  const state = snapshot(); verifier.configure(inferenceSelection(state, 'decision', protocols));
  const pending = verifier.verify(); const duplicate = verifier.verify();
  assert.equal(pending, duplicate);
  state.settings.decision.model = 'replacement'; verifier.configure(inferenceSelection(state, 'decision', protocols));
  assert.equal(verifier.getSnapshot().changed, true);
  assert.deepEqual(calls[1], ['inference.cancel', { request_id: 'opaque-two' }]);
  operation.resolve({ verified: true, message: 'Old choice', identity: null }); await pending;
  assert.equal(verifier.getSnapshot().result, null); assert.equal(verifier.getSnapshot().running, false);
});

test('leaving/cancelling ignores late errors and failed verification cannot become success', async () => {
  const operation = deferred();
  const client = { invoke: (method) => method === 'inference.verify' ? operation.promise : Promise.resolve(true) };
  const verifier = new InferenceVerificationController(client, () => 'opaque-three');
  verifier.configure(inferenceSelection(snapshot(), 'decision', protocols));
  const pending = verifier.verify(); verifier.cancel(); operation.reject({ code: 'network', message: 'Delayed failure' }); await pending;
  assert.equal(verifier.getSnapshot().result, null); assert.equal(verifier.getSnapshot().error, null);
  const failed = new InferenceVerificationController({ invoke: async () => ({ verified: false, message: 'Rejected', identity: null }) });
  failed.configure(inferenceSelection(snapshot(), 'decision', protocols)); await failed.verify();
  assert.equal(failed.getSnapshot().result.verified, false);
});

test('all inference copy exists in both interface languages with the same section labels', () => {
  assert.deepEqual(Object.keys(inferenceCopy.en).sort(), Object.keys(inferenceCopy.nl).sort());
  for (const value of Object.values(inferenceCopy.en)) assert.ok(value.trim());
  for (const value of Object.values(inferenceCopy.nl)) assert.ok(value.trim());
  assert.equal(inferenceCopy.en.decision, 'Decision processing');
  assert.equal(inferenceCopy.nl.reranking, 'Passages rangschikken');
});


test('managed Cloud is available for both capabilities only with its explicit metadata', () => {
  const cloud = { id: 'cloud', kind: 'pennel_cloud', name: 'Pennel Cloud', base_url: 'https://tampered.invalid' };
  for (const capability of ['decision', 'reranking']) {
    assert.equal(inferenceProviderEligible(cloud, capability, protocols), true);
    assert.equal(inferenceProviderEligible(cloud, capability, []), false);
    assert.equal(inferenceProtocolAllowed(cloud, capability, 'pennel_cloud', protocols), true);
    assert.equal(inferenceProtocolAllowed(cloud, capability, 'tei', protocols), false);
    assert.equal(inferenceProtocolAllowed(cloud, capability, 'pennel_cloud', protocols.map((entry) => ({ ...entry, managed: false }))), false);
    assert.equal(inferenceProtocolAllowed(cloud, capability, 'pennel_cloud', protocols.map((entry) => ({ ...entry, allowed_provider_kinds: undefined }))), false);
    const draft = selectInferenceProvider(inferenceDraft(capability), cloud, capability, protocols);
    assert.equal(draft.model, 'pennel-auto'); assert.equal(draft.protocol, 'pennel_cloud');
    assert.deepEqual(inferenceConfiguration(draft, capability, protocols), config({ provider_id: 'cloud', protocol: 'pennel_cloud', model: 'pennel-auto', upstream: null }));
    for (const changed of [{ model: 'other' }, { expected_model: 'other' }, { upstream: 'override' }]) assert.equal(inferenceConfiguration({ ...draft, ...changed }, capability, protocols), null);
  }
  assert.equal(inferenceProtocolAllowed(snapshot().settings.providers[0], 'decision', 'pennel_cloud', protocols), false);
});

test('leaving managed Cloud restores an editable compatible draft without changing saved settings', () => {
  const state = snapshot(), original = structuredClone(state.settings);
  const cloud = { id: 'cloud', kind: 'pennel_cloud', base_url: 'https://api.pennel.app' };
  const chosen = selectInferenceProvider(inferenceDraft('decision', config()), cloud, 'decision', protocols);
  assert.equal(chosen.upstream, ''); assert.equal(chosen.expected_model, '');
  const back = selectInferenceProvider(chosen, state.settings.providers[0], 'decision', protocols);
  assert.equal(back.protocol, 'open_router_decisions'); assert.equal(back.model, '');
  assert.deepEqual(state.settings, original);
});

test('Cloud verification requires sign-in and a current account epoch, without a user API key', () => {
  const state = snapshot();
  state.settings.providers.push({ id: 'cloud', kind: 'pennel_cloud', name: 'Pennel Cloud', base_url: 'https://api.pennel.app' });
  state.settings.decision = config({ provider_id: 'cloud', protocol: 'pennel_cloud', model: 'pennel-auto', upstream: null });
  state.cloud = { signed_in: false, account_epoch: 'one' };
  assert.equal(inferenceSelection(state, 'decision', protocols), null);
  state.cloud.signed_in = true;
  const first = inferenceSelection(state, 'decision', protocols);
  assert.ok(first); assert.equal(state.keys.cloud, undefined);
  state.cloud.account_epoch = 'two';
  assert.notEqual(inferenceSelection(state, 'decision', protocols).fingerprint, first.fingerprint);
  state.cloud.account_epoch = undefined;
  assert.equal(inferenceSelection(state, 'decision', protocols), null);
  state.cloud.account_epoch = 'two'; state.platform = 'android';
  assert.equal(inferenceSelection(state, 'decision', protocols), null);
});

test('account switching invalidates a pending managed sample and ignores its late success', async () => {
  const state = snapshot(), operation = deferred(), calls = [];
  state.settings.providers.push({ id: 'cloud', kind: 'pennel_cloud', name: 'Pennel Cloud', base_url: 'https://api.pennel.app' });
  state.settings.reranking = config({ provider_id: 'cloud', protocol: 'pennel_cloud', model: 'pennel-auto', upstream: null });
  state.cloud = { signed_in: true, account_epoch: 'one' };
  const verifier = new InferenceVerificationController({ invoke: (method, payload) => { calls.push([method, payload]); return method === 'inference.verify' ? operation.promise : Promise.resolve(true); } }, () => 'cloud-sample');
  verifier.configure(inferenceSelection(state, 'reranking', protocols));
  const pending = verifier.verify();
  state.cloud.account_epoch = 'two';
  verifier.configure(inferenceSelection(state, 'reranking', protocols));
  assert.deepEqual(calls[1], ['inference.cancel', { request_id: 'cloud-sample' }]);
  operation.resolve({ verified: true, message: 'Old account', identity: null });
  await pending;
  assert.equal(verifier.getSnapshot().result, null);
});
