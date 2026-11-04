import React from 'react';
import { useRouter } from 'expo-router';
import { Page, Row, Section } from '../components/ui';
import { useRuntime } from '../runtime';

export const settingsSections = ['dictation', 'meetings', 'appearance', 'sync-account', 'advanced', 'sync', 'recording', 'dictionary', 'styles', 'feedback', 'account', 'providers', 'models'] as const;
export default function Settings() {
  const { t } = useRuntime();
  const router = useRouter();
  const groups = [
    { section: 'dictation', title: 'navigation.dictation', help: 'mobile.simple.settings_dictation_help' },
    { section: 'meetings', title: 'navigation.meetings', help: 'mobile.simple.settings_meetings_help' },
    { section: 'appearance', title: 'navigation.appearance', help: 'mobile.simple.settings_appearance_help' },
    { section: 'sync-account', title: 'mobile.simple.settings_sync', help: 'mobile.simple.settings_sync_help' },
    { section: 'advanced', title: 'mobile.simple.settings_advanced', help: 'mobile.simple.settings_advanced_help' },
  ];
  return <Page title={t('common.settings')}>
    <Section>{groups.map(group => <Row key={group.section} title={t(group.title)} subtitle={t(group.help)} onPress={() => router.push({ pathname: '/settings/[section]', params: { section: group.section } })} />)}</Section>
  </Page>;
}
