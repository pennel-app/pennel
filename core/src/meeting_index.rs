use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::PennelError;
#[cfg(test)]
use crate::meetings::Segment;
use crate::meetings::{MeetingInfo, MeetingStore, SpeakerName, speaker_label};

const PASSAGE_CHARS: usize = 600;
const SCHEMA_VERSION: i64 = 3;
const TRANSCRIPT_RENDER_REVISION: &str = "speaker-estimates-v1";
const FILES: [(&str, PassageKind); 3] = [
    ("notes.md", PassageKind::Notes),
    ("transcript.md", PassageKind::Transcript),
    ("meeting.md", PassageKind::Summary),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassageKind {
    Notes,
    Transcript,
    Summary,
}

impl PassageKind {
    fn from_file(name: &str) -> Option<Self> {
        FILES
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, kind)| *kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize, serde::Deserialize)]
pub struct Passage {
    pub meeting_id: String,
    pub title: String,
    pub start_iso: String,
    pub kind: PassageKind,
    pub start_ms: Option<u64>,
    pub text: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub utterance_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub utterance_fingerprints: Vec<String>,
}

/// Full-text mirror of the Markdown meetings; the files stay the source of truth.
pub struct MeetingIndex {
    conn: Connection,
}

fn db(e: rusqlite::Error) -> PennelError {
    PennelError::Storage {
        message: e.to_string(),
    }
}

/// Words only, each as a quoted prefix term, so user input can never be read as FTS syntax.
pub(crate) fn fts_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect()
}

/// Cuts text longer than a passage on word boundaries, so nothing is dropped from the index.
fn split_long(text: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    for word in text.split(' ') {
        let current = parts.last_mut().expect("non-empty");
        if !current.is_empty() && current.len() + word.len() + 1 > PASSAGE_CHARS {
            parts.push(String::new());
        }
        let current = parts.last_mut().expect("non-empty");
        if !current.is_empty() {
            current.push(' ');
        }
        let room = PASSAGE_CHARS.saturating_sub(current.len());
        if word.len() > room {
            let mut cut = room;
            while !word.is_char_boundary(cut) {
                cut -= 1;
            }
            current.push_str(&word[..cut]);
            parts.extend(split_long(&word[cut..]));
        } else {
            current.push_str(word);
        }
    }
    parts.retain(|p| !p.is_empty());
    parts
}

fn pack(pieces: impl IntoIterator<Item = (Option<u64>, String)>) -> Vec<(Option<u64>, String)> {
    let mut passages: Vec<(Option<u64>, String)> = Vec::new();
    for (start, piece) in pieces {
        for part in split_long(&piece) {
            match passages.last_mut() {
                Some((_, text)) if text.len() + part.len() < PASSAGE_CHARS => {
                    text.push('\n');
                    text.push_str(&part);
                }
                _ => passages.push((start, part)),
            }
        }
    }
    passages
}

#[cfg(test)]
fn transcript_passages(segments: &[Segment], names: &[SpeakerName]) -> Vec<(Option<u64>, String)> {
    pack(
        segments
            .iter()
            .filter(|s| !s.failed && !s.text.trim().is_empty())
            .map(|s| {
                (
                    Some(s.start_ms),
                    format!("{}: {}", speaker_label(s, names), s.text.trim()),
                )
            }),
    )
}

fn text_passages(text: &str) -> Vec<(Option<u64>, String)> {
    pack(
        text.lines()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| (None, p.to_owned())),
    )
}

struct IndexedPassage {
    start: Option<u64>,
    text: String,
    utterance_ids: Vec<String>,
    utterance_fingerprints: Vec<String>,
}

