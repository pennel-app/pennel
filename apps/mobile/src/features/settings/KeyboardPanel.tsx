import React, { useState } from 'react';
import { useQuery, useRuntime } from '../../runtime';
import { ActionMenu, Actions, Body, Button, Field, NativeChoice, Notice, Row, Section, Toggle, confirm } from '../../components/ui';
import {
  defaultKeyboardSettings, deleteSnippetPatch, keyboardCopy, snippetPatch,
  typingLanguageChoice, typingLanguagePatch, validLearningWord,
  type KeyboardSettingsView, type KeyboardSnippetSettings, type SnippetProblem, type TypingLanguageChoice,
} from '../../keyboard-settings';

export function KeyboardPanel() {
  const { snapshot, run, busy, t, language } = useRuntime();
  const keyboard = useQuery<KeyboardSettingsView>('settings.keyboard');
  const settings = { ...defaultKeyboardSettings, ...keyboard.value };
  const labels = keyboardCopy[language];
  const supported = snapshot?.platform === 'ios';
  const saving = busy.has('keyboard-settings') || busy.has('keyboard-settings-read') || busy.has('keyboard-learning-remove');
  const editable = supported && keyboard.value !== null && !saving;
  const [practice, setPractice] = useState('');
  const [forgetWord, setForgetWord] = useState('');
  const [draft, setDraft] = useState<KeyboardSnippetSettings | null>(null);
  const [editing, setEditing] = useState(false);
  const [problem, setProblem] = useState<SnippetProblem | null>(null);
  const [notice, setNotice] = useState<'resetDone' | 'forgetDone' | null>(null);

  const save = async (patch: Record<string, unknown>): Promise<boolean> => {
    setNotice(null);
    const result = await run('keyboard-settings', 'settings.update', { patch });
    if (result === undefined) return false;
    await keyboard.refresh();
    return true;
  };
  const latestSettings = () => run<KeyboardSettingsView>('keyboard-settings-read', 'settings.keyboard');
  const saveSnippet = async () => {
    if (!draft || !editable) return;
    const latest = await latestSettings();
    if (!latest) return;
    const result = snippetPatch(latest.keyboard_snippets, draft, editing);
    if (result.problem) { setProblem(result.problem); return; }
    if (await save(result.patch)) { setDraft(null); setProblem(null); }
  };
  const removeSnippet = async (id: string) => {
    const latest = await latestSettings();
    if (!latest) return;
    if (await save(deleteSnippetPatch(latest.keyboard_snippets, id)) && draft?.id === id) setDraft(null);
  };
  const resetLearning = () => confirm(labels.resetTitle, labels.resetHelp, labels.reset, t('common.cancel'), () => {
    void save({ keyboard_learning_reset: true }).then((saved) => { if (saved) setNotice('resetDone'); });
  }, true);
  const forget = async () => {
    const word = forgetWord.trim();
    if (!editable || !validLearningWord(word)) return;
    setNotice(null);
    const result = await run('keyboard-learning-remove', 'keyboard.learning.remove', { word });
    if (result !== undefined) { setForgetWord(''); setNotice('forgetDone'); await keyboard.refresh(); }
  };

  return <>
    <Section title={labels.title} description={t('mobile.keyboard.setup')}>
      {keyboard.loading && !keyboard.value && <Notice>{labels.loading}</Notice>}
      {keyboard.error && <Notice error>{keyboard.error}</Notice>}
      {!supported && <Notice>{labels.unavailable}</Notice>}
      <NativeChoice label={t('mobile.keyboard.layout')} value={settings.layout} options={[
        { value: 'qwerty', label: 'QWERTY', disabled: !editable },
        { value: 'azerty', label: 'AZERTY', disabled: !editable },
      ]} onChange={(keyboard_layout) => void save({ keyboard_layout })} />
      <NativeChoice<TypingLanguageChoice> label={labels.languages} value={typingLanguageChoice(settings.keyboard_typing_languages)} options={(['both', 'nl', 'en'] as const).map((value) => ({ value, label: labels[value], disabled: !editable }))} onChange={(value) => void save(typingLanguagePatch(value))} />
      <Notice>{labels.languagesHelp}</Notice>
      <Toggle title={t('mobile.keyboard.suggestions')} description={t('mobile.keyboard.suggestions_help')} value={settings.suggestions} disabled={!editable} onChange={(keyboard_suggestions) => void save({ keyboard_suggestions })} />
      <Toggle title={labels.correction} description={labels.correctionHelp} value={settings.keyboard_autocorrection} disabled={!editable} onChange={(keyboard_autocorrection) => void save({ keyboard_autocorrection })} />
      <Toggle title={labels.swipe} description={labels.swipeHelp} value={settings.keyboard_swipe} disabled={!editable} onChange={(keyboard_swipe) => void save({ keyboard_swipe })} />
      <Notice>{t(snapshot?.session.full_access_seen ? 'mobile.keyboard.seen' : 'mobile.keyboard.unseen')}</Notice>
      <Button title={t('mobile.open_settings')} subtle disabled={!supported} onPress={() => void run('open-settings', 'system.openSettings')} />
    </Section>

    <Section title={labels.learning}>
      <Toggle title={labels.learning} description={labels.learningHelp} value={settings.keyboard_learning} disabled={!editable} onChange={(keyboard_learning) => void save({ keyboard_learning })} />
      <Body muted>{labels.learned}: {settings.keyboard_learned_count}</Body>
      <Field label={labels.forget} value={forgetWord} onChangeText={setForgetWord} autoCapitalize="none" autoCorrect={false} maxLength={64} editable={editable} onSubmitEditing={() => void forget()} returnKeyType="done" />
      <Notice>{labels.forgetHelp}</Notice>
      <Actions><Button title={labels.forget} subtle disabled={!editable || !validLearningWord(forgetWord.trim())} onPress={() => void forget()} /><Button title={labels.reset} subtle destructive disabled={!editable} onPress={resetLearning} /></Actions>
      {notice && <Notice>{labels[notice]}</Notice>}
    </Section>

    <Section title={labels.snippets} description={labels.snippetsHelp}>
      <Toggle title={labels.expansion} description={labels.expansionHelp} value={settings.keyboard_snippet_expansion} disabled={!editable} onChange={(keyboard_snippet_expansion) => void save({ keyboard_snippet_expansion })} />
      {settings.keyboard_snippets.length === 0 && <Notice>{labels.emptySnippets}</Notice>}
      {settings.keyboard_snippets.map((snippet) => <Row key={snippet.id} title={snippet.title} subtitle={snippet.keyword || undefined} trailing={<ActionMenu label={snippet.title} actions={[
        { label: labels.edit, disabled: !editable, onPress: () => { setDraft({ ...snippet }); setEditing(true); setProblem(null); } },
        { label: t('common.delete'), destructive: true, disabled: !editable, onPress: () => confirm(labels.deleteSnippet, labels.deleteHelp, t('common.delete'), t('common.cancel'), () => void removeSnippet(snippet.id), true) },
      ]} />} />)}
      {!draft && <Button title={labels.newSnippet} subtle icon="add" disabled={!editable || settings.keyboard_snippets.length >= 64} onPress={() => {
        setDraft({ id: `snippet-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`, title: '', keyword: '', text: '' }); setEditing(false); setProblem(null);
      }} />}
      {draft && <>
        <Field label={labels.snippetTitle} value={draft.title} onChangeText={(title) => { setDraft({ ...draft, title }); setProblem(null); }} maxLength={80} editable={!saving} />
        <Field label={labels.shortcut} value={draft.keyword} onChangeText={(keyword) => { setDraft({ ...draft, keyword }); setProblem(null); }} maxLength={32} autoCapitalize="none" autoCorrect={false} editable={!saving} />
        <Field label={labels.text} value={draft.text} onChangeText={(text) => { setDraft({ ...draft, text }); setProblem(null); }} multiline editable={!saving} />
        {problem && <Notice error>{labels[problem]}</Notice>}
        <Actions><Button title={t('common.cancel')} subtle disabled={saving} onPress={() => { setDraft(null); setProblem(null); }} /><Button title={t('common.save')} disabled={!editable} busy={saving} onPress={() => void saveSnippet()} /></Actions>
      </>}
      <Notice>{labels.snippetsAccess}</Notice>
    </Section>

    <Section title={labels.practice} description={labels.practiceHelp}>
      <Field label={labels.practice} value={practice} onChangeText={setPractice} onBlur={() => void keyboard.refresh()} placeholder={labels.practicePlaceholder} multiline editable={supported} />
      {practice.length > 0 && <Button title={labels.clear} subtle onPress={() => setPractice('')} />}
    </Section>

    <Section title={t('mobile.quick_start')} description={t('mobile.quick_start_help')}>
      <NativeChoice label={t('mobile.session.duration')} value={settings.duration} options={[300, 900, 3600].map((value) => ({ value, label: t('mobile.session.minutes', { count: value / 60 }), disabled: !editable }))} onChange={(session_duration_seconds) => void save({ session_duration_seconds })} />
    </Section>
  </>;
}
