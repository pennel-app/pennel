import React, { useEffect, useState } from 'react';
import { View } from 'react-native';
import { useRouter } from 'expo-router';
import Ionicons from '@expo/vector-icons/Ionicons';
import { useRuntime } from '../runtime';
import { clock } from '../format';
import { Body, Button, Notice, Page } from '../components/ui';
import { ownedFailureText } from '../errors';
import { workflowText } from '../workflow-copy';

export default function SessionReturn() {
  const { snapshot, busy, run, t, theme, language } = useRuntime();
  const router = useRouter();
  const session = snapshot?.session;
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const copy = (key: Parameters<typeof workflowText>[1]) => workflowText(language, key);
  const active = session !== undefined && session.phase !== 'off';
  const failed = Boolean(session?.error) && !active;
  const close = () => { if (router.canGoBack()) router.back(); else router.replace('/(tabs)/dictation'); };
  const status = session?.phase === 'ready'
    ? t('mobile.session.ready', { time: clock(Math.max(0, ((session.expires_at_ms ?? now) - now) / 1000)) })
    : session?.phase === 'listening' ? t('dictation.pad.listening')
    : session?.phase === 'processing' ? t('dictation.pad.processing')
    : busy.has('session') || !snapshot?.ready ? copy('sessionReturnStarting') : copy('sessionReturnStopped');
  return <Page bottomInset recordingStatus={false} scroll={false}>
    <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center', gap: theme.space[5], paddingHorizontal: theme.space[3] }}>
      <Ionicons accessible={false} name={failed ? 'alert-circle-outline' : active ? 'checkmark-circle' : 'mic-circle-outline'} size={72} color={failed ? theme.colors.accent : theme.colors.foreground} />
      <Body accessibilityRole="header" style={{ fontFamily: theme.font.display, fontSize: theme.size['display-title'], textAlign: 'center' }}>{copy('sessionReturnTitle')}</Body>
      <Body muted style={{ textAlign: 'center' }}>{status}</Body>
      {failed ? <Notice error>{ownedFailureText(language, session!.error!, session!.error_code)}</Notice> : <Body style={{ textAlign: 'center' }}>{copy('sessionReturnHelp')}</Body>}
    </View>
    <View style={{ gap: theme.space[2] }}>
      {failed && <Button title={t('mobile.session.start')} busy={busy.has('session')} disabled={!snapshot?.ready} onPress={() => void run('session', 'session.start')} />}
      {active && <Button title={t('mobile.session.stop')} subtle busy={busy.has('session')} onPress={() => void run('session', 'session.stop')} />}
      <Button title={t('common.close')} subtle onPress={close} />
    </View>
  </Page>;
}
