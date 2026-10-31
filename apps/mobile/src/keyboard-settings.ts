export interface KeyboardSnippetSettings { id: string; title: string; keyword: string; text: string }
export interface KeyboardSettingsView {
  layout: 'qwerty' | 'azerty'; suggestions: boolean; duration: number;
  keyboard_typing_languages: string[]; keyboard_autocorrection: boolean; keyboard_learning: boolean;
  keyboard_swipe: boolean; keyboard_snippet_expansion: boolean; keyboard_snippets: KeyboardSnippetSettings[];
  keyboard_haptic_style: 'off' | 'gentle' | 'crisp'; keyboard_learned_count: number;
}

export const defaultKeyboardSettings: KeyboardSettingsView = {
  layout: 'qwerty', suggestions: false, duration: 300,
  keyboard_typing_languages: ['nl', 'en'], keyboard_autocorrection: true, keyboard_learning: false,
  keyboard_swipe: false, keyboard_snippet_expansion: false, keyboard_snippets: [],
  keyboard_haptic_style: 'off', keyboard_learned_count: 0,
};

export type TypingLanguageChoice = 'both' | 'en' | 'nl';
export function typingLanguageChoice(languages: readonly string[]): TypingLanguageChoice {
  if (languages.length === 1 && languages[0] === 'en') return 'en';
  if (languages.length === 1 && languages[0] === 'nl') return 'nl';
  return 'both';
}

export function typingLanguagePatch(choice: TypingLanguageChoice) {
  return { keyboard_typing_languages: choice === 'both' ? ['nl', 'en'] : [choice] };
}

export type SnippetProblem = 'titleRequired' | 'shortcutInvalid' | 'shortcutExists' | 'textRequired' | 'textTooLong' | 'snippetLimit' | 'snippetChanged';
export type SnippetPatchResult = { patch: { keyboard_snippets: KeyboardSnippetSettings[] }; problem?: never } | { problem: SnippetProblem; patch?: never };

function utf8Length(text: string): number {
  let bytes = 0;
  for (const character of text) {
    const point = character.codePointAt(0)!;
    bytes += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
  }
  return bytes;
}

export function snippetPatch(current: readonly KeyboardSnippetSettings[], draft: KeyboardSnippetSettings, editing: boolean): SnippetPatchResult {
  if (!draft.id || [...draft.id].length > 128 || /[\s\u0000-\u001f\u007f-\u009f]/u.test(draft.id)) return { problem: 'snippetChanged' };
  if (editing && !current.some((snippet) => snippet.id === draft.id)) return { problem: 'snippetChanged' };
  if (!editing && current.some((snippet) => snippet.id === draft.id)) return { problem: 'snippetChanged' };
  if (!editing && current.length >= 64) return { problem: 'snippetLimit' };
  const title = draft.title.trim();
  if (!title || [...title].length > 80 || /[\u0000-\u001f\u007f-\u009f]/u.test(title)) return { problem: 'titleRequired' };
  if ([...draft.keyword].length > 32 || utf8Length(draft.keyword) > 128 || /[\s\u0000-\u001f\u007f-\u009f]/u.test(draft.keyword)) return { problem: 'shortcutInvalid' };
  if (draft.keyword && current.some((snippet) => snippet.id !== draft.id && snippet.keyword === draft.keyword)) return { problem: 'shortcutExists' };
  if (!draft.text.trim()) return { problem: 'textRequired' };
  if (utf8Length(draft.text) > 2048 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(draft.text)) return { problem: 'textTooLong' };
  const saved = { id: draft.id, title, keyword: draft.keyword, text: draft.text };
  return { patch: { keyboard_snippets: editing ? current.map((snippet) => snippet.id === draft.id ? saved : snippet) : [...current, saved] } };
}

export function deleteSnippetPatch(current: readonly KeyboardSnippetSettings[], id: string) {
  return { keyboard_snippets: current.filter((snippet) => snippet.id !== id) };
}

