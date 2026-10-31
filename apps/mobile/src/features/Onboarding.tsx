import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { View } from 'react-native';
import { useRouter } from 'expo-router';
import type { ProviderKind, ProviderKindInfo, RuntimeSnapshot, Settings } from '../domain';
import { useRuntime } from '../runtime';
import { Actions, Body, Button, Choice, Loading, Notice, Page, Section } from '../components/ui';
import { ActiveCleanupVerification } from './CleanupVerification';
import { ownedFailureText } from '../errors';
import { useHapticFeedback } from '../haptics';
import { workflowText } from '../workflow-copy';
import { restoreOnboardingStep, speechReadiness, type OnboardingGoal, type OnboardingStage } from '../workflow-state';
import { PracticeSession } from '../practice-session';

type SpeechChoice = 'apple' | 'cloud' | 'own';
const stages: OnboardingStage[] = ['welcome', 'engine', 'permissions', 'practice'];

export default function Onboarding() {
  const { snapshot, run, busy, t, theme, language, store } = useRuntime();
  const router = useRouter();
  const haptic = useHapticFeedback();
  const copy = (key: Parameters<typeof workflowText>[1]) => workflowText(language, key);
  const [stage, setStage] = useState<OnboardingStage>('welcome');
  const [goal, setGoal] = useState<OnboardingGoal>('dictation');
  const [speech, setSpeech] = useState<SpeechChoice>('apple');
  const [microphone, setMicrophone] = useState(false);
  const practiceSession = useRef(new PracticeSession()).current;
  const practice = useSyncExternalStore(practiceSession.subscribe, practiceSession.getSnapshot, practiceSession.getSnapshot);
  const mounted = useRef(true);
  const latestSnapshot = useRef(snapshot);
  latestSnapshot.current = snapshot;
  const initialized = useRef(false);
  const settings = snapshot?.settings;
  const phase = snapshot?.dictation.phase ?? 'idle';
  const ownsPractice = Boolean(snapshot && practiceSession.owns(snapshot.dictation));
  const previousPractice = snapshot ? practiceSession.recoveryCommand(snapshot.dictation) : null;
  const practicePhase = ownsPractice ? phase : 'idle';
  const practiceActive = practice.pending || (ownsPractice && phase !== 'idle');
  const succeeded = Boolean(practice.result?.trim());
  const transitioning = busy.has('onboarding-provider') || busy.has('onboarding-settings') || busy.has('onboarding-meeting');
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const current = latestSnapshot.current?.dictation;
      const payload = current ? practiceSession.command(current) : null;
      practiceSession.abandon();
      if (payload) void store.client.invoke('dictation.cancel', payload).catch(() => undefined);
    };
  }, [practiceSession, store]);
  useEffect(() => {
    if (!settings || initialized.current) return;
    initialized.current = true;
    const savedGoal = settings.onboarding.goal === 'meeting' ? 'meeting' : 'dictation';
    const selection = savedGoal === 'meeting' ? settings.meeting_stt : settings.dictation_stt;
    const provider = settings.providers.find((provider) => provider.id === selection.provider_id);
    setGoal(savedGoal);
    setSpeech(provider ? provider.kind === 'apple_speech' ? 'apple' : provider.kind === 'pennel_cloud' ? 'cloud' : 'own' : 'apple');
    setStage(restoreOnboardingStep(settings.onboarding.resume_step));
  }, [settings]);
  useEffect(() => { if (snapshot) practiceSession.observe(snapshot.dictation); }, [practiceSession, snapshot?.dictation]);
  const saveSettings = (patch: Partial<Settings>) => run<Settings>('onboarding-settings', 'settings.update', { patch });
  const progress = async (nextStage: OnboardingStage, nextGoal = goal, current = settings) => {
    if (!current) return false;
    const task = nextGoal === 'meeting' ? 'meeting_stt' : 'dictation_stt';
    const result = await saveSettings({ [task]: current[task], onboarding: { ...current.onboarding, goal: nextGoal, resume_step: nextStage, status: 'pending' } });
    if (!result) return false;
    setGoal(nextGoal); setStage(nextStage);
    return true;
  };
  const finish = async (status: 'completed' | 'skipped') => {
    if (!settings || practiceActive || (status === 'completed' && !succeeded)) return;
    const result = await saveSettings({ onboarding: { ...settings.onboarding, goal, resume_step: stage, status } });
    if (result !== undefined) router.replace(goal === 'dictation' ? '/(tabs)/dictation' : '/(tabs)');
  };
  const chooseSpeech = async () => {
    if (!settings) return;
    if (speech === 'own') {
      if (snapshot && speechReadiness(settings, snapshot.kinds, snapshot.keys, goal)) await progress('permissions');
      return;
    }
    const kind: ProviderKind = speech === 'cloud' ? 'pennel_cloud' : 'apple_speech';
    let current = settings;
    let provider = current.providers.find((provider) => provider.kind === kind);
    if (!provider) {
      const added = await run<Settings>('onboarding-provider', 'providers.add', { kind });
      if (!added) return;
      current = added;
      provider = current.providers.find((provider) => provider.kind === kind);
    }
    if (!provider) return;
    const info = snapshot?.kinds.find((info) => info.kind === kind) as (ProviderKindInfo & { default_stt_model?: string }) | undefined;
    const model = info?.default_stt_model ?? (kind === 'pennel_cloud' ? 'pennel-auto' : 'apple');
    await progress('permissions', goal, { ...current, [goal === 'meeting' ? 'meeting_stt' : 'dictation_stt']: { provider_id: provider.id, model } });
  };
  const startPractice = async () => {
    const before = latestSnapshot.current?.dictation;
    if (!before || before.phase !== 'idle' || practiceSession.getSnapshot().pending) return;
    const attempt = practiceSession.begin(before);
    const result = await run<RuntimeSnapshot>('onboarding-practice', 'dictation.start', { practice: true });
    const freshId = result?.dictation.practice && result.dictation.session_id && result.dictation.session_id !== before.session_id ? result.dictation.session_id : null;
    if (!mounted.current) {
      if (freshId) void store.client.invoke('dictation.cancel', { session_id: freshId }).catch(() => undefined);
      return;
    }
    if (result && practiceSession.acceptStart(attempt, result.dictation)) {
      if (result.dictation.phase === 'listening') haptic('recordingStarted');
    } else {
      practiceSession.rejectStart(attempt);
      if (freshId) void store.client.invoke('dictation.cancel', { session_id: freshId }).catch(() => undefined);
      haptic('error');
    }
  };
  const stopPractice = async () => {
    const current = latestSnapshot.current?.dictation;
    const payload = current ? practiceSession.command(current) : null;
    if (!payload || current?.phase !== 'listening') return;
    const result = await run<RuntimeSnapshot>('onboarding-practice', 'dictation.stop', payload);
    if (result?.dictation.practice && result.dictation.result_session_id === payload.session_id) {
      practiceSession.observe(result.dictation);
      haptic(result.dictation.error ? 'error' : 'recordingStopped');
    }
  };
  const cancelPractice = async () => {
    const current = latestSnapshot.current?.dictation;
    const payload = current ? practiceSession.command(current) : null;
    if (!payload) return;
    const result = await run('onboarding-cancel', 'dictation.cancel', payload);
    if (result !== undefined) { practiceSession.abandon(); haptic('recordingCancelled'); }
  };
  const startMeeting = async () => {
    const result = await run<RuntimeSnapshot>('onboarding-meeting', 'meetings.start', { title: '' });
    if (result?.meeting.id) { haptic('recordingStarted'); router.replace('/recording'); }
    else if (result === undefined) haptic('error');
  };
  if (!snapshot || !settings) return <Page title="Pennel"><Loading /></Page>;
  const microphoneReady = microphone || snapshot.capabilities.microphone?.status === 'available';
  const canUseSpeech = speech === 'cloud' ? snapshot.cloud.signed_in : speech === 'own' ? speechReadiness(settings, snapshot.kinds, snapshot.keys, goal) : snapshot.kinds.find((kind) => kind.kind === 'apple_speech')?.availability.status === 'available';
  return <Page title="Pennel" bottomInset>
    <Body muted accessibilityLabel={`${stages.indexOf(stage) + 1} / ${stages.length}`} style={{ marginBottom: theme.space[5], fontSize: theme.size.content }}>{stages.indexOf(stage) + 1} / {stages.length}</Body>
    {stage === 'welcome' && <>
      <Section title={copy('chooseTask')} description={copy('chooseTaskHelp')}>
        <Body>{t('mobile.onboarding.intro')}</Body>
        <Button title={copy('tryDictation')} icon="mic-outline" busy={transitioning} onPress={() => void progress('engine', 'dictation')} />
        <Button title={copy('recordMeeting')} icon="add" subtle disabled={transitioning} onPress={() => void progress('engine', 'meeting')} />
      </Section>
      <Section><Choice label={t('interface.language')} value={settings.ui_language} options={[{ value: 'system', label: t('appearance.system') }, { value: 'en', label: 'English' }, { value: 'nl', label: 'Nederlands' }]} onChange={(ui_language) => void saveSettings({ ui_language: ui_language as Settings['ui_language'] })} /></Section>
    </>}
    {stage === 'engine' && <Section title={copy('engine')} description={copy('engineHelp')}>
      <Choice label={t('common.provider')} value={speech} options={[{ value: 'apple', label: t('mobile.onboarding.apple') }, { value: 'cloud', label: t('mobile.onboarding.cloud') }, { value: 'own', label: t('mobile.onboarding.own') }]} onChange={setSpeech} />
      <Notice>{t(`mobile.onboarding.${speech}_help`)}</Notice>
      {speech === 'apple' && snapshot.kinds.find((kind) => kind.kind === 'apple_speech')?.availability.reason && <Notice>{snapshot.kinds.find((kind) => kind.kind === 'apple_speech')?.availability.reason}</Notice>}
      {speech === 'cloud' && !snapshot.cloud.signed_in && <Button title={t('account.login.sign_in')} subtle onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'account' } })} />}
      {speech === 'own' && <Button title={t('navigation.models')} subtle onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'models' } })} />}
      <Button title={t('mobile.onboarding.next')} disabled={!canUseSpeech} busy={transitioning} onPress={() => void chooseSpeech()} />
    </Section>}
    {stage === 'permissions' && <Section title={t('mobile.onboarding.microphone')} description={copy('permissionHelp')}>
      <Notice>{copy(microphoneReady ? 'permissionReady' : 'permissionMissing')}</Notice>
      {!microphoneReady && <><Button title={t('mobile.onboarding.microphone')} busy={busy.has('microphone-permission')} onPress={() => void run<{ granted: boolean }>('microphone-permission', 'permissions.microphone').then((result) => setMicrophone(result?.granted === true))} /><Button title={t('mobile.open_settings')} subtle onPress={() => void run('open-settings', 'system.openSettings')} /></>}
      <Button title={t('mobile.onboarding.next')} disabled={!microphoneReady} busy={transitioning} onPress={() => void progress('practice')} />
    </Section>}
    {stage === 'practice' && goal === 'dictation' && <Section title={copy('practiceTitle')} description={copy('practiceHelp')}>
      <Button title={practicePhase === 'listening' ? copy('practiceStop') : practicePhase === 'processing' ? t('dictation.pad.processing') : copy(succeeded ? 'practiceAgain' : 'practiceStart')} icon={practicePhase === 'listening' ? 'stop' : 'mic-outline'} disabled={practice.pending || practicePhase === 'processing' || (!ownsPractice && phase !== 'idle')} busy={busy.has('onboarding-practice')} onPress={() => void (practicePhase === 'listening' ? stopPractice() : startPractice())} />
      {previousPractice ? <><Notice>{copy('previousPractice')}</Notice><Button title={copy('cancelPreviousPractice')} subtle busy={busy.has('onboarding-recover')} onPress={() => void run('onboarding-recover', 'dictation.cancel', previousPractice).then((result) => { if (result !== undefined) haptic('recordingCancelled'); })} /></> : !ownsPractice && !practice.pending && phase !== 'idle' && <Notice>{copy('otherDictationActive')}</Notice>}
      {ownsPractice && practicePhase !== 'idle' && <><Button title={t('common.cancel')} subtle onPress={() => void cancelPractice()} /><ActiveCleanupVerification /></>}
      {snapshot.dictation.error && snapshot.dictation.practice && (snapshot.dictation.session_id === practice.sessionId || snapshot.dictation.result_session_id === practice.sessionId) && practice.sessionId && <Notice error>{ownedFailureText(language, snapshot.dictation.error, snapshot.dictation.error_code)}</Notice>}
      {succeeded && <View style={{ gap: theme.space[3] }}><Notice>{copy('practiceSuccess')}</Notice><Body selectable>{practice.result}</Body><Button title={t('common.copy')} subtle onPress={() => void run('copy-practice', 'dictation.copy', { text: practice.result }).then((result) => { if (result !== undefined) haptic('resultDelivered'); })} /><Notice>{copy('keyboardLater')}</Notice><Button title={t('mobile.onboarding.finish')} busy={transitioning} onPress={() => void finish('completed')} /></View>}
    </Section>}
    {stage === 'practice' && goal === 'meeting' && <Section title={copy('firstMeeting')} description={copy('firstMeetingHelp')}><Button title={t(snapshot.meeting.id && snapshot.meeting.phase !== 'idle' ? 'meetings.return_recording' : 'common.start_recording')} busy={busy.has('onboarding-meeting')} onPress={() => snapshot.meeting.id && snapshot.meeting.phase !== 'idle' ? router.replace('/recording') : void startMeeting()} /></Section>}
    <Actions>{stage !== 'welcome' && <Button title={t('common.back')} subtle disabled={practiceActive || transitioning} onPress={() => void progress(stages[Math.max(0, stages.indexOf(stage) - 1)])} />}<Button title={t('mobile.onboarding.skip')} subtle disabled={practiceActive || transitioning} onPress={() => void finish('skipped')} /></Actions>
  </Page>;
}
