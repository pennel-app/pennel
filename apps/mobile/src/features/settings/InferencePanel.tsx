import React, { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { useFocusEffect, useRouter } from 'expo-router';
import type { InferenceAvailability, InferenceCapability, InferenceConfig, InferenceProtocolInfo } from '../../domain';
import { useQuery, useRuntime } from '../../runtime';
import { Actions, Body, Button, Choice, Field, Notice, Row, Section } from '../../components/ui';
import { inferenceConfiguration, inferenceConfigIdentity, inferenceCopy, inferenceDraft, inferenceProviderEligible, inferenceProtocolAllowed, inferenceSelection, InferenceVerificationController, selectInferenceProvider, type InferenceDraft } from '../../inference';
import { localizedFailure } from '../../errors';
import { ownedMetadata } from '../../owned-labels';

export function InferencePanels() {
  const { snapshot } = useRuntime();
  const protocols = useQuery<InferenceProtocolInfo[]>('inference.protocols', {}, snapshot?.platform === 'ios');
  return <><InferencePanel capability="decision" protocols={protocols.value ?? []} /><InferencePanel capability="reranking" protocols={protocols.value ?? []} /></>;
}

function InferencePanel({ capability, protocols }: { capability: InferenceCapability; protocols: InferenceProtocolInfo[] }) {
  const { snapshot, store, language, run, busy, t } = useRuntime();
  const copy = inferenceCopy[language];
  const router = useRouter();
  const saved = snapshot?.settings[capability] ?? null;
  const savedKey = inferenceConfigIdentity(saved);
  const [draft, setDraft] = useState(() => inferenceDraft(capability, saved));
  const [advanced, setAdvanced] = useState(false);
  const verifier = useMemo(() => new InferenceVerificationController(store.client), [store]);
  const verification = useSyncExternalStore(verifier.subscribe, verifier.getSnapshot, verifier.getSnapshot);
  const provider = snapshot?.settings.providers.find((account) => account.id === draft.provider_id);
  const configuration = inferenceProtocolAllowed(provider, capability, draft.protocol, protocols) ? inferenceConfiguration(draft, capability, protocols) : null;
  const configurationKey = inferenceConfigIdentity(configuration);
  const draftSaved = saved !== null && configurationKey === savedKey;
  const selectedProtocol = protocols.find((entry) => entry.protocol === draft.protocol);
  const managed = draft.protocol === 'pennel_cloud' && provider?.kind === 'pennel_cloud';
  const cloudSignedIn = snapshot?.cloud.signed_in === true;
  const selected = draftSaved ? inferenceSelection(snapshot, capability, protocols) : null;
  const selectionKey = selected?.fingerprint ?? '';
  const supported = snapshot?.platform === 'ios';
  const availability = useQuery<InferenceAvailability>('inference.availability', { capability }, supported && saved !== null);
  const working = busy.has(`save-inference-${capability}`);
  useEffect(() => { setDraft(inferenceDraft(capability, saved)); }, [capability, savedKey]);
  useEffect(() => { verifier.configure(selected); }, [selectionKey, verifier]);
  useEffect(() => { if (saved !== null) void availability.refresh(); }, [savedKey, snapshot?.cloud.account_epoch]);
  useFocusEffect(useCallback(() => () => verifier.invalidate(), [verifier]));
  const change = <K extends keyof InferenceDraft>(field: K, value: InferenceDraft[K]) => {
    verifier.invalidate(); setDraft((current) => ({ ...current, [field]: value }));
  };
  const save = async (next: InferenceConfig | null) => {
    verifier.invalidate();
    await run(`save-inference-${capability}`, 'settings.update', { patch: { [capability]: next } });
  };
  const missingKey = Boolean(provider && selectedProtocol?.requires_key && !snapshot?.keys[provider.id]);
  const eligible = snapshot?.settings.providers.filter((account) => inferenceProviderEligible(account, capability, protocols)) ?? [];
  const formats = protocols.filter((entry) => entry[capability]);
  return <Section title={copy[capability]} description={copy.help}>
    {!supported && <Notice>{copy.android}</Notice>}
    <Choice label={copy.provider} value={draft.provider_id} options={[{ value: '', label: copy.disabled, disabled: !supported || working }, ...eligible.map((account) => ({ value: account.id, label: account.name, disabled: !supported || working }))]} onChange={(value) => {
      const account = eligible.find((entry) => entry.id === value);
      verifier.invalidate();
      setDraft((current) => selectInferenceProvider(current, account, capability, protocols));
    }} />
    {eligible.length === 0 && <Notice>{copy.providerHelp}</Notice>}
    {!managed && <Choice label={copy.protocol} value={draft.protocol} options={formats.length ? formats.map((entry) => ({ value: entry.protocol, label: entry.name, disabled: !supported || working || !inferenceProtocolAllowed(provider, capability, entry.protocol, protocols) })) : [{ value: draft.protocol, label: draft.protocol, disabled: true }]} onChange={(value) => change('protocol', value)} />}
    {!managed && <Field label={copy.model} value={draft.model} editable={supported && !working} onChangeText={(value) => change('model', value)} autoCapitalize="none" autoCorrect={false} />}
    {managed && <Body muted>{copy.managed}</Body>}
    <Button title={advanced ? copy.less : copy.advanced} subtle onPress={() => setAdvanced((value) => !value)} />
    {advanced && <>
      {!managed && <Field label={copy.expected} value={draft.expected_model} editable={supported && !working} onChangeText={(value) => change('expected_model', value)} autoCapitalize="none" autoCorrect={false} />}
      {draft.protocol.startsWith('open_router_') && <Field label={copy.upstream} value={draft.upstream} editable={supported && !working} onChangeText={(value) => change('upstream', value)} autoCapitalize="none" autoCorrect={false} />}
      <Field label={copy.bytes} value={draft.max_input_bytes} editable={supported && !working} onChangeText={(value) => change('max_input_bytes', value)} keyboardType="number-pad" />
      <Field label={copy.timeout} value={draft.timeout_secs} editable={supported && !working} onChangeText={(value) => change('timeout_secs', value)} keyboardType="number-pad" />
    </>}
    {provider && <Row title={copy.destination} subtitle={draftSaved && availability.value?.endpoint ? availability.value.endpoint : managed ? 'Pennel Cloud' : provider.base_url} />}
    {draft.protocol === 'local_decider' && <Notice>{copy.loopback}</Notice>}
    <Body muted>{saved ? managed && cloudSignedIn ? copy.cloudReady : copy.configured : copy.disabled}</Body>
    {managed && !cloudSignedIn && <><Notice>{copy.cloudSignIn}</Notice><Button title={copy.cloudAccount} subtle disabled={!supported} onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'account' } })} /></>}
    {!configuration && provider && <Notice>{copy.invalid}</Notice>}
    {saved && !draftSaved && <Notice>{copy.saveFirst}</Notice>}
    {draftSaved && availability.value?.available === false && <Notice>{ownedMetadata(language, availability.value.reason ?? '', 'providers.not_configured')}</Notice>}
    {missingKey && <Notice>{copy.missingKey}</Notice>}
    <Actions>
      <Button title={copy.save} disabled={!supported || !configuration || draftSaved} busy={working} onPress={() => { if (configuration) void save(configuration); }} />
      <Button title={copy.disable} subtle disabled={!supported || saved === null} busy={working} onPress={() => void save(null)} />
      <Button title={copy.test} subtle busy={verification.running} disabled={!supported || !draftSaved || !selected || availability.value?.available !== true || working} onPress={() => void verifier.verify()} />
      {verification.running && <Button title={copy.cancel} subtle onPress={verifier.cancel} />}
    </Actions>
    <Notice>{copy.sample}</Notice>
    {verification.changed && <Notice>{copy.changed}</Notice>}
    {verification.error && <Notice error>{localizedFailure(language, verification.error)}</Notice>}
    {verification.result && <>
      <Notice error={!verification.result.verified}>{ownedMetadata(language, verification.result.message, verification.result.verified ? 'providers.test_passed' : 'mobile.provider.test_failed')}</Notice>
      {verification.result.identity && <>
        <Row title={copy.actualModel} subtitle={verification.result.identity.actual_model ?? copy.unknown} />
        <Row title={copy.actualProvider} subtitle={verification.result.identity.actual_provider ?? copy.unknown} />
        {verification.result.identity.actual_revision && <Row title={copy.revision} subtitle={verification.result.identity.actual_revision} />}
        <Row title={copy.request} subtitle={verification.result.identity.request_id ?? copy.unknown} />
      </>}
    </>}
    {availability.error && <Notice error>{availability.error}</Notice>}
  </Section>;
}