fn canonical_transcript_passages(
    snapshot: &crate::MeetingTranscriptSnapshot,
    names: &[SpeakerName],
) -> Vec<IndexedPassage> {
    let mut passages: Vec<IndexedPassage> = Vec::new();
    for row in snapshot
        .utterances
        .iter()
        .filter(|row| !row.segment.failed && !row.segment.text.trim().is_empty())
    {
        let label = if snapshot.metadata_status == crate::TranscriptMetadataStatus::Absent {
            speaker_label(&row.segment, names)
        } else {
            crate::meeting_evidence::utterance_speaker_label(row, names)
        };
        let fingerprint = crate::utterance_evidence_revision(row);
        for part in split_long(&format!("{label}: {}", row.segment.text.trim())) {
            if let Some(last) = passages
                .last_mut()
                .filter(|last| last.text.len() + part.len() < PASSAGE_CHARS)
            {
                last.text.push('\n');
                last.text.push_str(&part);
                if !last.utterance_ids.contains(&row.metadata.utterance_id) {
                    last.utterance_ids.push(row.metadata.utterance_id.clone());
                    last.utterance_fingerprints.push(fingerprint.clone());
                }
            } else {
                passages.push(IndexedPassage {
                    start: Some(row.segment.start_ms),
                    text: part,
                    utterance_ids: vec![row.metadata.utterance_id.clone()],
                    utterance_fingerprints: vec![fingerprint.clone()],
                });
            }
        }
    }
    passages
}

/// Admission uses the same rendering as retrieval, so a refreshed name cannot validate old text.
pub(crate) fn frozen_passage_matches(
    passage: &Passage,
    rows: &[crate::MeetingUtterance],
    names: &[SpeakerName],
    metadata_status: crate::TranscriptMetadataStatus,
    text: Option<&str>,
) -> bool {
    if passage.kind != PassageKind::Transcript {
        return text.is_some_and(|text| {
            text_passages(text)
                .iter()
                .any(|(_, part)| part == &passage.text)
        });
    }
    let snapshot = crate::MeetingTranscriptSnapshot {
        meeting_uid: passage.meeting_uid.clone().unwrap_or_default(),
        revision: 0,
        transcript_digest: String::new(),
        metadata_digest: None,
        metadata_status,
        utterances: rows.to_vec(),
    };
    canonical_transcript_passages(&snapshot, names)
        .iter()
        .any(|current| {
            current.text == passage.text
                && current.utterance_ids == passage.utterance_ids
                && current.utterance_fingerprints == passage.utterance_fingerprints
        })
}

fn transcript_source_stamp(store: &MeetingStore, id: &str) -> Result<String, PennelError> {
    let root = store.dir_of(id);
    let mut stamps = Vec::new();
    for subdirectory in ["", "failed"] {
        let directory = root.join(subdirectory);
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PennelError::storage(error)),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        for entry in std::fs::read_dir(directory).map_err(PennelError::storage)? {
            let entry = entry.map_err(PennelError::storage)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relevant = if subdirectory.is_empty() {
                name == crate::meeting_utterances::MANIFEST
                    || name == crate::meeting_utterances::SNAPSHOT_JOURNAL
                    || (name.starts_with("transcript.meta.") && name.ends_with(".json"))
            } else {
                name.ends_with(".wav")
            };
            if relevant {
                let metadata =
                    std::fs::symlink_metadata(entry.path()).map_err(PennelError::storage)?;
                let modified = metadata
                    .modified()
                    .map_err(PennelError::storage)?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(PennelError::storage)?
                    .as_nanos();
                stamps.push((
                    format!("{subdirectory}/{name}"),
                    modified,
                    metadata.len(),
                    metadata.file_type().is_symlink(),
                ));
            }
        }
    }
    stamps.sort();
    serde_json::to_string(&stamps)
        // A speaker-label change must refresh cached passages even when source files did not change.
        .map(|value| {
            format!(
                "{TRANSCRIPT_RENDER_REVISION}:{}",
                crate::evidence_revision(&value)
            )
        })
        .map_err(PennelError::storage)
}

