use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::meetings::{
    MeetingStore, durable_write, durable_write_checked, replacement_gate, timestamp,
};
use crate::{ApiKeys, InterfaceLanguage, MeetingExport, PennelEngine, PennelError};

const HISTORY_PREFIX: &str = "pennel.summary.history.";
const ATTEMPT_PREFIX: &str = "pennel.summary.automatic.";
const SHAREABLE_FILE: &str = "pennel.summary.shareable.json";
const TRANSCRIPT_PROVENANCE: &str = "finalized_transcript_only_v1";
const FORMAT: &str = "pennel_summary_format";
const SOURCE: &str = "pennel_summary_source_revision";
const EDITED: &str = "pennel_summary_edited";
const BODY_REVISION: &str = "pennel_summary_body_revision";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MeetingOutputScope {
    Personal,
    Shareable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingSummaryDocument {
    pub meeting_uid: String,
    pub markdown: String,
    pub revision: String,
    pub source_revision: String,
    pub edited: bool,
    pub outdated: bool,
}

/// A generated draft plus the exact document/source versions its acceptance must compare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingSummaryCandidate {
    pub meeting_uid: String,
    pub markdown: String,
    pub revision: String,
    pub source_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingSummaryRevision {
    pub revision: String,
    pub markdown: String,
    pub source_revision: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ShareableMeetingSummary {
    pub meeting_uid: String,
    pub markdown: String,
    pub source_revision: String,
    pub generated_at: String,
    pub provider_id: String,
    pub model: String,
    pub provenance: String,
    pub outdated: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedRevision {
    schema_version: u32,
    meeting_uid: String,
    entry: MeetingSummaryRevision,
}

#[derive(Serialize, Deserialize)]
struct SavedShareable {
    schema_version: u32,
    markdown_revision: String,
    document: ShareableMeetingSummary,
}

struct SummaryInference<'a> {
    engine: &'a PennelEngine,
    cancellation: Arc<crate::InferenceCancellation>,
}

impl Drop for SummaryInference<'_> {
    fn drop(&mut self) {
        self.engine.finish_inference(&self.cancellation);
    }
}

fn changed() -> PennelError {
    PennelError::storage(
        "The summary or its source changed. Your saved text was kept. Reload before saving.",
    )
}

fn split_document(raw: &str) -> Result<(&str, &str, &str), PennelError> {
    let newline = if raw.starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let rest = raw
        .strip_prefix("---")
        .and_then(|rest| rest.strip_prefix(newline))
        .ok_or_else(|| PennelError::storage("The saved summary has invalid meeting metadata."))?;
    let (head, body) = rest
        .split_once(&format!("{newline}---{newline}"))
        .ok_or_else(|| PennelError::storage("The saved summary has invalid meeting metadata."))?;
    Ok((head, body, newline))
}

fn field<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == key).then(|| value.trim())
    })
}

fn regular_bytes(path: &Path) -> Result<Vec<u8>, PennelError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::read(path).map_err(PennelError::storage)
        }
        _ => Err(PennelError::storage(
            "A saved meeting file is not a regular local file.",
        )),
    }
}

fn checked_directory(store: &MeetingStore, id: &str) -> Result<PathBuf, PennelError> {
    let mut components = Path::new(id).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || id.contains('\\')
    {
        return Err(PennelError::NotFound);
    }
    let root = store.root().canonicalize().map_err(PennelError::storage)?;
    let metadata = fs::symlink_metadata(store.dir_of(id)).map_err(|_| PennelError::NotFound)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(PennelError::NotFound);
    }
    let directory = store
        .dir_of(id)
        .canonicalize()
        .map_err(|_| PennelError::NotFound)?;
    if directory.parent() != Some(root.as_path()) {
        return Err(PennelError::NotFound);
    }
    let document =
        fs::symlink_metadata(directory.join("meeting.md")).map_err(|_| PennelError::NotFound)?;
    if !document.is_file() || document.file_type().is_symlink() {
        return Err(PennelError::NotFound);
    }
    Ok(directory)
}

struct SourceFiles {
    raw: String,
    uid: String,
    notes: String,
    owner_revision: String,
    transcript_revision: String,
}

#[derive(Clone, Copy)]
enum SummarySave<'a> {
    Edit,
    Restore(&'a str),
    Accept,
    Generate(&'a str),
}

/// Called under the existing per-meeting storage gate; fingerprints also include external edits.
fn source_files(directory: &Path) -> Result<SourceFiles, PennelError> {
    let raw = String::from_utf8(regular_bytes(&directory.join("meeting.md"))?)
        .map_err(PennelError::storage)?;
    let (head, _, _) = split_document(&raw)?;
    let uid = field(head, "pennel_id")
        .filter(|uid| crate::meetings::valid_uid(uid))
        .ok_or(PennelError::NotFound)?
        .to_owned();
    let notes = String::from_utf8(regular_bytes(&directory.join("notes.md"))?)
        .map_err(PennelError::storage)?;
    let transcript = regular_bytes(&directory.join("transcript.md"))?;
    let manifest = regular_bytes(&directory.join(crate::meeting_utterances::MANIFEST))?;
    let mut parts = Vec::new();
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&manifest)
        && let Some(descriptors) = value.get("parts").and_then(|parts| parts.as_array())
    {
        if descriptors.len() > 4096 {
            return Err(PennelError::storage(
                "The transcript contains too many metadata parts.",
            ));
        }
        for part in descriptors {
            let hash = part
                .get("sha256")
                .and_then(|hash| hash.as_str())
                .filter(|hash| valid_revision(hash))
                .ok_or_else(|| PennelError::storage("The transcript metadata is invalid."))?;
            parts.push(regular_bytes(
                &directory.join(format!("transcript.meta.{hash}.json")),
            )?);
        }
    }
    let transcript_revision = crate::evidence_revision(
        &serde_json::to_string(&(&uid, transcript, manifest, parts))
            .map_err(PennelError::storage)?,
    );
    let names = field(head, "speakers").unwrap_or("");
    let owner_revision = crate::evidence_revision(
        &serde_json::to_string(&(&transcript_revision, &notes, names))
            .map_err(PennelError::storage)?,
    );
    Ok(SourceFiles {
        raw,
        uid,
        notes,
        owner_revision,
        transcript_revision,
    })
}

