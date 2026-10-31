import React, { useEffect, useRef, useState } from 'react';
import { AppState, Image } from 'react-native';
import { useLocalSearchParams, usePreventRemove, useRouter } from 'expo-router';
import type { CloudLinkRole, CloudLinkView, MobileCommand, NativeFailure } from '../../../modules/pennel-mobile/src/types';
import { useRuntime } from '../../runtime';
import { failure } from '../../state';
import { connectionFailureFor, localizedConnectionFailure } from '../../device-link';
import { useLinkAvailability } from '../../use-link-availability';
import { linkIsPending, restorableLink } from '../../sync-setup';
import { useHapticFeedback } from '../../haptics';
import { Actions, Body, Button, Field, Loading, Notice, Section, confirm } from '../../components/ui';

export function DeviceLinkPanel() {
  const { snapshot, store, t, language } = useRuntime();
  const params = useLocalSearchParams<{ role?: string }>();
  const role: CloudLinkRole = params.role === 'trusted_device' ? 'trusted_device' : 'new_device';
  const router = useRouter();
  const availability = useLinkAvailability();
  const [view, setView] = useState<CloudLinkView | null>(null);
  const [code, setCode] = useState('');
  const [manual, setManual] = useState(false);
  const [notice, setNotice] = useState<NativeFailure | null>(null);
  const [scanning, setScanning] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [otherConnection, setOtherConnection] = useState(false);
  const completedRequest = useRef<string | null>(null);
  const ownedRequest = useRef<string | null>(null);
  const previousRole = useRef(role);
  const previousAccount = useRef(snapshot?.cloud.account_epoch);
  const contextEpoch = useRef(0);
  const actionRevision = useRef(0);
  const activeOperation = useRef<string | null>(null);
  const feedback = useHapticFeedback();
  const pending = view ? linkIsPending(view.phase) : false;
  const canAdmit = availability.value?.available === true && !availability.checking && !otherConnection && !busy && !scanning;
  useEffect(() => {
    const epoch = ++contextEpoch.current;
    const revision = ++actionRevision.current;
    const accountChanged = previousAccount.current !== snapshot?.cloud.account_epoch;
    previousAccount.current = snapshot?.cloud.account_epoch;
    if (previousRole.current !== role) { ownedRequest.current = null; completedRequest.current = null; previousRole.current = role; }
    activeOperation.current = null;
    setView(null); setNotice(null); setBusy(null); setScanning(false); setCode(''); setManual(false); setOtherConnection(false);
    store.clearError();
    void store.client.invoke<CloudLinkView>('cloud.linkView').then((value) => {
      if (epoch !== contextEpoch.current || revision !== actionRevision.current) return;
      if (accountChanged && value.phase !== 'complete') {
        ownedRequest.current = null;
        setOtherConnection(linkIsPending(value.phase));
      } else {
        const restored = restorableLink(value, role, store.getSnapshot().snapshot?.cloud, ownedRequest.current);
        setView(restored);
        setOtherConnection(linkIsPending(value.phase) && restored === null);
      }
    }).catch((error) => { if (epoch === contextEpoch.current && revision === actionRevision.current) setNotice(failure(error)); });
    return () => { contextEpoch.current += 1; actionRevision.current += 1; };
  }, [role, snapshot?.cloud.account_epoch, snapshot?.cloud.account?.email, snapshot?.cloud.signed_in, store]);
  const perform = async (key: string, method: MobileCommand, payload?: Record<string, unknown>) => {
    if (activeOperation.current && method !== 'cloud.linkCancel') return;
    const epoch = contextEpoch.current;
    const revision = ++actionRevision.current;
    activeOperation.current = key;
    store.clearError();
    setBusy(key); setNotice(null);
    try {
      const next = await store.client.invoke<CloudLinkView>(method, payload);
      if (epoch !== contextEpoch.current || revision !== actionRevision.current) return;
      setView(next);
      void store.refresh();
      return next;
    } catch (error) {
      if (epoch === contextEpoch.current && revision === actionRevision.current) setNotice(failure(error));
    } finally { if (epoch === contextEpoch.current && revision === actionRevision.current) { activeOperation.current = null; setBusy(null); setView((value) => value ? { ...value } : null); } }
  };
  usePreventRemove(pending, ({ repeat }) => confirm(t('mobile.sync.link_leave'), t('mobile.sync.link_leave_help'), t('mobile.sync.link_cancel'), t('mobile.sync.link_continue'), () => { void perform('link-cancel', 'cloud.linkCancel').then((value) => { if (value) repeat(); }); }));
  useEffect(() => {
    if (!pending || busy) return;
    const epoch = contextEpoch.current;
    const revision = actionRevision.current;
    let alive = true;
    const current = () => alive && epoch === contextEpoch.current && revision === actionRevision.current;
    const timer = setTimeout(() => {
      if (AppState.currentState !== 'active') { setView((value) => value ? { ...value } : null); return; }
      void store.client.invoke<CloudLinkView>('cloud.linkPoll').then((value) => {
        if (!current()) return;
        setNotice(null); setView(value); void store.refresh();
      }).catch((error) => { if (current()) { setNotice(failure(error)); setView((value) => value ? { ...value } : null); } });
    }, 2000);
    return () => { alive = false; clearTimeout(timer); };
  }, [busy, pending, store, view]);
  useEffect(() => {
    if (view?.phase !== 'complete' || !view.request_id || ownedRequest.current !== view.request_id || completedRequest.current === view.request_id) return;
    completedRequest.current = view.request_id;
    feedback('deviceConnected');
  }, [feedback, view?.phase, view?.request_id]);
  const payload = () => ({ role, device_name: snapshot?.cloud.device_name || 'iPhone', platform: 'ios' });
  const join = async (value: string) => {
    if (!canAdmit || !value.trim()) return;
    setView(null);
    const next = await perform('link-join', 'cloud.linkJoin', { ...payload(), code: value.trim() });
    if (next) { ownedRequest.current = next.request_id; setCode(''); }
  };
  const scan = async () => {
    if (!canAdmit || activeOperation.current) return;
    const epoch = contextEpoch.current;
    const revision = ++actionRevision.current;
    activeOperation.current = 'scanner';
    store.clearError(); setView(null); setNotice(null); setScanning(true);
    try {
      const scanned = await store.client.invoke<{ code: string }>('cloud.scanConnectionCode');
      if (epoch !== contextEpoch.current || revision !== actionRevision.current) return;
      activeOperation.current = null;
      setScanning(false);
      const next = await perform('link-join', 'cloud.linkJoin', { ...payload(), code: scanned.code.trim() });
      if (next) { ownedRequest.current = next.request_id; setCode(''); }
    } catch (error) {
      if (epoch !== contextEpoch.current || revision !== actionRevision.current) return;
      const value = failure(error);
      if (value.code !== 'cancelled') { setManual(true); setNotice(value); }
    } finally { if (epoch === contextEpoch.current) { if (activeOperation.current === 'scanner') activeOperation.current = null; setScanning(false); } }
  };
  const start = async () => {
    if (!canAdmit) return;
    setView(null); setManual(false);
    const next = await perform('link-start', 'cloud.linkStart', payload());
    if (next) ownedRequest.current = next.request_id;
  };
  const reset = () => { actionRevision.current += 1; setView(null); setNotice(null); setCode(''); setManual(false); };
  const fallback = () => router.replace({ pathname: '/settings/[section]', params: { section: 'sync', fallback: snapshot?.cloud.signed_in ? 'recovery' : 'email' } });
  const terminal = view && ['cancelled', 'expired', 'failed'].includes(view.phase);
  const connectionError = connectionFailureFor(view, notice) ?? (!pending ? availability.error : null);
  const unavailable = !pending && !connectionError && availability.value?.available === false;
  return <>
    {!pending && view?.phase !== 'complete' && <Section>
      {availability.checking && <Body muted accessibilityLiveRegion="polite">{t('mobile.sync.availability_checking')}</Body>}
      {unavailable && <Notice>{localizedConnectionFailure(language, { code: availability.value?.reason_code || 'pairing_unavailable', message: '' })}</Notice>}
      {(!availability.value?.available || connectionError) && <Button title={t('mobile.sync.check_again')} subtle busy={availability.checking} disabled={!!busy || scanning} onPress={() => { setView(null); setNotice(null); void availability.refresh(); }} />}
    </Section>}
    {!view && <Section description={t('mobile.sync.code_help')}>
      {otherConnection && <><Notice>{t('mobile.sync.other_connection')}</Notice><Button title={t('mobile.sync.link_cancel')} onPress={() => { void perform('link-cancel', 'cloud.linkCancel').then((value) => { if (value) { setView(null); setOtherConnection(false); } }); }} /></>}
      <Button title={t('mobile.sync.scan')} icon="qr-code-outline" disabled={!canAdmit} busy={scanning || busy === 'link-join'} onPress={() => void scan()} />
      <Button title={t('mobile.sync.enter_code')} subtle disabled={!canAdmit} onPress={() => { actionRevision.current += 1; setManual(true); setNotice(null); }} />
      {manual && <>
        <Field label={t('mobile.sync.code_entry')} value={code} onChangeText={setCode} editable={canAdmit} autoCorrect={false} autoCapitalize="characters" autoComplete="off" maxLength={256} returnKeyType="go" onSubmitEditing={() => void join(code)} />
        <Button title={t('mobile.sync.join')} disabled={!canAdmit || !code.trim()} busy={busy === 'link-join'} onPress={() => void join(code)} />
      </>}
      <Button title={t('mobile.sync.show_code')} subtle disabled={!canAdmit} busy={busy === 'link-start'} onPress={() => void start()} />
    </Section>}
    {view && <Section title={t(view.phase === 'failed' || view.phase === 'expired' ? 'navigation.connection' : `mobile.sync.${view.phase}`)}>
      {pending && !['awaiting_approval', 'waiting_peer'].includes(view.phase) && <Loading />}
      {view.phase === 'waiting_peer' && <>
        <Notice>{t('mobile.sync.show_help')}</Notice>
        {view.qr_image && <Image source={{ uri: view.qr_image }} accessibilityLabel={t('mobile.sync.qr_label')} style={{ alignSelf: 'center', width: '75%', aspectRatio: 1 }} />}
        <Body muted>{t('mobile.sync.code_entry')}</Body>
        <Body selectable accessibilityLabel={t('mobile.sync.code_entry')} accessibilityValue={{ text: view.connection_code?.split('').join(' ') }}>{view.connection_code}</Body>
        <Notice>{t('mobile.sync.code_private')}</Notice>
      </>}
      {view.peer_name && <Body>{t('mobile.sync.peer', { name: view.peer_name, platform: view.peer_platform || '' })}</Body>}
      {view.account_email && <Notice>{t('account.signed_in_as', { email: view.account_email })}</Notice>}
      {view.verification_code && <><Body>{t('mobile.sync.matches')}</Body><Body selectable>{view.verification_code}</Body></>}
      {view.phase === 'awaiting_approval' && role === 'trusted_device' && <Button title={t('mobile.sync.approve')} busy={busy === 'link-approve'} onPress={() => void perform('link-approve', 'cloud.linkApprove')} />}
      {view.phase === 'complete' && <><Body>{t('mobile.sync.connected_help')}</Body><Button title={t('mobile.sync.view_status')} onPress={() => router.replace({ pathname: '/settings/[section]', params: { section: 'sync' } })} /></>}
      {pending && <Button title={t('mobile.sync.link_cancel')} subtle busy={busy === 'link-cancel'} onPress={() => void perform('link-cancel', 'cloud.linkCancel')} />}
      {terminal && <Actions><Button title={t('mobile.sync.start_again')} onPress={reset} /></Actions>}
    </Section>}
    {connectionError && <Notice error>{localizedConnectionFailure(language, connectionError)}</Notice>}
    {!pending && view?.phase !== 'complete' && <Section><Button title={t(snapshot?.cloud.signed_in ? 'mobile.sync.use_recovery' : 'mobile.sync.email')} subtle onPress={fallback} /></Section>}
  </>;
}