impl MeetingIndex {
    pub fn open(path: &Path) -> Result<Self, PennelError> {
        let conn = Connection::open(path).map_err(db)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db)?;
        if version != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS passages; DROP TABLE IF EXISTS chunks;",
            )
            .map_err(db)?;
        }
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS files (
                meeting_id TEXT NOT NULL, name TEXT NOT NULL, mtime INTEGER NOT NULL, len INTEGER NOT NULL,
                source_revision TEXT NOT NULL,
                PRIMARY KEY (meeting_id, name));
             CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL, name TEXT NOT NULL, start_ms INTEGER, text TEXT NOT NULL,
                meeting_uid TEXT, utterance_ids TEXT NOT NULL, utterance_fingerprints TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS chunks_file ON chunks (meeting_id, name);
             CREATE VIRTUAL TABLE IF NOT EXISTS passages USING fts5(
                text, content = 'chunks', content_rowid = 'id', tokenize = 'unicode61 remove_diacritics 2');
             CREATE TRIGGER IF NOT EXISTS chunks_insert AFTER INSERT ON chunks BEGIN
                INSERT INTO passages (rowid, text) VALUES (new.id, new.text); END;
             CREATE TRIGGER IF NOT EXISTS chunks_delete AFTER DELETE ON chunks BEGIN
                INSERT INTO passages (passages, rowid, text) VALUES ('delete', old.id, old.text); END;
             PRAGMA user_version = {SCHEMA_VERSION};"
        ))
        .map_err(db)?;
        Ok(Self { conn })
    }

    pub fn sync(&mut self, store: &MeetingStore) -> Result<(), PennelError> {
        let meetings: Vec<String> = store.list().into_iter().map(|m| m.id).collect();
        let present: std::collections::HashSet<&String> = meetings.iter().collect();
        let tx = self.conn.transaction().map_err(db)?;
        let known: Vec<(String, String, i64, i64, String)> = {
            let mut stmt = tx
                .prepare("SELECT meeting_id, name, mtime, len, source_revision FROM files")
                .map_err(db)?;
            stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?
        };
        let known: HashMap<(String, String), (i64, i64, String)> = known
            .into_iter()
            .map(|(id, name, mtime, len, revision)| ((id, name), (mtime, len, revision)))
            .collect();

        for (id, name) in known.keys() {
            if !present.contains(id) || store.file_stamp(id, name).is_none() {
                tx.execute(
                    "DELETE FROM chunks WHERE meeting_id = ?1 AND name = ?2",
                    params![id, name],
                )
                .map_err(db)?;
                tx.execute(
                    "DELETE FROM files WHERE meeting_id = ?1 AND name = ?2",
                    params![id, name],
                )
                .map_err(db)?;
            }
        }
        let unchanged = |id: &String, name: &str| {
            store
                .file_stamp(id, name)
                .map(|(mtime, len)| (mtime, len as i64))
                == known
                    .get(&(id.clone(), name.to_owned()))
                    .map(|(mtime, len, _)| (*mtime, *len))
        };
        for id in &meetings {
            // Speaker names live in meeting.md, so renaming a voice re-labels the transcript passages too.
            let names_changed = !unchanged(id, "meeting.md");
            for (name, kind) in FILES {
                let Some((mtime, len)) = store.file_stamp(id, name) else {
                    continue;
                };
                let source_revision = if kind == PassageKind::Transcript {
                    transcript_source_stamp(store, id)?
                } else {
                    String::new()
                };
                if unchanged(id, name)
                    && !(kind == PassageKind::Transcript && names_changed)
                    && known
                        .get(&(id.clone(), name.to_owned()))
                        .is_some_and(|(_, _, revision)| revision == &source_revision)
                {
                    continue;
                }
                let snapshot = (kind == PassageKind::Transcript)
                    .then(|| store.read_transcript_snapshot(id))
                    .transpose()?;
                let passages = match kind {
                    PassageKind::Transcript => canonical_transcript_passages(
                        snapshot.as_ref().expect("transcript snapshot"),
                        &store.speaker_names(id),
                    ),
                    PassageKind::Notes => text_passages(&store.read_notes(id))
                        .into_iter()
                        .map(|(start, text)| IndexedPassage {
                            start,
                            text,
                            utterance_ids: Vec::new(),
                            utterance_fingerprints: Vec::new(),
                        })
                        .collect(),
                    PassageKind::Summary => {
                        text_passages(&store.read_meeting(id).unwrap_or_default())
                            .into_iter()
                            .map(|(start, text)| IndexedPassage {
                                start,
                                text,
                                utterance_ids: Vec::new(),
                                utterance_fingerprints: Vec::new(),
                            })
                            .collect()
                    }
                };
                let uid = snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.meeting_uid.clone())
                    .or_else(|| store.uid_of(id));
                tx.execute(
                    "DELETE FROM chunks WHERE meeting_id = ?1 AND name = ?2",
                    params![id, name],
                )
                .map_err(db)?;
                for passage in passages {
                    let ids = serde_json::to_string(&passage.utterance_ids)
                        .map_err(PennelError::storage)?;
                    let fingerprints = serde_json::to_string(&passage.utterance_fingerprints)
                        .map_err(PennelError::storage)?;
                    tx.execute(
                        "INSERT INTO chunks (meeting_id, name, start_ms, text, meeting_uid, utterance_ids, utterance_fingerprints) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![id, name, passage.start.map(|ms| ms as i64), passage.text, uid, ids, fingerprints],
                    )
                    .map_err(db)?;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO files (meeting_id, name, mtime, len, source_revision) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, name, mtime, len as i64, source_revision],
                )
                .map_err(db)?;
            }
        }
        tx.commit().map_err(db)
    }

    /// Passages containing every word of `query` (prefix match), best first.
    #[cfg(test)]
    pub fn search(
        &self,
        store: &MeetingStore,
        query: &str,
        meeting_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Passage>, PennelError> {
        self.matching(store, &fts_terms(query).join(" "), meeting_id, limit)
    }

    /// Passages containing any of `words`, best first.
    pub fn search_any(
        &self,
        store: &MeetingStore,
        words: &[String],
        meeting_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Passage>, PennelError> {
        let terms: Vec<String> = words.iter().flat_map(|w| fts_terms(w)).collect();
        self.matching(store, &terms.join(" OR "), meeting_id, limit)
    }

    fn matching(
        &self,
        store: &MeetingStore,
        expression: &str,
        meeting_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Passage>, PennelError> {
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.meeting_id, c.name, c.start_ms, c.text, snippet(passages, 0, '«', '»', '…', 12), c.meeting_uid, c.utterance_ids, c.utterance_fingerprints
                 FROM passages JOIN chunks c ON c.id = passages.rowid
                 WHERE passages MATCH ?1 AND (?2 IS NULL OR c.meeting_id = ?2)
                 ORDER BY bm25(passages) LIMIT ?3",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map(params![expression, meeting_id, limit], row)
            .map_err(db)?;
        resolve(store, rows.collect::<Result<_, _>>().map_err(db)?)
    }

    /// Every matching meeting, best first, with its `per_meeting` best passages.
    pub fn search_grouped(
        &self,
        store: &MeetingStore,
        query: &str,
        per_meeting: u32,
    ) -> Result<Vec<Passage>, PennelError> {
        let expression = fts_terms(query).join(" ");
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "WITH hits AS (
                    SELECT c.meeting_id, c.name, c.start_ms, c.text, c.meeting_uid, c.utterance_ids, c.utterance_fingerprints,
                           snippet(passages, 0, '«', '»', '…', 12) AS snippet, bm25(passages) AS rank
                    FROM passages JOIN chunks c ON c.id = passages.rowid WHERE passages MATCH ?1),
                 ranked AS (
                    SELECT *, ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY rank) AS position,
                           MIN(rank) OVER (PARTITION BY meeting_id) AS best
                    FROM hits)
                 SELECT meeting_id, name, start_ms, text, snippet, meeting_uid, utterance_ids, utterance_fingerprints FROM ranked
                 WHERE position <= ?2 ORDER BY best, meeting_id, position",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map(params![expression, per_meeting], row)
            .map_err(db)?;
        resolve(store, rows.collect::<Result<_, _>>().map_err(db)?)
    }

    /// Passages from the most recent meetings, newest first, for questions without searchable words.
    pub fn recent(
        &self,
        store: &MeetingStore,
        meeting_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Passage>, PennelError> {
        let order: Vec<String> = match meeting_id {
            Some(id) => vec![id.to_owned()],
            None => store.list().into_iter().map(|m| m.id).collect(),
        };
        let mut out = Vec::new();
        let mut stmt = self
            .conn
            .prepare("SELECT meeting_id, name, start_ms, text, '', meeting_uid, utterance_ids, utterance_fingerprints FROM chunks WHERE meeting_id = ?1 ORDER BY id")
            .map_err(db)?;
        for id in order {
            let rows: Vec<RawRow> = stmt
                .query_map(params![id], row)
                .map_err(db)?
                .collect::<Result<_, _>>()
                .map_err(db)?;
            out.extend(resolve(store, rows)?);
            if out.len() >= limit as usize {
                break;
            }
        }
        out.truncate(limit as usize);
        Ok(out)
    }
}

type RawRow = (
    String,
    String,
    Option<i64>,
    String,
    String,
    Option<String>,
    String,
    String,
);

fn row(r: &rusqlite::Row) -> rusqlite::Result<RawRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

fn resolve(store: &MeetingStore, rows: Vec<RawRow>) -> Result<Vec<Passage>, PennelError> {
    let mut infos: HashMap<String, Option<MeetingInfo>> = HashMap::new();
    let mut snapshots: HashMap<String, Option<crate::MeetingTranscriptSnapshot>> = HashMap::new();
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, name, start, text, snippet, meeting_uid, ids, fingerprints)| {
                let info = infos
                    .entry(id.clone())
                    .or_insert_with(|| store.info(&id))
                    .clone()?;
                let utterance_ids: Vec<String> = serde_json::from_str(&ids).ok()?;
                let utterance_fingerprints: Vec<String> =
                    serde_json::from_str(&fingerprints).ok()?;
                let kind = PassageKind::from_file(&name)?;
                if kind != PassageKind::Transcript
                    && meeting_uid.is_some()
                    && meeting_uid != store.uid_of(&id)
                {
                    return None;
                }
                if !utterance_ids.is_empty() {
                    let snapshot = snapshots
                        .entry(id.clone())
                        .or_insert_with(|| store.read_transcript_snapshot(&id).ok())
                        .as_ref()?;
                    if kind != PassageKind::Transcript
                        || meeting_uid.as_deref() != Some(snapshot.meeting_uid.as_str())
                        || utterance_ids.len() != utterance_fingerprints.len()
                        || utterance_ids
                            .iter()
                            .collect::<std::collections::HashSet<_>>()
                            .len()
                            != utterance_ids.len()
                        || !utterance_ids.iter().zip(&utterance_fingerprints).all(
                            |(id, fingerprint)| {
                                let mut matching = snapshot
                                    .utterances
                                    .iter()
                                    .filter(|row| row.metadata.utterance_id == *id);
                                matching.next().is_some_and(|row| {
                                    crate::utterance_evidence_revision(row) == *fingerprint
                                }) && matching.next().is_none()
                            },
                        )
                    {
                        return None;
                    }
                } else if !utterance_fingerprints.is_empty() {
                    return None;
                }
                Some(Passage {
                    meeting_id: id,
                    title: info.title,
                    start_iso: info.start_iso,
                    kind,
                    start_ms: start.map(|ms| ms as u64),
                    snippet: if snippet.is_empty() {
                        text.chars().take(160).collect()
                    } else {
                        snippet
                    },
                    text,
                    meeting_uid,
                    utterance_ids,
                    utterance_fingerprints,
                })
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::Speaker;
    use chrono::TimeZone;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        index: MeetingIndex,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(&dir.path().join("meetings"));
        std::fs::create_dir_all(dir.path().join("meetings")).unwrap();
        let index = MeetingIndex::open(&dir.path().join("index.sqlite")).unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
        }
    }

    fn segment(start_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 1000,
            speaker: Speaker::Others,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    fn meeting(store: &MeetingStore, title: &str, hour: u32) -> String {
        store
            .create(
                title,
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 20, hour, 0, 0)
                    .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn finds_word_in_transcript_with_snippet() {
        let mut f = fixture();
        let id = meeting(&f.store, "Sync", 9);
        f.store
            .write_transcript(
                &id,
                &[segment(65_000, "We kiezen voor Parakeet als standaard")],
            )
            .unwrap();
        f.index.sync(&f.store).unwrap();
        let hits = f.index.search(&f.store, "parakeet", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, PassageKind::Transcript);
        assert_eq!(hits[0].start_ms, Some(65_000));
        assert_eq!(hits[0].title, "Sync");
        assert!(
            hits[0].snippet.contains("«Parakeet»"),
            "{}",
            hits[0].snippet
        );
    }

    #[test]
    fn canonical_index_bindings_survive_label_changes_and_reject_changed_sources() {
        let mut f = fixture();
        let id = meeting(&f.store, "Owned bindings", 9);
        let mut row = crate::MeetingUtterance::new(
            segment(1_250, "The deliverable is ready."),
            Some("microphone".into()),
        );
        row.segment.voice = Some(1);
        row.metadata.attribution = crate::AutomaticAttribution {
            kind: crate::AttributionKind::Single,
            voices: vec![1],
            turns: vec![],
            self_voice: false,
            provisional: false,
        };
        let before = f
            .store
            .write_transcript_snapshot(&id, None, std::slice::from_ref(&row))
            .unwrap();
        f.index.sync(&f.store).unwrap();
        let first = f
            .index
            .search(&f.store, "deliverable", None, 10)
            .unwrap()
            .remove(0);
        assert_eq!(
            first.meeting_uid.as_deref(),
            Some(before.meeting_uid.as_str())
        );
        assert_eq!(first.utterance_ids, vec![row.metadata.utterance_id.clone()]);
        assert_eq!(
            first.utterance_fingerprints,
            vec![crate::utterance_evidence_revision(&row)]
        );
        row.segment.voice = Some(2);
        row.metadata.attribution.voices = vec![2];
        row.metadata.attribution_revision += 1;
        f.store
            .write_transcript_snapshot(&id, Some(before.revision), &[row])
            .unwrap();
        f.index.sync(&f.store).unwrap();
        let relabeled = f
            .index
            .search(&f.store, "deliverable", None, 10)
            .unwrap()
            .remove(0);
        assert_eq!(first.utterance_ids, relabeled.utterance_ids);
        assert_eq!(
            first.utterance_fingerprints,
            relabeled.utterance_fingerprints
        );
        assert_ne!(first.text, relabeled.text);
        std::fs::write(
            f.store.dir_of(&id).join("transcript.md"),
            "[00:01.250] **Anderen 2:** A changed source.\n",
        )
        .unwrap();
        assert!(
            f.index
                .search(&f.store, "deliverable", None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn indexed_passages_keep_every_distinct_utterance_and_legacy_json_stays_readable() {
        let mut f = fixture();
        let id = meeting(&f.store, "Owned turns", 9);
        let rows = ["😀 Één ja.", "Ja.", "Nee."]
            .into_iter()
            .map(|text| {
                crate::MeetingUtterance::new(segment(100, text), Some("system_audio".into()))
            })
            .collect::<Vec<_>>();
        f.store.write_transcript_snapshot(&id, None, &rows).unwrap();
        f.index.sync(&f.store).unwrap();
        let passages = f.index.recent(&f.store, Some(&id), u32::MAX).unwrap();
        let transcript = passages
            .iter()
            .filter(|passage| passage.kind == PassageKind::Transcript)
            .collect::<Vec<_>>();
        let ids = transcript
            .iter()
            .flat_map(|passage| passage.utterance_ids.iter())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            ids,
            rows.iter().map(|row| &row.metadata.utterance_id).collect()
        );
        let legacy: Passage = serde_json::from_value(serde_json::json!({"meeting_id":"legacy", "title":"M", "start_iso":"", "kind":"transcript", "start_ms":0, "text":"Old source", "snippet":""})).unwrap();
        assert!(legacy.meeting_uid.is_none());
        assert!(legacy.utterance_ids.is_empty());
        assert!(
            serde_json::to_value(legacy)
                .unwrap()
                .get("utterance_ids")
                .is_none()
        );
    }

    #[test]
    fn cached_passages_never_retarget_a_new_meeting_reusing_the_same_folder() {
        let mut f = fixture();
        let id = meeting(&f.store, "Reusable title", 9);
        f.store
            .write_notes(&id, "A former meeting secret.")
            .unwrap();
        f.index.sync(&f.store).unwrap();
        f.store.delete(&id).unwrap();
        let replacement = meeting(&f.store, "Reusable title", 9);
        assert_eq!(replacement, id);
        f.store
            .write_notes(&replacement, "Different current notes.")
            .unwrap();
        assert!(
            f.index
                .search(&f.store, "former", None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn partial_metadata_sync_reindexes_when_the_missing_part_arrives() {
        let mut f = fixture();
        let id = meeting(&f.store, "Owned partial sync", 9);
        let row = crate::MeetingUtterance::new(
            segment(500, "A portable commitment."),
            Some("microphone".into()),
        );
        f.store
            .write_transcript_snapshot(&id, None, std::slice::from_ref(&row))
            .unwrap();
        f.index.sync(&f.store).unwrap();
        let original = f
            .index
            .search(&f.store, "portable", None, 10)
            .unwrap()
            .remove(0);
        let part = std::fs::read_dir(f.store.dir_of(&id))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("transcript.meta.")
                    && path.file_name().unwrap() != "transcript.meta.json"
            })
            .unwrap();
        let bytes = std::fs::read(&part).unwrap();
        std::fs::remove_file(&part).unwrap();
        f.index.sync(&f.store).unwrap();
        let pending = f
            .index
            .search(&f.store, "portable", None, 10)
            .unwrap()
            .remove(0);
        assert_ne!(pending.utterance_ids, original.utterance_ids);
        assert!(pending.text.starts_with("Unknown speaker:"));
        std::fs::write(&part, bytes).unwrap();
        f.index.sync(&f.store).unwrap();
        let restored = f
            .index
            .search(&f.store, "portable", None, 10)
            .unwrap()
            .remove(0);
        assert_eq!(restored.utterance_ids, original.utterance_ids);
        assert_eq!(
            restored.utterance_fingerprints,
            original.utterance_fingerprints
        );
    }

    #[test]
    fn diacritics_insensitive_and_prefix() {
        let mut f = fixture();
        let id = meeting(&f.store, "Plan", 9);
        f.store
            .write_notes(&id, "één actiepunt\n\nbudgetten herzien")
            .unwrap();
        f.index.sync(&f.store).unwrap();
        assert_eq!(f.index.search(&f.store, "een", None, 10).unwrap().len(), 1);
        assert_eq!(
            f.index.search(&f.store, "budget", None, 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn query_with_fts_syntax_is_literal() {
        let mut f = fixture();
        meeting(&f.store, "Plan", 9);
        f.index.sync(&f.store).unwrap();
        assert!(
            f.index
                .search(&f.store, "\"budget\" -x (a:b* NEAR", None, 10)
                .is_ok()
        );
        assert!(
            f.index
                .search(&f.store, "  ?! ", None, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            f.index
                .search_any(&f.store, &["or".into(), "and\"".into()], None, 10)
                .is_ok()
        );
    }

    #[test]
    fn sync_drops_removed_meetings() {
        let mut f = fixture();
        let a = meeting(&f.store, "A", 9);
        let b = meeting(&f.store, "B", 10);
        f.store.write_notes(&a, "kiwi").unwrap();
        f.store.write_notes(&b, "kiwi").unwrap();
        f.index.sync(&f.store).unwrap();
        assert_eq!(f.index.search(&f.store, "kiwi", None, 10).unwrap().len(), 2);
        f.store.delete(&b).unwrap();
        f.index.sync(&f.store).unwrap();
        let hits = f.index.search(&f.store, "kiwi", None, 10).unwrap();
        assert_eq!(
            hits.iter()
                .map(|h| h.meeting_id.as_str())
                .collect::<Vec<_>>(),
            [a.as_str()]
        );
    }

    #[test]
    fn sync_reindexes_changed_file_once() {
        let mut f = fixture();
        let id = meeting(&f.store, "A", 9);
        f.store
            .write_transcript(&id, &[segment(0, "eerste versie")])
            .unwrap();
        f.index.sync(&f.store).unwrap();
        f.store
            .write_transcript(&id, &[segment(0, "tweede versie met meer tekst")])
            .unwrap();
        f.index.sync(&f.store).unwrap();
        f.index.sync(&f.store).unwrap();
        assert!(
            f.index
                .search(&f.store, "eerste", None, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            f.index.search(&f.store, "tweede", None, 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn scoped_search() {
        let mut f = fixture();
        let a = meeting(&f.store, "A", 9);
        let b = meeting(&f.store, "B", 10);
        f.store.write_notes(&a, "mango").unwrap();
        f.store.write_notes(&b, "mango").unwrap();
        f.index.sync(&f.store).unwrap();
        let hits = f.index.search(&f.store, "mango", Some(&a), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, a);
    }

    #[test]
    fn transcript_passages_capped() {
        let segments: Vec<Segment> = (0..50)
            .map(|i| segment(i * 1000, &format!("zin nummer {i:02} {}", "x".repeat(20))))
            .collect();
        let passages = transcript_passages(&segments, &[]);
        assert!(passages.len() > 1);
        assert!(passages.iter().all(|(_, text)| text.len() <= PASSAGE_CHARS));
        let joined: String = passages
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.find("nummer 00").unwrap() < joined.find("nummer 49").unwrap());
        assert_eq!(passages[0].0, Some(0));
    }

    #[test]
    fn long_single_paragraph_is_fully_indexed() {
        let mut f = fixture();
        let id = meeting(&f.store, "Lang", 9);
        let note = format!("{} slotwoord", "bla ".repeat(500));
        f.store
            .write_notes(&id, &format!("- eerste\n- tweede\n{note}"))
            .unwrap();
        f.index.sync(&f.store).unwrap();
        assert_eq!(
            f.index
                .search(&f.store, "slotwoord", None, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(
            split_long(&"x".repeat(1500))
                .iter()
                .all(|p| p.len() <= PASSAGE_CHARS)
        );
        assert_eq!(split_long(&"é".repeat(400)).concat(), "é".repeat(400));
    }

    #[test]
    fn grouped_search_keeps_every_meeting() {
        let mut f = fixture();
        let busy = meeting(&f.store, "Druk", 9);
        let quiet = meeting(&f.store, "Stil", 10);
        let lines: Vec<String> = (0..80).map(|i| format!("budget punt {i}\n")).collect();
        f.store.write_notes(&busy, &lines.join("\n")).unwrap();
        f.store.write_notes(&quiet, "ook het budget").unwrap();
        f.index.sync(&f.store).unwrap();
        let hits = f.index.search_grouped(&f.store, "budget", 2).unwrap();
        let meetings: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.meeting_id.as_str()).collect();
        assert_eq!(
            meetings,
            [busy.as_str(), quiet.as_str()].into_iter().collect()
        );
        assert!(hits.iter().filter(|h| h.meeting_id == busy).count() <= 2);
    }

    #[test]
    fn old_schema_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE passages USING fts5(meeting_id UNINDEXED, name UNINDEXED, start_ms UNINDEXED, text);").unwrap();
        drop(conn);
        let store = MeetingStore::new(&dir.path().join("m"));
        std::fs::create_dir_all(dir.path().join("m")).unwrap();
        let id = store.create("A", chrono::Local::now()).unwrap();
        store.write_notes(&id, "papaya").unwrap();
        let mut index = MeetingIndex::open(&path).unwrap();
        index.sync(&store).unwrap();
        assert_eq!(index.search(&store, "papaya", None, 5).unwrap().len(), 1);
    }

    #[test]
    fn renamed_speaker_is_searchable() {
        let mut f = fixture();
        let id = meeting(&f.store, "Sync", 9);
        let mut said = segment(0, "de offerte is klaar");
        said.voice = Some(1);
        f.store.write_transcript(&id, &[said]).unwrap();
        f.index.sync(&f.store).unwrap();
        f.store.rename_speaker(&id, 1, "Annelies").unwrap();
        f.index.sync(&f.store).unwrap();
        let hits = f.index.search(&f.store, "annelies", None, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, PassageKind::Transcript);
    }

    #[test]
    fn recent_returns_newest_meeting_first() {
        let mut f = fixture();
        let old = meeting(&f.store, "Oud", 8);
        let new = meeting(&f.store, "Nieuw", 11);
        f.store.write_notes(&old, "oude notitie").unwrap();
        f.store.write_notes(&new, "nieuwe notitie").unwrap();
        f.index.sync(&f.store).unwrap();
        let recent = f.index.recent(&f.store, None, 1).unwrap();
        assert_eq!(recent[0].meeting_id, new);
    }
}
