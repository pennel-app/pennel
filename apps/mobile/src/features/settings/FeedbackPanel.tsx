import React, { useState } from 'react';
import { View } from 'react-native';
import type { Settings } from '../../domain';
import { useQuery, useRuntime } from '../../runtime';
import { Actions, Button, Field, NativeChoice, Notice, Section, Toggle } from '../../components/ui';

type FeedbackStyle = 'off' | 'gentle' | 'crisp';
type TrialProfile = 'baseline' | 'prepared' | 'light' | 'rigid' | 'off';
type PreviewProfile = FeedbackStyle | TrialProfile;
type KeyboardFeedbackSettings = { feedback_style: FeedbackStyle; trial_active: boolean };

const copy = {
  en: {
    title: 'Feedback', app: 'Haptic feedback', appHelp: 'Feel important recording actions, results, and connection changes.',
    typing: 'Keyboard feel', typingHelp: 'Choose light taps while typing with Pennel. Device settings, recording and hardware support can affect what you feel.',
    off: 'Off', gentle: 'Gentle', crisp: 'Crisp', preview: 'Feel this choice',
    practice: 'Try typing', practicePlaceholder: 'Type a short message…',
    practiceHelp: 'Choose Pennel with the globe key, then type here. This field is not saved; keyboard learning follows your saved choice.', clear: 'Clear practice text',
    compare: 'Compare without labels', compareHelp: 'Try the samples before revealing their names. Samples do not change your saved keyboard feel. Hold briefly, then release each button.',
    sample: 'Sample', reveal: 'Reveal names', again: 'Shuffle again',
    trial: 'Try selected sample on Pennel', trialHelp: 'The selected feel lasts up to five minutes. Your saved choice returns automatically.',
    trialDisabled: 'Choose Gentle or Crisp first to try a sample on the keyboard.',
    trialActive: 'A temporary keyboard feel is selected. Choose a daily feel above or end the trial to return to it.', endTrial: 'End keyboard trial',
    unavailable: 'Haptic previews are available on a supported iPhone.',
    profiles: { baseline: 'Soft tap · standard timing', prepared: 'Soft tap · early preparation', light: 'Light tap', rigid: 'Original crisp tap', off: 'No haptics' },
  },
  nl: {
    title: 'Feedback', app: 'Haptische feedback', appHelp: 'Voel belangrijke opnameacties, resultaten en verbindingswijzigingen.',
    typing: 'Toetsenbordgevoel', typingHelp: 'Kies lichte tikjes tijdens het typen met Pennel. Toestelinstellingen, opnames en hardware kunnen beïnvloeden wat je voelt.',
    off: 'Uit', gentle: 'Zacht', crisp: 'Duidelijk', preview: 'Voel deze keuze',
    practice: 'Probeer te typen', practicePlaceholder: 'Typ een kort bericht…',
    practiceHelp: 'Kies Pennel met de wereldboltoets en typ hier. Dit veld wordt niet opgeslagen; het toetsenbord leert volgens je bewaarde keuze.', clear: 'Oefentekst wissen',
    compare: 'Vergelijk zonder namen', compareHelp: 'Probeer de voorbeelden voordat je hun namen bekijkt. Je bewaarde toetsenbordgevoel verandert niet. Houd elke knop even vast en laat los.',
    sample: 'Voorbeeld', reveal: 'Namen tonen', again: 'Opnieuw schudden',
    trial: 'Gekozen voorbeeld op Pennel proberen', trialHelp: 'Dit gevoel blijft maximaal vijf minuten actief. Daarna keert je bewaarde keuze terug.',
    trialDisabled: 'Kies eerst Zacht of Duidelijk om een voorbeeld op het toetsenbord te proberen.',
    trialActive: 'Er is een tijdelijk toetsenbordgevoel gekozen. Kies hierboven een dagelijks gevoel of beëindig de proef om terug te keren.', endTrial: 'Toetsenbordproef stoppen',
    unavailable: 'Trillingsvoorbeelden zijn beschikbaar op een ondersteunde iPhone.',
    profiles: { baseline: 'Zachte tik · gewone timing', prepared: 'Zachte tik · vroeg voorbereid', light: 'Lichte tik', rigid: 'Oorspronkelijke scherpe tik', off: 'Geen trillingen' },
  },
};

function shuffledSamples(): TrialProfile[] {
  const samples: TrialProfile[] = ['baseline', 'prepared', 'light', 'rigid', 'off'];
  for (let index = samples.length - 1; index > 0; index -= 1) {
    const other = Math.floor(Math.random() * (index + 1));
    [samples[index], samples[other]] = [samples[other]!, samples[index]!];
  }
  return samples;
}

