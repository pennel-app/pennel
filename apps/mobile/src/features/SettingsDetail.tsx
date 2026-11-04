import React from 'react';
import { Stack, useLocalSearchParams, useRouter } from 'expo-router';
import { Page, Row, Section } from '../components/ui';
import { useRuntime } from '../runtime';
import { AccountPanel, DevicesPanel, RecoveryPanel } from './settings/AccountPanel';
import { SyncPanel } from './settings/SyncPanel';
import { DeviceLinkPanel } from './settings/DeviceLinkPanel';
import { FeedbackPanel } from './settings/FeedbackPanel';
import { AppearancePanel, DictionaryPanel, DictationPanel, MeetingsPanel, ModelsPanel, ProvidersPanel, RecordingPanel, StylesPanel } from './settings/Panels';

export default function SettingsDetail() {
  const { section } = useLocalSearchParams<{ section: string }>();
  const { snapshot, t } = useRuntime();
  const router = useRouter();
  const key = Array.isArray(section) ? section[0] : section;
  const links = (sections: string[]) => <Section>{sections.map(section => <Row key={section} title={t(`navigation.${section}`)} onPress={() => router.push({ pathname: '/settings/[section]', params: { section } })} />)}</Section>;
  const grouped = key === 'sync-account' ? links(['sync', 'account', 'devices', 'recovery']) : key === 'advanced' ? links(['models', 'providers', 'recording', 'styles']) : null;
  const panel = key === 'providers' ? <ProvidersPanel /> : key === 'models' ? <ModelsPanel /> : key === 'dictation' ? <DictationPanel /> : key === 'meetings' ? <MeetingsPanel /> : key === 'recording' ? <RecordingPanel /> : key === 'dictionary' ? <DictionaryPanel /> : key === 'styles' ? <StylesPanel /> : key === 'appearance' ? <AppearancePanel /> : key === 'account' ? <AccountPanel /> : key === 'sync' ? <SyncPanel /> : key === 'connection' ? <DeviceLinkPanel /> : key === 'devices' ? <DevicesPanel /> : key === 'recovery' ? <RecoveryPanel /> : key === 'feedback' ? <FeedbackPanel /> : null;
  return <Page inset={false}><Stack.Screen options={{ title: t(key === 'sync-account' ? 'mobile.simple.settings_sync' : key === 'advanced' ? 'mobile.simple.settings_advanced' : `navigation.${key}`) }} />{snapshot?.ready && <>{grouped ?? panel}{key === 'dictation' && links(['models', 'dictionary', 'styles'])}{key === 'meetings' && links(['models', 'recording'])}{key === 'appearance' && links(['feedback'])}</>}</Page>;
}
