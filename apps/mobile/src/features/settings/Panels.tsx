import React, { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { useFocusEffect, useRouter } from 'expo-router';
import { View } from 'react-native';
import type { InterfaceLanguage, MeetingTemplate, ModelChoice, ProviderAccount, ProviderKind, ProviderKindInfo, Settings } from '../../domain';
import { useQuery, useRuntime } from '../../runtime';
import { Actions, Body, Button, Choice, Field, Notice, Row, Section, Toggle, confirm } from '../../components/ui';
import { ModelVerification, selectedVerification, sameVerificationSelection, verificationPayload, type ModelTestResult, type VerificationTask } from '../../model-verification';
import { localizedFailure } from '../../errors';
import { ownedMetadata, templateLabel } from '../../owned-labels';
import { inferenceCopy } from '../../inference';
import { InferencePanels } from './InferencePanel';
import { CleanupVerificationPanel } from '../CleanupVerification';
import { KeyboardPanel } from './KeyboardPanel';

function useSettings() {
  const runtime = useRuntime();
  const settings = runtime.snapshot!.settings;
  const save = (patch: Partial<Settings>, extra: Record<string, unknown> = {}) => runtime.run<Settings>('settings-save', 'settings.update', { patch, ...extra });
  return { ...runtime, settings, save };
}

function ProviderEditor({ provider }: { provider: ProviderAccount }) {
  const { settings, snapshot, save, run, t, busy, language } = useSettings();
  const inferenceLabels = inferenceCopy[language];
  const [name, setName] = useState(provider.name);
  const [baseUrl, setBaseUrl] = useState(provider.base_url);
  const [key, setKey] = useState('');
  const [notice, setNotice] = useState<{ kind: 'key' } | { kind: 'availability'; result: ModelTestResult } | null>(null);
  const kind = snapshot?.kinds.find((kind) => kind.kind === provider.kind);
  const usesUrl = !['local', 'apple_speech', 'apple_intelligence', 'pennel_cloud', 'claude_code', 'codex', 'open_code', 'copilot', 'cursor_agent', 'gemini_cli'].includes(provider.kind);
  const unsaved = baseUrl.trim() !== provider.base_url || key.trim().length > 0;
  useEffect(() => { setNotice(null); }, [provider.base_url]);
  return <Section title={provider.name}>
    {kind?.availability.reason && kind.availability.status !== 'available' && <Notice>{kind.availability.reason}</Notice>}
    <Field label={t('common.name')} value={name} onChangeText={setName} />
    {usesUrl && <Field label={t('mobile.provider.base_url')} value={baseUrl} onChangeText={(value) => { setBaseUrl(value); setNotice(null); }} keyboardType="url" autoCapitalize="none" autoCorrect={false} />}
    {(name !== provider.name || baseUrl !== provider.base_url) && <Button title={t('common.save')} subtle busy={busy.has('settings-save')} onPress={() => void save({ providers: settings.providers.map((account) => account.id === provider.id ? { ...account, name: name.trim() || account.name, base_url: baseUrl.trim() } : account) })} />}
    {(kind?.requires_key || provider.kind === 'custom') && <><Notice>{provider.kind === 'custom' ? inferenceLabels.customKeyHelp : t(snapshot?.keys[provider.id] ? 'mobile.provider.key_configured' : 'mobile.provider.key_missing')}</Notice><Field label={provider.kind === 'custom' ? inferenceLabels.optionalKey : t('mobile.provider.key')} value={key} onChangeText={(value) => { setKey(value); setNotice(null); }} secureTextEntry autoCapitalize="none" autoCorrect={false} autoComplete="off" textContentType="none" /><Button title={t('common.save')} subtle disabled={!key.trim()} busy={busy.has(`key-${provider.id}`)} onPress={() => { void run(`key-${provider.id}`, 'providers.setKey', { provider_id: provider.id, key: key.trim() }).then((value) => { if (value !== undefined) { setKey(''); setNotice({ kind: 'key' }); } }); }} /></>}
    {notice && <Notice error={notice.kind === 'availability' && !notice.result.ok}>{notice.kind === 'key' ? t('mobile.provider.key_saved') : ownedMetadata(language, notice.result.message, notice.result.ok ? 'mobile.provider.test_success' : 'mobile.provider.test_failed')}</Notice>}
    {unsaved && <Notice>{t('mobile.provider.save_before_check')}</Notice>}
    <Actions><Button title={t('mobile.provider.test')} subtle disabled={kind?.availability.status !== 'available' || unsaved} busy={busy.has(`test-${provider.id}`)} onPress={() => { void run<ModelTestResult>(`test-${provider.id}`, 'providers.test', { provider_id: provider.id }).then((result) => { if (result) setNotice({ kind: 'availability', result: { ok: result.ok === true, message: result.message } }); }); }} /><Button title={t('common.delete')} subtle destructive onPress={() => confirm(t('common.delete'), t('mobile.provider.delete_help'), t('common.delete'), t('common.cancel'), () => void run('provider-remove', 'providers.remove', { provider_id: provider.id, id: provider.id }), true)} /></Actions>
  </Section>;
}

export function ProvidersPanel() {
  const { settings, snapshot, run, t, busy } = useSettings();
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<ProviderKind>('open_ai');
  return <>{settings.providers.map((provider) => <ProviderEditor key={provider.id} provider={provider} />)}<Section title={t('mobile.provider.new')}>{adding ? <><Choice label={t('common.provider')} value={selected} options={(snapshot?.kinds ?? []).map((kind) => ({ value: kind.kind, label: `${kind.label}${kind.availability.status !== 'available' && kind.availability.reason ? ` · ${kind.availability.reason}` : ''}`, disabled: kind.availability.status === 'unsupported' }))} onChange={setSelected} /><Actions><Button title={t('common.cancel')} subtle onPress={() => setAdding(false)} /><Button title={t('dictation.add')} busy={busy.has('provider-add')} onPress={() => { void run('provider-add', 'providers.add', { kind: selected }).then((value) => { if (value !== undefined) setAdding(false); }); }} /></Actions></> : <Button title={t('mobile.provider.new')} subtle icon="add" onPress={() => setAdding(true)} />}</Section></>;
}

function TaskModel({ task, label, speech }: { task: VerificationTask; label: string; speech: boolean }) {
  const { settings, snapshot, save, t, run, busy, store, language } = useSettings();
  const choice = settings[task];
  const provider = settings.providers.find((provider) => provider.id === choice.provider_id);
  const kind = snapshot?.kinds.find((kind) => kind.kind === provider?.kind);
  const [model, setModel] = useState(choice.model);
  const [models, setModels] = useState<{ id: string; name?: string | null }[]>([]);
  const [search, setSearch] = useState('');
  const [preparing, setPreparing] = useState(false);
  const [changedBeforeTest, setChangedBeforeTest] = useState(false);
  const verification = useMemo(() => new ModelVerification(async (selection) => {
    const result = await store.run<ModelTestResult>(`verify-model-${task}`, 'providers.test', verificationPayload(selection));
    if (!sameVerificationSelection(selection, selectedVerification(store.getSnapshot().snapshot, task))) {
      throw { code: 'provider_changed', message: 'The selected task configuration changed during verification.' };
    }
    return result;
  }), [store, task]);
  const outcome = useSyncExternalStore(verification.subscribe, verification.getSnapshot, verification.getSnapshot);
  const current = selectedVerification(snapshot, task);
  const selectionKey = JSON.stringify(current);
  const draftSaved = model.trim() === choice.model;
  useEffect(() => { setModel(choice.model); setModels([]); setChangedBeforeTest(false); }, [choice.model, choice.provider_id]);
  useEffect(() => { verification.configure(draftSaved ? current : null); }, [draftSaved, selectionKey, verification]);
  useFocusEffect(useCallback(() => () => verification.invalidate(), [verification]));
  const testing = preparing || outcome.running;
  const anotherTest = [...busy].some((key) => key.startsWith('verify-model-'));
  const locked = testing || anotherTest || busy.has('settings-save');
  const hasCredentials = provider?.kind === 'pennel_cloud' ? snapshot?.cloud.signed_in === true : !kind?.requires_key || Boolean(provider && snapshot?.keys[provider.id]);
  const canTest = Boolean(provider && model.trim() && kind?.availability.status === 'available' && hasCredentials);
  const eligible = settings.providers.filter((provider) => { const kind = snapshot?.kinds.find((kind) => kind.kind === provider.kind); return speech ? kind?.stt : kind?.llm; });
  const setChoice = (next: ModelChoice) => save({ [task]: next });
  const testModel = async () => {
    if (!provider || !canTest || locked) return;
    setPreparing(true); setChangedBeforeTest(false); verification.invalidate();
    try {
      const requestedModel = model.trim();
      if (requestedModel !== choice.model && await setChoice({ ...choice, model: requestedModel }) === undefined) return;
      const selected = selectedVerification(store.getSnapshot().snapshot, task);
      if (!selected || selected.provider_id !== provider.id || selected.model !== requestedModel) { setChangedBeforeTest(true); return; }
      verification.configure(selected);
      await verification.verify();
    } finally { setPreparing(false); }
  };
  const readiness = testing ? 'providers.testing' : !current || !draftSaved ? 'providers.not_configured' : outcome.result?.ok ? 'providers.test_passed' : 'providers.configured';
  return <Section title={label}>
    <Choice label={t('common.provider')} value={choice.provider_id} options={[{ value: '', label: t('mobile.none_selected'), disabled: locked }, ...eligible.map((provider) => ({ value: provider.id, label: provider.name, disabled: locked || snapshot?.kinds.find((kind) => kind.kind === provider.kind)?.availability.status !== 'available' }))]} onChange={(providerId) => {
      const account = settings.providers.find((provider) => provider.id === providerId);
      const info = snapshot?.kinds.find((kind) => kind.kind === account?.kind) as (ProviderKindInfo & { default_stt_model?: string; default_llm_model?: string }) | undefined;
      verification.invalidate();
      void setChoice({ provider_id: providerId, model: (speech ? info?.default_stt_model : info?.default_llm_model) ?? '' });
    }} />
    {provider && <>
      <Field label={t('mobile.provider.model_id')} value={model} editable={!locked} onChangeText={(value) => { setModel(value); setChangedBeforeTest(false); verification.invalidate(); }} autoCapitalize="none" autoCorrect={false} />
      {!draftSaved && <Button title={t('common.save')} subtle disabled={locked} onPress={() => void setChoice({ ...choice, model: model.trim() })} />}
      <Button title={t('common.model')} subtle disabled={locked} busy={busy.has(`models-${task}`)} onPress={() => { void run<{ id: string; name?: string | null }[]>(`models-${task}`, 'providers.models', { provider_id: provider.id, task }).then((value) => { if (value) setModels(value); }); }} />
      {models.length > 0 && <><Field label={t('common.search')} value={search} onChangeText={setSearch} autoCapitalize="none" autoCorrect={false} />{models.filter((item) => `${item.id} ${item.name ?? ''}`.toLowerCase().includes(search.toLowerCase())).slice(0, 30).map((item) => <Row key={item.id} title={item.name || item.id} subtitle={item.name ? item.id : undefined} disabled={locked} onPress={() => { verification.invalidate(); setModel(item.id); void setChoice({ ...choice, model: item.id }); }} />)}</>}
    </>}
    <Body muted>{t(readiness)}</Body>
    <Button title={t(testing ? 'providers.testing' : speech ? 'providers.test_speech' : 'providers.test_text')} busy={testing} disabled={!canTest || locked} onPress={() => void testModel()} />
    <Notice>{t('mobile.models.sample_help', { provider: provider?.name ?? t('mobile.none_selected') })}</Notice>
    {testing && <Notice>{t(outcome.changed ? 'mobile.models.previous_running' : 'mobile.models.running')}</Notice>}
    {changedBeforeTest && <Notice error>{t('mobile.models.changed')}</Notice>}
    {outcome.error && <Notice error>{localizedFailure(language, outcome.error)}</Notice>}
    {outcome.result && draftSaved && <Notice error={!outcome.result.ok}>{ownedMetadata(language, outcome.result.message, outcome.result.ok ? speech ? 'mobile.models.speech_passed' : 'mobile.models.text_passed' : 'mobile.models.failed')}</Notice>}
    {provider && !hasCredentials && <Notice>{provider.kind === 'pennel_cloud' ? t('meetings.speaker_cloud_sign_in') : t('providers.add_key', { provider: provider.name })}</Notice>}
    {kind?.availability.status !== 'available' && kind?.availability.reason && <Notice>{kind.availability.reason}</Notice>}
  </Section>;
}

export function ModelsPanel() {
  const { t, snapshot, busy } = useSettings();
  const router = useRouter();
  const testing = [...busy].some((key) => key.startsWith('verify-model-'));
  return <><Section><Button title={t('mobile.provider.new')} subtle disabled={testing} onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'providers' } })} /></Section><TaskModel task="dictation_stt" label={t('mobile.provider.task.dictation')} speech /><TaskModel task="meeting_stt" label={t('mobile.provider.task.meetings')} speech /><TaskModel task="llm" label={t('mobile.provider.task.text')} speech={false} /><InferencePanels /><Section title={t('navigation.models')}><Notice>{snapshot?.capabilities.local_models?.reason || t('mobile.local_models')}</Notice></Section></>;
}

