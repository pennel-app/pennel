import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useFocusEffect, useLocalSearchParams, usePreventRemove, useRouter } from 'expo-router';
import type { CloudLinkView, CloudRecoveryDisplay, CloudSyncSetup } from '../../../modules/pennel-mobile/src/types';
import { useQuery, useRuntime } from '../../runtime';
import { dateLabel } from '../../format';
import { localizedFailure } from '../../errors';
import { linkIsPending, recoveryDisplayFor, requiresRecovery, syncSetupStage, syncStatusKey } from '../../sync-setup';
import { localizedConnectionFailure } from '../../device-link';
import { useLinkAvailability } from '../../use-link-availability';
import { Actions, Body, Button, Field, Loading, Notice, Row, Section, confirm } from '../../components/ui';
import { CloudSignIn } from './CloudSignIn';
import { RecoveryBackup } from './RecoveryBackup';

export function SyncPanel() {
  const { snapshot, run, store, t, busy, error, language } = useRuntime();
  const cloud = snapshot!.cloud;
  const setup = useQuery<CloudSyncSetup>('cloud.setup', {}, cloud.signed_in);
  const link = useQuery<CloudLinkView>('cloud.linkView');
  const availability = useLinkAvailability();
  const params = useLocalSearchParams<{ fallback?: string }>();
  const router = useRouter();
  const [email, setEmail] = useState(Boolean(cloud.login?.start) || params.fallback === 'email');
  const [showUnlock, setShowUnlock] = useState(params.fallback === 'recovery');
  const [unlock, setUnlock] = useState('');
  const [recovery, setRecovery] = useState<CloudRecoveryDisplay | null>(null);
  const shown = recoveryDisplayFor(cloud, recovery);
  const previousAccount = useRef(cloud.account_epoch);
  const [backupFinished, setBackupFinished] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const stage = syncSetupStage(cloud, setup.value);
  const pendingConnection = link.value && linkIsPending(link.value.phase) ? link.value : null;
  const canConnect = pendingConnection !== null || (availability.value?.available === true && !availability.checking);
  const admissionFailure = availability.error ?? (availability.value?.available === false ? { code: availability.value.reason_code || 'pairing_unavailable', message: '' } : null);
  useFocusEffect(useCallback(() => { void setup.refresh(); void link.refresh(); }, [setup.refresh, link.refresh, cloud.account_epoch]));
  useEffect(() => {
    if (params.fallback === 'email') setEmail(true);
    if (params.fallback === 'recovery' && cloud.signed_in) setShowUnlock(true);
  }, [params.fallback, cloud.signed_in]);
  useEffect(() => { if (requiresRecovery(error)) setShowUnlock(true); }, [error]);
  useEffect(() => {
    if (!cloud.signed_in || previousAccount.current !== cloud.account_epoch) {
      previousAccount.current = cloud.account_epoch;
      setRecovery(null); setUnlock(''); setShowUnlock(false); setNotice(null); setBackupFinished(false);
    } else if (recovery && !recoveryDisplayFor(cloud, recovery)) setRecovery(null);
  }, [cloud.signed_in, cloud.account_epoch, cloud.recovery_display_token, recovery]);
  useEffect(() => {
    if (!cloud.signed_in || (cloud.recovery_confirmed && setup.value?.phase !== 'ready')) return;
    let alive = true;
    void run<CloudRecoveryDisplay>('recovery-pending', 'cloud.pendingRecovery').then((value) => { if (alive) setRecovery(recoveryDisplayFor(store.getSnapshot().snapshot?.cloud, value)); });
    return () => { alive = false; };
  }, [cloud.recovery_confirmed, cloud.signed_in, cloud.account_epoch, cloud.recovery_display_token, run, setup.value?.phase, store]);
  usePreventRemove(shown !== null && !backupFinished, ({ repeat }) => confirm(t('mobile.sync.backup_leave'), t('mobile.sync.backup_leave_help'), t('mobile.sync.leave'), t('mobile.sync.stay'), () => { setRecovery(null); repeat(); }));
  const refresh = async () => { await setup.refresh(); };
  const codeResult = (value: CloudRecoveryDisplay | undefined) => { setBackupFinished(false); setRecovery(recoveryDisplayFor(store.getSnapshot().snapshot?.cloud, value)); };
  const create = async () => { codeResult(await run<CloudRecoveryDisplay>('sync-prepare', 'cloud.prepareSync')); await refresh(); };
  const replaceCode = async () => { codeResult(await run<CloudRecoveryDisplay>('recovery-create', 'cloud.recoveryCreate')); await refresh(); };
  const saved = async () => {
    if (!shown) return;
    const value = await run('recovery-confirm', 'cloud.recoveryConfirm', { confirmation_token: shown.confirmation_token });
    if (value === undefined) return;
    setBackupFinished(true); setRecovery(null); setNotice(t('mobile.cloud.confirmed')); await refresh();
  };
  const connect = () => router.push({ pathname: '/settings/[section]', params: { section: 'connection', role: pendingConnection?.role ?? (setup.value?.can_approve_devices ? 'trusted_device' : 'new_device') } });
  const connectTitle = t(pendingConnection ? 'mobile.sync.continue_connection' : 'mobile.sync.connect_device');
  const report = cloud.report;
  return <>
    <Section title={t('mobile.sync.intro')} description={t('mobile.sync.privacy')}>
      <Body accessibilityLiveRegion="polite">{t(syncStatusKey(cloud, setup.value))}</Body>
      {report?.last_success_ms ? <Notice>{t('mobile.sync.last_success', { time: dateLabel(new Date(report.last_success_ms).toISOString(), language) })}</Notice> : cloud.sync_enabled && <Notice>{t('mobile.sync.no_success')}</Notice>}
      {report && report.status === 'synced' && <Notice>{t('mobile.sync.counts', { uploaded: report.uploaded, downloaded: report.downloaded })}</Notice>}
      {(report?.conflicts ?? 0) > 0 && <Notice>{t('mobile.sync.conflicts', { count: report!.conflicts })}</Notice>}
      {cloud.notice && <Notice error>{cloud.brake ? t('account.sync.confirm_deletes_detail') : localizedFailure(language, { code: cloud.status, message: cloud.notice })}</Notice>}
    </Section>
    {!pendingConnection && (['sign_in', 'unlock', 'connected'].includes(stage) || showUnlock) && <Section>
      {availability.checking && <Body muted accessibilityLiveRegion="polite">{t('mobile.sync.availability_checking')}</Body>}
      {admissionFailure && <Notice error={Boolean(availability.error)}>{localizedConnectionFailure(language, admissionFailure)}</Notice>}
      {!availability.value?.available && <Button title={t('mobile.sync.check_again')} subtle busy={availability.checking} onPress={() => void availability.refresh()} />}
    </Section>}
    {stage === 'sign_in' && <>
      <Section><Button title={connectTitle} icon="phone-portrait-outline" disabled={!canConnect} onPress={connect} /></Section>
      {email ? <CloudSignIn /> : <Section><Button title={t('mobile.sync.email')} subtle onPress={() => setEmail(true)} /><Notice>{t('mobile.sync.email_help')}</Notice></Section>}
    </>}
    {stage === 'checking' && <Section>{!setup.error && <Loading />}{setup.error && <><Notice error>{setup.error}</Notice><Button title={t('common.retry')} onPress={() => void refresh()} /></>}</Section>}
    {stage !== 'checking' && setup.error && <Section><Notice error>{setup.error}</Notice><Button title={t('common.retry')} subtle onPress={() => void refresh()} /></Section>}
    {stage === 'create' && <Section title={t('mobile.sync.create')} description={t('mobile.sync.create_help')}><Button title={t('mobile.sync.create_action')} busy={busy.has('sync-prepare')} onPress={() => void create()} /></Section>}
    {cloud.signed_in && (stage === 'unlock' || showUnlock) && <Section title={t('mobile.sync.unlock')} description={t('mobile.sync.unlock_help')}>
      <Button title={connectTitle} disabled={!canConnect} onPress={connect} />
      {showUnlock ? <>
        <Field label={t('mobile.cloud.recovery_entry')} value={unlock} onChangeText={setUnlock} autoCorrect={false} autoCapitalize="characters" secureTextEntry autoComplete="off" />
        <Notice>{t('mobile.sync.recovery_help')}</Notice>
        <Button title={t('account.recovery.unlock')} busy={busy.has('recovery-unlock')} disabled={!unlock.trim()} onPress={() => { void run('recovery-unlock', 'cloud.recoveryUnlock', { code: unlock.trim() }).then(async (value) => { if (value !== undefined) { setUnlock(''); setShowUnlock(false); await refresh(); } }); }} />
      </> : <Button title={t('mobile.sync.use_recovery')} subtle onPress={() => setShowUnlock(true)} />}
    </Section>}
    {cloud.signed_in && (stage === 'backup' || shown) && <RecoveryBackup display={shown} onSaved={saved} onReplace={replaceCode} />}
    {stage === 'resume' && !shown && <Section description={t('mobile.sync.resume_help')}><Button title={t('mobile.sync.resume')} busy={busy.has('sync-resume')} onPress={() => { void run('sync-resume', 'cloud.resumeSync').then(refresh); }} /></Section>}
    {stage === 'connected' && !shown && <Section>
      <Button title={t(pendingConnection ? 'mobile.sync.continue_connection' : 'mobile.sync.add_device')} icon="add" disabled={!canConnect || (!pendingConnection && !setup.value?.can_approve_devices)} onPress={connect} />
      <Actions><Button title={t('account.sync.now')} subtle busy={cloud.syncing || busy.has('sync')} onPress={() => { void run('sync', 'cloud.sync').then(refresh); }} /><Button title={t('mobile.sync.pause')} subtle busy={busy.has('sync-pause')} onPress={() => { void run('sync-pause', 'cloud.setSync', { enabled: false }).then(refresh); }} /></Actions>
      {cloud.brake && <Button title={t('account.sync.confirm_deletes')} subtle onPress={() => confirm(t('account.sync.confirm_deletes_title'), t('account.sync.confirm_deletes_detail'), t('account.sync.confirm_deletes'), t('common.cancel'), () => { void run('sync-confirm-deletes', 'cloud.confirmDeletes').then(refresh); }, true)} />}
      <Button title={t('mobile.sync.open_meetings')} subtle onPress={() => router.navigate('/(tabs)')} />
    </Section>}
    {notice && <Notice>{notice}</Notice>}
    {cloud.signed_in && <Section><Row title={t('mobile.sync.recovery_admin')} subtitle={t('mobile.sync.recovery_admin_help')} onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'recovery' } })} /></Section>}
    <Section><Row title={t('mobile.sync.account')} subtitle={t('mobile.sync.account_help')} onPress={() => router.push({ pathname: '/settings/[section]', params: { section: 'account' } })} /></Section>
  </>;
}
