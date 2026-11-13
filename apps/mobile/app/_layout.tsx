import React, { useEffect, useRef } from 'react';
import { Appearance } from 'react-native';
import { Stack, useRouter, useSegments } from 'expo-router';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { RuntimeProvider, useRuntime } from '../src/runtime';

export const unstable_settings = { anchor: '(tabs)' };

function Navigation() {
  const { snapshot, theme, t, run } = useRuntime();
  const router = useRouter();
  const segments = useSegments();
  const checkedOnboarding = useRef(false);
  const navigationSequence = useRef(0);
  const appearance = snapshot?.settings.appearance ?? 'system';
  useEffect(() => { Appearance.setColorScheme(appearance === 'system' ? 'auto' : appearance); }, [appearance]);
  useEffect(() => {
    if (!snapshot?.ready) return;
    if (!checkedOnboarding.current) {
      checkedOnboarding.current = true;
      if (snapshot.settings.onboarding.status === 'pending') router.replace('/onboarding');
    }
    const requested = snapshot.navigation;
    if (!requested || requested.sequence <= navigationSequence.current) return;
    navigationSequence.current = requested.sequence;
    if (requested.route === 'meeting') {
      const finished = snapshot.meeting.phase === 'idle' && requested.meeting_id;
      if (finished && segments[segments.length - 1] !== 'recording') router.push({ pathname: '/meeting/[id]', params: { id: requested.meeting_id! } });
      else if (!finished) router.push('/recording');
    }
    else if (requested.route === 'sync') router.navigate({ pathname: '/settings/[section]', params: { section: 'sync' } });
    else router.navigate('/(tabs)/dictation');
    void run('navigation-consume', 'navigation.consume', { sequence: requested.sequence });
  }, [router, run, segments, snapshot]);
  return <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: theme.colors.surface }, headerStyle: { backgroundColor: theme.colors.surface }, headerTintColor: theme.colors.foreground, headerTitleStyle: { fontFamily: theme.font.medium }, headerBackTitle: t('common.back'), headerShadowVisible: false }}><Stack.Screen name="(tabs)" /><Stack.Screen name="meeting/[id]" options={{ headerShown: true, title: t('navigation.meetings') }} /><Stack.Screen name="settings/[section]" options={{ headerShown: true, title: t('common.settings') }} /><Stack.Screen name="recording" options={{ presentation: 'fullScreenModal' }} /><Stack.Screen name="session-return" options={{ presentation: 'fullScreenModal' }} /><Stack.Screen name="onboarding" options={{ presentation: 'fullScreenModal', gestureEnabled: false }} /></Stack>;
}

export default function RootLayout() {
  return <SafeAreaProvider><RuntimeProvider><Navigation /></RuntimeProvider></SafeAreaProvider>;
}