export function DictationPanel() {
  const { settings, save, t, language } = useSettings();
  const appleAutomatic = settings.language === null && settings.providers.find((provider) => provider.id === settings.dictation_stt.provider_id)?.kind === 'apple_speech';
  return <>
    <Section><Choice label={t('dictation.mode.title')} value={settings.mode} options={[{ value: 'fast', label: t('dictation.mode.fast') }, { value: 'clean', label: t('dictation.mode.clean') }]} onChange={(mode) => void save({ mode })} /><Choice label={t('dictation.language.title')} value={settings.language ?? ''} options={[{ value: '', label: t('common.automatic') }, { value: 'nl', label: t('dictation.language.dutch') }, { value: 'en', label: t('dictation.language.english') }, { value: 'fr', label: t('dictation.language.french') }, { value: 'de', label: t('dictation.language.german') }]} onChange={(language) => void save({ language: language || null })} /><Choice label={t('dictation.history.retention')} value={settings.history_retention_days} options={[30, 90, 365, 0].map((value) => ({ value, label: value ? t('dictation.history.days', { days: value }) : t('dictation.history.always') }))} onChange={(history_retention_days) => void save({ history_retention_days })} /></Section>
    <CleanupVerificationPanel />
    {appleAutomatic && <Notice>{language === 'nl' ? 'Automatisch gebruikt bij Apple dicteren de voorkeurstaal van je iPhone. Kies Nederlands of Engels om de spreektaal vast te zetten.' : 'Automatic uses your iPhone’s preferred language with Apple dictation. Choose Dutch or English to set a specific speech language.'}</Notice>}
    <KeyboardPanel />
    <Notice>{t('mobile.keyboard.goal')}</Notice>
  </>;
}