fn valid_revision(revision: &str) -> bool {
    revision.len() == 64
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn document(files: &SourceFiles) -> Result<MeetingSummaryDocument, PennelError> {
    let (head, body, _) = split_document(&files.raw)?;
    let markdown = if field(head, FORMAT) == Some("exact_v1") {
        body
    } else {
        body.trim()
    }
    .to_owned();
    let saved_source = field(head, SOURCE);
    Ok(MeetingSummaryDocument {
        meeting_uid: files.uid.clone(),
        revision: crate::evidence_revision(&files.raw),
        source_revision: files.owner_revision.clone(),
        edited: field(head, EDITED) == Some("true")
            || (!markdown.trim().is_empty()
                && (field(head, EDITED) != Some("false")
                    || field(head, BODY_REVISION)
                        != Some(crate::evidence_revision(&markdown).as_str()))),
        outdated: !markdown.trim().is_empty()
            && saved_source != Some(files.owner_revision.as_str()),
        markdown,
    })
}

fn sync_directory(directory: &Path) -> Result<(), PennelError> {
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(PennelError::storage)?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

/// Stage and fsync before the last comparison, so a failed save never truncates the previous file.
fn compare_write(
    path: &Path,
    expected: &[u8],
    bytes: &[u8],
    verify_source: impl FnOnce() -> Result<(), PennelError>,
) -> Result<(), PennelError> {
    durable_write_checked(path, bytes, || {
        verify_source()?;
        if regular_bytes(path)? != expected {
            return Err(changed());
        }
        Ok(())
    })
}

impl MeetingStore {
    fn summary_files(&self, id: &str) -> Result<SourceFiles, PennelError> {
        let directory = checked_directory(self, id)?;
        let gate = replacement_gate(&directory);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        source_files(&directory)
    }

    fn read_owner_summary(&self, id: &str) -> Result<MeetingSummaryDocument, PennelError> {
        document(&self.summary_files(id)?)
    }

    fn save_owner_summary(
        &self,
        id: &str,
        expected_revision: &str,
        expected_source: &str,
        markdown: &str,
        mode: SummarySave<'_>,
    ) -> Result<MeetingSummaryDocument, PennelError> {
        let directory = checked_directory(self, id)?;
        let gate = replacement_gate(&directory);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let before = source_files(&directory)?;
        let current = document(&before)?;
        if current.revision != expected_revision || current.source_revision != expected_source {
            return Err(changed());
        }
        let (head, _, newline) = split_document(&before.raw)?;
        let previous_source = field(head, SOURCE).unwrap_or_else(|| {
            if current.markdown.trim().is_empty() {
                &before.owner_revision
            } else {
                "unknown"
            }
        });
        let summary_source = match mode {
            SummarySave::Edit => previous_source,
            SummarySave::Restore(source) => source,
            SummarySave::Accept | SummarySave::Generate(_) => &before.owner_revision,
        };
        let edited = !matches!(mode, SummarySave::Generate(_));
        let template = match mode {
            SummarySave::Generate(template) => Some(template),
            _ => None,
        };
        let mut lines: Vec<String> = head
            .lines()
            .filter(|line| {
                let key = line.split(':').next().unwrap_or("").trim();
                !matches!(
                    key,
                    FORMAT
                        | SOURCE
                        | EDITED
                        | BODY_REVISION
                        | "pennel_summary_revision"
                        | "pennel_summary_outdated"
                ) && !(template.is_some() && key == "template")
            })
            .map(str::to_owned)
            .collect();
        if let Some(template) = template {
            lines.push(format!("template: {}", template.replace(['\n', '\r'], " ")));
        }
        lines.extend([
            format!("{FORMAT}: exact_v1"),
            format!("{SOURCE}: {summary_source}"),
            format!("{EDITED}: {edited}"),
            format!("{BODY_REVISION}: {}", crate::evidence_revision(markdown)),
            format!("pennel_summary_revision: {}", uuid::Uuid::now_v7()),
        ]);
        let next = format!(
            "---{newline}{}{newline}---{newline}{markdown}",
            lines.join(newline)
        );
        let saved = SavedRevision {
            schema_version: 1,
            meeting_uid: before.uid.clone(),
            entry: MeetingSummaryRevision {
                revision: current.revision.clone(),
                markdown: current.markdown,
                source_revision: previous_source.to_owned(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        };
        let history = directory.join(format!("{HISTORY_PREFIX}{}.json", saved.entry.revision));
        if history.exists() {
            let previous: SavedRevision =
                serde_json::from_slice(&regular_bytes(&history)?).map_err(PennelError::storage)?;
            if previous.schema_version != 1
                || previous.meeting_uid != saved.meeting_uid
                || previous.entry.revision != saved.entry.revision
                || previous.entry.markdown != saved.entry.markdown
            {
                return Err(PennelError::storage(
                    "The prior summary could not be preserved. Your saved text was kept.",
                ));
            }
        } else {
            durable_write(
                &history,
                &serde_json::to_vec(&saved).map_err(PennelError::storage)?,
            )?;
        }
        // A note/transcript edited by another application while history was synced is also a conflict.
        compare_write(
            &directory.join("meeting.md"),
            before.raw.as_bytes(),
            next.as_bytes(),
            || {
                let verified = source_files(&directory)?;
                if verified.owner_revision != before.owner_revision || verified.raw != before.raw {
                    return Err(changed());
                }
                Ok(())
            },
        )?;
        document(&source_files(&directory)?)
    }

    fn owner_summary_history(&self, id: &str) -> Result<Vec<MeetingSummaryRevision>, PennelError> {
        let directory = checked_directory(self, id)?;
        let uid = self.summary_files(id)?.uid;
        let mut history = Vec::new();
        for entry in fs::read_dir(&directory).map_err(PennelError::storage)? {
            let entry = entry.map_err(PennelError::storage)?;
            let name = entry.file_name();
            let Some(revision) = name
                .to_str()
                .and_then(|name| name.strip_prefix(HISTORY_PREFIX))
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if !valid_revision(revision) {
                continue;
            }
            let saved: SavedRevision = serde_json::from_slice(&regular_bytes(&entry.path())?)
                .map_err(PennelError::storage)?;
            if saved.schema_version != 1
                || saved.meeting_uid != uid
                || saved.entry.revision != revision
            {
                return Err(PennelError::storage(
                    "A prior summary revision could not be verified.",
                ));
            }
            history.push(saved.entry);
        }
        history.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(b.revision.cmp(&a.revision))
        });
        Ok(history)
    }

    fn admit_automatic_summary(
        &self,
        id: &str,
        expected: &MeetingSummaryDocument,
    ) -> Result<bool, PennelError> {
        let directory = checked_directory(self, id)?;
        let gate = replacement_gate(&directory);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let files = source_files(&directory)?;
        let current = document(&files)?;
        if current.revision != expected.revision
            || current.source_revision != expected.source_revision
        {
            return Err(changed());
        }
        if current.edited || (!current.markdown.trim().is_empty() && !current.outdated) {
            return Ok(false);
        }
        let marker = directory.join(format!("{ATTEMPT_PREFIX}{}.json", files.owner_revision));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&marker) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            result => result.map_err(PennelError::storage)?,
        };
        // A partial marker still consumes the attempt: restart/failure must never imply consent to retry.
        let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"meeting_uid":files.uid,"source_revision":files.owner_revision,"admitted_at":chrono::Utc::now().to_rfc3339()})).map_err(PennelError::storage)?;
        file.write_all(&bytes).map_err(PennelError::storage)?;
        file.sync_all().map_err(PennelError::storage)?;
        sync_directory(&directory)?;
        Ok(true)
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn meeting_summary(&self, id: String) -> Result<MeetingSummaryDocument, PennelError> {
        self.claim_library()?;
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        store.read_owner_summary(&id)
    }

    pub fn save_meeting_summary(
        &self,
        id: String,
        expected_revision: String,
        expected_source_revision: String,
        markdown: String,
    ) -> Result<MeetingSummaryDocument, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        self.meeting_store().save_owner_summary(
            &id,
            &expected_revision,
            &expected_source_revision,
            &markdown,
            SummarySave::Edit,
        )
    }

    pub fn meeting_summary_history(
        &self,
        id: String,
    ) -> Result<Vec<MeetingSummaryRevision>, PennelError> {
        self.claim_library()?;
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        store.owner_summary_history(&id)
    }

    /// Explicit acceptance after reviewing a generated candidate establishes its current source binding.
    pub fn accept_meeting_summary(
        &self,
        id: String,
        expected_revision: String,
        expected_source_revision: String,
        markdown: String,
    ) -> Result<MeetingSummaryDocument, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        self.meeting_store().save_owner_summary(
            &id,
            &expected_revision,
            &expected_source_revision,
            &markdown,
            SummarySave::Accept,
        )
    }

    pub fn restore_meeting_summary(
        &self,
        id: String,
        revision: String,
        expected_revision: String,
        expected_source_revision: String,
    ) -> Result<MeetingSummaryDocument, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        let entry = store
            .owner_summary_history(&id)?
            .into_iter()
            .find(|entry| entry.revision == revision)
            .ok_or(PennelError::NotFound)?;
        store.save_owner_summary(
            &id,
            &expected_revision,
            &expected_source_revision,
            &entry.markdown,
            SummarySave::Restore(&entry.source_revision),
        )
    }

    pub fn generate_meeting_summary(
        &self,
        id: String,
        template_id: String,
        keys: ApiKeys,
    ) -> Result<MeetingSummaryCandidate, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        self.prepare_owner_summary(&id, &template_id, &keys)
    }

    pub fn summarize_meeting_automatically(
        &self,
        id: String,
        template_id: String,
        keys: ApiKeys,
    ) -> Result<Option<String>, PennelError> {
        if self.settings().summary_automation != crate::SummaryAutomation::Automatic {
            return Ok(None);
        }
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let cancellation = crate::InferenceCancellation::new();
        self.register_inference(&cancellation);
        let _registration = SummaryInference {
            engine: self,
            cancellation: cancellation.clone(),
        };
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        self.require_finalized_summary_source(&store, &id)?;
        let before = store.read_owner_summary(&id)?;
        if !store.admit_automatic_summary(&id, &before)? {
            return Ok(None);
        }
        let candidate = self.prepare_owner_summary_at(
            &id,
            &template_id,
            &keys,
            Some(&before),
            Some(&cancellation),
        )?;
        // The durable admission is bound to its original versions, including edits before provider setup.
        if candidate.revision != before.revision
            || candidate.source_revision != before.source_revision
        {
            return Err(changed());
        }
        let _consent = self.automatic_summary_policy_guard(&cancellation)?;
        let saved = store.save_owner_summary(
            &id,
            &candidate.revision,
            &candidate.source_revision,
            &candidate.markdown,
            SummarySave::Generate(&template_id),
        )?;
        Ok(Some(saved.markdown))
    }

    pub fn generate_shareable_meeting_summary(
        &self,
        id: String,
        template_id: String,
        keys: ApiKeys,
    ) -> Result<ShareableMeetingSummary, PennelError> {
        let _ = template_id;
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        self.require_finalized_summary_source(&store, &id)?;
        let before = store.summary_files(&id)?;
        let shareable_path = checked_directory(&store, &id)?.join(SHAREABLE_FILE);
        let previous_output = regular_bytes(&shareable_path)?;
        let transcript = permitted_transcript(&store, &id)?;
        let after = store.summary_files(&id)?;
        if before.transcript_revision != after.transcript_revision || before.uid != after.uid {
            return Err(changed());
        }
        if transcript.trim().is_empty() {
            return Err(PennelError::NoSpeech);
        }
        // Personal template instructions, titles, identity aliases and notes are not shareable sources.
        let spec = crate::templates::TemplateSpec::parse("General", "");
        let settings = self.settings();
        let (account, model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let body =
            self.generate_summary_body(&spec, "", &transcript, (&account, &model), &keys, None)?;
        let document = ShareableMeetingSummary {
            meeting_uid: before.uid,
            markdown: body,
            source_revision: before.transcript_revision,
            generated_at: chrono::Utc::now().to_rfc3339(),
            provider_id: account.id,
            model,
            provenance: TRANSCRIPT_PROVENANCE.into(),
            outdated: false,
        };
        let directory = checked_directory(&store, &id)?;
        let gate = replacement_gate(&directory);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        compare_write(
            &shareable_path,
            &previous_output,
            &serde_json::to_vec(&SavedShareable {
                schema_version: 1,
                markdown_revision: crate::evidence_revision(&document.markdown),
                document: document.clone(),
            })
            .map_err(PennelError::storage)?,
            || {
                let current = source_files(&directory)?;
                if current.uid != document.meeting_uid
                    || current.transcript_revision != document.source_revision
                {
                    return Err(changed());
                }
                Ok(())
            },
        )?;
        Ok(document)
    }

    pub fn shareable_meeting_summary(
        &self,
        id: String,
    ) -> Result<Option<ShareableMeetingSummary>, PennelError> {
        self.claim_library()?;
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        read_shareable(&store, &id)
    }

    /// Returns the exact bytes that callers must preview before handing this output to copy/export.
    pub fn preview_meeting_export(
        &self,
        id: String,
        scope: MeetingOutputScope,
        include_transcript: bool,
        language: InterfaceLanguage,
    ) -> Result<MeetingExport, PennelError> {
        self.claim_library()?;
        let store = self.meeting_store();
        checked_directory(&store, &id)?;
        store.ensure_uid(&id)?;
        if scope == MeetingOutputScope::Personal {
            let before = store.summary_files(&id)?;
            let output = self.export_meeting_localized(id.clone(), include_transcript, language)?;
            let after = store.summary_files(&id)?;
            if before.raw != after.raw || before.owner_revision != after.owner_revision {
                return Err(changed());
            }
            return Ok(output);
        }
        let _edit = self.edit_meeting(&id)?;
        self.require_finalized_summary_source(&store, &id)?;
        let summary = read_shareable(&store, &id)?.filter(|summary| !summary.outdated).ok_or_else(|| PennelError::storage("Generate a current transcript-only summary before sharing. Personal notes were kept private."))?;
        let title = match language {
            InterfaceLanguage::Dutch => "Samenvatting",
            InterfaceLanguage::English => "Summary",
        }
        .to_owned();
        let mut markdown = summary.markdown;
        if include_transcript {
            let transcript = permitted_transcript_text(&store, &id, Some(language))?;
            if transcript.trim().is_empty() {
                return Err(PennelError::NoSpeech);
            }
            let heading = crate::ui_text(language, "meetings.transcript", &[]);
            markdown.push_str(&format!("\n\n## {heading}\n\n{transcript}\n"));
        }
        if store.summary_files(&id)?.transcript_revision != summary.source_revision {
            return Err(changed());
        }
        Ok(MeetingExport {
            title,
            html: crate::export::markdown_to_html(&markdown),
            markdown,
        })
    }
}

