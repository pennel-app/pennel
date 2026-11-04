export interface PersonSource {
  source_id: string;
  account_id: string | null;
  record_id: string | null;
  revision: string | null;
}

export interface MeetingPerson {
  id: string;
  display_name: string;
  origin: "manual" | "attendee" | "contact";
  private_alias: string | null;
  email: string | null;
  source: PersonSource | null;
  source_status: "snapshot" | "available" | "unavailable";
}

export interface SpeakerPersonLink {
  id: string;
  person_id: string;
  utterance_ids: string[];
  state: "linked" | "needs_review";
}

export interface PersonMention {
  id: string;
  person_id: string;
  visible_name: string;
  start_byte: number;
  end_byte: number;
  state: "linked" | "unresolved";
}

export interface PeopleCalendarContext {
  source: string;
  source_id: string | null;
  account_id: string | null;
  calendar_id: string;
  event_id: string | null;
  external_event_id: string | null;
  instance_id: string | null;
  source_revision: string;
  participants: { source_id: string | null; name: string | null; email: string | null; response: string | null; is_self: boolean | null }[];
}

export interface PeopleSnapshot {
  schema_version: number;
  meeting_uid: string;
  revision: number;
  notes_revision: string;
  source_revision: string;
  people: MeetingPerson[];
  speaker_links: SpeakerPersonLink[];
  mentions: PersonMention[];
  calendar: PeopleCalendarContext | null;
  recovery_status?: "ready" | "not_written" | "needs_review" | "confirmation_pending";
}

export interface PeopleUpdateResult {
  snapshot: PeopleSnapshot;
  notes?: string;
  selection_start?: number;
  selection_end?: number;
}

type PersonFields = { display_name: string; private_alias?: string | null; email?: string | null; source?: PersonSource | null };
export type PeopleAction =
  | ({ action: "create_person"; origin?: MeetingPerson["origin"] } & PersonFields)
  | ({ action: "replace_source"; person_id: string } & PersonFields)
  | { action: "rename_person"; person_id: string; display_name: string }
  | { action: "set_private_alias"; person_id: string; private_alias: string | null }
  | { action: "remove_person"; person_id: string }
  | { action: "invalidate_source"; source_id: string; account_id?: string | null }
  | { action: "link_speaker"; person_id: string; utterance_ids: string[]; expected_source_revision: string; link_id?: string }
  | { action: "unlink_speaker"; link_id: string }
  | { action: "insert_mention"; person_id: string; visible_name: string; notes: string; start_byte: number; end_byte: number; expected_notes_revision: string }
  | { action: "relink_mention"; mention_id: string; person_id: string; expected_notes_revision: string }
  | { action: "unlink_mention"; mention_id: string };
export type PeopleRequest = PeopleAction & { expected_revision: number };

export interface ChosenContact {
  source: string;
  source_id: string;
  source_revision: string;
  display_name: string;
  email: string | null;
  selected_at_ms: number;
}
export interface ContactPickResult {
  status: "selected" | "cancelled" | "unavailable";
  contact?: ChosenContact;
  reason?: string;
}

export interface PeopleSelection {
  text: string;
  revision: number;
  selection_start: number;
  selection_end: number;
  selection_revision: number;
  composing: boolean;
}

export function participantGroups(people: MeetingPerson[], query: string): { kind: "meeting" | "contacts"; people: MeetingPerson[] }[] {
  const term = query.trim().toLocaleLowerCase();
  const matching = people.filter(person => !term || [person.display_name, person.private_alias, person.email].some(value => value?.toLocaleLowerCase().includes(term)));
  return ([{ kind: "meeting", people: matching.filter(person => person.origin !== "contact") },
    { kind: "contacts", people: matching.filter(person => person.origin === "contact") }] as const)
    .filter(group => group.people.length > 0);
}

export function participantHint(person: MeetingPerson, people: MeetingPerson[], calendar: PeopleCalendarContext | null): { ordinal: number | null; is_self: boolean; response: string | null } {
  const label = (person: MeetingPerson) => (person.private_alias || person.display_name).toLocaleLowerCase();
  const duplicates = people.filter(other => label(other) === label(person));
  const ordinal = duplicates.length > 1 ? duplicates.findIndex(other => other.id === person.id) + 1 : null;
  const sameSource = person.origin === "attendee" && person.source_status !== "unavailable" && person.source?.record_id
    && calendar && person.source.source_id === (calendar.source_id ?? calendar.source) && person.source.account_id === calendar.account_id;
  const candidates = sameSource ? calendar.participants.filter(participant => participant.source_id === person.source!.record_id) : [];
  const participant = candidates.length === 1 ? candidates[0] : null;
  return { ordinal, is_self: participant?.is_self === true, response: participant?.response ?? null };
}

export function noteSelectionBytes(text: string, start: number, end: number): { start_byte: number; end_byte: number } | null {
  const boundary = (index: number) => Number.isInteger(index) && index >= 0 && index <= text.length
    && !(index > 0 && index < text.length && /[\uD800-\uDBFF]/u.test(text[index - 1]) && /[\uDC00-\uDFFF]/u.test(text[index]));
  if (start > end || !boundary(start) || !boundary(end)) return null;
  const encoder = new TextEncoder();
  return { start_byte: encoder.encode(text.slice(0, start)).length, end_byte: encoder.encode(text.slice(0, end)).length };
}

export function samePeopleSelection(before: PeopleSelection, after: PeopleSelection): boolean {
  return !before.composing && !after.composing && before.text === after.text && before.revision === after.revision
    && before.selection_start === after.selection_start && before.selection_end === after.selection_end
    && before.selection_revision === after.selection_revision;
}

/** A committed write belongs to its captured draft even when the picker no longer owns focus. */
export function canAcknowledgePeopleNotes(before: PeopleSelection, current: PeopleSelection, saved: string, owner: string | null, currentOwner: string | null): boolean {
  return owner !== null && owner === currentOwner && !current.composing && (current.text === before.text || current.text === saved);
}

interface SpeakerTurn { utterance_id?: string | null; speaker: string; voice: number | null; attribution?: { kind: string } | null }
export function speakerLinkUtteranceIds(current: SpeakerTurn, turns: SpeakerTurn[]): string[] {
  const unambiguous = (turn: SpeakerTurn) => !turn.attribution || turn.attribution.kind === "single";
  if (!current.utterance_id || !unambiguous(current)) return [];
  if (current.voice === null) return [current.utterance_id];
  return turns.filter(turn => turn.utterance_id && turn.speaker === current.speaker && turn.voice === current.voice && unambiguous(turn)).map(turn => turn.utterance_id!);
}

export class PeopleLoadEpoch {
  private epoch = 0;
  private meeting = "";
  private session: string | undefined;
  begin(meeting: string, session?: string): number { this.meeting = meeting; this.session = session; return ++this.epoch; }
  invalidate(): void { ++this.epoch; }
  accepts(epoch: number, meeting: string, session?: string): boolean { return epoch === this.epoch && meeting === this.meeting && session === this.session; }
}