export function MeetingsPanel() {
  const { settings, snapshot, save, t, language } = useSettings();
  const templates = useQuery<MeetingTemplate[]>('meetings.templates');
  const consent = () => confirm(t('mobile.speakers.consent_title'), t('mobile.speakers.consent'), t('mobile.speakers.allow'), t('common.cancel'), () => void save({ diarization: true, speaker_recognition: 'cloud' }, { cloud_audio_consent: true }));
  return <>
    <Section><Choice label={t('meetings.template')} value={settings.default_template} options={(templates.value ?? []).map((template) => ({ value: template.id, label: templateLabel(template, language) }))} onChange={(default_template) => void save({ default_template })} /><Choice label={t('settings.audio.title')} value={settings.meeting_audio_retention_days} options={[30, 90, 365, 0].map((value) => ({ value, label: value ? t('dictation.history.days', { days: value }) : t('dictation.history.always') }))} onChange={(meeting_audio_retention_days) => void save({ meeting_audio_retention_days })} /></Section>
    <Section title={t('mobile.speakers.choice')}><Choice label={t('mobile.speakers.choice')} value={settings.speaker_recognition} options={[{ value: 'local', label: `${t('mobile.speakers.local')} · ${t('mobile.speakers.local_help')}`, disabled: true }, { value: 'cloud', label: t('mobile.speakers.cloud') }]} onChange={(speaker_recognition) => void save({ speaker_recognition })} /><Toggle title={t('mobile.speakers.auto')} value={settings.diarization && settings.speaker_recognition === 'cloud'} disabled={!snapshot?.cloud.signed_in} onChange={(enabled) => enabled ? consent() : void save({ diarization: false })} />{!snapshot?.cloud.signed_in && <Notice>{t('meetings.speaker_cloud_sign_in')}</Notice>}<Notice>{t('mobile.speakers.consent')}</Notice></Section>
    <Section><Notice>{t('mobile.microphone_only')}</Notice></Section>
  </>;
}