export function FeedbackPanel() {
  const { snapshot, store, run, busy, language } = useRuntime();
  const keyboard = useQuery<KeyboardFeedbackSettings>('settings.keyboard');
  const [practice, setPractice] = useState('');
  const [samples, setSamples] = useState(shuffledSamples);
  const [selectedSample, setSelectedSample] = useState<number | null>(null);
  const [revealed, setRevealed] = useState(false);
  const labels = copy[language];
  const supported = snapshot?.platform === 'ios';
  const saving = busy.has('feedback-settings');
  const sampling = busy.has('feedback-preview');
  const trialWorking = busy.has('feedback-trial');
  const style = keyboard.value?.feedback_style ?? (snapshot?.settings.keyboard_haptics === true ? 'gentle' : 'off');
  const save = async (patch: { haptic_feedback?: boolean; keyboard_haptic_style?: FeedbackStyle }) => {
    const result = await run<Settings>('feedback-settings', 'settings.update', { patch });
    if (result !== undefined) await keyboard.refresh();
  };
  const prepare = (profile: PreviewProfile) => {
    if (!supported || sampling) return;
    void store.client.invoke('keyboard.feedbackPreview', { profile, prepare_only: true }).catch(() => {});
  };
  const preview = (profile: PreviewProfile) => void run('feedback-preview', 'keyboard.feedbackPreview', { profile });
  const trial = async (profile: TrialProfile | null) => {
    const result = await run('feedback-trial', 'keyboard.feedbackTrial', { profile });
    if (result !== undefined) await keyboard.refresh();
  };
  return <>
    <Section title={labels.title}>
      <Toggle title={labels.app} description={labels.appHelp} value={snapshot?.settings.haptic_feedback !== false} disabled={saving} onChange={(haptic_feedback) => void save({ haptic_feedback })} />
      <NativeChoice label={labels.typing} segmented value={style} options={(['off', 'gentle', 'crisp'] as const).map((value) => ({ value, label: labels[value], disabled: saving || trialWorking || !supported }))} onChange={(keyboard_haptic_style) => void save({ keyboard_haptic_style })} />
      <Notice>{labels.typingHelp}</Notice>
      {!supported && <Notice>{labels.unavailable}</Notice>}
      <View onTouchStart={() => prepare(style)}><Button title={labels.preview} subtle disabled={!supported || saving} busy={sampling} onPress={() => preview(style)} /></View>
      {keyboard.error && <Notice error>{keyboard.error}</Notice>}
    </Section>
    <Section title={labels.practice} description={labels.practiceHelp}>
      <Field label={labels.practice} value={practice} onChangeText={setPractice} onBlur={() => void keyboard.refresh()} placeholder={labels.practicePlaceholder} multiline editable={supported} />
      {practice.length > 0 && <Button title={labels.clear} subtle onPress={() => setPractice('')} />}
    </Section>
    <Section title={labels.compare} description={labels.compareHelp}>
      <Actions>{samples.map((profile, index) => {
        const letter = String.fromCharCode(65 + index);
        const title = `${labels.sample} ${letter}${revealed ? ` · ${labels.profiles[profile]}` : ''}`;
        return <View key={letter} onTouchStart={() => prepare(profile)}><Button title={title} subtle disabled={!supported || saving || sampling} onPress={() => { setSelectedSample(index); preview(profile); }} /></View>;
      })}</Actions>
      <Actions>
        {!revealed && <Button title={labels.reveal} subtle onPress={() => setRevealed(true)} />}
        <Button title={labels.again} subtle disabled={sampling || trialWorking || keyboard.value?.trial_active} onPress={() => { setSamples(shuffledSamples()); setSelectedSample(null); setRevealed(false); }} />
      </Actions>
      {selectedSample !== null && <>
        <Notice>{`${labels.sample} ${String.fromCharCode(65 + selectedSample)} · ${labels.trialHelp}`}</Notice>
        {style === 'off' && <Notice>{labels.trialDisabled}</Notice>}
        <Button title={labels.trial} subtle disabled={!supported || style === 'off' || saving || sampling} busy={trialWorking} onPress={() => { const profile = samples[selectedSample]; if (profile) void trial(profile); }} />
      </>}
      {keyboard.value?.trial_active && <><Notice>{labels.trialActive}</Notice><Button title={labels.endTrial} subtle busy={trialWorking} onPress={() => void trial(null)} /></>}
    </Section>
  </>;
}