impl PennelEngine {
    pub(crate) fn prepare_owner_summary(
        &self,
        id: &str,
        template_id: &str,
        keys: &ApiKeys,
    ) -> Result<MeetingSummaryCandidate, PennelError> {
        self.prepare_owner_summary_at(id, template_id, keys, None, None)
    }

    fn prepare_owner_summary_at(
        &self,
        id: &str,
        template_id: &str,
        keys: &ApiKeys,
        expected: Option<&MeetingSummaryDocument>,
        automatic_cancellation: Option<&Arc<crate::InferenceCancellation>>,
    ) -> Result<MeetingSummaryCandidate, PennelError> {
        let store = self.meeting_store();
        checked_directory(&store, id)?;
        store.ensure_uid(id)?;
        let before = store.summary_files(id)?;
        if expected.is_some_and(|expected| {
            expected.meeting_uid != before.uid
                || expected.revision != crate::evidence_revision(&before.raw)
                || expected.source_revision != before.owner_revision
        }) {
            return Err(changed());
        }
        let snapshot = store.read_transcript_snapshot(id)?;
        let transcript =
            crate::summary::transcript_snapshot_text(&snapshot, &store.speaker_names(id));
        let after = store.summary_files(id)?;
        if before.owner_revision != after.owner_revision || before.raw != after.raw {
            return Err(changed());
        }
        if transcript.trim().is_empty() && before.notes.trim().is_empty() {
            return Err(PennelError::NoSpeech);
        }
        let settings = self.settings();
        let (account, model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let spec = crate::templates::TemplateStore::new(&store.root()).spec(template_id);
        let body = self.generate_summary_body(
            &spec,
            &before.notes,
            &transcript,
            (&account, &model),
            keys,
            automatic_cancellation,
        )?;
        Ok(MeetingSummaryCandidate {
            meeting_uid: before.uid,
            markdown: body,
            revision: crate::evidence_revision(&before.raw),
            source_revision: before.owner_revision,
        })
    }

    pub(crate) fn commit_generated_summary(
        &self,
        id: &str,
        template_id: &str,
        candidate: &MeetingSummaryCandidate,
    ) -> Result<String, PennelError> {
        self.meeting_store()
            .save_owner_summary(
                id,
                &candidate.revision,
                &candidate.source_revision,
                &candidate.markdown,
                SummarySave::Generate(template_id),
            )
            .map(|saved| saved.markdown)
    }

    fn require_finalized_summary_source(
        &self,
        store: &MeetingStore,
        id: &str,
    ) -> Result<(), PennelError> {
        // edit_meeting also rejects active capture/finalization. Imported legacy transcripts have no recording metadata.
        if store.recording_metadata(id)?.is_some_and(|metadata| {
            metadata.ended_at.is_none()
                || metadata
                    .transcription_attempts
                    .iter()
                    .any(|attempt| attempt.ended_at.is_none())
        }) {
            return Err(PennelError::storage(
                "The meeting transcript is still being finalized.",
            ));
        }
        Ok(())
    }

    fn generate_summary_body(
        &self,
        spec: &crate::templates::TemplateSpec,
        notes: &str,
        transcript: &str,
        selection: (&crate::ProviderAccount, &str),
        keys: &ApiKeys,
        automatic: Option<&Arc<crate::InferenceCancellation>>,
    ) -> Result<String, PennelError> {
        use crate::summary::{
            BLOCK_CHARS, MAP_REDUCE_ABOVE, blocks, chunk_messages, summary_messages,
        };
        let (account, model) = selection;
        let cancellation = automatic
            .cloned()
            .unwrap_or_else(crate::InferenceCancellation::new);
        self.register_inference(&cancellation);
        let _registration = SummaryInference {
            engine: self,
            cancellation: cancellation.clone(),
        };
        let key = self.credential(account, keys)?;
        if crate::provider::requires_key(account.kind) && key.is_none() {
            return Err(PennelError::MissingApiKey { task: "llm".into() });
        }
        let provider = self.endpoint(account, model.to_owned());
        let ask = |messages: &[crate::llm::ChatMessage]| {
            let check_policy = || {
                if cancellation.is_cancelled()
                    || (automatic.is_some()
                        && self.settings().summary_automation
                            != crate::SummaryAutomation::Automatic)
                {
                    Err(PennelError::Cancelled)
                } else {
                    Ok(())
                }
            };
            check_policy()?;
            let result = self.complete_llm(
                &provider,
                key.as_deref(),
                messages,
                180,
                crate::llm::Reasoning::Low,
            );
            check_policy()?;
            result
        };
        let messages = if account.kind == crate::ProviderKind::AppleIntelligence {
            crate::summary::apple_summary_messages(spec, notes, transcript, ask)?
        } else {
            let source = if transcript.chars().count() > MAP_REDUCE_ABOVE {
                blocks(transcript, BLOCK_CHARS)
                    .iter()
                    .map(|part| ask(&chunk_messages(part)))
                    .collect::<Result<Vec<_>, _>>()?
                    .join("\n\n")
            } else {
                transcript.to_owned()
            };
            summary_messages(spec, notes, &source)
        };
        ask(&messages).map(|body| body.trim().to_owned())
    }
}

fn permitted_transcript(store: &MeetingStore, id: &str) -> Result<String, PennelError> {
    permitted_transcript_text(store, id, None)
}

fn permitted_transcript_text(
    store: &MeetingStore,
    id: &str,
    language: Option<InterfaceLanguage>,
) -> Result<String, PennelError> {
    let snapshot = store.read_committed_transcript_snapshot(id)?;
    crate::meeting_utterances::assert_writable(&snapshot)?;
    if store.unparsed_transcript_lines(id) != 0 {
        return Err(PennelError::storage(
            "The saved transcript has unrecognized lines. Review it before sharing.",
        ));
    }
    let legacy = snapshot.metadata_status == crate::TranscriptMetadataStatus::Absent;
    Ok(snapshot
        .utterances
        .iter()
        .filter(|row| !row.segment.failed && !row.segment.text.trim().is_empty())
        .map(|row| {
            let speaker = match language {
                Some(language) if legacy => {
                    crate::export::exported_speaker_label(&row.segment, &[], language)
                }
                Some(language) => {
                    crate::meeting_evidence::localized_utterance_speaker_label(row, &[], language)
                }
                None if legacy => crate::meetings::speaker_label(&row.segment, &[]),
                None => crate::meeting_evidence::utterance_speaker_label(row, &[]),
            };
            // Legacy capture inference depends on audio files outside the shareable source revision.
            let source_id = row.metadata.source_id.as_deref().filter(|_| !legacy);
            let source = match language {
                Some(language) => crate::ui_text(
                    language,
                    match source_id {
                        Some("microphone") => "meetings.attribution_source_microphone",
                        Some("system") => "meetings.attribution_source_system",
                        Some("system_audio") => "meetings.attribution_source_system_audio",
                        Some("room_microphone") => "meetings.attribution_source_room_microphone",
                        _ => "meetings.attribution_source_unknown",
                    },
                    &[],
                ),
                None => match source_id {
                    Some("microphone") => "Microphone",
                    Some("system" | "system_audio") => "System audio",
                    Some("room_microphone") => "Room microphone",
                    _ => "Unknown source",
                }
                .into(),
            };
            format!(
                "[{}] {speaker} ({source}): {}",
                timestamp(row.segment.start_ms),
                row.segment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn read_shareable(
    store: &MeetingStore,
    id: &str,
) -> Result<Option<ShareableMeetingSummary>, PennelError> {
    let directory = checked_directory(store, id)?;
    let gate = replacement_gate(&directory);
    let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
    let files = source_files(&directory)?;
    let bytes = regular_bytes(&directory.join(SHAREABLE_FILE))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let saved: SavedShareable = serde_json::from_slice(&bytes).map_err(|_| {
        PennelError::storage("The shareable summary's source provenance could not be verified.")
    })?;
    let mut document = saved.document;
    if saved.schema_version != 1
        || saved.markdown_revision != crate::evidence_revision(&document.markdown)
        || document.meeting_uid != files.uid
        || document.provenance != TRANSCRIPT_PROVENANCE
        || document.markdown.trim().is_empty()
        || !valid_revision(&document.source_revision)
    {
        return Err(PennelError::storage(
            "The shareable summary's source provenance could not be verified.",
        ));
    }
    document.outdated = document.source_revision != files.transcript_revision;
    Ok(Some(document))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture() -> (tempfile::TempDir, MeetingStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Private title", chrono::Local::now()).unwrap();
        store.write_notes(&id, "Private notes").unwrap();
        store
            .write_transcript(&id, &[segment("The team approved a Friday launch.")])
            .unwrap();
        (dir, store, id)
    }

    fn segment(text: &str) -> crate::Segment {
        crate::Segment {
            start_ms: 0,
            end_ms: 1_000,
            speaker: crate::Speaker::Others,
            text: text.into(),
            failed: false,
            voice: Some(1),
        }
    }

    fn engine(dir: &tempfile::TempDir, server: &mockito::Server) -> std::sync::Arc<PennelEngine> {
        let engine =
            PennelEngine::new(dir.path().join("data").to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = server.url();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        engine
    }

    fn keys() -> ApiKeys {
        ApiKeys {
            by_provider: HashMap::from([("groq".into(), "test-key".into())]),
        }
    }

    fn response(body: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"choices":[{"message":{"content":body}}]})).unwrap()
    }

    fn meeting(engine: &PennelEngine) -> String {
        let store = engine.meeting_store();
        let id = store
            .create("PRIVATE_TITLE_8d91", chrono::Local::now())
            .unwrap();
        store.write_notes(&id, "PRIVATE_NOTE_60a1").unwrap();
        store
            .write_transcript(&id, &[segment("The team approved a Friday launch.")])
            .unwrap();
        id
    }

    fn save(store: &MeetingStore, id: &str, body: &str) -> MeetingSummaryDocument {
        let before = store.read_owner_summary(id).unwrap();
        store
            .save_owner_summary(
                id,
                &before.revision,
                &before.source_revision,
                body,
                SummarySave::Edit,
            )
            .unwrap()
    }

    #[test]
    fn summary_policy_upgrades_without_authorizing_work() {
        let settings = crate::default_settings();
        assert_eq!(
            settings.summary_automation,
            crate::SummaryAutomation::Unanswered
        );
        let mut legacy = serde_json::to_value(settings).unwrap();
        legacy.as_object_mut().unwrap().remove("summary_automation");
        let restored: crate::Settings = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            restored.summary_automation,
            crate::SummaryAutomation::Unanswered
        );
        for (policy, wire) in [
            (crate::SummaryAutomation::Unanswered, "unanswered"),
            (crate::SummaryAutomation::Manual, "manual"),
            (crate::SummaryAutomation::Automatic, "automatic"),
        ] {
            assert_eq!(serde_json::to_value(policy).unwrap(), wire);
        }
    }

    #[test]
    fn exact_owner_edits_survive_metadata_writes_and_retain_prior_revisions() {
        let (_dir, store, id) = fixture();
        let initial = store.read_owner_summary(&id).unwrap();
        assert!(initial.markdown.is_empty());
        let first = save(&store, &id, "\n  # Exact title\r\n\nFirst draft.  \n\n");
        let second = save(&store, &id, "Second draft\n\n");
        assert_ne!(first.revision, second.revision);
        let history = store.owner_summary_history(&id).unwrap();
        assert!(
            history
                .iter()
                .any(|entry| entry.revision == first.revision && entry.markdown == first.markdown)
        );
        assert!(
            history
                .iter()
                .any(|entry| entry.revision == initial.revision && entry.markdown.is_empty())
        );
        let renamed = store.rename(&id, "Renamed owner meeting").unwrap();
        store.set_duration(&renamed, 42).unwrap();
        let current = store.read_owner_summary(&renamed).unwrap();
        assert_eq!(current.markdown, second.markdown);
        assert_eq!(current.meeting_uid, initial.meeting_uid);
        assert_eq!(current.source_revision, initial.source_revision);
        assert_eq!(store.owner_summary_history(&renamed).unwrap(), history);
    }

    #[test]
    fn save_compares_summary_and_private_note_source_versions() {
        let (_dir, store, id) = fixture();
        let first = save(&store, &id, "Current owner text\n");
        let newer = save(&store, &id, "Newer owner text\n");
        assert!(
            store
                .save_owner_summary(
                    &id,
                    &first.revision,
                    &first.source_revision,
                    "Stale editor",
                    SummarySave::Edit
                )
                .is_err()
        );
        assert_eq!(
            store.read_owner_summary(&id).unwrap().markdown,
            newer.markdown
        );
        store.write_notes(&id, "A changed private source").unwrap();
        assert!(
            store
                .save_owner_summary(
                    &id,
                    &newer.revision,
                    &newer.source_revision,
                    "Stale source",
                    SummarySave::Edit
                )
                .is_err()
        );
        let current = store.read_owner_summary(&id).unwrap();
        assert!(current.outdated);
        assert_eq!(current.markdown, newer.markdown);
    }

    #[test]
    fn external_summary_edit_cannot_be_replaced_by_an_open_editor() {
        let (_dir, store, id) = fixture();
        let first = save(&store, &id, "Original owner text\n");
        let path = store.dir_of(&id).join("meeting.md");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("Original owner text", "Edited in Obsidian"),
        )
        .unwrap();
        assert!(
            store
                .save_owner_summary(
                    &id,
                    &first.revision,
                    &first.source_revision,
                    "Stale editor",
                    SummarySave::Edit
                )
                .is_err()
        );
        assert_eq!(
            store.read_owner_summary(&id).unwrap().markdown,
            "Edited in Obsidian\n"
        );
    }

    #[test]
    fn failed_history_write_keeps_current_summary_and_a_comparison_failure_keeps_external_text() {
        let (_dir, store, id) = fixture();
        let before = save(&store, &id, "Retained on disk\n");
        let path = store.dir_of(&id).join("meeting.md");
        let raw = fs::read(&path).unwrap();
        fs::create_dir(
            store
                .dir_of(&id)
                .join(format!("{HISTORY_PREFIX}{}.json", before.revision)),
        )
        .unwrap();
        assert!(
            store
                .save_owner_summary(
                    &id,
                    &before.revision,
                    &before.source_revision,
                    "Would destroy it",
                    SummarySave::Edit
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), raw);
        assert!(compare_write(&path, b"stale bytes", b"replacement", || Ok(())).is_err());
        assert_eq!(fs::read(&path).unwrap(), raw);
    }

    #[test]
    fn automatic_admission_is_durable_and_bound_to_uuid_and_source_after_rename() {
        let (_dir, store, id) = fixture();
        let before = store.read_owner_summary(&id).unwrap();
        assert!(store.admit_automatic_summary(&id, &before).unwrap());
        assert!(!store.admit_automatic_summary(&id, &before).unwrap());
        let renamed = store.rename(&id, "Renamed after failed attempt").unwrap();
        let reopened = MeetingStore::new(&store.root());
        let same_source = reopened.read_owner_summary(&renamed).unwrap();
        assert_eq!(same_source.meeting_uid, before.meeting_uid);
        assert_eq!(same_source.source_revision, before.source_revision);
        assert!(
            !reopened
                .admit_automatic_summary(&renamed, &same_source)
                .unwrap()
        );
        reopened
            .write_transcript(&renamed, &[segment("A distinct finalized revision.")])
            .unwrap();
        let new_source = reopened.read_owner_summary(&renamed).unwrap();
        assert!(
            reopened
                .admit_automatic_summary(&renamed, &new_source)
                .unwrap()
        );
    }

    #[test]
    fn legacy_or_manually_edited_owner_summary_never_accepts_automatic_replacement() {
        let (_dir, store, id) = fixture();
        store
            .write_meeting(&id, 10, "algemeen", "Legacy owner text")
            .unwrap();
        let legacy = store.read_owner_summary(&id).unwrap();
        assert!(legacy.edited);
        assert!(legacy.outdated);
        assert!(!store.admit_automatic_summary(&id, &legacy).unwrap());
        save(&store, &id, "Explicit owner edit");
        store.write_notes(&id, "New note source").unwrap();
        let edited = store.read_owner_summary(&id).unwrap();
        assert!(!store.admit_automatic_summary(&id, &edited).unwrap());
    }

    #[test]
    fn unanswered_and_manual_policies_do_no_model_work_and_failed_automatic_attempt_does_not_retry()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        for policy in [
            crate::SummaryAutomation::Unanswered,
            crate::SummaryAutomation::Manual,
        ] {
            let mut settings = engine.settings();
            settings.summary_automation = policy;
            engine.update_settings(settings).unwrap();
            assert_eq!(
                engine
                    .summarize_meeting_automatically(id.clone(), "algemeen".into(), keys())
                    .unwrap(),
                None
            );
        }
        let mut settings = engine.settings();
        settings.summary_automation = crate::SummaryAutomation::Automatic;
        engine.update_settings(settings).unwrap();
        let error = engine
            .summarize_meeting_automatically(
                id.clone(),
                "algemeen".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(error, PennelError::MissingApiKey { .. }));
        assert_eq!(
            engine
                .summarize_meeting_automatically(id.clone(), "algemeen".into(), keys())
                .unwrap(),
            None
        );
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body(response("Manual retry succeeds"))
            .expect(1)
            .create();
        assert_eq!(
            engine
                .summarize_meeting(id, "algemeen".into(), keys())
                .unwrap(),
            "Manual retry succeeds"
        );
        provider.assert();
    }

    #[test]
    fn owner_generation_is_reviewable_without_mutation_and_acceptance_is_cas() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let before = engine.meeting_summary(id.clone()).unwrap();
        let provider = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex("PRIVATE_NOTE_60a1".into()))
            .with_body(response("Candidate only\n"))
            .expect(1)
            .create();
        let candidate = engine
            .generate_meeting_summary(id.clone(), "algemeen".into(), keys())
            .unwrap();
        assert_eq!(engine.meeting_summary(id.clone()).unwrap(), before);
        let saved = engine
            .accept_meeting_summary(
                id.clone(),
                candidate.revision.clone(),
                candidate.source_revision.clone(),
                candidate.markdown.clone(),
            )
            .unwrap();
        assert_eq!(saved.markdown, "Candidate only");
        assert!(
            engine
                .save_meeting_summary(
                    id.clone(),
                    candidate.revision,
                    candidate.source_revision,
                    "Stale candidate".into()
                )
                .is_err()
        );
        let initial = engine
            .meeting_summary_history(id.clone())
            .unwrap()
            .into_iter()
            .find(|entry| entry.revision == before.revision)
            .unwrap();
        let restored = engine
            .restore_meeting_summary(
                id.clone(),
                initial.revision,
                saved.revision,
                saved.source_revision,
            )
            .unwrap();
        assert!(restored.markdown.is_empty());
        assert!(
            engine
                .meeting_summary_history(id)
                .unwrap()
                .iter()
                .any(|entry| entry.markdown == "Candidate only")
        );
        provider.assert();
    }

    #[test]
    fn provider_failure_retains_the_existing_owner_document() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let before = save(
            &engine.meeting_store(),
            &id,
            "Do not erase this document\n\n",
        );
        for status in [400, 499] {
            let provider = server
                .mock("POST", "/chat/completions")
                .with_status(status)
                .with_body("cancelled or failed")
                .expect(1)
                .create();
            assert!(
                engine
                    .summarize_meeting(id.clone(), "algemeen".into(), keys())
                    .is_err()
            );
            assert_eq!(engine.meeting_summary(id.clone()).unwrap(), before);
            provider.assert();
            provider.remove();
        }
    }

    #[test]
    fn generated_summary_cannot_overwrite_an_external_edit_during_provider_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        save(&engine.meeting_store(), &id, "Original body\n");
        let path = engine.meeting_store().dir_of(&id).join("meeting.md");
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |_| {
                let raw = fs::read_to_string(&path).unwrap();
                fs::write(
                    &path,
                    raw.replace("Original body", "Concurrent external edit"),
                )
                .unwrap();
                response("Generated replacement")
            })
            .expect(1)
            .create();
        assert!(
            engine
                .summarize_meeting(id.clone(), "algemeen".into(), keys())
                .is_err()
        );
        assert_eq!(
            engine.meeting_summary(id).unwrap().markdown,
            "Concurrent external edit\n"
        );
        provider.assert();
    }

    #[test]
    fn generated_summary_cannot_commit_after_the_transcript_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let before = save(&engine.meeting_store(), &id, "Retained owner draft");
        let path = engine.meeting_store().dir_of(&id).join("transcript.md");
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |_| {
                fs::write(&path, "[00:00] **Anderen 1:** Changed finalized source\n").unwrap();
                response("Generated from old source")
            })
            .expect(1)
            .create();
        assert!(
            engine
                .summarize_meeting(id.clone(), "algemeen".into(), keys())
                .is_err()
        );
        assert_eq!(
            engine.meeting_summary(id).unwrap().markdown,
            before.markdown
        );
        provider.assert();
    }

    #[test]
    fn shareable_generation_and_preview_exclude_all_private_context_and_keep_personal_export() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let store = engine.meeting_store();
        save(&store, &id, "PRIVATE_OWNER_SUMMARY_7b2c");
        store
            .set_speaker_names(
                &id,
                &[crate::SpeakerName {
                    voice: 1,
                    name: "PRIVATE_ALIAS_b148".into(),
                }],
            )
            .unwrap();
        store
            .set_attendees(&id, &["PRIVATE_CONTACT_0bd4".into()])
            .unwrap();
        let created: serde_json::Value = serde_json::from_str(
            &engine.update_meeting_people_json(id.clone(), serde_json::json!({
                "expected_revision": 0, "action": "create_person", "origin": "contact",
                "display_name": "PRIVATE_CONTACT_NAME_6cf0", "private_alias": "PRIVATE_CONTACT_ALIAS_6cf0",
                "email": "PRIVATE_EMAIL_6cf0@example.test",
                "source": { "source_id": "PRIVATE_SOURCE_6cf0", "account_id": "owner", "record_id": "card", "revision": "one" }
            }).to_string()).unwrap()
        ).unwrap();
        let state = &created["snapshot"];
        let contact = state["people"]
            .as_array()
            .unwrap()
            .iter()
            .find(|person| person["display_name"] == "PRIVATE_CONTACT_NAME_6cf0")
            .unwrap();
        engine.update_meeting_people_json(id.clone(), serde_json::json!({
            "expected_revision": state["revision"], "action": "link_speaker", "person_id": contact["id"],
            "utterance_ids": [store.read_transcript_snapshot(&id).unwrap().utterances[0].metadata.utterance_id],
            "expected_source_revision": state["source_revision"]
        }).to_string()).unwrap();
        let template = engine
            .save_template(
                None,
                "PRIVATE_TEMPLATE_9032".into(),
                "PRIVATE_TEMPLATE_INSTRUCTION_2407".into(),
            )
            .unwrap();
        fs::write(
            store.dir_of(&id).join("private-chat.json"),
            "PRIVATE_CHAT_12d0",
        )
        .unwrap();
        let sentinels = [
            "PRIVATE_NOTE_60a1",
            "PRIVATE_ALIAS_b148",
            "PRIVATE_TITLE_8d91",
            "PRIVATE_OWNER_SUMMARY_7b2c",
            "PRIVATE_CONTACT_0bd4",
            "PRIVATE_TEMPLATE_9032",
            "PRIVATE_TEMPLATE_INSTRUCTION_2407",
            "PRIVATE_CHAT_12d0",
            "PRIVATE_CONTACT_NAME_6cf0",
            "PRIVATE_CONTACT_ALIAS_6cf0",
            "PRIVATE_EMAIL_6cf0",
            "PRIVATE_SOURCE_6cf0",
        ];
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |request| {
                let body = std::str::from_utf8(request.body().unwrap()).unwrap();
                assert!(body.contains("The team approved a Friday launch."));
                assert!(body.contains(
                    "[00:00] Speaker 1 (Unknown source): The team approved a Friday launch."
                ));
                for sentinel in sentinels {
                    assert!(
                        !body.contains(sentinel),
                        "private source crossed the provider boundary: {sentinel}"
                    );
                }
                response("## Summary\n- The team approved a Friday launch.")
            })
            .expect(1)
            .create();
        let generated = engine
            .generate_shareable_meeting_summary(id.clone(), template, keys())
            .unwrap();
        assert_eq!(generated.provenance, TRANSCRIPT_PROVENANCE);
        let output = engine
            .preview_meeting_export(
                id.clone(),
                MeetingOutputScope::Shareable,
                true,
                InterfaceLanguage::English,
            )
            .unwrap();
        for sentinel in sentinels {
            assert!(!output.markdown.contains(sentinel));
            assert!(!output.html.contains(sentinel));
            assert!(!output.title.contains(sentinel));
        }
        assert!(
            output
                .markdown
                .contains("[00:00] Speaker 1 (Unknown source): The team approved a Friday launch.")
        );
        assert!(output.html.contains("Speaker 1 (Unknown source)"));
        let summary_only = engine
            .preview_meeting_export(
                id.clone(),
                MeetingOutputScope::Shareable,
                false,
                InterfaceLanguage::English,
            )
            .unwrap();
        assert_eq!(summary_only.markdown, generated.markdown);
        assert!(!summary_only.markdown.contains("[00:00]"));
        assert!(
            engine
                .preview_meeting_export(
                    id.clone(),
                    MeetingOutputScope::Personal,
                    true,
                    InterfaceLanguage::English
                )
                .unwrap()
                .markdown
                .contains("PRIVATE_OWNER_SUMMARY_7b2c")
        );
        store.write_notes(&id, "Different private note").unwrap();
        let people: serde_json::Value =
            serde_json::from_str(&engine.meeting_people_snapshot_json(id.clone()).unwrap())
                .unwrap();
        engine
            .update_meeting_people_json(
                id.clone(),
                serde_json::json!({
                    "expected_revision": people["revision"], "action": "set_private_alias",
                    "person_id": contact["id"], "private_alias": "A changed private alias"
                })
                .to_string(),
            )
            .unwrap();
        assert!(
            !engine
                .shareable_meeting_summary(id.clone())
                .unwrap()
                .unwrap()
                .outdated
        );
        store
            .write_transcript(&id, &[segment("A newer approved source.")])
            .unwrap();
        assert!(
            engine
                .shareable_meeting_summary(id.clone())
                .unwrap()
                .unwrap()
                .outdated
        );
        assert!(
            engine
                .preview_meeting_export(
                    id,
                    MeetingOutputScope::Shareable,
                    false,
                    InterfaceLanguage::English
                )
                .is_err()
        );
        provider.assert();
    }

    #[test]
    fn shareable_exports_localize_labels_without_changing_canonical_provider_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let store = engine.meeting_store();
        let mut rows: Vec<_> = [
            ("system", "A numbered voice."),
            ("room_microphone", "A confirmed person."),
            ("system_audio", "An explicitly identified self voice."),
            ("microphone", "  Exact\twords remain intact.  "),
            ("room_microphone", "Mixed words."),
            ("system", "Overlapping words."),
            ("system", "Provisional words."),
            ("PRIVATE_CAPTURE_ALIAS_7f9c", "An unfamiliar capture lane."),
            ("microphone", "A retained correction needs review."),
            ("microphone", "FAILED_PRIVATE_WORDS_913d"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (source, text))| {
            let mut segment = segment(text);
            segment.start_ms = index as u64 * 1_000;
            segment.end_ms = segment.start_ms + 1_000;
            crate::MeetingUtterance::new(segment, Some(source.into()))
        })
        .collect();
        rows[0].metadata.attribution.kind = crate::AttributionKind::Single;
        rows[0].metadata.attribution.voices = vec![1];
        rows[1].metadata.manual = Some(crate::ManualCorrection {
            name: Some("Canonical Noor".into()),
            voice: Some(2),
            source_fingerprint: rows[1].metadata.source_fingerprint.clone(),
            review_required: false,
        });
        rows[2].metadata.attribution.kind = crate::AttributionKind::Single;
        rows[2].metadata.attribution.self_voice = true;
        rows[3].segment.speaker = crate::Speaker::Me;
        rows[3].segment.voice = None;
        rows[4].metadata.attribution.kind = crate::AttributionKind::Mixed;
        rows[4].metadata.attribution.voices = vec![1, 2];
        rows[5].metadata.attribution.kind = crate::AttributionKind::Overlap;
        rows[5].metadata.attribution.voices = vec![1, 2];
        rows[6].metadata.attribution.kind = crate::AttributionKind::Single;
        rows[6].metadata.attribution.voices = vec![3];
        rows[6].metadata.attribution.provisional = true;
        rows[8].metadata.manual = Some(crate::ManualCorrection {
            name: Some("STALE_CORRECTION_2b16".into()),
            voice: Some(2),
            source_fingerprint: "0".repeat(64),
            review_required: true,
        });
        rows[9].segment.failed = true;
        store.write_transcript_snapshot(&id, None, &rows).unwrap();
        store
            .set_speaker_names(
                &id,
                &[crate::SpeakerName {
                    voice: 1,
                    name: "PRIVATE_FRONTMATTER_ALIAS_8301".into(),
                }],
            )
            .unwrap();

        let rendered = permitted_transcript(&store, &id).unwrap();
        assert_eq!(rendered, [
            "[00:00] Speaker 1 (System audio): A numbered voice.",
            "[00:01] Canonical Noor (Room microphone): A confirmed person.",
            "[00:02] Me (System audio): An explicitly identified self voice.",
            "[00:03] Unknown speaker (Microphone):   Exact\twords remain intact.  ",
            "[00:04] Mixed speakers (Room microphone): Mixed words.",
            "[00:05] Overlapping speakers (System audio): Overlapping words.",
            "[00:06] Speaker 3 (provisional) (System audio): Provisional words.",
            "[00:07] Unknown speaker (Unknown source): An unfamiliar capture lane.",
            "[00:08] Unknown speaker (correction needs review) (Microphone): A retained correction needs review.",
        ].join("\n"));
        for private in [
            "PRIVATE_FRONTMATTER_ALIAS_8301",
            "PRIVATE_CAPTURE_ALIAS_7f9c",
            "STALE_CORRECTION_2b16",
            "FAILED_PRIVATE_WORDS_913d",
        ] {
            assert!(!rendered.contains(private));
        }
        let original = fs::read(store.dir_of(&id).join("transcript.md")).unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |request| {
                let body: serde_json::Value =
                    serde_json::from_slice(request.body().unwrap()).unwrap();
                captured.lock().unwrap().push(body["messages"].clone());
                response("Unchanged generated words.")
            })
            .expect(2)
            .create();
        for ui_language in [
            crate::InterfaceLanguagePreference::English,
            crate::InterfaceLanguagePreference::Dutch,
        ] {
            let mut settings = engine.settings();
            settings.ui_language = ui_language;
            engine.update_settings(settings).unwrap();
            engine
                .generate_shareable_meeting_summary(id.clone(), "algemeen".into(), keys())
                .unwrap();
            for (language, expected) in [
                (
                    InterfaceLanguage::English,
                    [
                        "[00:00] Speaker 1 (Computer audio): A numbered voice.",
                        "[00:01] Canonical Noor (Room microphone): A confirmed person.",
                        "[00:02] Me (Computer audio): An explicitly identified self voice.",
                        "[00:03] Unknown speaker (Microphone):   Exact\twords remain intact.  ",
                        "[00:04] Mixed speakers (Room microphone): Mixed words.",
                        "[00:05] Overlapping speakers (Computer audio): Overlapping words.",
                        "[00:06] Speaker 3 (Provisional) (Computer audio): Provisional words.",
                        "[00:07] Unknown speaker (Unknown source): An unfamiliar capture lane.",
                        "[00:08] Unknown speaker (Speaker correction needs review) (Microphone): A retained correction needs review.",
                    ],
                ),
                (
                    InterfaceLanguage::Dutch,
                    [
                        "[00:00] Anderen 1 (Computeraudio): A numbered voice.",
                        "[00:01] Canonical Noor (Ruimtemicrofoon): A confirmed person.",
                        "[00:02] Ik (Computeraudio): An explicitly identified self voice.",
                        "[00:03] Onbekende spreker (Microfoon):   Exact\twords remain intact.  ",
                        "[00:04] Meerdere sprekers (Ruimtemicrofoon): Mixed words.",
                        "[00:05] Overlappende sprekers (Computeraudio): Overlapping words.",
                        "[00:06] Anderen 3 (Voorlopig) (Computeraudio): Provisional words.",
                        "[00:07] Onbekende spreker (Onbekende bron): An unfamiliar capture lane.",
                        "[00:08] Onbekende spreker (Controleer de sprekercorrectie) (Microfoon): A retained correction needs review.",
                    ],
                ),
            ] {
                let output = engine
                    .preview_meeting_export(
                        id.clone(),
                        MeetingOutputScope::Shareable,
                        true,
                        language,
                    )
                    .unwrap();
                assert!(
                    output
                        .markdown
                        .starts_with("Unchanged generated words.\n\n")
                );
                for line in expected {
                    assert!(
                        output.markdown.contains(line),
                        "Missing localized export line: {line}"
                    );
                    assert!(
                        output.html.contains(line.trim()),
                        "Missing localized HTML line: {line}"
                    );
                }
                for private in [
                    "PRIVATE_FRONTMATTER_ALIAS_8301",
                    "PRIVATE_CAPTURE_ALIAS_7f9c",
                    "STALE_CORRECTION_2b16",
                    "FAILED_PRIVATE_WORDS_913d",
                    "PRIVATE_NOTE_60a1",
                ] {
                    assert!(!output.markdown.contains(private));
                    assert!(!output.html.contains(private));
                }
                let summary_only = engine
                    .preview_meeting_export(
                        id.clone(),
                        MeetingOutputScope::Shareable,
                        false,
                        language,
                    )
                    .unwrap();
                assert_eq!(summary_only.markdown, "Unchanged generated words.");
            }
        }
        provider.assert();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        let content = requests[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains(&rendered));
        assert_eq!(
            fs::read(store.dir_of(&id).join("transcript.md")).unwrap(),
            original
        );
    }

    #[test]
    fn permitted_legacy_transcript_preserves_original_labels_without_inventing_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let store = engine.meeting_store();
        let id = store.create("Legacy", chrono::Local::now()).unwrap();
        fs::write(
            store.dir_of(&id).join("transcript.md"),
            "[00:00] **Ik:** My original words.\n[00:01] **Anderen 2:** Numbered original words.\n[00:02] **Anderen:** Unidentified original words.\n",
        )
        .unwrap();
        assert_eq!(
            store
                .read_committed_transcript_snapshot(&id)
                .unwrap()
                .metadata_status,
            crate::TranscriptMetadataStatus::Absent
        );
        let canonical = permitted_transcript(&store, &id).unwrap();
        assert_eq!(
            canonical,
            [
                "[00:00] Ik (Unknown source): My original words.",
                "[00:01] Anderen 2 (Unknown source): Numbered original words.",
                "[00:02] Anderen (Unknown source): Unidentified original words.",
            ]
            .join("\n")
        );
        store
            .set_speaker_names(
                &id,
                &[crate::SpeakerName {
                    voice: 2,
                    name: "PRIVATE_LEGACY_ALIAS".into(),
                }],
            )
            .unwrap();
        let before = fs::read(store.dir_of(&id).join("transcript.md")).unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |request| {
                let body: serde_json::Value =
                    serde_json::from_slice(request.body().unwrap()).unwrap();
                captured.lock().unwrap().push(body["messages"].clone());
                response("Unchanged legacy summary.")
            })
            .expect(2)
            .create();
        for ui_language in [
            crate::InterfaceLanguagePreference::English,
            crate::InterfaceLanguagePreference::Dutch,
        ] {
            let mut settings = engine.settings();
            settings.ui_language = ui_language;
            engine.update_settings(settings).unwrap();
            engine
                .generate_shareable_meeting_summary(id.clone(), "algemeen".into(), keys())
                .unwrap();
            for (language, lines) in [
                (
                    InterfaceLanguage::English,
                    [
                        "[00:00] Me (Unknown source): My original words.",
                        "[00:01] Speaker 2 (Unknown source): Numbered original words.",
                        "[00:02] Others (Unknown source): Unidentified original words.",
                    ],
                ),
                (
                    InterfaceLanguage::Dutch,
                    [
                        "[00:00] Ik (Onbekende bron): My original words.",
                        "[00:01] Anderen 2 (Onbekende bron): Numbered original words.",
                        "[00:02] Anderen (Onbekende bron): Unidentified original words.",
                    ],
                ),
            ] {
                let output = engine
                    .preview_meeting_export(
                        id.clone(),
                        MeetingOutputScope::Shareable,
                        true,
                        language,
                    )
                    .unwrap();
                for line in lines {
                    assert!(
                        output.markdown.contains(line),
                        "Missing localized legacy label: {line}"
                    );
                    assert!(output.html.contains(line));
                }
                assert!(!output.markdown.contains("PRIVATE_LEGACY_ALIAS"));
                assert!(!output.html.contains("PRIVATE_LEGACY_ALIAS"));
            }
        }
        assert_eq!(
            fs::read(store.dir_of(&id).join("transcript.md")).unwrap(),
            before
        );
        assert!(
            !store
                .dir_of(&id)
                .join(crate::meeting_utterances::MANIFEST)
                .exists()
        );
        provider.assert();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        let content = requests[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains(&canonical));
    }

    #[test]
    fn shareable_attribution_and_source_changes_reject_inflight_and_saved_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let store = engine.meeting_store();
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body(response("A transcript-only summary."))
            .expect(1)
            .create();
        engine
            .generate_shareable_meeting_summary(id.clone(), "algemeen".into(), keys())
            .unwrap();
        provider.assert();
        store
            .set_speaker_names(
                &id,
                &[crate::SpeakerName {
                    voice: 1,
                    name: "PRIVATE_NEW_ALIAS".into(),
                }],
            )
            .unwrap();
        assert!(
            !engine
                .shareable_meeting_summary(id.clone())
                .unwrap()
                .unwrap()
                .outdated
        );
        let before = store.read_transcript_snapshot(&id).unwrap();
        let mut rows = vec![crate::MeetingUtterance::new(
            before.utterances[0].segment.clone(),
            Some("room_microphone".into()),
        )];
        rows[0].metadata.attribution.kind = crate::AttributionKind::Single;
        rows[0].metadata.attribution.voices = vec![2];
        store
            .write_transcript_snapshot(&id, Some(before.revision), &rows)
            .unwrap();
        assert!(
            engine
                .shareable_meeting_summary(id.clone())
                .unwrap()
                .unwrap()
                .outdated
        );
        assert!(
            engine
                .preview_meeting_export(
                    id.clone(),
                    MeetingOutputScope::Shareable,
                    false,
                    InterfaceLanguage::English
                )
                .is_err()
        );

        let provider_store = MeetingStore::new(&store.root());
        let provider_id = id.clone();
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |_| {
                let before = provider_store
                    .read_transcript_snapshot(&provider_id)
                    .unwrap();
                let mut rows = before.utterances;
                rows[0].metadata.attribution.voices = vec![3];
                provider_store
                    .write_transcript_snapshot(&provider_id, Some(before.revision), &rows)
                    .unwrap();
                response("Generated from the previous attribution.")
            })
            .expect(1)
            .create();
        assert!(
            engine
                .generate_shareable_meeting_summary(id.clone(), "algemeen".into(), keys())
                .is_err()
        );
        assert_eq!(
            engine
                .shareable_meeting_summary(id)
                .unwrap()
                .unwrap()
                .markdown,
            "A transcript-only summary."
        );
        provider.assert();
    }

    #[test]
    fn permitted_transcript_keeps_unavailable_metadata_and_unparsed_lines_closed() {
        for failure in ["corrupt", "unsupported", "stale", "pending", "unparsed"] {
            let (_dir, store, id) = fixture();
            let root = store.dir_of(&id);
            let manifest = root.join(crate::meeting_utterances::MANIFEST);
            match failure {
                "corrupt" => fs::write(manifest, "not metadata").unwrap(),
                "unsupported" => {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
                    value["schema_version"] = 100.into();
                    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "stale" => fs::write(
                    root.join("transcript.md"),
                    "[00:00] **Anderen 1:** Changed words.\n",
                )
                .unwrap(),
                "pending" => fs::write(
                    root.join(crate::meeting_utterances::SNAPSHOT_JOURNAL),
                    "pending",
                )
                .unwrap(),
                "unparsed" => {
                    fs::remove_file(manifest).unwrap();
                    fs::write(
                        root.join("transcript.md"),
                        "[00:00] **Ik:** Preserved.\nUnrecognized words.\n",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(permitted_transcript(&store, &id).is_err(), "{failure}");
        }
    }

    #[test]
    fn legacy_notes_missing_transcript_and_unknown_provenance_fail_closed_for_shareable_output() {
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = engine
            .meeting_store()
            .create("Legacy", chrono::Local::now())
            .unwrap();
        let store = engine.meeting_store();
        store.write_notes(&id, "PRIVATE_NOTE_ONLY_462b").unwrap();
        store
            .write_meeting(&id, 3, "algemeen", "LEGACY_UNKNOWN_PRIVATE_SUMMARY")
            .unwrap();
        assert!(
            engine
                .preview_meeting_export(
                    id.clone(),
                    MeetingOutputScope::Shareable,
                    false,
                    InterfaceLanguage::English
                )
                .is_err()
        );
        assert_eq!(
            engine
                .generate_shareable_meeting_summary(id.clone(), "algemeen".into(), keys())
                .unwrap_err(),
            PennelError::NoSpeech
        );
        fs::write(
            store.dir_of(&id).join(SHAREABLE_FILE),
            "{\"markdown\":\"unproven legacy text\"}",
        )
        .unwrap();
        assert!(engine.shareable_meeting_summary(id.clone()).is_err());
        assert!(
            engine
                .preview_meeting_export(
                    id.clone(),
                    MeetingOutputScope::Shareable,
                    true,
                    InterfaceLanguage::English
                )
                .is_err()
        );
        assert!(
            engine
                .preview_meeting_export(
                    id,
                    MeetingOutputScope::Personal,
                    false,
                    InterfaceLanguage::English
                )
                .unwrap()
                .markdown
                .contains("LEGACY_UNKNOWN_PRIVATE_SUMMARY")
        );
    }

    #[test]
    fn summary_companions_are_owner_sync_files_and_follow_the_meeting_rename() {
        let (_dir, store, id) = fixture();
        let before = store.read_owner_summary(&id).unwrap();
        store.admit_automatic_summary(&id, &before).unwrap();
        save(&store, &id, "Owner edit");
        let renamed = store.rename(&id, "Synced rename").unwrap();
        for entry in fs::read_dir(store.dir_of(&renamed)).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.starts_with("pennel.summary.") {
                assert!(crate::cloud::sync::scan::is_syncable(&name));
                assert!(
                    crate::cloud::sync::scan::parse_key(&format!(
                        "meeting/{}/{name}",
                        before.meeting_uid
                    ))
                    .is_some()
                );
            }
        }
    }

    #[test]
    fn source_is_rechecked_after_the_replacement_has_been_staged_and_synced() {
        let (_dir, store, id) = fixture();
        let before = store.summary_files(&id).unwrap();
        let directory = store.dir_of(&id);
        let path = directory.join("meeting.md");
        let result = compare_write(&path, before.raw.as_bytes(), b"staged replacement", || {
            // This hook runs after staging+fsync, at the same boundary as the production source CAS.
            fs::write(directory.join("notes.md"), "An external late note edit").unwrap();
            if source_files(&directory)?.owner_revision != before.owner_revision {
                return Err(changed());
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), before.raw);
    }

    #[test]
    fn edits_and_restores_preserve_old_source_freshness_until_explicit_candidate_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let store = engine.meeting_store();
        let original = save(&store, &id, "Summary based on the original source");
        assert!(!original.outdated);
        store
            .write_transcript(&id, &[segment("The source changed after this summary.")])
            .unwrap();
        let current = engine.meeting_summary(id.clone()).unwrap();
        assert!(current.outdated);
        let edited = engine
            .save_meeting_summary(
                id.clone(),
                current.revision,
                current.source_revision,
                "A small spelling edit".into(),
            )
            .unwrap();
        assert!(edited.outdated);
        let restored = engine
            .restore_meeting_summary(
                id.clone(),
                original.revision,
                edited.revision,
                edited.source_revision,
            )
            .unwrap();
        assert!(restored.outdated);
        let accepted = engine
            .accept_meeting_summary(
                id,
                restored.revision,
                restored.source_revision,
                "Reviewed current-source candidate".into(),
            )
            .unwrap();
        assert!(!accepted.outdated);
    }

    #[test]
    fn an_external_edit_of_a_generated_body_is_protected_after_later_source_changes() {
        let (_dir, store, id) = fixture();
        let initial = store.read_owner_summary(&id).unwrap();
        store
            .save_owner_summary(
                &id,
                &initial.revision,
                &initial.source_revision,
                "Generated owner body",
                SummarySave::Generate("algemeen"),
            )
            .unwrap();
        let path = store.dir_of(&id).join("meeting.md");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("Generated owner body", "Manually changed outside Pennel"),
        )
        .unwrap();
        store
            .write_notes(&id, "Changed notes following the edit")
            .unwrap();
        let current = store.read_owner_summary(&id).unwrap();
        assert!(current.edited);
        assert!(current.outdated);
        assert!(!store.admit_automatic_summary(&id, &current).unwrap());
    }

    #[test]
    fn revoking_automation_stops_subsequent_map_reduce_dispatch_and_keeps_attempt_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        engine
            .meeting_store()
            .write_transcript(&id, &[segment(&"long meeting source ".repeat(2_500))])
            .unwrap();
        let mut settings = engine.settings();
        settings.summary_automation = crate::SummaryAutomation::Automatic;
        engine.update_settings(settings).unwrap();
        let callback_engine = engine.clone();
        let provider = server
            .mock("POST", "/chat/completions")
            .with_body_from_request(move |_| {
                let mut settings = callback_engine.settings();
                settings.summary_automation = crate::SummaryAutomation::Manual;
                callback_engine.update_settings(settings).unwrap();
                response("The first reduction completed after consent was revoked.")
            })
            .expect(1)
            .create();
        assert_eq!(
            engine
                .summarize_meeting_automatically(id.clone(), "algemeen".into(), keys())
                .unwrap_err(),
            PennelError::Cancelled
        );
        assert!(
            engine
                .meeting_summary(id.clone())
                .unwrap()
                .markdown
                .is_empty()
        );
        let mut settings = engine.settings();
        settings.summary_automation = crate::SummaryAutomation::Automatic;
        engine.update_settings(settings).unwrap();
        assert!(
            engine
                .summarize_meeting_automatically(id, "algemeen".into(), keys())
                .unwrap()
                .is_none()
        );
        provider.assert();
    }

    #[test]
    fn native_model_cancellation_keeps_the_previous_owner_document() {
        struct CancelledModel;
        impl crate::SystemLanguageModel for CancelledModel {
            fn complete(
                &self,
                _: Vec<crate::LanguageModelMessage>,
                _: u64,
            ) -> Result<String, PennelError> {
                Err(PennelError::Cancelled)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server);
        let id = meeting(&engine);
        let before = save(&engine.meeting_store(), &id, "Existing exact text\n\n");
        let mut settings = engine.settings();
        let account = crate::new_account(
            crate::ProviderKind::AppleIntelligence,
            settings.providers.clone(),
        );
        settings.llm = crate::ModelChoice {
            provider_id: account.id.clone(),
            model: "apple-intelligence".into(),
        };
        settings.providers.push(account);
        engine.update_settings(settings).unwrap();
        engine.set_system_language_model(std::sync::Arc::new(CancelledModel));
        assert_eq!(
            engine
                .summarize_meeting(id.clone(), "algemeen".into(), keys())
                .unwrap_err(),
            PennelError::Cancelled
        );
        assert_eq!(engine.meeting_summary(id).unwrap(), before);
    }

    #[test]
    fn personal_scoped_export_cannot_escape_the_library_or_follow_meeting_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server);
        meeting(&engine);
        let outside = MeetingStore::new(&dir.path().join("outside"));
        let outside_id = outside
            .create("Outside private meeting", chrono::Local::now())
            .unwrap();
        outside
            .write_notes(&outside_id, "Outside private note")
            .unwrap();
        let traversal = format!("../outside/{outside_id}");
        assert!(
            engine
                .preview_meeting_export(
                    traversal,
                    MeetingOutputScope::Personal,
                    true,
                    InterfaceLanguage::English
                )
                .is_err()
        );
        assert!(
            engine
                .preview_meeting_export(
                    outside.dir_of(&outside_id).to_string_lossy().into_owned(),
                    MeetingOutputScope::Personal,
                    false,
                    InterfaceLanguage::English
                )
                .is_err()
        );
        #[cfg(unix)]
        {
            let root = engine.meeting_store().root();
            std::os::unix::fs::symlink(outside.dir_of(&outside_id), root.join("linked-meeting"))
                .unwrap();
            assert!(
                engine
                    .preview_meeting_export(
                        "linked-meeting".into(),
                        MeetingOutputScope::Personal,
                        true,
                        InterfaceLanguage::English
                    )
                    .is_err()
            );
            let local = engine
                .meeting_store()
                .create("Local symlinked document", chrono::Local::now())
                .unwrap();
            let path = engine.meeting_store().dir_of(&local).join("meeting.md");
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(outside.dir_of(&outside_id).join("meeting.md"), path)
                .unwrap();
            assert!(
                engine
                    .preview_meeting_export(
                        local,
                        MeetingOutputScope::Personal,
                        true,
                        InterfaceLanguage::English
                    )
                    .is_err()
            );
        }
    }
}
