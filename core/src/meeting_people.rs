//! Owner-local, explicit identity links. Nothing here becomes a canonical speaker label.
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{MeetingTranscriptSnapshot, PennelEngine, PennelError};

const VERSION: u32 = 1;
const MAX_PEOPLE: usize = 2_000;
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

fn invalid(message: &str) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}
fn uid() -> String {
    uuid::Uuid::now_v7().to_string()
}
fn hash(text: &str) -> String {
    crate::evidence_revision(text)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersonSource {
    source_id: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    record_id: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Person {
    id: String,
    display_name: String,
    #[serde(default)]
    name_is_custom: bool,
    origin: String,
    private_alias: Option<String>,
    email: Option<String>,
    source: Option<PersonSource>,
    source_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpeakerLink {
    id: String,
    person_id: String,
    utterance_ids: Vec<String>,
    state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceAnchor {
    utterance_id: String,
    source_fingerprint: String,
    attribution_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinkRecord {
    view: SpeakerLink,
    anchors: Vec<SourceAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Mention {
    id: String,
    person_id: String,
    visible_name: String,
    start_byte: usize,
    end_byte: usize,
    state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MentionRecord {
    view: Mention,
    exact_text: String,
    notes_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CalendarParticipant {
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    is_self: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CalendarContext {
    source: String,
    #[serde(default)]
    source_id: Option<String>,
    calendar_id: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    external_event_id: Option<String>,
    #[serde(default)]
    instance_id: Option<String>,
    source_revision: String,
    #[serde(default)]
    participants: Vec<CalendarParticipant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MentionReceipt {
    before_hash: String,
    intended_hash: String,
    replaced_end_byte: usize,
    mention: MentionRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Document {
    schema_version: u32,
    revision: u64,
    people: Vec<Person>,
    links: Vec<LinkRecord>,
    mentions: Vec<MentionRecord>,
    calendar: Option<CalendarContext>,
    pending: Option<MentionReceipt>,
    recovery_status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PeopleSnapshot {
    schema_version: u32,
    meeting_uid: String,
    revision: u64,
    notes_revision: String,
    source_revision: String,
    people: Vec<Person>,
    speaker_links: Vec<SpeakerLink>,
    mentions: Vec<Mention>,
    calendar: Option<CalendarContext>,
    recovery_status: String,
}
#[derive(Debug, Serialize)]
struct PeopleUpdateResult {
    snapshot: PeopleSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection_end: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PeopleRequest {
    expected_revision: u64,
    #[serde(flatten)]
    action: PeopleAction,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PeopleAction {
    SetPrivateAlias {
        person_id: String,
        private_alias: Option<String>,
    },
    RenamePerson {
        person_id: String,
        display_name: String,
    },
    CreatePerson {
        display_name: String,
        #[serde(default)]
        origin: Option<String>,
        #[serde(default)]
        private_alias: Option<String>,
        #[serde(default)]
        email: Option<String>,
        #[serde(default)]
        source: Option<PersonSource>,
    },
    ReplaceSource {
        person_id: String,
        display_name: String,
        #[serde(default)]
        private_alias: Option<String>,
        #[serde(default)]
        email: Option<String>,
        source: PersonSource,
    },
    RemovePerson {
        person_id: String,
    },
    InvalidateSource {
        source_id: String,
        #[serde(default)]
        account_id: Option<String>,
    },
    LinkSpeaker {
        person_id: String,
        utterance_ids: Vec<String>,
        expected_source_revision: String,
        #[serde(default)]
        link_id: Option<String>,
    },
    UnlinkSpeaker {
        link_id: String,
    },
    InsertMention {
        person_id: String,
        visible_name: String,
        notes: String,
        start_byte: usize,
        end_byte: usize,
        expected_notes_revision: String,
    },
    RelinkMention {
        mention_id: String,
        person_id: String,
        expected_notes_revision: String,
    },
    UnlinkMention {
        mention_id: String,
    },
}

fn open(data: &Path) -> Result<Connection, PennelError> {
    let path = data.join("people.sqlite");
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err(PennelError::storage(
            "The local people store is unavailable.",
        ));
    }
    let connection = Connection::open(&path).map_err(PennelError::storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(PennelError::storage)?;
    }
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(PennelError::storage)?;
    if version > VERSION {
        return Err(PennelError::storage(
            "The local people store requires a newer Pennel version.",
        ));
    }
    connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA busy_timeout=2500;
        CREATE TABLE IF NOT EXISTS meeting_people (library TEXT NOT NULL, meeting_uid TEXT NOT NULL, revision INTEGER NOT NULL, document TEXT NOT NULL, PRIMARY KEY(library,meeting_uid)); PRAGMA user_version=1;").map_err(PennelError::storage)?;
    Ok(connection)
}

fn library(store: &crate::meetings::MeetingStore) -> Result<String, PennelError> {
    Ok(hash(
        &store
            .root()
            .canonicalize()
            .map_err(PennelError::storage)?
            .to_string_lossy(),
    ))
}
fn write(
    connection: &Connection,
    library: &str,
    meeting: &str,
    document: &Document,
) -> Result<(), PennelError> {
    let text = serde_json::to_string(document).map_err(PennelError::storage)?;
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("This meeting's local people metadata is full."));
    }
    let revision =
        i64::try_from(document.revision).map_err(|_| invalid("The person revision is invalid."))?;
    connection.execute("INSERT INTO meeting_people(library,meeting_uid,revision,document) VALUES (?1,?2,?3,?4) ON CONFLICT(library,meeting_uid) DO UPDATE SET revision=excluded.revision,document=excluded.document", params![library, meeting, revision, text]).map_err(PennelError::storage)?;
    Ok(())
}
fn read(
    connection: &Connection,
    library: &str,
    meeting: &str,
    attendees: Vec<String>,
) -> Result<Document, PennelError> {
    let text: Option<String> = connection
        .query_row(
            "SELECT document FROM meeting_people WHERE library=?1 AND meeting_uid=?2",
            params![library, meeting],
            |r| r.get(0),
        )
        .optional()
        .map_err(PennelError::storage)?;
    if let Some(text) = text {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("The local people metadata is too large."));
        }
        let document: Document = serde_json::from_str(&text).map_err(|_| {
            invalid("The local people metadata could not be read. Its original data was kept.")
        })?;
        if document.schema_version != VERSION {
            return Err(invalid(
                "The local people metadata requires a different Pennel version.",
            ));
        }
        return Ok(document);
    }
    let document = Document {
        schema_version: VERSION,
        revision: 0,
        people: attendees
            .into_iter()
            .take(MAX_PEOPLE)
            .map(|display_name| Person {
                id: uid(),
                display_name,
                name_is_custom: false,
                origin: "attendee".into(),
                private_alias: None,
                email: None,
                source: None,
                source_status: "available".into(),
            })
            .collect(),
        links: vec![],
        mentions: vec![],
        calendar: None,
        pending: None,
        recovery_status: "ready".into(),
    };
    write(connection, library, meeting, &document)?;
    Ok(document)
}
fn sources(snapshot: &MeetingTranscriptSnapshot) -> String {
    let rows: Vec<_> = snapshot
        .utterances
        .iter()
        .map(|row| {
            (
                &row.metadata.utterance_id,
                &row.metadata.source_fingerprint,
                row.metadata.attribution_revision,
            )
        })
        .collect();
    hash(&serde_json::to_string(&(snapshot.metadata_status, rows)).expect("source identities"))
}
fn recover(document: &mut Document, notes: &str) -> bool {
    let Some(receipt) = document.pending.take() else {
        return false;
    };
    let current = hash(notes);
    if current == receipt.intended_hash {
        rebase_mentions(document, &receipt);
        document.mentions.push(receipt.mention);
        document.recovery_status = "ready".into();
    } else if current == receipt.before_hash {
        document.recovery_status = "not_written".into();
    } else {
        let mut mention = receipt.mention;
        mention.view.state = "unresolved".into();
        document.mentions.push(mention);
        document.recovery_status = "needs_review".into();
    }
    document.revision += 1;
    true
}
fn rebase_mentions(document: &mut Document, receipt: &MentionReceipt) {
    let start = receipt.mention.view.start_byte;
    let removed = receipt.replaced_end_byte - start;
    let inserted = receipt.mention.exact_text.len();
    for record in &mut document.mentions {
        if record.notes_revision != receipt.before_hash {
            continue;
        }
        if record.view.end_byte <= start {
            record.notes_revision = receipt.intended_hash.clone();
        } else if record.view.start_byte >= receipt.replaced_end_byte {
            record.view.start_byte = record.view.start_byte + inserted - removed;
            record.view.end_byte = record.view.end_byte + inserted - removed;
            record.notes_revision = receipt.intended_hash.clone();
        } else {
            record.view.state = "unresolved".into();
        }
    }
}
fn snapshot(
    document: &Document,
    meeting_uid: String,
    notes: &str,
    transcript: &MeetingTranscriptSnapshot,
) -> PeopleSnapshot {
    let notes_revision = hash(notes);
    let safe_sources = matches!(
        transcript.metadata_status,
        crate::TranscriptMetadataStatus::Valid | crate::TranscriptMetadataStatus::Absent
    );
    let speaker_links = document
        .links
        .iter()
        .map(|record| {
            let mut view = record.view.clone();
            for anchor in &record.anchors {
                let exact = transcript.utterances.iter().find(|row| {
                    row.metadata.utterance_id == anchor.utterance_id
                        && row.metadata.source_fingerprint == anchor.source_fingerprint
                        && row.metadata.attribution_revision == anchor.attribution_revision
                });
                if exact.is_none() {
                    view.state = "needs_review".into();
                }
            }
            if !safe_sources
                || document.people.iter().any(|person| {
                    person.id == view.person_id && person.source_status == "unavailable"
                })
            {
                view.state = "needs_review".into();
            }
            // Never silently transfer identity to a split, replacement, or recycled acoustic number.
            view
        })
        .collect();
    let mentions = document
        .mentions
        .iter()
        .map(|record| {
            let mut view = record.view.clone();
            if record.notes_revision != notes_revision
                || notes.get(view.start_byte..view.end_byte) != Some(record.exact_text.as_str())
            {
                view.state = "unresolved".into();
            }
            view
        })
        .collect();
    PeopleSnapshot {
        schema_version: VERSION,
        meeting_uid,
        revision: document.revision,
        notes_revision,
        source_revision: sources(transcript),
        people: document.people.clone(),
        speaker_links,
        mentions,
        calendar: document.calendar.clone(),
        recovery_status: document.recovery_status.clone(),
    }
}
fn name(value: &str) -> Result<String, PennelError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 256 || trimmed.chars().any(char::is_control)
    {
        return Err(invalid("Enter a single-line person name."));
    }
    Ok(trimmed.into())
}
fn person<'a>(document: &'a Document, id: &str) -> Result<&'a Person, PennelError> {
    document
        .people
        .iter()
        .find(|p| p.id == id)
        .ok_or(PennelError::NotFound)
}
fn mention_text(label: &str) -> String {
    let mut text = String::from("@");
    for c in label.chars() {
        if "\\`*_{}[]<>#!|".contains(c) {
            text.push('\\');
        }
        text.push(c);
    }
    text
}

#[uniffi::export]
impl PennelEngine {
    pub fn meeting_people_snapshot_json(&self, id: String) -> Result<String, PennelError> {
        self.claim_library()?;
        let _sync = self.sync_gate.lock().unwrap();
        let _gate = self.people_metadata_guard(&id)?;
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        let meeting = store.ensure_uid(&id)?;
        let library = library(&store)?;
        let connection = open(&self.data_dir())?;
        let mut document = read(&connection, &library, &meeting, store.attendees(&id))?;
        let notes = self.meeting_notes_checked(&id)?;
        if recover(&mut document, &notes) {
            write(&connection, &library, &meeting, &document)?;
        }
        let transcript = store.read_transcript_snapshot(&id)?;
        serde_json::to_string(&snapshot(&document, meeting, &notes, &transcript))
            .map_err(PennelError::storage)
    }

    pub fn update_meeting_people_json(
        &self,
        id: String,
        request_json: String,
    ) -> Result<String, PennelError> {
        if request_json.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("The person update is too large."));
        }
        let request: PeopleRequest = serde_json::from_str(&request_json)
            .map_err(|_| invalid("The person update is invalid."))?;
        self.claim_library()?;
        let _sync = self.sync_gate.lock().unwrap();
        let _gate = self.people_metadata_guard(&id)?;
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        let meeting = store.ensure_uid(&id)?;
        let library = library(&store)?;
        let connection = open(&self.data_dir())?;
        let mut document = read(&connection, &library, &meeting, store.attendees(&id))?;
        let mut notes = self.meeting_notes_checked(&id)?;
        if recover(&mut document, &notes) {
            write(&connection, &library, &meeting, &document)?;
        }
        if document.revision != request.expected_revision {
            return Err(invalid(
                "The meeting's people changed. Refresh the picker and try again.",
            ));
        }
        let transcript = store.read_transcript_snapshot(&id)?;
        let mut edited = None;
        let mut selection = None;
        match request.action {
            PeopleAction::SetPrivateAlias {
                person_id,
                private_alias,
            } => {
                let person = document
                    .people
                    .iter_mut()
                    .find(|p| p.id == person_id)
                    .ok_or(PennelError::NotFound)?;
                person.private_alias = private_alias
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(name)
                    .transpose()?;
            }
            PeopleAction::RenamePerson {
                person_id,
                display_name,
            } => {
                let person = document
                    .people
                    .iter_mut()
                    .find(|p| p.id == person_id)
                    .ok_or(PennelError::NotFound)?;
                person.display_name = name(&display_name)?;
                person.name_is_custom = true;
            }
            PeopleAction::CreatePerson {
                display_name,
                origin,
                private_alias,
                email,
                source,
            } => {
                if document.people.len() >= MAX_PEOPLE {
                    return Err(invalid("This meeting's participant list is full."));
                }
                let origin = origin.unwrap_or_else(|| "manual".into());
                if !["manual", "contact"].contains(&origin.as_str()) {
                    return Err(invalid(
                        "Choose a manual person or an explicitly selected contact.",
                    ));
                }
                if origin == "contact" && source.is_none() {
                    return Err(invalid("The selected contact has no source identity."));
                }
                document.people.push(Person {
                    id: uid(),
                    display_name: name(&display_name)?,
                    name_is_custom: origin == "manual",
                    origin: origin.clone(),
                    private_alias,
                    email,
                    source,
                    source_status: if origin == "contact" {
                        "snapshot"
                    } else {
                        "available"
                    }
                    .into(),
                });
            }
            PeopleAction::ReplaceSource {
                person_id,
                display_name,
                private_alias,
                email,
                source,
            } => {
                let person = document
                    .people
                    .iter_mut()
                    .find(|p| p.id == person_id)
                    .ok_or(PennelError::NotFound)?;
                let display_name = name(&display_name)?;
                if !person.name_is_custom {
                    person.display_name = display_name;
                }
                person.private_alias = private_alias;
                person.email = email;
                person.source = Some(source);
                person.source_status = "snapshot".into();
            }
            PeopleAction::RemovePerson { person_id } => {
                person(&document, &person_id)?;
                document.people.retain(|p| p.id != person_id);
                document.links.retain(|l| l.view.person_id != person_id);
                document.mentions.retain(|m| m.view.person_id != person_id);
            }
            PeopleAction::InvalidateSource {
                source_id,
                account_id,
            } => {
                for person in &mut document.people {
                    if person.source.as_ref().is_some_and(|source| {
                        source.source_id == source_id
                            && (account_id.is_none() || source.account_id == account_id)
                    }) {
                        person.source_status = "unavailable".into();
                        person.private_alias = None;
                        person.email = None;
                        // Source record identifiers can themselves contain email addresses.
                        // Preserve the local person identity, never a revoked source's details.
                        if person.origin == "contact" && !person.name_is_custom {
                            person.display_name.clear();
                        }
                        person.source = None;
                    }
                }
                if document.calendar.as_ref().is_some_and(|c| {
                    c.source_id.as_deref().unwrap_or(&c.source) == source_id
                        && (account_id.is_none() || c.account_id == account_id)
                }) {
                    document.calendar = None;
                }
            }
            PeopleAction::LinkSpeaker {
                person_id,
                utterance_ids,
                expected_source_revision,
                link_id,
            } => {
                person(&document, &person_id)?;
                if expected_source_revision != sources(&transcript)
                    || utterance_ids.is_empty()
                    || !matches!(
                        transcript.metadata_status,
                        crate::TranscriptMetadataStatus::Valid
                            | crate::TranscriptMetadataStatus::Absent
                    )
                {
                    return Err(invalid(
                        "The speaker sources changed. Refresh them before linking.",
                    ));
                }
                let mut anchors = Vec::new();
                for utterance_id in utterance_ids {
                    if anchors
                        .iter()
                        .any(|a: &SourceAnchor| a.utterance_id == utterance_id)
                    {
                        continue;
                    }
                    let row = transcript
                        .utterances
                        .iter()
                        .find(|row| {
                            row.metadata.utterance_id == utterance_id && !row.segment.failed
                        })
                        .ok_or_else(|| invalid("A selected speaker source is unavailable."))?;
                    anchors.push(SourceAnchor {
                        utterance_id,
                        source_fingerprint: row.metadata.source_fingerprint.clone(),
                        attribution_revision: row.metadata.attribution_revision,
                    });
                }
                let id = if let Some(link_id) = link_id {
                    if !document.links.iter().any(|l| l.view.id == link_id) {
                        return Err(PennelError::NotFound);
                    }
                    document.links.retain(|l| l.view.id != link_id);
                    link_id
                } else {
                    uid()
                };
                // A finalized utterance belongs to at most one explicit person association.
                for link in &mut document.links {
                    link.anchors.retain(|old| {
                        !anchors
                            .iter()
                            .any(|new| new.utterance_id == old.utterance_id)
                    });
                    link.view.utterance_ids = link
                        .anchors
                        .iter()
                        .map(|a| a.utterance_id.clone())
                        .collect();
                }
                document.links.retain(|l| !l.anchors.is_empty());
                document.links.push(LinkRecord {
                    view: SpeakerLink {
                        id,
                        person_id,
                        utterance_ids: anchors.iter().map(|a| a.utterance_id.clone()).collect(),
                        state: "linked".into(),
                    },
                    anchors,
                });
            }
            PeopleAction::UnlinkSpeaker { link_id } => {
                if !document.links.iter().any(|l| l.view.id == link_id) {
                    return Err(PennelError::NotFound);
                }
                document.links.retain(|l| l.view.id != link_id);
            }
            PeopleAction::InsertMention {
                person_id,
                visible_name,
                notes: expected_notes,
                start_byte,
                end_byte,
                expected_notes_revision,
            } => {
                person(&document, &person_id)?;
                let visible_name = name(&visible_name)?;
                if notes != expected_notes
                    || hash(&notes) != expected_notes_revision
                    || start_byte > end_byte
                    || notes.get(start_byte..end_byte).is_none()
                {
                    return Err(invalid(
                        "The notes or selection changed. Your current text was kept.",
                    ));
                }
                let text = mention_text(&visible_name);
                let mut intended = notes.clone();
                intended.replace_range(start_byte..end_byte, &text);
                let mention = MentionRecord {
                    view: Mention {
                        id: uid(),
                        person_id,
                        visible_name,
                        start_byte,
                        end_byte: start_byte + text.len(),
                        state: "linked".into(),
                    },
                    exact_text: text,
                    notes_revision: hash(&intended),
                };
                document.pending = Some(MentionReceipt {
                    before_hash: hash(&notes),
                    intended_hash: hash(&intended),
                    replaced_end_byte: end_byte,
                    mention,
                });
                document.revision += 1;
                write(&connection, &library, &meeting, &document)?;
                store.write_notes_if_unchanged(&id, &notes, &intended)?;
                let receipt = document.pending.take().expect("prepared mention receipt");
                rebase_mentions(&mut document, &receipt);
                let inserted = receipt.mention.exact_text.len();
                document.mentions.push(receipt.mention);
                document.recovery_status = "ready".into();
                selection = Some(intended[..start_byte + inserted].encode_utf16().count());
                edited = Some(intended.clone());
                notes = intended;
            }
            PeopleAction::RelinkMention {
                mention_id,
                person_id,
                expected_notes_revision,
            } => {
                person(&document, &person_id)?;
                if hash(&notes) != expected_notes_revision {
                    return Err(invalid(
                        "The notes or selection changed. Your current text was kept.",
                    ));
                }
                let mention = document
                    .mentions
                    .iter_mut()
                    .find(|m| m.view.id == mention_id)
                    .ok_or(PennelError::NotFound)?;
                if mention.notes_revision != hash(&notes)
                    || notes.get(mention.view.start_byte..mention.view.end_byte)
                        != Some(mention.exact_text.as_str())
                {
                    return Err(invalid(
                        "This mention's text changed. Select it again before linking.",
                    ));
                }
                mention.view.person_id = person_id;
                mention.view.state = "linked".into();
                mention.notes_revision = hash(&notes);
            }
            PeopleAction::UnlinkMention { mention_id } => {
                if !document.mentions.iter().any(|m| m.view.id == mention_id) {
                    return Err(PennelError::NotFound);
                }
                document.mentions.retain(|m| m.view.id != mention_id);
            }
        }
        document.revision += 1;
        if let Err(error) = write(&connection, &library, &meeting, &document) {
            if edited.is_none() {
                return Err(error);
            }
            // The canonical Markdown is already saved. Return it so an old editor draft cannot
            // overwrite it; the durable pending receipt is reconciled on the next successful read.
            document.recovery_status = "confirmation_pending".into();
        }
        serde_json::to_string(&PeopleUpdateResult {
            snapshot: snapshot(&document, meeting, &notes, &transcript),
            notes: edited,
            selection_start: selection,
            selection_end: selection,
        })
        .map_err(PennelError::storage)
    }

    pub fn set_meeting_calendar_context_json(
        &self,
        id: String,
        context_json: String,
    ) -> Result<String, PennelError> {
        if context_json.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("The calendar participant context is too large."));
        }
        let context: CalendarContext = serde_json::from_str(&context_json)
            .map_err(|_| invalid("The calendar participant context is invalid."))?;
        if context.participants.len() > MAX_PEOPLE {
            return Err(invalid("The calendar participant context is too large."));
        }
        self.claim_library()?;
        let _sync = self.sync_gate.lock().unwrap();
        let _gate = self.people_metadata_guard(&id)?;
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        let meeting = store.ensure_uid(&id)?;
        let library = library(&store)?;
        let connection = open(&self.data_dir())?;
        let mut document = read(&connection, &library, &meeting, vec![])?;
        if document.calendar.as_ref() == Some(&context) {
            let notes = self.meeting_notes_checked(&id)?;
            let transcript = store.read_transcript_snapshot(&id)?;
            return serde_json::to_string(&snapshot(&document, meeting, &notes, &transcript))
                .map_err(PennelError::storage);
        }
        // Revisions from a different event/source do not retarget existing person IDs.
        let same_event = document.calendar.as_ref().is_some_and(|old| {
            old.source == context.source
                && old.source_id.is_some()
                && old.source_id == context.source_id
                && old.account_id == context.account_id
                && old.calendar_id == context.calendar_id
                && old.event_id.is_some()
                && old.event_id == context.event_id
                && old.instance_id == context.instance_id
        });
        for person in &mut document.people {
            if person.origin == "attendee" && person.source.is_some() {
                person.source_status = "unavailable".into();
                person.email = None;
            }
        }
        for participant in &context.participants {
            let existing = if same_event
                && participant.source_id.as_ref().is_some_and(|id| {
                    context
                        .participants
                        .iter()
                        .filter(|p| p.source_id.as_ref() == Some(id))
                        .count()
                        == 1
                }) {
                participant.source_id.as_ref().and_then(|record_id| {
                    document.people.iter().position(|p| {
                        p.origin == "attendee"
                            && p.source.as_ref().is_some_and(|s| {
                                Some(s.source_id.as_str()) == context.source_id.as_deref()
                                    && s.account_id == context.account_id
                                    && s.record_id.as_ref() == Some(record_id)
                            })
                    })
                })
            } else {
                None
            };
            let display_name = participant.name.clone().unwrap_or_default();
            let source = PersonSource {
                source_id: context
                    .source_id
                    .clone()
                    .unwrap_or_else(|| context.source.clone()),
                account_id: context.account_id.clone(),
                record_id: participant.source_id.clone(),
                revision: Some(context.source_revision.clone()),
            };
            if let Some(index) = existing {
                let p = &mut document.people[index];
                if !p.name_is_custom {
                    p.display_name = display_name;
                }
                p.email = participant.email.clone();
                p.source = Some(source);
                p.source_status = "available".into();
            } else {
                if document.people.len() >= MAX_PEOPLE {
                    return Err(invalid("This meeting's participant list is full."));
                }
                document.people.push(Person {
                    id: uid(),
                    display_name,
                    name_is_custom: false,
                    origin: "attendee".into(),
                    private_alias: None,
                    email: participant.email.clone(),
                    source: Some(source),
                    source_status: "available".into(),
                });
            }
        }
        document.calendar = Some(context);
        document.revision += 1;
        write(&connection, &library, &meeting, &document)?;
        let notes = self.meeting_notes_checked(&id)?;
        let transcript = store.read_transcript_snapshot(&id)?;
        serde_json::to_string(&snapshot(&document, meeting, &notes, &transcript))
            .map_err(PennelError::storage)
    }

    /// Public projections intentionally contain authored mention labels only, never contact cards.
    pub fn meeting_people_public_projection_json(&self, id: String) -> Result<String, PennelError> {
        let owner: PeopleSnapshot = serde_json::from_str(&self.meeting_people_snapshot_json(id)?)
            .map_err(PennelError::storage)?;
        serde_json::to_string(&serde_json::json!({"schema_version":VERSION,"mentions":owner.mentions.iter().filter(|m|m.state=="linked").map(|m|serde_json::json!({"visible_name":m.visible_name,"start_byte":m.start_byte,"end_byte":m.end_byte})).collect::<Vec<_>>()})).map_err(PennelError::storage)
    }
}

#[cfg(test)]
mod tests {
    use crate::PennelEngine;
    use serde_json::{Value, json};

    fn meeting() -> (tempfile::TempDir, std::sync::Arc<PennelEngine>, String) {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let id = engine
            .meeting_store()
            .create("People", chrono::Local::now())
            .unwrap();
        (dir, engine, id)
    }

    fn snapshot(engine: &PennelEngine, id: &str) -> Value {
        serde_json::from_str(&engine.meeting_people_snapshot_json(id.into()).unwrap()).unwrap()
    }

    fn update(engine: &PennelEngine, id: &str, request: Value) -> Value {
        serde_json::from_str(
            &engine
                .update_meeting_people_json(id.into(), request.to_string())
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn same_named_people_remain_distinct_and_survive_rename_and_restart() {
        let (dir, engine, id) = meeting();
        let first = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Alex"}),
        );
        let second = update(
            &engine,
            &id,
            json!({"expected_revision":first["snapshot"]["revision"],"action":"create_person","display_name":"Alex"}),
        );
        let people = second["snapshot"]["people"].as_array().unwrap();
        assert_eq!(people.len(), 2);
        assert_ne!(people[0]["id"], people[1]["id"]);
        let edited = update(
            &engine,
            &id,
            json!({"expected_revision":second["snapshot"]["revision"],"action":"rename_person","person_id":people[0]["id"],"display_name":"Alexander"}),
        );
        assert_eq!(edited["snapshot"]["people"][0]["id"], people[0]["id"]);
        assert_eq!(edited["snapshot"]["people"][0]["display_name"], "Alexander");
        let aliased = update(
            &engine,
            &id,
            json!({"expected_revision":edited["snapshot"]["revision"],"action":"set_private_alias","person_id":people[0]["id"],"private_alias":"Private guest label"}),
        );
        assert_eq!(aliased["snapshot"]["people"][0]["id"], people[0]["id"]);
        assert_eq!(
            aliased["snapshot"]["people"][0]["private_alias"],
            "Private guest label"
        );
        assert!(
            !engine
                .meeting_people_public_projection_json(id.clone())
                .unwrap()
                .contains("Private guest label")
        );
        assert!(
            !engine
                .export_meeting(id.clone(), true)
                .unwrap()
                .markdown
                .contains("Private guest label")
        );
        let renamed = engine.rename_meeting(id, "A new title".into()).unwrap();
        let expected = snapshot(&engine, &renamed);
        drop(engine);
        let reopened =
            PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(snapshot(&reopened, &renamed), expected);
    }

    #[test]
    fn mentions_preserve_exact_markdown_and_do_not_retarget_edited_text() {
        let (_dir, engine, id) = meeting();
        let original = "## Één\r\n\r\n[link](https://example.test) `@plain` Alex\r\n";
        engine
            .set_meeting_notes(id.clone(), original.into())
            .unwrap();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Alias only"}),
        );
        let state = &created["snapshot"];
        let person = state["people"][0]["id"].clone();
        let start = original.find("Alex").unwrap();
        let inserted = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":person,"visible_name":"Visible","notes":original,"start_byte":start,"end_byte":start+4,"expected_notes_revision":state["notes_revision"]}),
        );
        let intended = "## Één\r\n\r\n[link](https://example.test) `@plain` @Visible\r\n";
        assert_eq!(engine.meeting_notes(id.clone()), intended);
        assert_eq!(inserted["notes"], intended);
        let relinked = update(
            &engine,
            &id,
            json!({"expected_revision":inserted["snapshot"]["revision"],"action":"relink_mention","mention_id":inserted["snapshot"]["mentions"][0]["id"],"person_id":person,"expected_notes_revision":inserted["snapshot"]["notes_revision"]}),
        );
        assert_eq!(relinked["snapshot"]["mentions"][0]["state"], "linked");
        engine
            .set_meeting_notes(id.clone(), intended.replace("## Één", "## Twee!"))
            .unwrap();
        assert_eq!(snapshot(&engine, &id)["mentions"][0]["state"], "unresolved");
        let state = snapshot(&engine, &id);
        assert!(engine.update_meeting_people_json(id.clone(),json!({"expected_revision":state["revision"],"action":"relink_mention","mention_id":state["mentions"][0]["id"],"person_id":person,"expected_notes_revision":inserted["snapshot"]["notes_revision"]}).to_string()).is_err());
        assert!(engine.update_meeting_people_json(id.clone(),json!({"expected_revision":state["revision"],"action":"relink_mention","mention_id":state["mentions"][0]["id"],"person_id":person,"expected_notes_revision":state["notes_revision"]}).to_string()).is_err());
        update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"unlink_mention","mention_id":state["mentions"][0]["id"]}),
        );
        assert_eq!(
            engine.meeting_notes(id),
            intended.replace("## Één", "## Twee!")
        );
    }

    #[test]
    fn speaker_links_preserve_corrections_and_stale_links_keep_their_identity() {
        let (_dir, engine, id) = meeting();
        let row = crate::Segment {
            start_ms: 0,
            end_ms: 1000,
            speaker: crate::Speaker::Others,
            text: "A source".into(),
            failed: false,
            voice: Some(1),
        };
        engine
            .meeting_store()
            .write_transcript(&id, std::slice::from_ref(&row))
            .unwrap();
        engine
            .rename_speaker(id.clone(), 1, "Authored label".into())
            .unwrap();
        let before = engine.meeting_utterance_snapshot(id.clone()).unwrap();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Private lookup"}),
        );
        let state = &created["snapshot"];
        let linked = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"link_speaker","person_id":state["people"][0]["id"],"utterance_ids":[before.utterances[0].metadata.utterance_id],"expected_source_revision":state["source_revision"]}),
        );
        assert_eq!(
            engine.meeting_utterance_snapshot(id.clone()).unwrap(),
            before
        );
        assert_eq!(
            engine.meeting_speakers(id.clone())[0].name,
            "Authored label"
        );
        let link = &linked["snapshot"]["speaker_links"][0];
        assert_eq!(link["state"], "linked");
        let mut changed = row;
        changed.text = "Replacement source".into();
        engine
            .meeting_store()
            .write_transcript(&id, &[changed])
            .unwrap();
        let stale = snapshot(&engine, &id);
        assert_eq!(stale["speaker_links"][0]["state"], "needs_review");
        assert_eq!(
            stale["speaker_links"][0]["utterance_ids"],
            link["utterance_ids"]
        );
        let error=engine.update_meeting_people_json(id.clone(),json!({"expected_revision":stale["revision"],"action":"link_speaker","person_id":stale["people"][0]["id"],"utterance_ids":link["utterance_ids"],"expected_source_revision":state["source_revision"]}).to_string());
        assert!(error.is_err());
        update(
            &engine,
            &id,
            json!({"expected_revision":stale["revision"],"action":"unlink_speaker","link_id":link["id"]}),
        );
        assert!(
            snapshot(&engine, &id)["speaker_links"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn contact_revocation_preserves_authored_mentions_and_excludes_private_projections() {
        let (dir, engine, id) = meeting();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Private card","private_alias":"Private nickname","email":"private@example.test","origin":"contact","source":{"source_id":"contacts","account_id":"owner","record_id":"card","revision":"one"}}),
        );
        let state = &created["snapshot"];
        let inserted = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":state["people"][0]["id"],"visible_name":"Approved name","notes":"","start_byte":0,"end_byte":0,"expected_notes_revision":state["notes_revision"]}),
        );
        let public = engine
            .meeting_people_public_projection_json(id.clone())
            .unwrap();
        let export = engine.export_meeting(id.clone(), true).unwrap().markdown;
        assert!(public.contains("Approved name"));
        assert!(export.contains("Approved name"));
        for private in [
            "Private card",
            "Private nickname",
            "private@example.test",
            "\"card\"",
        ] {
            assert!(!public.contains(private));
            assert!(!export.contains(private));
        }
        let store = engine.meeting_store();
        let files = crate::cloud::sync::scan::scan(
            &store.root(),
            &[(engine.meeting_uid(id.clone()).unwrap(), id.clone())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert!(dir.path().join("people.sqlite").is_file());
        assert!(
            files
                .iter()
                .all(|file| !file.rel_path.to_string_lossy().contains("people.sqlite"))
        );
        let revoked = update(
            &engine,
            &id,
            json!({"expected_revision":inserted["snapshot"]["revision"],"action":"invalidate_source","source_id":"contacts","account_id":"owner"}),
        );
        assert_eq!(
            revoked["snapshot"]["people"][0]["id"],
            state["people"][0]["id"]
        );
        assert_eq!(
            revoked["snapshot"]["people"][0]["source_status"],
            "unavailable"
        );
        assert_eq!(revoked["snapshot"]["people"][0]["email"], Value::Null);
        assert_eq!(
            revoked["snapshot"]["people"][0]["private_alias"],
            Value::Null
        );
        assert_eq!(engine.meeting_notes(id), "@Approved name");
    }

    #[test]
    fn durable_notes_are_returned_when_mention_confirmation_fails_and_retry_recovers_once() {
        let (dir, engine, id) = meeting();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Els"}),
        );
        let connection = rusqlite::Connection::open(dir.path().join("people.sqlite")).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_confirmation BEFORE UPDATE ON meeting_people WHEN json_extract(NEW.document,'$.pending') IS NULL BEGIN SELECT RAISE(FAIL,'injected storage failure'); END;").unwrap();
        let state = &created["snapshot"];
        let inserted = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":state["people"][0]["id"],"visible_name":"Els","notes":"","start_byte":0,"end_byte":0,"expected_notes_revision":state["notes_revision"]}),
        );
        assert_eq!(inserted["notes"], "@Els");
        assert_eq!(
            inserted["snapshot"]["recovery_status"],
            "confirmation_pending"
        );
        assert_eq!(engine.meeting_notes(id.clone()), "@Els");
        connection
            .execute_batch("DROP TRIGGER fail_confirmation;")
            .unwrap();
        let recovered = snapshot(&engine, &id);
        assert_eq!(recovered["recovery_status"], "ready");
        assert_eq!(recovered["mentions"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot(&engine, &id)["mentions"], recovered["mentions"]);
    }

    #[test]
    fn calendar_import_preserves_duplicate_guests_and_unknown_fields_without_merging() {
        let (_dir, engine, id) = meeting();
        let context = json!({"source":"eventkit","source_id":"calendar-source","calendar_id":"calendar","event_id":"event","instance_id":"instance","source_revision":"one","participants":[{"source_id":"mailto:shared@example.test","name":"Alex","response":"declined","is_self":false},{"source_id":"mailto:shared@example.test","name":"Alex","is_self":true},{"name":"Guest"},{}]});
        let first: Value = serde_json::from_str(
            &engine
                .set_meeting_calendar_context_json(id.clone(), context.to_string())
                .unwrap(),
        )
        .unwrap();
        let people = first["people"].as_array().unwrap();
        assert_eq!(people.len(), 4);
        assert_ne!(people[0]["id"], people[1]["id"]);
        assert_eq!(first["calendar"]["participants"][0]["response"], "declined");
        assert_eq!(first["calendar"]["participants"][1]["is_self"], true);
        assert_eq!(first["calendar"]["participants"][3]["is_self"], Value::Null);
        assert_eq!(first["calendar"]["participants"][2]["email"], Value::Null);
        let repeated: Value = serde_json::from_str(
            &engine
                .set_meeting_calendar_context_json(id.clone(), context.to_string())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first, repeated);
        let mut changed = context;
        changed["source_revision"] = json!("two");
        let refreshed: Value = serde_json::from_str(
            &engine
                .set_meeting_calendar_context_json(id, changed.to_string())
                .unwrap(),
        )
        .unwrap();
        let available = refreshed["people"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["source_status"] == "available")
            .collect::<Vec<_>>();
        assert_eq!(available.len(), 4);
        assert_ne!(available[0]["id"], available[1]["id"]);
        assert_eq!(refreshed["people"][0]["source_status"], "unavailable");
    }

    #[test]
    fn missing_calendar_source_identity_can_be_revoked_without_retaining_private_source_details() {
        let (_dir, engine, id) = meeting();
        let notes = "## Authored\r\nKeep **this** text.\r\n";
        engine.set_meeting_notes(id.clone(), notes.into()).unwrap();
        let context = json!({"source":"eventkit","source_id":null,"calendar_id":"known-calendar","event_id":"event","instance_id":"instance","source_revision":"one","participants":[{"source_id":"mailto:private-attendee@example.test","name":"Historical attendee","email":"private-attendee@example.test"}]});
        let imported: Value = serde_json::from_str(
            &engine
                .set_meeting_calendar_context_json(id.clone(), context.to_string())
                .unwrap(),
        )
        .unwrap();
        let person_id = imported["people"][0]["id"].clone();
        let renamed = update(
            &engine,
            &id,
            json!({"expected_revision":imported["revision"],"action":"rename_person","person_id":person_id,"display_name":"Authored attendee"}),
        );
        let revoked = update(
            &engine,
            &id,
            json!({"expected_revision":renamed["snapshot"]["revision"],"action":"invalidate_source","source_id":"eventkit"}),
        );
        let state = &revoked["snapshot"];
        assert_eq!(state["calendar"], Value::Null);
        assert_eq!(state["people"][0]["id"], person_id);
        assert_eq!(state["people"][0]["display_name"], "Authored attendee");
        assert_eq!(state["people"][0]["source_status"], "unavailable");
        assert_eq!(state["people"][0]["email"], Value::Null);
        assert_eq!(state["people"][0]["source"], Value::Null);
        assert!(!state.to_string().contains("private-attendee@example.test"));
        assert_eq!(engine.meeting_notes(id), notes);
    }

    #[test]
    fn stale_people_and_note_selections_cannot_overwrite_current_text() {
        let (_dir, engine, id) = meeting();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Els"}),
        );
        assert!(engine.update_meeting_people_json(id.clone(),json!({"expected_revision":0,"action":"create_person","display_name":"Late result"}).to_string()).is_err());
        let state = &created["snapshot"];
        engine
            .set_meeting_notes(id.clone(), "Current owner text".into())
            .unwrap();
        assert!(engine.update_meeting_people_json(id.clone(),json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":state["people"][0]["id"],"visible_name":"Els","notes":"","start_byte":0,"end_byte":0,"expected_notes_revision":state["notes_revision"]}).to_string()).is_err());
        assert_eq!(engine.meeting_notes(id.clone()), "Current owner text");
        assert_eq!(
            snapshot(&engine, &id)["people"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn newer_people_store_is_preserved_and_does_not_disable_engine_or_ordinary_notes() {
        let (dir, engine, id) = meeting();
        snapshot(&engine, &id);
        let connection = rusqlite::Connection::open(dir.path().join("people.sqlite")).unwrap();
        connection
            .execute_batch("PRAGMA user_version=9000;")
            .unwrap();
        drop(connection);
        assert!(engine.meeting_people_snapshot_json(id.clone()).is_err());
        engine.set_meeting_notes(id.clone(), "Kept".into()).unwrap();
        drop(engine);
        let reopened =
            PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(reopened.meeting_notes(id.clone()), "Kept");
        assert!(reopened.meeting_people_snapshot_json(id).is_err());
        let version: u32 = rusqlite::Connection::open(dir.path().join("people.sqlite"))
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 9000);
    }

    #[test]
    fn later_explicit_insertions_rebase_only_exact_known_mentions() {
        let (_dir, engine, id) = meeting();
        let created = update(
            &engine,
            &id,
            json!({"expected_revision":0,"action":"create_person","display_name":"Noor"}),
        );
        let state = &created["snapshot"];
        let first = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":state["people"][0]["id"],"visible_name":"Noor","notes":"","start_byte":0,"end_byte":0,"expected_notes_revision":state["notes_revision"]}),
        );
        let state = &first["snapshot"];
        let second = update(
            &engine,
            &id,
            json!({"expected_revision":state["revision"],"action":"insert_mention","person_id":state["people"][0]["id"],"visible_name":"Éls","notes":"@Noor","start_byte":0,"end_byte":0,"expected_notes_revision":state["notes_revision"]}),
        );
        assert_eq!(second["notes"], "@Éls@Noor");
        assert_eq!(second["selection_start"], 4);
        assert_eq!(second["snapshot"]["mentions"][0]["start_byte"], 5);
        assert!(
            second["snapshot"]["mentions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["state"] == "linked")
        );
    }
}