export function RecordingPanel() { const { t } = useSettings(); return <Section description={t('mobile.recording.disabled')}>{['call_detection', 'calendar', 'system_audio', 'echo_cancellation'].map((key) => <Unavailable key={key} capability={key} />)}</Section>; }
function Unavailable({ capability }: { capability: string }) { const { t, snapshot } = useSettings(); const labels: Record<string, string> = { call_detection: 'settings.recording.detect_title', calendar: 'settings.calendar.title', system_audio: 'dictation.processing.audio', echo_cancellation: 'settings.recording.echo' }; return <Row title={t(labels[capability] ?? 'common.unavailable')} subtitle={snapshot?.capabilities[capability]?.reason || t('mobile.recording.disabled')} disabled />; }
export function StylesPanel() { const { t } = useSettings(); return <Section title={t('dictation.style.title')} description={t('mobile.styles.disabled')}><Choice label={t('dictation.style.title')} value="neutral" options={[{ value: 'neutral', label: t('dictation.style.neutral') }, ...(['casual', 'formal', 'literal'] as const).map((value) => ({ value, label: t(`dictation.style.${value}`), disabled: true }))]} onChange={() => {}} /></Section>; }

export function DictionaryPanel() {
  const { settings, save, t, busy } = useSettings();
  const [term, setTerm] = useState('');
  const add = async () => { const value = term.trim(); if (!value) return; const next = [...new Set([...settings.dictionary, value])]; const result = await save({ dictionary: next }); if (result !== undefined) setTerm(''); };
  return <Section description={t('dictation.dictionary.help')}><Field label={t('mobile.new_term')} placeholder={t('dictation.dictionary.example')} value={term} onChangeText={setTerm} autoCorrect={false} returnKeyType="done" onSubmitEditing={() => void add()} /><Button title={t('dictation.add')} subtle disabled={!term.trim()} busy={busy.has('settings-save')} onPress={() => void add()} />{settings.dictionary.map((value) => <Row key={value} title={value} trailing={<Button title={t('common.delete')} subtle destructive onPress={() => void save({ dictionary: settings.dictionary.filter((term) => term !== value) })} />} />)}</Section>;
}

export function AppearancePanel() {
  const { settings, save, t } = useSettings();
  const router = useRouter();
  return <><Section description={t('appearance.help')}><Choice label={t('appearance.title')} value={settings.appearance} options={[{ value: 'system', label: t('appearance.system') }, { value: 'light', label: t('appearance.light') }, { value: 'dark', label: t('appearance.dark') }]} onChange={(appearance) => void save({ appearance })} /></Section><Section description={t('interface.language_help')}><Choice label={t('interface.language')} value={settings.ui_language} options={[{ value: 'system', label: t('appearance.system') }, { value: 'en', label: 'English' }, { value: 'nl', label: 'Nederlands' }]} onChange={(ui_language) => void save({ ui_language: ui_language as 'system' | InterfaceLanguage })} /></Section><Section title={t('intro.title')} description={t('intro.help')}><Button title={t('intro.open')} subtle onPress={() => router.push('/onboarding')} /></Section></>;
}