export function validLearningWord(word: string): boolean {
  return (2 <= [...word].length && [...word].length <= 64) && /^[\p{L}'’-]+$/u.test(word);
}

export const keyboardCopy = {
  en: {
    title: 'Pennel keyboard', languages: 'Typing languages', both: 'Dutch + English', en: 'English', nl: 'Dutch',
    languagesHelp: 'Type Dutch, English, or mixed sentences. This choice is separate from the app language and speech recognition.',
    correction: 'Autocorrection', correctionHelp: 'Correct a small set of common typos. Other changes stay in suggestions. Backspace restores the original word immediately after a correction.',
    learning: 'Learn my typing', learningHelp: 'Remember words and word pairs locally to improve suggestions. Learning is off until you enable it. Turning it off keeps existing learning until you reset it.',
    learned: 'Learned words', reset: 'Reset keyboard learning', resetTitle: 'Reset keyboard learning?',
    resetHelp: 'Delete learned words and word pairs. Your saved dictionary and snippets stay available.',
    resetDone: 'Keyboard learning was reset.', forget: 'Forget a word', forgetHelp: 'Enter a learned word to remove it and related word pairs. Learned words are not displayed here.',
    forgetDone: 'The word and its related pairs were removed from keyboard learning.',
    swipe: 'Swipe typing · experimental', swipeHelp: 'An early version with a limited word list. It is off by default while we test its accuracy on iPhone.',
    snippets: 'Snippets', snippetsHelp: 'Save phrases for explicit insertion. An optional exact shortcut can expand at a word boundary.',
    expansion: 'Expand snippet shortcuts', expansionHelp: 'Only exact saved shortcuts expand. You can still insert snippets explicitly when this is off.',
    newSnippet: 'New snippet', snippetTitle: 'Snippet name', shortcut: 'Shortcut (optional)', text: 'Snippet text',
    edit: 'Edit', deleteSnippet: 'Delete snippet?', deleteHelp: 'Remove this saved snippet from the keyboard.',
    titleRequired: 'Enter a name of up to 80 characters.', shortcutInvalid: 'Use a shortcut of up to 32 characters without spaces.',
    shortcutExists: 'This shortcut is already used by another snippet.', textRequired: 'Enter the text to insert.',
    textTooLong: 'This snippet is too long or contains unsupported characters. Shorten it and try again.',
    snippetLimit: 'You can save up to 64 snippets.', snippetChanged: 'This snippet changed or was removed. Open it again before saving.',
    emptySnippets: 'No snippets saved yet.', snippetsAccess: 'Snippets stored by the app need Full Access in the keyboard.',
    practice: 'Try your keyboard', practiceHelp: 'Choose Pennel with the globe key, then type here. The field is not saved. If you enabled keyboard learning, it may remember words.',
    practicePlaceholder: 'Type a short Dutch or English message…', clear: 'Clear practice text',
    loading: 'Loading keyboard settings…', unavailable: 'Keyboard controls are available on iPhone.',
    noSharedChange: 'The app could not refresh keyboard settings. Try again.',
  },
  nl: {
    title: 'Pennel-toetsenbord', languages: 'Typtalen', both: 'Nederlands + Engels', en: 'Engels', nl: 'Nederlands',
    languagesHelp: 'Typ Nederlands, Engels of gemengde zinnen. Deze keuze staat los van de apptaal en spraakherkenning.',
    correction: 'Autocorrectie', correctionHelp: 'Corrigeert een beperkt aantal veelvoorkomende typefouten. Andere wijzigingen blijven suggesties. Met backspace herstel je direct na een correctie het oorspronkelijke woord.',
    learning: 'Van mijn typen leren', learningHelp: 'Onthoud woorden en woordparen lokaal voor betere suggesties. Leren staat uit totdat je het inschakelt. Uitschakelen bewaart eerdere kennis totdat je die wist.',
    learned: 'Geleerde woorden', reset: 'Toetsenbordkennis wissen', resetTitle: 'Toetsenbordkennis wissen?',
    resetHelp: 'Verwijder geleerde woorden en woordparen. Je bewaarde woordenboek en fragmenten blijven beschikbaar.',
    resetDone: 'De toetsenbordkennis is gewist.', forget: 'Een woord vergeten', forgetHelp: 'Voer een geleerd woord in om het en de bijbehorende woordparen te verwijderen. Geleerde woorden worden hier niet weergegeven.',
    forgetDone: 'Het woord en de bijbehorende woordparen zijn uit de toetsenbordkennis verwijderd.',
    swipe: 'Vegen om te typen · experimenteel', swipeHelp: 'Een eerste versie met een beperkte woordenlijst. Staat standaard uit terwijl we de nauwkeurigheid op iPhone testen.',
    snippets: 'Fragmenten', snippetsHelp: 'Bewaar tekst om zelf in te voegen. Een optionele exacte afkorting kan aan een woordgrens worden uitgebreid.',
    expansion: 'Afkortingen uitbreiden', expansionHelp: 'Alleen exacte bewaarde afkortingen worden uitgebreid. Als dit uitstaat kun je fragmenten nog steeds zelf invoegen.',
    newSnippet: 'Nieuw fragment', snippetTitle: 'Naam van het fragment', shortcut: 'Afkorting (optioneel)', text: 'Tekst van het fragment',
    edit: 'Bewerken', deleteSnippet: 'Fragment verwijderen?', deleteHelp: 'Verwijder dit bewaarde fragment van het toetsenbord.',
    titleRequired: 'Voer een naam van maximaal 80 tekens in.', shortcutInvalid: 'Gebruik een afkorting van maximaal 32 tekens zonder spaties.',
    shortcutExists: 'Deze afkorting wordt al voor een ander fragment gebruikt.', textRequired: 'Voer de tekst in die je wilt invoegen.',
    textTooLong: 'Dit fragment is te lang of bevat niet-ondersteunde tekens. Kort de tekst in en probeer het opnieuw.',
    snippetLimit: 'Je kunt maximaal 64 fragmenten bewaren.', snippetChanged: 'Dit fragment is gewijzigd of verwijderd. Open het opnieuw voordat je het bewaart.',
    emptySnippets: 'Er zijn nog geen fragmenten bewaard.', snippetsAccess: 'Fragmenten uit de app hebben Volledige toegang nodig in het toetsenbord.',
    practice: 'Probeer je toetsenbord', practiceHelp: 'Kies Pennel met de wereldboltoets en typ hier. Het veld wordt niet opgeslagen. Als toetsenbordleren aanstaat, kan het woorden onthouden.',
    practicePlaceholder: 'Typ een kort Nederlands of Engels bericht…', clear: 'Oefentekst wissen',
    loading: 'Toetsenbordinstellingen laden…', unavailable: 'Toetsenbordinstellingen zijn beschikbaar op iPhone.',
    noSharedChange: 'De app kon de toetsenbordinstellingen niet vernieuwen. Probeer het opnieuw.',
  },
} as const;
