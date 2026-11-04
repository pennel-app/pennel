import React, { useMemo } from 'react';
import { DynamicColorIOS, Platform } from 'react-native';
import { NativeTabs } from 'expo-router/native-tabs';
import { useRuntime } from '../../src/runtime';
import { makeTheme } from '../../src/theme';

export default function TabLayout() {
  const { theme, t } = useRuntime();
  const foreground = useMemo(() => Platform.OS === 'ios' ? DynamicColorIOS({ light: makeTheme('light').colors.foreground, dark: makeTheme('dark').colors.foreground }) : theme.colors.foreground, [theme.colors.foreground]);
  return <NativeTabs tintColor={foreground} labelStyle={{ fontFamily: theme.font.medium, fontSize: theme.size.caption, color: foreground }}>
    <NativeTabs.Trigger name="dictation" accessibilityLabel={t('navigation.dictation')}><NativeTabs.Trigger.Label>{t('navigation.dictation')}</NativeTabs.Trigger.Label><NativeTabs.Trigger.Icon sf={{ default: 'mic', selected: 'mic.fill' }} md="mic" /></NativeTabs.Trigger>
    <NativeTabs.Trigger name="index" accessibilityLabel={t('navigation.meetings')}><NativeTabs.Trigger.Label>{t('navigation.meetings')}</NativeTabs.Trigger.Label><NativeTabs.Trigger.Icon sf="waveform" md="graphic_eq" /></NativeTabs.Trigger>
    <NativeTabs.Trigger name="ask" hidden />
    <NativeTabs.Trigger name="settings" accessibilityLabel={t('common.settings')}><NativeTabs.Trigger.Label>{t('common.settings')}</NativeTabs.Trigger.Label><NativeTabs.Trigger.Icon sf={{ default: 'gearshape', selected: 'gearshape.fill' }} md="settings" /></NativeTabs.Trigger>
  </NativeTabs>;
}
