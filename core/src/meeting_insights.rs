use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::meeting_utterances::{
    MeetingTranscriptSnapshot, MeetingUtterance, TranscriptMetadataStatus,
};
use crate::meetings::{MeetingStore, durable_write};
use crate::{InferenceCancellation, InferenceIdentity, PennelError, utterance_evidence_revision};

pub const MAX_INSIGHTS_BYTES: usize = 3_000_000;
const INSIGHTS_FILE: &str = "insights.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightKind {
    Action,
    Decision,
    OpenQuestion,
    ChangedCommitment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightExtractionScope {
    #[default]
    Passage,
    ExactSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightAssessmentScope {
    pub utterance_id: String,
    pub kinds: Vec<InsightKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentState {
    Tentative,
    Confirmed,
    Retracted,
    Unresolved,
    Changed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightReviewState {
    Proposed,
    Accepted,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightSourceState {
    Current,
    Stale,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightStoreStatus {
    Missing,
    Ready,
    Unsupported,
    Corrupt,
    TooLarge,
    WrongMeeting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightSource {
    pub meeting_uid: String,
    pub utterance_id: String,
    pub source_fingerprint: String,
    pub quote: String,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl InsightSource {
    pub fn from_utterance(
        meeting_uid: &str,
        row: &MeetingUtterance,
        start_byte: u32,
        end_byte: u32,
    ) -> Result<Self, PennelError> {
        let quote = row
            .segment
            .text
            .get(start_byte as usize..end_byte as usize)
            .filter(|quote| !quote.trim().is_empty())
            .ok_or_else(|| invalid("The source selection is invalid."))?;
        if row.segment.failed {
            return Err(invalid("Unfinished speech cannot support a suggestion."));
        }
        Ok(Self {
            meeting_uid: meeting_uid.into(),
            utterance_id: row.metadata.utterance_id.clone(),
            source_fingerprint: utterance_evidence_revision(row),
            quote: quote.into(),
            start_byte,
            end_byte,
            start_ms: row.segment.start_ms,
            end_ms: row.segment.end_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedInsightDetail {
    pub text: String,
    pub source_index: u32,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightProcessing {
    pub identity: InferenceIdentity,
    pub qualification_id: String,
    pub language_policy: String,
    pub task: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightPriorEvidence {
    pub kind: InsightKind,
    pub source: InsightSource,
    pub target_item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightCandidate {
    pub kind: InsightKind,
    #[serde(default)]
    pub extraction_scope: InsightExtractionScope,
    pub commitment: CommitmentState,
    pub text: String,
    pub owner: Option<SupportedInsightDetail>,
    pub due: Option<SupportedInsightDetail>,
    pub sources: Vec<InsightSource>,
    pub evidence_cutoff_ms: u64,
    pub transcript_revision: u64,
    pub transcript_digest: String,
    pub processing: InsightProcessing,
    pub related_item_ids: Vec<String>,
    #[serde(default)]
    pub prior_evidence: Vec<InsightPriorEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightItem {
    pub id: String,
    pub dedup_key: String,
    pub revision: u64,
    #[serde(flatten)]
    pub proposal: InsightCandidate,
    pub review_state: InsightReviewState,
    pub edited_text: Option<String>,
    pub edit_claim: Option<String>,
    pub source_state: InsightSourceState,
    pub source_history: Vec<InsightSource>,
    pub source_reassociated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightInsertionState {
    Pending,
    Acknowledged,
    NotInserted,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightInsertionReceipt {
    pub attempt_id: String,
    pub item_id: String,
    pub item_revision: u64,
    pub before_notes_hash: String,
    pub intended_notes_hash: String,
    pub markdown: String,
    pub state: InsightInsertionState,
    pub manual_resolution: Option<InsightInsertionResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightInsertionResolution {
    pub state: InsightInsertionState,
    pub notes_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InsightSnapshot {
    pub meeting_uid: String,
    pub revision: u64,
    pub status: InsightStoreStatus,
    pub items: Vec<InsightItem>,
    pub insertion_receipts: Vec<InsightInsertionReceipt>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightReviewUpdate {
    pub review_state: Option<InsightReviewState>,
    pub edited_text: Option<String>,
    #[serde(default)]
    pub clear_edit: bool,
    pub edit_claim: Option<String>,
    #[serde(default)]
    pub release_edit_claim: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightInsertionRequest {
    pub attempt_id: String,
    pub item_id: String,
    pub item_revision: u64,
    pub before_notes_hash: String,
    pub intended_notes: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InsightInsertionPreparation {
    pub snapshot: InsightSnapshot,
    pub receipt: InsightInsertionReceipt,
    pub stage: bool,
    pub intended_notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightSourceReassociation {
    pub item_id: String,
    pub item_revision: u64,
    pub source_index: u32,
    pub source: InsightSource,
    pub evidence_cutoff_ms: u64,
    pub edit_claim: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InsightDocument {
    schema_version: u32,
    meeting_uid: String,
    revision: u64,
    items: Vec<InsightItem>,
    insertion_receipts: Vec<InsightInsertionReceipt>,
}

fn invalid(message: &str) -> PennelError {
    PennelError::storage(message)
}

impl std::ops::Deref for InsightItem {
    type Target = InsightCandidate;
    fn deref(&self) -> &Self::Target {
        &self.proposal
    }
}

impl std::ops::DerefMut for InsightItem {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.proposal
    }
}

impl InsightItem {
    pub fn display_text(&self) -> &str {
        self.edited_text.as_deref().unwrap_or(&self.proposal.text)
    }
    fn protected(&self) -> bool {
        self.review_state != InsightReviewState::Proposed
            || self.edited_text.is_some()
            || self.edit_claim.is_some()
            || self.source_reassociated
    }
}

fn hash(bytes: &[u8]) -> String {
    crate::meeting_utterances::digest(bytes)
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}
fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn insight_dedup_key(candidate: &InsightCandidate) -> String {
    let primary = candidate
        .sources
        .iter()
        .find_map(|source| {
            source.quote.find(&candidate.text).map(|offset| {
                (
                    &source.utterance_id,
                    &source.source_fingerprint,
                    u64::from(source.start_byte) + offset as u64,
                )
            })
        })
        .or_else(|| {
            candidate.sources.first().map(|source| {
                (
                    &source.utterance_id,
                    &source.source_fingerprint,
                    u64::from(source.start_byte),
                )
            })
        });
    hash(
        serde_json::to_string(&(candidate.kind, normalized(&candidate.text), primary))
            .expect("primitive proposal identity")
            .as_bytes(),
    )
}

fn prior_semantics(candidate: &InsightCandidate) -> Vec<String> {
    let mut prior: Vec<_> = candidate
        .prior_evidence
        .iter()
        .map(|prior| {
            serde_json::to_string(&(prior.kind, &prior.source)).expect("primitive prior evidence")
        })
        .collect();
    prior.sort();
    prior.dedup();
    prior
}

fn semantic_key(candidate: &InsightCandidate) -> String {
    hash(
        serde_json::to_string(&(
            candidate.kind,
            candidate.commitment,
            normalized(&candidate.text),
            candidate
                .owner
                .as_ref()
                .map(|owner| normalized(&owner.text)),
            candidate.due.as_ref().map(|due| normalized(&due.text)),
            prior_semantics(candidate),
        ))
        .expect("primitive suggestion meaning")
        .as_bytes(),
    )
}

fn unique_prior_evidence(candidate: &InsightCandidate) -> HashMap<String, &InsightPriorEvidence> {
    candidate
        .prior_evidence
        .iter()
        .map(|prior| {
            (
                serde_json::to_string(&(prior.kind, &prior.source))
                    .expect("primitive prior evidence"),
                prior,
            )
        })
        .collect::<HashMap<_, _>>()
}

fn same_prior_evidence(
    left: &InsightCandidate,
    right: &InsightCandidate,
    current: &MeetingTranscriptSnapshot,
) -> bool {
    let left = unique_prior_evidence(left);
    let mut right = unique_prior_evidence(right)
        .into_values()
        .map(Some)
        .collect::<Vec<_>>();
    if left.len() != right.len() {
        return false;
    }
    for prior in left.into_values() {
        let matches: Vec<_> = right
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                candidate.is_some_and(|candidate| {
                    candidate.kind == prior.kind
                        && same_source_lineage(&prior.source, &candidate.source, current)
                })
            })
            .map(|(index, _)| index)
            .collect();
        if matches.len() != 1 {
            return false;
        }
        right[matches[0]] = None;
    }
    true
}

fn same_meaning(
    left: &InsightCandidate,
    right: &InsightCandidate,
    current: &MeetingTranscriptSnapshot,
) -> bool {
    left.kind == right.kind
        && left.commitment == right.commitment
        && left.text == right.text
        && left.owner.as_ref().map(|detail| &detail.text)
            == right.owner.as_ref().map(|detail| &detail.text)
        && left.due.as_ref().map(|detail| &detail.text)
            == right.due.as_ref().map(|detail| &detail.text)
        && same_prior_evidence(left, right, current)
}

fn primary_source(candidate: &InsightCandidate) -> Option<&InsightSource> {
    candidate
        .sources
        .iter()
        .find(|source| source.quote.contains(&candidate.text))
        .or_else(|| candidate.sources.first())
}

fn same_source_lineage(
    old: &InsightSource,
    new: &InsightSource,
    current: &MeetingTranscriptSnapshot,
) -> bool {
    if old.meeting_uid != new.meeting_uid
        || old.quote != new.quote
        || old.start_byte != new.start_byte
        || old.end_byte != new.end_byte
        || old.start_ms != new.start_ms
        || old.end_ms != new.end_ms
    {
        return false;
    }
    if old.utterance_id == new.utterance_id && old.source_fingerprint == new.source_fingerprint {
        return true;
    }
    let Some(row) = current
        .utterances
        .iter()
        .find(|row| row.metadata.utterance_id == new.utterance_id)
    else {
        return false;
    };
    row.metadata.lineage == [old.utterance_id.clone()]
        && !current
            .utterances
            .iter()
            .any(|row| row.metadata.utterance_id == old.utterance_id)
        && current
            .utterances
            .iter()
            .filter(|row| row.metadata.lineage.contains(&old.utterance_id))
            .count()
            == 1
}

fn retained_key(
    candidate: &InsightCandidate,
    document: &InsightDocument,
    current: &MeetingTranscriptSnapshot,
) -> String {
    let key = insight_dedup_key(candidate);
    if document.items.iter().any(|item| item.dedup_key == key) {
        return key;
    }
    let matching: Vec<_> = document
        .items
        .iter()
        .filter(|item| {
            if !same_meaning(&item.proposal, candidate, current) {
                return false;
            }
            let (Some(old), Some(new)) =
                (primary_source(&item.proposal), primary_source(candidate))
            else {
                return false;
            };
            if !same_source_lineage(old, new, current) {
                return false;
            }
            for (old_detail, new_detail) in [
                (item.owner.as_ref(), candidate.owner.as_ref()),
                (item.due.as_ref(), candidate.due.as_ref()),
            ] {
                if let (Some(old_detail), Some(new_detail)) = (old_detail, new_detail) {
                    let Some(old) = item.sources.get(old_detail.source_index as usize) else {
                        return false;
                    };
                    let Some(new) = candidate.sources.get(new_detail.source_index as usize) else {
                        return false;
                    };
                    if !same_source_lineage(old, new, current) {
                        return false;
                    }
                }
            }
            true
        })
        .collect();
    if matching.len() == 1 {
        matching[0].dedup_key.clone()
    } else {
        key
    }
}

fn linked_change(
    key: &str,
    existing: &InsightItem,
    candidate: &InsightCandidate,
) -> (String, InsightCandidate) {
    let mut candidate = candidate.clone();
    if !candidate.related_item_ids.contains(&existing.id) {
        candidate.related_item_ids.push(existing.id.clone());
    }
    (
        hash(format!("{}:{}", key, semantic_key(&candidate)).as_bytes()),
        candidate,
    )
}

fn detail_valid(detail: &SupportedInsightDetail, sources: &[InsightSource]) -> bool {
    !detail.text.trim().is_empty()
        && sources
            .get(detail.source_index as usize)
            .and_then(|source| {
                source
                    .quote
                    .get(detail.start_byte as usize..detail.end_byte as usize)
            })
            == Some(detail.text.as_str())
}

fn source_shape_valid(source: &InsightSource, uid: &str, cutoff: u64) -> bool {
    source.meeting_uid == uid
        && valid_id(&source.utterance_id)
        && valid_hash(&source.source_fingerprint)
        && source.start_byte < source.end_byte
        && source.end_byte as usize - source.start_byte as usize == source.quote.len()
        && !source.quote.trim().is_empty()
        && source.start_ms <= source.end_ms
        && source.end_ms <= cutoff
}

fn provably_earlier(prior: &InsightSource, event: &InsightSource) -> bool {
    if prior.meeting_uid != event.meeting_uid {
        return false;
    }
    if prior.utterance_id == event.utterance_id {
        prior.end_byte <= event.start_byte
    } else {
        prior.start_ms < prior.end_ms
            && event.start_ms < event.end_ms
            && prior.end_ms <= event.start_ms
    }
}

fn compatible_primary_evidence(primary: &InsightSource, quoted: &InsightSource) -> bool {
    if primary.meeting_uid != quoted.meeting_uid
        || primary.utterance_id != quoted.utterance_id
        || primary.source_fingerprint != quoted.source_fingerprint
        || primary.start_ms != quoted.start_ms
        || primary.end_ms != quoted.end_ms
    {
        return false;
    }
    if primary.start_byte <= quoted.start_byte && quoted.end_byte <= primary.end_byte {
        primary.quote.get(
            (quoted.start_byte - primary.start_byte) as usize
                ..(quoted.end_byte - primary.start_byte) as usize,
        ) == Some(quoted.quote.as_str())
    } else if quoted.start_byte <= primary.start_byte && primary.end_byte <= quoted.end_byte {
        quoted.quote.get(
            (primary.start_byte - quoted.start_byte) as usize
                ..(primary.end_byte - quoted.start_byte) as usize,
        ) == Some(primary.quote.as_str())
    } else {
        false
    }
}

fn resolve_prior_evidence(
    document: &mut InsightDocument,
    touched: &HashSet<String>,
) -> HashSet<String> {
    let targets: Vec<_> = document
        .items
        .iter()
        .filter_map(|item| {
            primary_source(&item.proposal)
                .map(|source| (item.id.clone(), item.kind, source.clone()))
        })
        .collect();
    let mut retained = HashSet::new();
    for item in &mut document.items {
        if !touched.contains(&item.id) || item.protected() {
            continue;
        }
        let Some(event) = primary_source(&item.proposal).cloned() else {
            continue;
        };
        let item_id = item.id.clone();
        let mut resolved = Vec::new();
        for prior in &mut item.proposal.prior_evidence {
            prior.target_item_id = None;
            if !provably_earlier(&prior.source, &event) {
                continue;
            }
            let matches: Vec<_> = targets
                .iter()
                .filter(|(id, kind, primary)| {
                    id != &item_id
                        && *kind == prior.kind
                        && compatible_primary_evidence(primary, &prior.source)
                        && provably_earlier(primary, &event)
                })
                .collect();
            for (id, _, _) in &matches {
                retained.insert(id.clone());
            }
            if matches.len() == 1 {
                let id = matches[0].0.clone();
                prior.target_item_id = Some(id.clone());
                resolved.push(id);
            }
        }
        for id in resolved {
            if !item.proposal.related_item_ids.contains(&id) {
                item.proposal.related_item_ids.push(id);
            }
        }
    }
    retained
}

fn proposal_valid(candidate: &InsightCandidate, uid: &str) -> bool {
    !candidate.text.trim().is_empty()
        && !candidate.sources.is_empty()
        && valid_hash(&candidate.transcript_digest)
        && candidate
            .sources
            .iter()
            .all(|source| source_shape_valid(source, uid, candidate.evidence_cutoff_ms))
        && candidate.prior_evidence.iter().all(|prior| {
            source_shape_valid(&prior.source, uid, candidate.evidence_cutoff_ms)
                && prior.target_item_id.as_ref().is_none_or(|id| valid_id(id))
        })
        && candidate
            .owner
            .as_ref()
            .is_none_or(|detail| detail_valid(detail, &candidate.sources))
        && candidate
            .due
            .as_ref()
            .is_none_or(|detail| detail_valid(detail, &candidate.sources))
        && (candidate
            .sources
            .iter()
            .any(|source| source.quote.contains(&candidate.text))
            || candidate
                .sources
                .iter()
                .map(|source| source.quote.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                == candidate.text)
        && !candidate.processing.qualification_id.trim().is_empty()
        && !candidate.processing.language_policy.trim().is_empty()
        && !candidate.processing.task.trim().is_empty()
        && !candidate
            .processing
            .identity
            .requested_model
            .trim()
            .is_empty()
        && !candidate.processing.identity.endpoint.trim().is_empty()
        && candidate.related_item_ids.iter().all(|id| valid_id(id))
}

fn source_state(
    source: &InsightSource,
    current: Option<&MeetingTranscriptSnapshot>,
) -> InsightSourceState {
    let Some(current) = current else {
        return InsightSourceState::Unavailable;
    };
    if !matches!(
        current.metadata_status,
        TranscriptMetadataStatus::Valid | TranscriptMetadataStatus::Absent
    ) {
        return InsightSourceState::Unavailable;
    }
    let Some(row) = current
        .utterances
        .iter()
        .find(|row| row.metadata.utterance_id == source.utterance_id)
    else {
        return InsightSourceState::Missing;
    };
    if current.meeting_uid != source.meeting_uid
        || row.segment.failed
        || utterance_evidence_revision(row) != source.source_fingerprint
        || row
            .segment
            .text
            .get(source.start_byte as usize..source.end_byte as usize)
            != Some(source.quote.as_str())
        || row.segment.start_ms != source.start_ms
        || row.segment.end_ms != source.end_ms
    {
        return InsightSourceState::Stale;
    }
    InsightSourceState::Current
}

fn item_source_state(
    item: &InsightItem,
    current: Option<&MeetingTranscriptSnapshot>,
) -> InsightSourceState {
    let states: Vec<_> = item
        .sources
        .iter()
        .chain(item.prior_evidence.iter().map(|prior| &prior.source))
        .map(|source| source_state(source, current))
        .collect();
    for state in [
        InsightSourceState::Unavailable,
        InsightSourceState::Missing,
        InsightSourceState::Stale,
    ] {
        if states.contains(&state) {
            return state;
        }
    }
    InsightSourceState::Current
}

fn source_compatible(
    frozen: &MeetingTranscriptSnapshot,
    current: &MeetingTranscriptSnapshot,
) -> bool {
    if frozen.meeting_uid != current.meeting_uid
        || !matches!(
            frozen.metadata_status,
            TranscriptMetadataStatus::Valid | TranscriptMetadataStatus::Absent
        )
        || !matches!(
            current.metadata_status,
            TranscriptMetadataStatus::Valid | TranscriptMetadataStatus::Absent
        )
    {
        return false;
    }
    let current: HashMap<_, _> = current
        .utterances
        .iter()
        .filter(|row| !row.segment.failed)
        .map(|row| (row.metadata.utterance_id.as_str(), row))
        .collect();
    frozen
        .utterances
        .iter()
        .filter(|row| !row.segment.failed)
        .all(|row| {
            current
                .get(row.metadata.utterance_id.as_str())
                .is_some_and(|now| {
                    utterance_evidence_revision(row) == utterance_evidence_revision(now)
                        && row.segment.text == now.segment.text
                })
        })
}

fn assessment_pairs<'a>(
    scope: &'a [InsightAssessmentScope],
    frozen: &MeetingTranscriptSnapshot,
) -> Result<HashSet<(&'a str, InsightKind)>, PennelError> {
    let known: HashSet<_> = frozen
        .utterances
        .iter()
        .map(|row| row.metadata.utterance_id.as_str())
        .collect();
    let mut pairs = HashSet::new();
    for assessed in scope {
        if !known.contains(assessed.utterance_id.as_str()) {
            return Err(invalid(
                "The assessed passage does not belong to the frozen transcript.",
            ));
        }
        for kind in &assessed.kinds {
            pairs.insert((assessed.utterance_id.as_str(), *kind));
        }
    }
    Ok(pairs)
}

fn single_lineage_aliases(current: &MeetingTranscriptSnapshot) -> HashMap<&str, &MeetingUtterance> {
    let ids: HashSet<_> = current
        .utterances
        .iter()
        .map(|row| row.metadata.utterance_id.as_str())
        .collect();
    let mut counts = HashMap::new();
    for row in &current.utterances {
        for old in &row.metadata.lineage {
            *counts.entry(old.as_str()).or_insert(0usize) += 1;
        }
    }
    current
        .utterances
        .iter()
        .filter_map(|row| {
            if row.metadata.lineage.len() != 1 {
                return None;
            }
            let old = row.metadata.lineage[0].as_str();
            (!ids.contains(old) && counts.get(old) == Some(&1)).then_some((old, row))
        })
        .collect()
}

fn item_assessed(
    item: &InsightItem,
    pairs: &HashSet<(&str, InsightKind)>,
    aliases: &HashMap<&str, &MeetingUtterance>,
) -> bool {
    let Some(primary) = primary_source(&item.proposal) else {
        return false;
    };
    if pairs.contains(&(primary.utterance_id.as_str(), item.kind)) {
        return true;
    }
    let Some(row) = aliases.get(primary.utterance_id.as_str()) else {
        return false;
    };
    pairs.contains(&(row.metadata.utterance_id.as_str(), item.kind))
        && row.segment.start_ms == primary.start_ms
        && row.segment.end_ms == primary.end_ms
        && !row.segment.failed
        && row
            .segment
            .text
            .get(primary.start_byte as usize..primary.end_byte as usize)
            == Some(primary.quote.as_str())
}

fn document_valid(document: &InsightDocument) -> bool {
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    if document.schema_version != 1
        || document.revision == 0
        || !crate::meetings::valid_uid(&document.meeting_uid)
        || document.items.iter().any(|item| {
            !valid_id(&item.id)
                || item.revision == 0
                || !valid_hash(&item.dedup_key)
                || !ids.insert(item.id.clone())
                || !keys.insert(item.dedup_key.clone())
                || !proposal_valid(&item.proposal, &document.meeting_uid)
                || item
                    .edited_text
                    .as_ref()
                    .is_some_and(|text| text.trim().is_empty())
                || item
                    .edit_claim
                    .as_ref()
                    .is_some_and(|claim| !valid_id(claim))
                || item
                    .source_history
                    .iter()
                    .any(|source| !source_shape_valid(source, &document.meeting_uid, u64::MAX))
        })
    {
        return false;
    }
    let mut attempts = HashSet::new();
    document.items.iter().all(|item| {
        item.related_item_ids
            .iter()
            .all(|id| ids.contains(id) && id != &item.id)
            && item.prior_evidence.iter().all(|prior| {
                prior.target_item_id.as_ref().is_none_or(|id| {
                    id != &item.id
                        && document
                            .items
                            .iter()
                            .any(|target| target.id == *id && target.kind == prior.kind)
                })
            })
    }) && document.insertion_receipts.iter().all(|receipt| {
        valid_id(&receipt.attempt_id)
            && attempts.insert(receipt.attempt_id.clone())
            && ids.contains(&receipt.item_id)
            && receipt.item_revision > 0
            && valid_hash(&receipt.before_notes_hash)
            && valid_hash(&receipt.intended_notes_hash)
            && !receipt.markdown.trim().is_empty()
            && receipt.manual_resolution.as_ref().is_none_or(|resolution| {
                matches!(
                    resolution.state,
                    InsightInsertionState::Acknowledged | InsightInsertionState::NotInserted
                ) && resolution.state == receipt.state
                    && valid_hash(&resolution.notes_hash)
            })
    })
}

fn empty_document(uid: &str) -> InsightDocument {
    InsightDocument {
        schema_version: 1,
        meeting_uid: uid.into(),
        revision: 0,
        items: Vec::new(),
        insertion_receipts: Vec::new(),
    }
}

fn read_document(
    root: &Path,
    uid: &str,
) -> Result<(InsightStoreStatus, Option<InsightDocument>), PennelError> {
    let path = root.join(INSIGHTS_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((InsightStoreStatus::Missing, Some(empty_document(uid))));
        }
        Err(_) => return Err(invalid("The reviewed suggestions could not be read.")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok((InsightStoreStatus::Corrupt, None));
    }
    if metadata.len() > MAX_INSIGHTS_BYTES as u64 {
        return Ok((InsightStoreStatus::TooLarge, None));
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_INSIGHTS_BYTES));
    fs::File::open(&path)
        .and_then(|file| {
            file.take(MAX_INSIGHTS_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| invalid("The reviewed suggestions could not be read."))?;
    if bytes.len() > MAX_INSIGHTS_BYTES {
        return Ok((InsightStoreStatus::TooLarge, None));
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok((InsightStoreStatus::Corrupt, None));
    };
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version != 1)
    {
        return Ok((InsightStoreStatus::Unsupported, None));
    }
    // Initial v1 files held passage-level proposals before the explicit scope tag existed.
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && let Some(items) = value
            .get_mut("items")
            .and_then(serde_json::Value::as_array_mut)
    {
        for item in items {
            if let Some(item) = item.as_object_mut() {
                item.entry("extraction_scope")
                    .or_insert_with(|| serde_json::json!("passage"));
                item.entry("prior_evidence")
                    .or_insert_with(|| serde_json::json!([]));
            }
        }
    }
    let Ok(document) = serde_json::from_value::<InsightDocument>(value.clone()) else {
        return Ok((InsightStoreStatus::Corrupt, None));
    };
    if serde_json::to_value(&document).ok().as_ref() != Some(&value) {
        return Ok((InsightStoreStatus::Unsupported, None));
    }
    if document.meeting_uid != uid {
        return Ok((InsightStoreStatus::WrongMeeting, None));
    }
    if !document_valid(&document) {
        return Ok((InsightStoreStatus::Corrupt, None));
    }
    Ok((InsightStoreStatus::Ready, Some(document)))
}

fn snapshot(
    status: InsightStoreStatus,
    uid: &str,
    document: Option<InsightDocument>,
    current: Option<&MeetingTranscriptSnapshot>,
) -> InsightSnapshot {
    let mut document = document.unwrap_or_else(|| empty_document(uid));
    for item in &mut document.items {
        item.source_state = item_source_state(item, current);
    }
    InsightSnapshot {
        meeting_uid: uid.into(),
        revision: document.revision,
        status,
        items: document.items,
        insertion_receipts: document.insertion_receipts,
    }
}

fn writable(
    status: InsightStoreStatus,
    document: Option<InsightDocument>,
    expected: u64,
) -> Result<InsightDocument, PennelError> {
    if !matches!(
        status,
        InsightStoreStatus::Missing | InsightStoreStatus::Ready
    ) {
        return Err(invalid(
            "This suggestion file is read-only. Keep its existing data and resolve the format before editing.",
        ));
    }
    let document = document.ok_or_else(|| invalid("The suggestion file is unavailable."))?;
    if document.revision != expected {
        return Err(invalid(
            "The reviewed suggestions changed. Reload them before saving.",
        ));
    }
    Ok(document)
}

fn current_notes(root: &Path) -> Result<String, PennelError> {
    match fs::read_to_string(root.join("notes.md")) {
        Ok(notes) => Ok(notes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err(invalid("The acknowledged notes could not be read.")),
    }
}

#[cfg(test)]
thread_local! { static FAIL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
pub(crate) fn fail_next_insight_write() {
    FAIL_WRITE.with(|flag| flag.set(true));
}

fn save_document(
    root: &Path,
    document: &mut InsightDocument,
    cancellation: Option<&InferenceCancellation>,
) -> Result<(), PennelError> {
    document.revision = document
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("The suggestion revision cannot be advanced."))?;
    if !document_valid(document) {
        return Err(invalid("The reviewed suggestions contain invalid data."));
    }
    let bytes = serde_json::to_vec(document)
        .map_err(|_| invalid("The reviewed suggestions could not be encoded."))?;
    if bytes.len() > MAX_INSIGHTS_BYTES {
        return Err(invalid(
            "The reviewed suggestions exceed the portable storage limit. Existing work was kept.",
        ));
    }
    #[cfg(test)]
    if FAIL_WRITE.with(|flag| flag.replace(false)) {
        return Err(invalid("Owned suggestion write failure."));
    }
    if cancellation.is_some_and(InferenceCancellation::is_cancelled) {
        return Err(PennelError::Cancelled);
    }
    durable_write(&root.join(INSIGHTS_FILE), &bytes)
}

pub(crate) fn require_current_source(
    root: &Path,
    current: &MeetingTranscriptSnapshot,
) -> Result<(), PennelError> {
    let text = match fs::read(root.join("transcript.md")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err(invalid("The finalized source could not be read.")),
    };
    let metadata = match fs::read(root.join(crate::meeting_utterances::MANIFEST)) {
        Ok(bytes) => Some(hash(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(invalid("The finalized source metadata could not be read.")),
    };
    if hash(&text) != current.transcript_digest || metadata != current.metadata_digest {
        return Err(invalid(
            "The finalized source changed before suggestions could be saved.",
        ));
    }
    Ok(())
}

impl MeetingStore {
    pub fn read_insights(&self, id: &str) -> Result<InsightSnapshot, PennelError> {
        let current = self.read_transcript_snapshot(id).ok();
        let uid = self
            .uid_of(id)
            .or_else(|| current.as_ref().map(|source| source.meeting_uid.clone()))
            .ok_or(PennelError::NotFound)?;
        let root = self.dir_of(id);
        let gate = crate::meetings::replacement_gate(&root);
        let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
        let (status, document) = read_document(&root, &uid)?;
        Ok(snapshot(status, &uid, document, current.as_ref()))
    }

    fn mutate_insights(
        &self,
        id: &str,
        expected_revision: u64,
        cancellation: Option<&InferenceCancellation>,
        update: impl FnOnce(
            &mut InsightDocument,
            Option<&MeetingTranscriptSnapshot>,
            &Path,
        ) -> Result<(), PennelError>,
    ) -> Result<InsightSnapshot, PennelError> {
        if cancellation.is_some_and(InferenceCancellation::is_cancelled) {
            return Err(PennelError::Cancelled);
        }
        let current = self.read_transcript_snapshot(id).ok();
        let uid = self.uid_of(id).ok_or_else(|| {
            invalid("This meeting needs a stable identity before reviewing suggestions.")
        })?;
        let root = self.dir_of(id);
        let gate = crate::meetings::replacement_gate(&root);
        let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
        let (status, document) = read_document(&root, &uid)?;
        let mut document = writable(status, document, expected_revision)?;
        update(&mut document, current.as_ref(), &root)?;
        save_document(&root, &mut document, cancellation)?;
        Ok(snapshot(
            InsightStoreStatus::Ready,
            &uid,
            Some(document),
            current.as_ref(),
        ))
    }

    /// Compatibility callers declare all four kinds assessed for every frozen centre.
    #[allow(dead_code)]
    pub fn merge_insight_candidates(
        &self,
        id: &str,
        expected_revision: u64,
        candidates: &[InsightCandidate],
        frozen: &MeetingTranscriptSnapshot,
        cancellation: &InferenceCancellation,
    ) -> Result<InsightSnapshot, PennelError> {
        let scope: Vec<_> = frozen
            .utterances
            .iter()
            .map(|row| InsightAssessmentScope {
                utterance_id: row.metadata.utterance_id.clone(),
                kinds: vec![
                    InsightKind::Action,
                    InsightKind::Decision,
                    InsightKind::OpenQuestion,
                    InsightKind::ChangedCommitment,
                ],
            })
            .collect();
        self.merge_scoped_insight_candidates(
            id,
            expected_revision,
            candidates,
            &scope,
            frozen,
            cancellation,
        )
    }

    pub fn merge_scoped_insight_candidates(
        &self,
        id: &str,
        expected_revision: u64,
        candidates: &[InsightCandidate],
        scope: &[InsightAssessmentScope],
        frozen: &MeetingTranscriptSnapshot,
        cancellation: &InferenceCancellation,
    ) -> Result<InsightSnapshot, PennelError> {
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        let pairs = assessment_pairs(scope, frozen)?;
        if pairs.is_empty() {
            if !candidates.is_empty() {
                return Err(invalid("Suggestions require an assessed passage and kind."));
            }
            let existing = self.read_insights(id)?;
            if existing.meeting_uid != frozen.meeting_uid {
                return Err(invalid(
                    "The unchecked source belongs to a different meeting.",
                ));
            }
            return Ok(existing);
        }
        self.mutate_insights(id, expected_revision, Some(cancellation), |document, current, root| {
            let current = current.ok_or_else(|| invalid("The finalized transcript is unavailable."))?;
            if !source_compatible(frozen, current) { return Err(invalid("The source changed after this analysis began.")); }
            require_current_source(root, current)?;
            let mut incoming: HashMap<String, InsightCandidate> = HashMap::new();
            for candidate in candidates {
                if !proposal_valid(candidate, &document.meeting_uid)
                    || primary_source(candidate).is_none_or(|source|!pairs.contains(&(source.utterance_id.as_str(),candidate.kind)))
                    || candidate.transcript_revision != frozen.revision || candidate.transcript_digest != frozen.transcript_digest
                    || candidate.evidence_cutoff_ms > frozen.utterances.iter().filter(|row| !row.segment.failed).map(|row| row.segment.end_ms).max().unwrap_or(0)
                    || candidate.sources.iter().chain(candidate.prior_evidence.iter().map(|prior|&prior.source))
                        .any(|source| source_state(source, Some(frozen)) != InsightSourceState::Current)
                    || candidate.sources.iter().any(|source| source.end_ms > candidate.evidence_cutoff_ms) {
                    return Err(invalid("A suggestion contains unsupported source, owner or deadline information."));
                }
                let key = retained_key(candidate, document, current);
                if let Some(previous) = incoming.get(&key) {
                    if semantic_key(previous) != semantic_key(candidate) { return Err(invalid("The analysis returned conflicting suggestions for the same evidence.")); }
                    if previous.evidence_cutoff_ms >= candidate.evidence_cutoff_ms { continue; }
                }
                let mut candidate=candidate.clone();
                for prior in &mut candidate.prior_evidence {prior.target_item_id=None;}
                incoming.insert(key, candidate);
            }
            let linked: HashSet<_> = document.items.iter().flat_map(|item| item.related_item_ids.iter().cloned())
                .chain(document.items.iter().flat_map(|item|item.prior_evidence.iter().filter_map(|prior|prior.target_item_id.clone())))
                .chain(candidates.iter().flat_map(|candidate| candidate.related_item_ids.iter().cloned()))
                .chain(document.insertion_receipts.iter().map(|receipt| receipt.item_id.clone())).collect();
            let mut retained:HashSet<_> = incoming.keys().cloned().collect();
            for (key,candidate) in &incoming {
                if let Some(item) = document.items.iter().find(|item| item.dedup_key == *key)
                    && item.protected() && !same_meaning(&item.proposal,candidate,current)
                    && candidate.evidence_cutoff_ms > item.evidence_cutoff_ms {
                    retained.insert(linked_change(key,item,candidate).0);
                }
            }
            let aliases=single_lineage_aliases(current);
            let positives:HashSet<_>=candidates.iter().filter_map(|candidate|primary_source(candidate)
                .map(|source|(source.utterance_id.as_str(),candidate.kind))).collect();
            // A positive partial extraction does not assert the absence of other events.
            let retiring:HashSet<_>=document.items.iter().filter(|item|!(item.protected() || linked.contains(&item.id) || retained.contains(&item.dedup_key)
                || item_assessed(item,&positives,&aliases) || !item_assessed(item,&pairs,&aliases))).map(|item|item.id.clone()).collect();
            let mut incoming: Vec<_> = incoming.into_iter().collect(); incoming.sort_by(|left,right| left.0.cmp(&right.0));
            let mut touched=HashSet::new();
            for (mut key, mut candidate) in incoming {
                if let Some(index) = document.items.iter().position(|item| item.dedup_key == key) {
                    let existing = &document.items[index];
                    if existing.protected() {
                        if same_meaning(&existing.proposal,&candidate,current)
                            || candidate.evidence_cutoff_ms <= existing.evidence_cutoff_ms { continue; }
                        (key,candidate) = linked_change(&key, existing, &candidate);
                        if let Some(change) = document.items.iter_mut().find(|item| item.dedup_key == key) {
                            if !change.protected() {
                                change.proposal=candidate;
                                change.revision=change.revision.checked_add(1).ok_or_else(||invalid("The item revision cannot be advanced."))?;
                                touched.insert(change.id.clone());
                            }
                            continue;
                        }
                    } else {
                        let item = &mut document.items[index];
                        item.proposal = candidate;
                        item.revision = item.revision.checked_add(1).ok_or_else(|| invalid("The item revision cannot be advanced."))?;
                        touched.insert(item.id.clone());
                        continue;
                    }
                }
                document.items.push(InsightItem { id: uuid::Uuid::now_v7().to_string(), dedup_key: key, revision: 1, proposal: candidate,
                    review_state: InsightReviewState::Proposed, edited_text: None, edit_claim: None, source_state: InsightSourceState::Current,
                    source_history: Vec::new(), source_reassociated: false });
                touched.insert(document.items.last().expect("newly added item").id.clone());
            }
            // Resolve only after every new id exists, before retiring unreviewed history.
            let prior_targets=resolve_prior_evidence(document,&touched);
            document.items.retain(|item|!retiring.contains(&item.id)||prior_targets.contains(&item.id));
            Ok(())
        })
    }

    pub fn claim_insight_edit(
        &self,
        id: &str,
        store_revision: u64,
        item_id: &str,
        item_revision: u64,
        claim_id: &str,
    ) -> Result<InsightSnapshot, PennelError> {
        if !valid_id(claim_id) {
            return Err(invalid("The edit claim is invalid."));
        }
        self.mutate_insights(id, store_revision, None, |document, _, _| {
            let item = document
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .ok_or(PennelError::NotFound)?;
            if item.revision != item_revision {
                return Err(invalid("This suggestion changed before editing began."));
            }
            if item
                .edit_claim
                .as_deref()
                .is_some_and(|claim| claim != claim_id)
            {
                return Err(invalid(
                    "This suggestion has another retained edit. Resume or release it first.",
                ));
            }
            item.edit_claim = Some(claim_id.into());
            item.revision = item
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("The item revision cannot be advanced."))?;
            Ok(())
        })
    }

    pub fn update_insight_review(
        &self,
        id: &str,
        store_revision: u64,
        item_id: &str,
        item_revision: u64,
        update: &InsightReviewUpdate,
    ) -> Result<InsightSnapshot, PennelError> {
        self.mutate_insights(id, store_revision, None, |document, _, _| {
            let item = document
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .ok_or(PennelError::NotFound)?;
            if item.revision != item_revision {
                return Err(invalid(
                    "This suggestion changed before the review was saved.",
                ));
            }
            if item.edit_claim.is_some() && item.edit_claim != update.edit_claim {
                return Err(invalid(
                    "This suggestion has a retained edit. Its edit claim is required.",
                ));
            }
            if update.clear_edit && update.edited_text.is_some() {
                return Err(invalid("Choose either edited text or reset, not both."));
            }
            if update
                .edited_text
                .as_ref()
                .is_some_and(|text| text.trim().is_empty())
            {
                return Err(invalid("The edited suggestion cannot be empty."));
            }
            if let Some(state) = update.review_state {
                item.review_state = state;
            }
            if let Some(text) = &update.edited_text {
                item.edited_text = Some(text.clone());
            }
            if update.clear_edit {
                item.edited_text = None;
            }
            if update.release_edit_claim {
                item.edit_claim = None;
            }
            item.revision = item
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("The item revision cannot be advanced."))?;
            Ok(())
        })
    }

    pub fn export_insights_markdown(&self, id: &str) -> Result<String, PennelError> {
        let snapshot = self.read_insights(id)?;
        if !matches!(
            snapshot.status,
            InsightStoreStatus::Missing | InsightStoreStatus::Ready
        ) {
            return Err(invalid("This suggestion file cannot be exported safely."));
        }
        Ok(snapshot
            .items
            .iter()
            .filter(|item| item.review_state == InsightReviewState::Accepted)
            .map(insight_export_markdown)
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    pub fn reassociate_insight_source(
        &self,
        id: &str,
        store_revision: u64,
        request: &InsightSourceReassociation,
    ) -> Result<InsightSnapshot, PennelError> {
        self.mutate_insights(id, store_revision, None, |document, current, root| {
            let current = current.ok_or_else(|| invalid("The current transcript is unavailable."))?;
            require_current_source(root, current)?;
            if source_state(&request.source, Some(current)) != InsightSourceState::Current {
                return Err(invalid("The replacement source is not a current finalized passage."));
            }
            let item = document.items.iter_mut().find(|item| item.id == request.item_id).ok_or(PennelError::NotFound)?;
            if item.revision != request.item_revision || item.edit_claim.is_some() && item.edit_claim != request.edit_claim {
                return Err(invalid("The suggestion changed or has another retained edit."));
            }
            let previous = item.sources.get(request.source_index as usize).ok_or_else(|| invalid("The source selection is invalid."))?.clone();
            if previous.quote != request.source.quote {
                return Err(invalid("Choose a finalized passage matching the frozen quote. Changed wording needs a separate review."));
            }
            let cutoff = request.evidence_cutoff_ms;
            if cutoff < request.source.end_ms || cutoff < item.evidence_cutoff_ms {
                return Err(invalid("The reassociated source cutoff is invalid."));
            }
            if !item.source_history.contains(&previous) { item.source_history.push(previous); }
            item.proposal.sources[request.source_index as usize] = request.source.clone();
            item.proposal.evidence_cutoff_ms = cutoff;
            item.source_reassociated = true;
            item.revision = item.revision.checked_add(1).ok_or_else(|| invalid("The item revision cannot be advanced."))?;
            Ok(())
        })
    }

    pub fn prepare_insight_insertion(
        &self,
        id: &str,
        store_revision: u64,
        request: &InsightInsertionRequest,
    ) -> Result<InsightInsertionPreparation, PennelError> {
        if !valid_id(&request.attempt_id)
            || !valid_hash(&request.before_notes_hash)
            || request.markdown.trim().is_empty()
            || !request.intended_notes.contains(&request.markdown)
        {
            return Err(invalid("The explicit note insertion is invalid."));
        }
        let current = self.read_transcript_snapshot(id).ok();
        let uid = self.uid_of(id).ok_or(PennelError::NotFound)?;
        let root = self.dir_of(id);
        let gate = crate::meetings::replacement_gate(&root);
        let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
        let (status, loaded) = read_document(&root, &uid)?;
        if !matches!(status, InsightStoreStatus::Ready) {
            return Err(invalid(
                "Review a saved suggestion before adding it to notes.",
            ));
        }
        let mut document = loaded.ok_or_else(|| invalid("The suggestion file is unavailable."))?;
        let intended_hash = hash(request.intended_notes.as_bytes());
        if let Some(receipt) = document
            .insertion_receipts
            .iter()
            .find(|receipt| receipt.attempt_id == request.attempt_id)
        {
            if receipt.item_id != request.item_id
                || receipt.item_revision != request.item_revision
                || receipt.markdown != request.markdown
                || receipt.before_notes_hash != request.before_notes_hash
                || receipt.intended_notes_hash != intended_hash
            {
                return Err(invalid(
                    "This insertion attempt already belongs to different note content.",
                ));
            }
            let receipt = receipt.clone();
            return Ok(InsightInsertionPreparation {
                snapshot: snapshot(status, &uid, Some(document), current.as_ref()),
                receipt,
                stage: false,
                intended_notes: None,
            });
        }
        if let Some(receipt) = document.insertion_receipts.iter().find(|receipt| {
            receipt.item_id == request.item_id
                && receipt.state != InsightInsertionState::NotInserted
        }) {
            let receipt = receipt.clone();
            return Ok(InsightInsertionPreparation {
                snapshot: snapshot(status, &uid, Some(document), current.as_ref()),
                receipt,
                stage: false,
                intended_notes: None,
            });
        }
        if document.revision != store_revision {
            return Err(invalid(
                "The reviewed suggestions changed before insertion.",
            ));
        }
        if hash(current_notes(&root)?.as_bytes()) != request.before_notes_hash {
            return Err(invalid(
                "The acknowledged notes changed before insertion. Keep the current draft and reload their saved state.",
            ));
        }
        if intended_hash == request.before_notes_hash {
            return Err(invalid(
                "The note insertion does not change the acknowledged notes.",
            ));
        }
        let item = document
            .items
            .iter_mut()
            .find(|item| item.id == request.item_id)
            .ok_or(PennelError::NotFound)?;
        if item.revision != request.item_revision || item.edit_claim.is_some() {
            return Err(invalid(
                "Finish the retained suggestion edit before inserting it.",
            ));
        }
        if item.review_state == InsightReviewState::Dismissed {
            return Err(invalid(
                "Reopen this dismissed suggestion before inserting it.",
            ));
        }
        item.review_state = InsightReviewState::Accepted;
        item.revision = item
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("The item revision cannot be advanced."))?;
        let receipt = InsightInsertionReceipt {
            attempt_id: request.attempt_id.clone(),
            item_id: request.item_id.clone(),
            item_revision: request.item_revision,
            before_notes_hash: request.before_notes_hash.clone(),
            intended_notes_hash: intended_hash,
            markdown: request.markdown.clone(),
            state: InsightInsertionState::Pending,
            manual_resolution: None,
        };
        document.insertion_receipts.push(receipt.clone());
        save_document(&root, &mut document, None)?;
        Ok(InsightInsertionPreparation {
            snapshot: snapshot(
                InsightStoreStatus::Ready,
                &uid,
                Some(document),
                current.as_ref(),
            ),
            receipt,
            stage: true,
            intended_notes: Some(request.intended_notes.clone()),
        })
    }

    pub fn validate_insight_note_write(
        &self,
        id: &str,
        attempt_id: &str,
        text: &str,
    ) -> Result<bool, PennelError> {
        let snapshot = self.read_insights(id)?;
        if snapshot.status != InsightStoreStatus::Ready {
            return Err(invalid("The insertion receipt cannot be verified."));
        }
        let receipt = snapshot
            .insertion_receipts
            .iter()
            .find(|receipt| receipt.attempt_id == attempt_id)
            .ok_or_else(|| {
                invalid("The insertion attempt belongs to no saved suggestion in this meeting.")
            })?;
        Ok(hash(text.as_bytes()) == receipt.intended_notes_hash)
    }

    pub fn acknowledge_insight_insertion(
        &self,
        id: &str,
        store_revision: u64,
        attempt_id: &str,
        actual_notes_hash: &str,
    ) -> Result<InsightSnapshot, PennelError> {
        if !valid_hash(actual_notes_hash) {
            return Err(invalid("The note acknowledgement is invalid."));
        }
        let existing = self.read_insights(id)?;
        if existing.status == InsightStoreStatus::Ready
            && existing.insertion_receipts.iter().any(|receipt| {
                receipt.attempt_id == attempt_id
                    && receipt.state == InsightInsertionState::Acknowledged
                    && receipt.intended_notes_hash == actual_notes_hash
            })
        {
            return Ok(existing);
        }
        self.mutate_insights(id, store_revision, None, |document, _, root| {
            if hash(current_notes(root)?.as_bytes()) != actual_notes_hash {
                return Err(invalid(
                    "The acknowledged note content is no longer the saved version.",
                ));
            }
            let receipt = document
                .insertion_receipts
                .iter_mut()
                .find(|receipt| receipt.attempt_id == attempt_id)
                .ok_or(PennelError::NotFound)?;
            let resolved = resolve_receipt(receipt, actual_notes_hash);
            if resolved != receipt.state {
                receipt.manual_resolution = None;
            }
            receipt.state = resolved;
            Ok(())
        })
    }

    pub fn reconcile_insight_insertions(
        &self,
        id: &str,
        store_revision: u64,
    ) -> Result<InsightSnapshot, PennelError> {
        let existing = self.read_insights(id)?;
        if existing.status != InsightStoreStatus::Ready {
            return Err(invalid(
                "The insertion receipts cannot be recovered safely.",
            ));
        }
        if !existing.insertion_receipts.iter().any(|receipt| {
            matches!(
                receipt.state,
                InsightInsertionState::Pending | InsightInsertionState::NeedsReview
            )
        }) {
            return Ok(existing);
        }
        self.mutate_insights(id, store_revision, None, |document, _, root| {
            let notes_hash = hash(current_notes(root)?.as_bytes());
            for receipt in &mut document.insertion_receipts {
                if matches!(
                    receipt.state,
                    InsightInsertionState::Pending | InsightInsertionState::NeedsReview
                ) {
                    receipt.state = resolve_receipt(receipt, &notes_hash);
                }
            }
            Ok(())
        })
    }

    pub fn resolve_insight_insertion(
        &self,
        id: &str,
        store_revision: u64,
        attempt_id: &str,
        decision: InsightInsertionState,
        notes_hash: &str,
    ) -> Result<InsightSnapshot, PennelError> {
        if !matches!(
            decision,
            InsightInsertionState::Acknowledged | InsightInsertionState::NotInserted
        ) || !valid_hash(notes_hash)
        {
            return Err(invalid("Choose an explicit reviewed insertion outcome."));
        }
        self.mutate_insights(id,store_revision,None,|document,_,root| {
            if hash(current_notes(root)?.as_bytes())!=notes_hash {return Err(invalid("The saved notes changed after this insertion was inspected."));}
            let receipt=document.insertion_receipts.iter_mut().find(|receipt|receipt.attempt_id==attempt_id).ok_or(PennelError::NotFound)?;
            if !matches!(receipt.state,InsightInsertionState::Pending|InsightInsertionState::NeedsReview) {
                return Err(invalid("This insertion already has a confirmed outcome."));
            }
            if notes_hash==receipt.before_notes_hash||notes_hash==receipt.intended_notes_hash {
                return Err(invalid("The saved notes already provide an exact outcome. Recover the receipt from that hash."));
            }
            receipt.state=decision;
            receipt.manual_resolution=Some(InsightInsertionResolution {state:decision,notes_hash:notes_hash.into()});
            Ok(())
        })
    }
}

fn resolve_receipt(receipt: &InsightInsertionReceipt, notes_hash: &str) -> InsightInsertionState {
    if receipt.state == InsightInsertionState::Acknowledged
        || notes_hash == receipt.intended_notes_hash
    {
        InsightInsertionState::Acknowledged
    } else if notes_hash == receipt.before_notes_hash {
        InsightInsertionState::NotInserted
    } else {
        InsightInsertionState::NeedsReview
    }
}

fn insight_export_destination(identity: &InferenceIdentity) -> String {
    let Ok((_, local)) = crate::inference::validate_origin(&identity.endpoint) else {
        return "unavailable".into();
    };
    let Ok(uri) = identity.endpoint.parse::<ureq::http::Uri>() else {
        return "unavailable".into();
    };
    let Some(authority) = uri
        .authority()
        .map(|authority| authority.as_str())
        .filter(|authority| {
            authority
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".:-[]".contains(&byte))
        })
    else {
        return "unavailable".into();
    };
    if local {
        format!("local service at `{authority}`")
    } else {
        format!("`{authority}`")
    }
}

fn insight_export_markdown(item: &InsightItem) -> String {
    let mut output = item.display_text().to_owned();
    output.push_str("\n\n---\n\n**Source evidence (original transcript)**\n\n");
    output.push_str("Meeting file: [Transcript](transcript.md). References below identify the frozen utterances.\n\n");
    output.push_str(&format!(
        "Evidence cutoff: [{}] ({} ms)\n\nProcessing destination: {}\n\nSource status: {}",
        crate::meetings::timestamp(item.evidence_cutoff_ms),
        item.evidence_cutoff_ms,
        insight_export_destination(&item.processing.identity),
        match item.source_state {
            InsightSourceState::Current => "current",
            InsightSourceState::Stale => "stale; transcript changed, frozen evidence retained",
            InsightSourceState::Missing =>
                "missing; source anchor no longer exists, frozen evidence retained",
            InsightSourceState::Unavailable =>
                "unavailable; source could not be revalidated, frozen evidence retained",
        },
    ));
    for (index, source) in item.sources.iter().enumerate() {
        let meeting_uid = if valid_id(&source.meeting_uid) {
            &source.meeting_uid
        } else {
            "unavailable"
        };
        output.push_str(&format!(
            "\n\n**Source {}**\n\nMeeting UID: `{meeting_uid}`\n\nUtterance: `{}`\n\nUTF-8 bytes: {}..{}\n\nEvidence revision: `{}`\n\nUtterance range: [{}]–[{}] ({}–{} ms)\n\n> [{}] ",
            index + 1,
            source.utterance_id,
            source.start_byte,
            source.end_byte,
            source.source_fingerprint,
            crate::meetings::timestamp(source.start_ms),
            crate::meetings::timestamp(source.end_ms),
            source.start_ms,
            source.end_ms,
            crate::meetings::timestamp(source.start_ms),
        ));
        output.push_str(&source.quote.replace('\n', "\n> "));
    }
    output
}

pub fn insight_item_markdown(item: &InsightItem) -> String {
    if let Some(edited) = &item.edited_text {
        return edited.clone();
    }
    let mut output = item.display_text().to_owned();
    for source in &item.sources {
        let seconds = source.start_ms / 1000;
        output.push_str(&format!("\n\n> [{:02}:{:02}] ", seconds / 60, seconds % 60));
        output.push_str(&source.quote.replace('\n', "\n> "));
    }
    output
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{InferenceProtocol, Segment, Speaker};

    fn fixture() -> (
        tempfile::TempDir,
        MeetingStore,
        String,
        MeetingTranscriptSnapshot,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Reviewed suggestions", chrono::Local::now())
            .unwrap();
        let row = MeetingUtterance::new(
            Segment {
                start_ms: 1_000,
                end_ms: 5_000,
                speaker: Speaker::Others,
                text: "Jan will send the report on Friday.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        );
        let source = store
            .write_transcript_snapshot(&id, Some(0), &[row])
            .unwrap();
        (directory, store, id, source)
    }

    pub(crate) fn candidate(source: &MeetingTranscriptSnapshot) -> InsightCandidate {
        let row = &source.utterances[0];
        InsightCandidate {
            kind: InsightKind::Action,
            extraction_scope: InsightExtractionScope::Passage,
            commitment: CommitmentState::Tentative,
            text: row.segment.text.clone(),
            owner: None,
            due: None,
            sources: vec![
                InsightSource::from_utterance(
                    &source.meeting_uid,
                    row,
                    0,
                    row.segment.text.len() as u32,
                )
                .unwrap(),
            ],
            evidence_cutoff_ms: source
                .utterances
                .iter()
                .filter(|row| !row.segment.failed)
                .map(|row| row.segment.end_ms)
                .max()
                .unwrap_or(0),
            transcript_revision: source.revision,
            transcript_digest: source.transcript_digest.clone(),
            processing: InsightProcessing {
                identity: InferenceIdentity {
                    protocol: InferenceProtocol::LocalDecider,
                    endpoint: "http://127.0.0.1:9000/decide".into(),
                    requested_model: "owned-fixture".into(),
                    requested_upstream: None,
                    actual_model: Some("owned-fixture".into()),
                    actual_revision: Some("v1".into()),
                    actual_provider: Some("local".into()),
                    request_id: None,
                },
                qualification_id: "owned-test-only".into(),
                language_policy: "en".into(),
                task: "events".into(),
            },
            related_item_ids: Vec::new(),
            prior_evidence: Vec::new(),
        }
    }

    #[test]
    fn reviewed_edits_and_dismissed_duplicates_survive_reopen_and_regeneration() {
        let (_directory, store, id, source) = fixture();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let item = &first.items[0];
        let claimed = store
            .claim_insight_edit(
                &id,
                1,
                &item.id,
                item.revision,
                &uuid::Uuid::now_v7().to_string(),
            )
            .unwrap();
        let item = &claimed.items[0];
        let accepted = store
            .update_insight_review(
                &id,
                claimed.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Accepted),
                    edited_text: Some("My deliberately edited **Markdown**.".into()),
                    edit_claim: item.edit_claim.clone(),
                    release_edit_claim: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let repeated = store
            .merge_insight_candidates(
                &id,
                accepted.revision,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(
            repeated.items[0].edited_text.as_deref(),
            Some("My deliberately edited **Markdown**.")
        );
        assert_eq!(repeated.items[0].review_state, InsightReviewState::Accepted);
        assert_eq!(store.read_insights(&id).unwrap(), repeated);
        assert!(
            store
                .export_insights_markdown(&id)
                .unwrap()
                .contains("My deliberately edited **Markdown**.")
        );
        let item = &repeated.items[0];
        let dismissed = store
            .update_insight_review(
                &id,
                repeated.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Dismissed),
                    ..Default::default()
                },
            )
            .unwrap();
        let regenerated = store
            .merge_insight_candidates(
                &id,
                dismissed.revision,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(regenerated.items.len(), 1);
        assert_eq!(
            regenerated.items[0].review_state,
            InsightReviewState::Dismissed
        );
    }

    #[test]
    fn unsupported_or_corrupt_store_is_preserved_read_only() {
        let (_directory, store, id, source) = fixture();
        for bytes in [
            br#"{"schema_version":9000,"private_extra":"kept"}"#.as_slice(),
            b"broken json".as_slice(),
        ] {
            fs::write(store.dir_of(&id).join(INSIGHTS_FILE), bytes).unwrap();
            let snapshot = store.read_insights(&id).unwrap();
            assert!(matches!(
                snapshot.status,
                InsightStoreStatus::Unsupported | InsightStoreStatus::Corrupt
            ));
            assert!(
                store
                    .merge_insight_candidates(
                        &id,
                        0,
                        &[candidate(&source)],
                        &source,
                        &InferenceCancellation::new()
                    )
                    .is_err()
            );
            assert_eq!(
                fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn unsupported_owner_dates_cancel_and_stale_cas_never_replace_reviewed_work() {
        let (_directory, store, id, source) = fixture();
        let good = candidate(&source);
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                std::slice::from_ref(&good),
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let before = fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        let mut unsupported = good.clone();
        unsupported.owner = Some(SupportedInsightDetail {
            text: "Invented owner".into(),
            source_index: 0,
            start_byte: 0,
            end_byte: 3,
        });
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    first.revision,
                    &[unsupported],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        let cancelled = InferenceCancellation::new();
        cancelled.cancel();
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    first.revision,
                    std::slice::from_ref(&good),
                    &source,
                    &cancelled
                )
                .is_err()
        );
        assert!(
            store
                .merge_insight_candidates(&id, 0, &[good], &source, &InferenceCancellation::new())
                .is_err()
        );
        assert_eq!(
            fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
            before
        );
    }

    #[test]
    fn an_edit_claim_survives_regeneration_before_draft_text_is_saved() {
        let (_directory, store, id, source) = fixture();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let token = uuid::Uuid::now_v7().to_string();
        let item = &first.items[0];
        let claimed = store
            .claim_insight_edit(&id, first.revision, &item.id, item.revision, &token)
            .unwrap();
        let fresh = store
            .merge_insight_candidates(
                &id,
                claimed.revision,
                &[],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(fresh.items[0].id, item.id);
        assert_eq!(
            store.read_insights(&id).unwrap().items[0]
                .edit_claim
                .as_deref(),
            Some(token.as_str())
        );
        let item = &fresh.items[0];
        assert!(
            store
                .update_insight_review(
                    &id,
                    fresh.revision,
                    &item.id,
                    item.revision,
                    &InsightReviewUpdate {
                        review_state: Some(InsightReviewState::Dismissed),
                        ..Default::default()
                    }
                )
                .is_err()
        );
        let released = store
            .update_insight_review(
                &id,
                fresh.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    edit_claim: Some(token),
                    release_edit_claim: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    released.revision,
                    &[],
                    &source,
                    &InferenceCancellation::new()
                )
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn stale_missing_and_explicitly_reassociated_sources_preserve_review_and_history() {
        let (_directory, store, id, source) = fixture();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let accepted = accept(&store, &id, &first);
        let mut changed = source.utterances.clone();
        changed[0].segment.text = "No such commitment was made.".into();
        store
            .write_transcript_snapshot(&id, Some(source.revision), &changed)
            .unwrap();
        let stale = store.read_insights(&id).unwrap();
        assert_eq!(stale.items[0].source_state, InsightSourceState::Stale);
        assert_eq!(stale.items[0].sources, accepted.items[0].sources);
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    stale.revision,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        let mut replacement = MeetingUtterance::new(
            source.utterances[0].segment.clone(),
            Some("room_microphone".into()),
        );
        replacement.segment.start_ms = 1_200;
        replacement.segment.end_ms = 5_200;
        let current = store
            .write_transcript_snapshot(&id, None, &[replacement])
            .unwrap();
        let missing = store.read_insights(&id).unwrap();
        assert_eq!(missing.items[0].source_state, InsightSourceState::Missing);
        let replacement = InsightSource::from_utterance(
            &current.meeting_uid,
            &current.utterances[0],
            0,
            current.utterances[0].segment.text.len() as u32,
        )
        .unwrap();
        let recovered = store
            .reassociate_insight_source(
                &id,
                missing.revision,
                &InsightSourceReassociation {
                    item_id: missing.items[0].id.clone(),
                    item_revision: missing.items[0].revision,
                    source_index: 0,
                    source: replacement,
                    evidence_cutoff_ms: 5_200,
                    edit_claim: None,
                },
            )
            .unwrap();
        assert_eq!(
            recovered.items[0].review_state,
            InsightReviewState::Accepted
        );
        assert_eq!(recovered.items[0].source_state, InsightSourceState::Current);
        assert_eq!(recovered.items[0].source_history, accepted.items[0].sources);
        assert!(recovered.items[0].source_reassociated);
    }

    #[test]
    fn atomic_write_fault_and_future_identity_fields_keep_the_original_store() {
        let (_directory, store, id, source) = fixture();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let path = store.dir_of(&id).join(INSIGHTS_FILE);
        let bytes = fs::read(&path).unwrap();
        fail_next_insight_write();
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    first.revision,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mut future: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        future["items"][0]["processing"]["identity"]["future_identity_field"] =
            serde_json::json!("preserved");
        let future = serde_json::to_vec(&future).unwrap();
        fs::write(&path, &future).unwrap();
        assert_eq!(
            store.read_insights(&id).unwrap().status,
            InsightStoreStatus::Unsupported
        );
        assert!(
            store
                .update_insight_review(
                    &id,
                    first.revision,
                    &first.items[0].id,
                    first.items[0].revision,
                    &InsightReviewUpdate::default()
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), future);
    }

    #[test]
    fn insertion_is_staged_once_and_confirmed_only_after_an_acknowledged_normal_note_save() {
        let (_directory, store, id, source) = fixture();
        store
            .write_notes(&id, "Private acknowledged note.")
            .unwrap();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let markdown = insight_item_markdown(&first.items[0]);
        let request = InsightInsertionRequest {
            attempt_id: uuid::Uuid::now_v7().to_string(),
            item_id: first.items[0].id.clone(),
            item_revision: first.items[0].revision,
            before_notes_hash: hash(b"Private acknowledged note."),
            intended_notes: format!("Private acknowledged note.\n\n{markdown}"),
            markdown,
        };
        let prepared = store
            .prepare_insight_insertion(&id, first.revision, &request)
            .unwrap();
        assert!(prepared.stage);
        assert_eq!(prepared.receipt.state, InsightInsertionState::Pending);
        assert_eq!(store.read_notes(&id), "Private acknowledged note.");
        let replay = store
            .prepare_insight_insertion(&id, first.revision, &request)
            .unwrap();
        assert!(!replay.stage);
        assert!(replay.intended_notes.is_none());
        let file = fs::read_to_string(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        assert!(!file.contains("Private acknowledged note."));
        assert!(
            store
                .acknowledge_insight_insertion(
                    &id,
                    prepared.snapshot.revision,
                    &request.attempt_id,
                    &prepared.receipt.intended_notes_hash
                )
                .is_err()
        );
        assert!(
            store
                .validate_insight_note_write(&id, &request.attempt_id, &request.intended_notes)
                .unwrap()
        );
        assert!(
            !store
                .validate_insight_note_write(
                    &id,
                    &request.attempt_id,
                    "A deliberate manual note edit"
                )
                .unwrap()
        );
        store.write_notes(&id, &request.intended_notes).unwrap();
        let done = store
            .acknowledge_insight_insertion(
                &id,
                prepared.snapshot.revision,
                &request.attempt_id,
                &prepared.receipt.intended_notes_hash,
            )
            .unwrap();
        assert_eq!(
            done.insertion_receipts[0].state,
            InsightInsertionState::Acknowledged
        );
        store
            .write_notes(&id, "Edited after acknowledgement.")
            .unwrap();
        assert_eq!(
            store
                .reconcile_insight_insertions(&id, done.revision)
                .unwrap()
                .insertion_receipts[0]
                .state,
            InsightInsertionState::Acknowledged
        );
    }

    #[test]
    fn insertion_restart_recovery_uses_only_hash_proof_and_never_repeats_notes() {
        for outcome in [
            InsightInsertionState::NotInserted,
            InsightInsertionState::Acknowledged,
            InsightInsertionState::NeedsReview,
        ] {
            let (_directory, store, id, source) = fixture();
            store.write_notes(&id, "Before.").unwrap();
            let first = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let markdown = insight_item_markdown(&first.items[0]);
            let request = InsightInsertionRequest {
                attempt_id: uuid::Uuid::now_v7().to_string(),
                item_id: first.items[0].id.clone(),
                item_revision: first.items[0].revision,
                before_notes_hash: hash(b"Before."),
                intended_notes: format!("Before.\n\n{markdown}"),
                markdown,
            };
            let prepared = store
                .prepare_insight_insertion(&id, first.revision, &request)
                .unwrap();
            let actual = match outcome {
                InsightInsertionState::NotInserted => "Before.",
                InsightInsertionState::Acknowledged => request.intended_notes.as_str(),
                _ => "Different manually edited notes.",
            };
            store.write_notes(&id, actual).unwrap();
            let recovered = store
                .reconcile_insight_insertions(&id, prepared.snapshot.revision)
                .unwrap();
            assert_eq!(recovered.insertion_receipts[0].state, outcome);
            assert_eq!(store.read_notes(&id), actual);
            if outcome == InsightInsertionState::NeedsReview {
                let mut retry = request.clone();
                retry.attempt_id = uuid::Uuid::now_v7().to_string();
                retry.before_notes_hash = hash(actual.as_bytes());
                assert!(
                    !store
                        .prepare_insight_insertion(&id, recovered.revision, &retry)
                        .unwrap()
                        .stage
                );
            }
        }
    }

    #[test]
    fn store_cas_allows_only_one_concurrent_generation_and_portable_limit_refuses_oversize() {
        let (_directory, store, id, source) = fixture();
        let root = store.root();
        let mut workers = Vec::new();
        for _ in 0..2 {
            let root = root.clone();
            let id = id.clone();
            let source = source.clone();
            workers.push(std::thread::spawn(move || {
                MeetingStore::new(&root)
                    .merge_insight_candidates(
                        &id,
                        0,
                        &[candidate(&source)],
                        &source,
                        &InferenceCancellation::new(),
                    )
                    .is_ok()
            }));
        }
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| usize::from(worker.join().unwrap()))
                .sum::<usize>(),
            1
        );
        let before = fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        let row = MeetingUtterance::new(
            Segment {
                start_ms: 1_000,
                end_ms: 5_000,
                speaker: Speaker::Others,
                text: "x".repeat(1_600_000),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        );
        let large = store
            .write_transcript_snapshot(&id, Some(source.revision), &[row])
            .unwrap();
        let proposal = candidate(&large);
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    1,
                    &[proposal],
                    &large,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert_eq!(
            fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
            before
        );
    }

    #[test]
    fn lineage_aliases_never_hide_changed_details_or_ambiguous_split_sources() {
        for change in 0..4 {
            let (_directory, store, id, source) = fixture();
            let legacy = MeetingUtterance::new(source.utterances[0].segment.clone(), None);
            let old = store
                .write_transcript_snapshot(
                    &id,
                    Some(source.revision),
                    std::slice::from_ref(&legacy),
                )
                .unwrap();
            let first = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[candidate(&old)],
                    &old,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let item = &first.items[0];
            let dismissed = store
                .update_insight_review(
                    &id,
                    first.revision,
                    &item.id,
                    item.revision,
                    &InsightReviewUpdate {
                        review_state: Some(InsightReviewState::Dismissed),
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut replacement =
                MeetingUtterance::new(legacy.segment.clone(), Some("room_microphone".into()));
            replacement.metadata.lineage = vec![legacy.metadata.utterance_id.clone()];
            let mut rows = vec![replacement];
            if change == 3 {
                let mut second = MeetingUtterance::new(
                    Segment {
                        start_ms: 6_000,
                        end_ms: 9_000,
                        speaker: Speaker::Others,
                        text: "Additional uncertain turn.".into(),
                        failed: false,
                        voice: None,
                    },
                    Some("room_microphone".into()),
                );
                second.metadata.lineage = vec![legacy.metadata.utterance_id];
                rows.push(second);
            }
            let current = store
                .write_transcript_snapshot(&id, Some(old.revision), &rows)
                .unwrap();
            let mut proposal = candidate(&current);
            match change {
                0 => {
                    proposal.owner = Some(SupportedInsightDetail {
                        text: "Jan".into(),
                        source_index: 0,
                        start_byte: 0,
                        end_byte: 3,
                    })
                }
                1 => {
                    let start = proposal.sources[0].quote.find("Friday").unwrap() as u32;
                    proposal.due = Some(SupportedInsightDetail {
                        text: "Friday".into(),
                        source_index: 0,
                        start_byte: start,
                        end_byte: start + 6,
                    });
                }
                2 => {
                    proposal.kind = InsightKind::ChangedCommitment;
                    proposal.commitment = CommitmentState::Retracted;
                }
                _ => {}
            }
            let after = store
                .merge_insight_candidates(
                    &id,
                    dismissed.revision,
                    &[proposal],
                    &current,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            assert_eq!(
                after.items.len(),
                2,
                "A different or uncertain proposal was hidden in case {change}"
            );
            assert!(after.items.iter().any(|item| item.id == first.items[0].id
                && item.review_state == InsightReviewState::Dismissed));
            assert!(
                after
                    .items
                    .iter()
                    .any(|item| item.review_state == InsightReviewState::Proposed)
            );
        }
    }

    #[test]
    fn wrong_meeting_and_oversized_existing_files_remain_read_only_without_replacement() {
        let (_directory, store, id, source) = fixture();
        store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let path = store.dir_of(&id).join(INSIGHTS_FILE);
        let bytes = fs::read(&path).unwrap();
        let other = store
            .create("Different meeting", chrono::Local::now())
            .unwrap();
        fs::write(store.dir_of(&other).join(INSIGHTS_FILE), &bytes).unwrap();
        assert_eq!(
            store.read_insights(&other).unwrap().status,
            InsightStoreStatus::WrongMeeting
        );
        let huge = vec![b'x'; MAX_INSIGHTS_BYTES + 1];
        fs::write(&path, &huge).unwrap();
        assert_eq!(
            store.read_insights(&id).unwrap().status,
            InsightStoreStatus::TooLarge
        );
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), huge);
    }

    #[test]
    fn edited_markdown_keeps_exact_note_staging_and_exports_separate_original_evidence() {
        let (_directory, store, id, source) = fixture();
        let mut proposal = candidate(&source);
        proposal.owner = Some(SupportedInsightDetail {
            text: "Jan".into(),
            source_index: 0,
            start_byte: 0,
            end_byte: 3,
        });
        let start = proposal.sources[0].quote.find("Friday").unwrap() as u32;
        proposal.due = Some(SupportedInsightDetail {
            text: "Friday".into(),
            source_index: 0,
            start_byte: start,
            end_byte: start + 6,
        });
        let first = store
            .merge_insight_candidates(&id, 0, &[proposal], &source, &InferenceCancellation::new())
            .unwrap();
        let edited = "Els will handle the revised task on Monday.\n\nA deliberate **user edit**.\n";
        let item = &first.items[0];
        let reviewed = store
            .update_insight_review(
                &id,
                first.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Accepted),
                    edited_text: Some(edited.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(insight_item_markdown(&reviewed.items[0]), edited);
        let exported = store.export_insights_markdown(&id).unwrap();
        let provenance = exported.strip_prefix(edited).unwrap();
        assert!(provenance.starts_with("\n\n---\n\n**Source evidence (original transcript)**"));
        assert!(provenance.contains("[Transcript](transcript.md)"));
        assert!(!provenance.contains("[Metadata](transcript.meta.json)"));
        assert!(provenance.contains(&format!("Meeting UID: `{}`", source.meeting_uid)));
        assert!(provenance.contains(&format!(
            "Utterance: `{}`",
            source.utterances[0].metadata.utterance_id
        )));
        assert!(provenance.contains(&format!(
            "Evidence revision: `{}`",
            utterance_evidence_revision(&source.utterances[0])
        )));
        assert!(provenance.contains(&format!("> [00:01] {}", source.utterances[0].segment.text)));
        assert!(provenance.contains("Evidence cutoff: [00:05] (5000 ms)"));
        assert!(provenance.contains("Processing destination: local service at `127.0.0.1:9000`"));
        assert!(provenance.contains("Source status: current"));
        assert!(!provenance.contains("Owner: Jan"));
        assert!(!provenance.contains("Due: Friday"));
        assert!(!exported.contains("pennel://"));
        assert_eq!(reviewed.items[0].owner.as_ref().unwrap().text, "Jan");
        assert_eq!(reviewed.items[0].due.as_ref().unwrap().text, "Friday");
        assert_eq!(
            reviewed.items[0].sources[0].quote,
            source.utterances[0].segment.text
        );
        let request = InsightInsertionRequest {
            attempt_id: uuid::Uuid::now_v7().to_string(),
            item_id: reviewed.items[0].id.clone(),
            item_revision: reviewed.items[0].revision,
            before_notes_hash: hash(b""),
            intended_notes: edited.into(),
            markdown: insight_item_markdown(&reviewed.items[0]),
        };
        let prepared = store
            .prepare_insight_insertion(&id, reviewed.revision, &request)
            .unwrap();
        assert!(prepared.stage);
        assert_eq!(prepared.intended_notes.as_deref(), Some(edited));
        assert_eq!(prepared.receipt.markdown, edited);
        assert_eq!(
            prepared.receipt.intended_notes_hash,
            hash(edited.as_bytes())
        );
        store.write_notes(&id, edited).unwrap();
        let done = store
            .acknowledge_insight_insertion(
                &id,
                prepared.snapshot.revision,
                &request.attempt_id,
                &hash(edited.as_bytes()),
            )
            .unwrap();
        assert_eq!(
            done.insertion_receipts[0].state,
            InsightInsertionState::Acknowledged
        );
        assert_eq!(store.read_notes(&id), edited);
        assert!(
            !store
                .prepare_insight_insertion(&id, done.revision, &request)
                .unwrap()
                .stage
        );
        assert_eq!(store.export_insights_markdown(&id).unwrap(), exported);
    }

    #[test]
    fn accepted_export_retains_frozen_stale_sources_and_sanitizes_processing_destinations() {
        for (endpoint, expected_destination) in [
            (
                "https://review.example:8443/private-route",
                "`review.example:8443`",
            ),
            (
                "https://user:private-credential@review.example/private-route?private-query",
                "unavailable",
            ),
            ("file:///private-route", "unavailable"),
        ] {
            let (_directory, store, id, source) = fixture();
            let mut proposal = candidate(&source);
            proposal.processing.identity.endpoint = endpoint.into();
            let first = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[proposal],
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let edited = "My accepted **exact** edit.\n";
            store
                .update_insight_review(
                    &id,
                    first.revision,
                    &first.items[0].id,
                    first.items[0].revision,
                    &InsightReviewUpdate {
                        review_state: Some(InsightReviewState::Accepted),
                        edited_text: Some(edited.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut changed = source.utterances.clone();
            changed[0].segment.text = "The revised source is different.".into();
            store
                .write_transcript_snapshot(&id, Some(source.revision), &changed)
                .unwrap();
            let exported = store.export_insights_markdown(&id).unwrap();
            assert!(exported.starts_with(edited));
            assert!(exported.contains(&format!("> [00:01] {}", source.utterances[0].segment.text)));
            assert!(exported.contains("Source status: stale"));
            assert!(exported.contains("frozen evidence retained"));
            assert!(exported.contains(&format!("Processing destination: {expected_destination}")));
            for private in ["private-route", "private-credential", "private-query"] {
                assert!(!exported.contains(private));
            }
        }
    }

    #[test]
    fn ambiguous_insertions_require_an_explicit_review_bound_to_the_saved_notes_hash() {
        for decision in [
            InsightInsertionState::Acknowledged,
            InsightInsertionState::NotInserted,
        ] {
            let (_directory, store, id, source) = fixture();
            store.write_notes(&id, "Before.").unwrap();
            let first = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let markdown = insight_item_markdown(&first.items[0]);
            let request = InsightInsertionRequest {
                attempt_id: uuid::Uuid::now_v7().to_string(),
                item_id: first.items[0].id.clone(),
                item_revision: first.items[0].revision,
                before_notes_hash: hash(b"Before."),
                intended_notes: format!("Before.\n\n{markdown}"),
                markdown,
            };
            let prepared = store
                .prepare_insight_insertion(&id, first.revision, &request)
                .unwrap();
            let notes = "The user manually revised this insertion and surrounding notes.";
            store.write_notes(&id, notes).unwrap();
            let pending = store
                .reconcile_insight_insertions(&id, prepared.snapshot.revision)
                .unwrap();
            assert_eq!(
                pending.insertion_receipts[0].state,
                InsightInsertionState::NeedsReview
            );
            assert!(
                store
                    .resolve_insight_insertion(
                        &id,
                        pending.revision,
                        &request.attempt_id,
                        decision,
                        &hash(b"Stale inspected notes")
                    )
                    .is_err()
            );
            let resolved = store
                .resolve_insight_insertion(
                    &id,
                    pending.revision,
                    &request.attempt_id,
                    decision,
                    &hash(notes.as_bytes()),
                )
                .unwrap();
            assert_eq!(resolved.insertion_receipts[0].state, decision);
            let manual = resolved.insertion_receipts[0]
                .manual_resolution
                .as_ref()
                .unwrap();
            assert_eq!(manual.state, decision);
            assert_eq!(manual.notes_hash, hash(notes.as_bytes()));
            assert_eq!(store.read_notes(&id), notes);
        }
    }

    #[test]
    fn scoped_negative_retires_only_the_assessed_kind_and_abstentions_do_not_write() {
        let (_directory, store, id, source) = fixture();
        let mut rows = source.utterances.clone();
        rows[0].segment.text = "Jan will send it. We approved the draft. Who checks it?".into();
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let proposals: Vec<_> = [
            InsightKind::Action,
            InsightKind::Decision,
            InsightKind::OpenQuestion,
        ]
        .into_iter()
        .map(|kind| {
            let mut proposal = candidate(&source);
            proposal.kind = kind;
            proposal
        })
        .collect();
        let first = store
            .merge_insight_candidates(&id, 0, &proposals, &source, &InferenceCancellation::new())
            .unwrap();
        assert_eq!(first.items.len(), 3);
        let scope = [InsightAssessmentScope {
            utterance_id: source.utterances[0].metadata.utterance_id.clone(),
            kinds: vec![InsightKind::Action],
        }];
        let assessed = store
            .merge_scoped_insight_candidates(
                &id,
                first.revision,
                &[],
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(assessed.items.len(), 2);
        for kind in [InsightKind::Decision, InsightKind::OpenQuestion] {
            let old = first.items.iter().find(|item| item.kind == kind).unwrap();
            let retained = assessed
                .items
                .iter()
                .find(|item| item.kind == kind)
                .unwrap();
            assert_eq!(old, retained);
        }
        let bytes = fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        let unchecked = store
            .merge_scoped_insight_candidates(
                &id,
                0,
                &[],
                &[],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(unchecked, assessed);
        assert_eq!(
            fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
            bytes
        );
    }

    #[test]
    fn scoped_assessment_allows_context_sources_but_rejects_unassessed_primary_pairs() {
        let (_directory, store, id, source) = fixture();
        let mut rows = source.utterances.clone();
        rows.push(MeetingUtterance::new(
            Segment {
                start_ms: 6_000,
                end_ms: 9_000,
                speaker: Speaker::Others,
                text: "Jan confirmed the decision.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        ));
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let mut previous = candidate(&source);
        previous.kind = InsightKind::Decision;
        previous.text = source.utterances[1].segment.text.clone();
        previous.sources = vec![
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[1],
                0,
                source.utterances[1].segment.text.len() as u32,
            )
            .unwrap(),
        ];
        let first = store
            .merge_insight_candidates(&id, 0, &[previous], &source, &InferenceCancellation::new())
            .unwrap();
        let mut action = candidate(&source);
        action.sources.push(
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[1],
                0,
                source.utterances[1].segment.text.len() as u32,
            )
            .unwrap(),
        );
        action.owner = Some(SupportedInsightDetail {
            text: "Jan".into(),
            source_index: 1,
            start_byte: 0,
            end_byte: 3,
        });
        let scope = [InsightAssessmentScope {
            utterance_id: source.utterances[0].metadata.utterance_id.clone(),
            kinds: vec![InsightKind::Action],
        }];
        let next = store
            .merge_scoped_insight_candidates(
                &id,
                first.revision,
                std::slice::from_ref(&action),
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(next.items.len(), 2);
        assert!(next.items.iter().any(|item| item.id == first.items[0].id));
        action.kind = InsightKind::OpenQuestion;
        let bytes = fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        assert!(
            store
                .merge_scoped_insight_candidates(
                    &id,
                    next.revision,
                    &[action],
                    &scope,
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert!(
            store
                .merge_scoped_insight_candidates(
                    &id,
                    next.revision,
                    &[],
                    &[InsightAssessmentScope {
                        utterance_id: uuid::Uuid::now_v7().to_string(),
                        kinds: vec![InsightKind::Action]
                    }],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert_eq!(
            fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
            bytes
        );
    }

    #[test]
    fn exact_span_extraction_keeps_multiple_actions_inside_the_same_utterance() {
        let (_directory, store, id, source) = fixture();
        let text = "I will send the report. I will book the room.";
        let mut rows = source.utterances.clone();
        rows[0].segment.text = text.into();
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let split = text.find("I will book").unwrap();
        let proposals: Vec<_> = [(0, split - 1), (split, text.len())]
            .into_iter()
            .map(|(start, end)| {
                let mut proposal = candidate(&source);
                proposal.extraction_scope = InsightExtractionScope::ExactSpan;
                proposal.sources = vec![
                    InsightSource::from_utterance(
                        &source.meeting_uid,
                        &source.utterances[0],
                        start as u32,
                        end as u32,
                    )
                    .unwrap(),
                ];
                proposal.text = proposal.sources[0].quote.clone();
                proposal
            })
            .collect();
        let scope = [InsightAssessmentScope {
            utterance_id: source.utterances[0].metadata.utterance_id.clone(),
            kinds: vec![InsightKind::Action],
        }];
        let saved = store
            .merge_scoped_insight_candidates(
                &id,
                0,
                &proposals,
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(saved.items.len(), 2);
        assert_ne!(saved.items[0].dedup_key, saved.items[1].dedup_key);
        assert!(
            saved
                .items
                .iter()
                .all(|item| item.extraction_scope == InsightExtractionScope::ExactSpan)
        );
        let repeated = store
            .merge_scoped_insight_candidates(
                &id,
                saved.revision,
                &proposals,
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let ids: HashSet<_> = saved.items.iter().map(|item| &item.id).collect();
        assert!(repeated.items.iter().all(|item| ids.contains(&item.id)));
        let partial = store
            .merge_scoped_insight_candidates(
                &id,
                repeated.revision,
                std::slice::from_ref(&proposals[0]),
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(
            partial.items.len(),
            2,
            "A positive partial extraction withdrew an unchecked earlier action"
        );
        assert!(partial.items.iter().all(|item| ids.contains(&item.id)));
        let negative = store
            .merge_scoped_insight_candidates(
                &id,
                partial.revision,
                &[],
                &scope,
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert!(
            negative.items.is_empty(),
            "An explicit assessed negative did not retire unreviewed action candidates"
        );
    }

    #[test]
    fn legacy_passage_scope_defaults_safely_and_keeps_unknown_fields_read_only() {
        let (_directory, store, id, source) = fixture();
        store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let path = store.dir_of(&id).join(INSIGHTS_FILE);
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(legacy["items"][0]["extraction_scope"], "passage");
        legacy["items"][0]
            .as_object_mut()
            .unwrap()
            .remove("extraction_scope");
        legacy["items"][0]
            .as_object_mut()
            .unwrap()
            .remove("prior_evidence");
        let bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&path, &bytes).unwrap();
        let read = store.read_insights(&id).unwrap();
        assert_eq!(read.status, InsightStoreStatus::Ready);
        assert_eq!(
            read.items[0].extraction_scope,
            InsightExtractionScope::Passage
        );
        assert!(read.items[0].prior_evidence.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        legacy["items"][0]["unknown_future_field"] = serde_json::json!({"keep":"exact"});
        let bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            store.read_insights(&id).unwrap().status,
            InsightStoreStatus::Unsupported
        );
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    read.revision,
                    &[candidate(&source)],
                    &source,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn an_unchecked_first_run_creates_no_store_or_processing_revision() {
        let (_directory, store, id, source) = fixture();
        let unchecked = store
            .merge_scoped_insight_candidates(
                &id,
                0,
                &[],
                &[InsightAssessmentScope {
                    utterance_id: source.utterances[0].metadata.utterance_id.clone(),
                    kinds: Vec::new(),
                }],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(unchecked.status, InsightStoreStatus::Missing);
        assert_eq!(unchecked.revision, 0);
        assert!(!store.dir_of(&id).join(INSIGHTS_FILE).exists());
    }

    fn prior_story() -> (
        tempfile::TempDir,
        MeetingStore,
        String,
        MeetingTranscriptSnapshot,
        InsightCandidate,
        InsightCandidate,
    ) {
        let (directory, store, id, source) = fixture();
        let mut rows = source.utterances.clone();
        rows.push(MeetingUtterance::new(
            Segment {
                start_ms: 6_000,
                end_ms: 9_000,
                speaker: Speaker::Others,
                text: "We retract that report commitment.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        ));
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let mut action = candidate(&source);
        action.commitment = CommitmentState::Confirmed;
        let mut changed = candidate(&source);
        changed.kind = InsightKind::ChangedCommitment;
        changed.commitment = CommitmentState::Retracted;
        changed.text = source.utterances[1].segment.text.clone();
        changed.sources = vec![
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[1],
                0,
                changed.text.len() as u32,
            )
            .unwrap(),
        ];
        let start = source.utterances[0]
            .segment
            .text
            .find("send the report")
            .unwrap() as u32;
        let prior = InsightSource::from_utterance(
            &source.meeting_uid,
            &source.utterances[0],
            start,
            start + "send the report".len() as u32,
        )
        .unwrap();
        changed.prior_evidence = vec![InsightPriorEvidence {
            kind: InsightKind::Action,
            source: prior,
            target_item_id: None,
        }];
        (directory, store, id, source, action, changed)
    }

    #[test]
    fn first_atomic_batch_resolves_a_typed_prior_to_another_new_item_independent_of_order() {
        let (_directory, store, id, source, action, changed) = prior_story();
        let saved = store
            .merge_insight_candidates(
                &id,
                0,
                &[changed.clone(), action.clone()],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.items.len(), 2);
        let action_item = saved
            .items
            .iter()
            .find(|item| item.kind == InsightKind::Action)
            .unwrap();
        let changed_item = saved
            .items
            .iter()
            .find(|item| item.kind == InsightKind::ChangedCommitment)
            .unwrap();
        assert_eq!(
            changed_item.prior_evidence[0].target_item_id.as_deref(),
            Some(action_item.id.as_str())
        );
        assert!(changed_item.related_item_ids.contains(&action_item.id));
        let repeated = store
            .merge_insight_candidates(
                &id,
                saved.revision,
                &[action, changed],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let repeated_item = repeated
            .items
            .iter()
            .find(|item| item.id == changed_item.id)
            .unwrap();
        assert_eq!(
            repeated_item.prior_evidence[0].target_item_id.as_deref(),
            Some(action_item.id.as_str())
        );
    }

    #[test]
    fn ambiguous_wrong_kind_or_forged_prior_targets_remain_explicitly_unresolved() {
        for ambiguous in [false, true] {
            let (_directory, store, id, source, action, mut changed) = prior_story();
            changed.prior_evidence[0].target_item_id = Some(uuid::Uuid::now_v7().to_string());
            let mut proposals = vec![action.clone(), changed];
            if ambiguous {
                let mut precise = action;
                precise.extraction_scope = InsightExtractionScope::ExactSpan;
                precise.sources = vec![proposals[1].prior_evidence[0].source.clone()];
                precise.text = precise.sources[0].quote.clone();
                proposals.push(precise);
            } else {
                proposals[1].prior_evidence[0].kind = InsightKind::Decision;
            }
            let saved = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &proposals,
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let changed = saved
                .items
                .iter()
                .find(|item| item.kind == InsightKind::ChangedCommitment)
                .unwrap();
            assert!(changed.prior_evidence[0].target_item_id.is_none());
            assert_eq!(changed.prior_evidence[0].source.quote, "send the report");
            assert!(changed.related_item_ids.is_empty());
        }
    }

    #[test]
    fn typed_prior_links_require_safe_chronology_and_revalidate_prior_sources() {
        for overlap in [false, true] {
            let (_directory, store, id, source, _, _) = prior_story();
            let mut rows = source.utterances.clone();
            if overlap {
                rows[0].segment.end_ms = 8_000;
            } else {
                rows[0].segment.start_ms = 10_000;
                rows[0].segment.end_ms = 12_000;
            }
            let source = store
                .write_transcript_snapshot(&id, Some(source.revision), &rows)
                .unwrap();
            let target_row = source
                .utterances
                .iter()
                .find(|row| row.segment.text.starts_with("Jan"))
                .unwrap();
            let event_row = source
                .utterances
                .iter()
                .find(|row| row.segment.text.starts_with("We retract"))
                .unwrap();
            let mut action = candidate(&source);
            action.text = target_row.segment.text.clone();
            action.sources = vec![
                InsightSource::from_utterance(
                    &source.meeting_uid,
                    target_row,
                    0,
                    action.text.len() as u32,
                )
                .unwrap(),
            ];
            let mut changed = candidate(&source);
            changed.kind = InsightKind::ChangedCommitment;
            changed.commitment = CommitmentState::Retracted;
            changed.text = event_row.segment.text.clone();
            changed.sources = vec![
                InsightSource::from_utterance(
                    &source.meeting_uid,
                    event_row,
                    0,
                    changed.text.len() as u32,
                )
                .unwrap(),
            ];
            changed.prior_evidence = vec![InsightPriorEvidence {
                kind: InsightKind::Action,
                source: action.sources[0].clone(),
                target_item_id: None,
            }];
            let saved = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[action, changed],
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let event = saved
                .items
                .iter()
                .find(|item| item.kind == InsightKind::ChangedCommitment)
                .unwrap();
            assert!(event.prior_evidence[0].target_item_id.is_none());
        }
        let (_directory, store, id, source, action, changed) = prior_story();
        let saved = store
            .merge_insight_candidates(
                &id,
                0,
                &[action, changed.clone()],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let mut rows = source.utterances.clone();
        rows[0].segment.text = "Different earlier evidence.".into();
        let current = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let stale = store.read_insights(&id).unwrap();
        assert_eq!(
            stale
                .items
                .iter()
                .find(|item| item.kind == InsightKind::ChangedCommitment)
                .unwrap()
                .source_state,
            InsightSourceState::Stale
        );
        let mut wrong = changed;
        wrong.transcript_revision = current.revision;
        wrong.transcript_digest = current.transcript_digest.clone();
        assert!(
            store
                .merge_insight_candidates(
                    &id,
                    saved.revision,
                    &[wrong],
                    &current,
                    &InferenceCancellation::new()
                )
                .is_err()
        );
    }

    #[test]
    fn same_utterance_priors_use_exact_byte_order_and_reviewed_links_never_retarget() {
        let (_directory, store, id, source) = fixture();
        let text = "We agreed to send the report. We now retract that agreement.";
        let mut rows = source.utterances.clone();
        rows[0].segment.text = text.into();
        rows[0].segment.end_ms = 9_000;
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let split = text.find("We now").unwrap();
        let mut action = candidate(&source);
        action.commitment = CommitmentState::Confirmed;
        action.extraction_scope = InsightExtractionScope::ExactSpan;
        action.sources = vec![
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[0],
                0,
                (split - 1) as u32,
            )
            .unwrap(),
        ];
        action.text = action.sources[0].quote.clone();
        let mut changed = candidate(&source);
        changed.kind = InsightKind::ChangedCommitment;
        changed.commitment = CommitmentState::Retracted;
        changed.extraction_scope = InsightExtractionScope::ExactSpan;
        changed.sources = vec![
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[0],
                split as u32,
                text.len() as u32,
            )
            .unwrap(),
        ];
        changed.text = changed.sources[0].quote.clone();
        changed.prior_evidence = vec![InsightPriorEvidence {
            kind: InsightKind::Action,
            source: action.sources[0].clone(),
            target_item_id: None,
        }];
        let saved = store
            .merge_insight_candidates(
                &id,
                0,
                &[action.clone(), changed.clone()],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let target = saved
            .items
            .iter()
            .find(|item| item.kind == InsightKind::Action)
            .unwrap();
        let event = saved
            .items
            .iter()
            .find(|item| item.kind == InsightKind::ChangedCommitment)
            .unwrap();
        assert_eq!(
            event.prior_evidence[0].target_item_id.as_deref(),
            Some(target.id.as_str())
        );
        let reviewed = store
            .update_insight_review(
                &id,
                saved.revision,
                &event.id,
                event.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Accepted),
                    edited_text: Some("A retained user interpretation.".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut broad = action.clone();
        broad.extraction_scope = InsightExtractionScope::Passage;
        broad.sources = vec![
            InsightSource::from_utterance(
                &source.meeting_uid,
                &source.utterances[0],
                0,
                text.len() as u32,
            )
            .unwrap(),
        ];
        broad.text = text.into();
        let again = store
            .merge_insight_candidates(
                &id,
                reviewed.revision,
                &[broad, action, changed],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let protected = again.items.iter().find(|item| item.id == event.id).unwrap();
        assert_eq!(protected.prior_evidence, event.prior_evidence);
        assert_eq!(
            protected.edited_text.as_deref(),
            Some("A retained user interpretation.")
        );
    }

    #[test]
    fn dismissed_relations_survive_proven_main_and_prior_aliases_at_a_later_cutoff() {
        for proven_prior in [true, false] {
            let (_directory, store, id, source, action, changed) = prior_story();
            let saved = store
                .merge_insight_candidates(
                    &id,
                    0,
                    &[action, changed],
                    &source,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let old = saved
                .items
                .iter()
                .find(|item| item.kind == InsightKind::ChangedCommitment)
                .unwrap();
            let dismissed = store
                .update_insight_review(
                    &id,
                    saved.revision,
                    &old.id,
                    old.revision,
                    &InsightReviewUpdate {
                        review_state: Some(InsightReviewState::Dismissed),
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut rows = source.utterances.clone();
            for (index, row) in rows.iter_mut().enumerate() {
                let old = row.metadata.utterance_id.clone();
                row.metadata.utterance_id = uuid::Uuid::now_v7().to_string();
                row.metadata.lineage = if index == 1 || proven_prior {
                    vec![old]
                } else {
                    Vec::new()
                };
            }
            rows.push(MeetingUtterance::new(
                Segment {
                    start_ms: 10_000,
                    end_ms: 12_000,
                    speaker: Speaker::Others,
                    text: "Unrelated later speech.".into(),
                    failed: false,
                    voice: None,
                },
                Some("room_microphone".into()),
            ));
            let current = store
                .write_transcript_snapshot(&id, Some(source.revision), &rows)
                .unwrap();
            let mut action = candidate(&current);
            action.commitment = CommitmentState::Confirmed;
            let mut changed = candidate(&current);
            changed.kind = InsightKind::ChangedCommitment;
            changed.commitment = CommitmentState::Retracted;
            changed.text = current.utterances[1].segment.text.clone();
            changed.sources = vec![
                InsightSource::from_utterance(
                    &current.meeting_uid,
                    &current.utterances[1],
                    0,
                    changed.text.len() as u32,
                )
                .unwrap(),
            ];
            let start = current.utterances[0]
                .segment
                .text
                .find("send the report")
                .unwrap() as u32;
            changed.prior_evidence = vec![InsightPriorEvidence {
                kind: InsightKind::Action,
                source: InsightSource::from_utterance(
                    &current.meeting_uid,
                    &current.utterances[0],
                    start,
                    start + "send the report".len() as u32,
                )
                .unwrap(),
                target_item_id: None,
            }];
            let refreshed = store
                .merge_insight_candidates(
                    &id,
                    dismissed.revision,
                    &[action, changed],
                    &current,
                    &InferenceCancellation::new(),
                )
                .unwrap();
            let retained = refreshed
                .items
                .iter()
                .find(|item| item.id == old.id)
                .unwrap();
            assert_eq!(retained.review_state, InsightReviewState::Dismissed);
            assert_eq!(retained.prior_evidence, old.prior_evidence);
            if proven_prior {
                assert_eq!(
                    refreshed
                        .items
                        .iter()
                        .filter(|item| item.kind == InsightKind::ChangedCommitment)
                        .count(),
                    1,
                    "Proven aliases resurrected a dismissed relation at a newer cutoff"
                );
            } else {
                assert!(
                    refreshed
                        .items
                        .iter()
                        .any(|item| item.kind == InsightKind::ChangedCommitment
                            && item.review_state == InsightReviewState::Proposed),
                    "An unproven changed relation was hidden"
                );
            }
        }
    }

    fn accept(store: &MeetingStore, id: &str, snapshot: &InsightSnapshot) -> InsightSnapshot {
        let item = &snapshot.items[0];
        store
            .update_insight_review(
                id,
                snapshot.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Accepted),
                    ..Default::default()
                },
            )
            .unwrap()
    }

    #[test]
    fn appended_evidence_keeps_old_cutoff_and_changed_commitments_get_stable_linked_items() {
        let (_directory, store, id, source) = fixture();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let reviewed = accept(&store, &id, &first);
        let mut rows = source.utterances.clone();
        rows.push(MeetingUtterance::new(
            Segment {
                start_ms: 6_000,
                end_ms: 9_000,
                speaker: Speaker::Others,
                text: "We retract that commitment.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        ));
        let current = store
            .write_transcript_snapshot(&id, Some(source.revision), &rows)
            .unwrap();
        let old = store
            .merge_insight_candidates(
                &id,
                reviewed.revision,
                &[candidate(&source)],
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(old.items[0].evidence_cutoff_ms, 5_000);
        assert_eq!(old.items[0].source_state, InsightSourceState::Current);
        let mut retracted = candidate(&current);
        retracted.commitment = CommitmentState::Retracted;
        retracted.evidence_cutoff_ms = 9_000;
        retracted.sources.push(
            InsightSource::from_utterance(
                &current.meeting_uid,
                &current.utterances[1],
                0,
                current.utterances[1].segment.text.len() as u32,
            )
            .unwrap(),
        );
        let changed = store
            .merge_insight_candidates(
                &id,
                old.revision,
                std::slice::from_ref(&retracted),
                &current,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(changed.items.len(), 2);
        let proposal = changed
            .items
            .iter()
            .find(|item| item.review_state == InsightReviewState::Proposed)
            .unwrap();
        assert_eq!(proposal.kind, InsightKind::Action);
        assert_eq!(proposal.commitment, CommitmentState::Retracted);
        assert_eq!(
            proposal.related_item_ids,
            vec![reviewed.items[0].id.clone()]
        );
        let again = store
            .merge_insight_candidates(
                &id,
                changed.revision,
                &[retracted],
                &current,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert!(again.items.iter().any(|item| item.id == proposal.id));
        assert_eq!(again.items.len(), 2);
    }

    #[test]
    fn proven_identity_only_lineage_does_not_resurrect_a_dismissed_proposal() {
        let (_directory, store, id, source) = fixture();
        let mut legacy = MeetingUtterance::new(source.utterances[0].segment.clone(), None);
        let old_source = store
            .write_transcript_snapshot(&id, Some(source.revision), std::slice::from_ref(&legacy))
            .unwrap();
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                &[candidate(&old_source)],
                &old_source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let item = &first.items[0];
        let dismissed = store
            .update_insight_review(
                &id,
                first.revision,
                &item.id,
                item.revision,
                &InsightReviewUpdate {
                    review_state: Some(InsightReviewState::Dismissed),
                    ..Default::default()
                },
            )
            .unwrap();
        let old_id = legacy.metadata.utterance_id.clone();
        legacy.metadata.utterance_id = uuid::Uuid::now_v7().to_string();
        legacy.metadata.source_id = Some("room_microphone".into());
        legacy.metadata.lineage = vec![old_id];
        let upgraded = store
            .write_transcript_snapshot(&id, Some(old_source.revision), &[legacy])
            .unwrap();
        let after = store
            .merge_insight_candidates(
                &id,
                dismissed.revision,
                &[candidate(&upgraded)],
                &upgraded,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(after.items.len(), 1);
        assert_eq!(after.items[0].id, item.id);
        assert_eq!(after.items[0].review_state, InsightReviewState::Dismissed);
        assert_eq!(after.items[0].source_state, InsightSourceState::Missing);
    }

    #[test]
    fn source_revision_cutoff_and_exact_supported_unicode_details_are_verified() {
        let (_directory, store, id, source) = fixture();
        let text = "Élodie zal vrijdag rapporteren, yes.";
        let row = MeetingUtterance::new(
            Segment {
                start_ms: 1_000,
                end_ms: 5_000,
                speaker: Speaker::Others,
                text: text.into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        );
        let source = store
            .write_transcript_snapshot(&id, Some(source.revision), &[row])
            .unwrap();
        let mut valid = candidate(&source);
        valid.processing.language_policy = "mixed".into();
        valid.owner = Some(SupportedInsightDetail {
            text: "Élodie".into(),
            source_index: 0,
            start_byte: 0,
            end_byte: "Élodie".len() as u32,
        });
        let start = text.find("vrijdag").unwrap() as u32;
        valid.due = Some(SupportedInsightDetail {
            text: "vrijdag".into(),
            source_index: 0,
            start_byte: start,
            end_byte: start + 7,
        });
        let first = store
            .merge_insight_candidates(
                &id,
                0,
                std::slice::from_ref(&valid),
                &source,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert_eq!(first.items[0].owner.as_ref().unwrap().text, "Élodie");
        let before = fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap();
        for broken in 0..3 {
            let mut invalid = valid.clone();
            match broken {
                0 => invalid.transcript_digest = "f".repeat(64),
                1 => invalid.transcript_revision += 1,
                _ => invalid.evidence_cutoff_ms = 500_000,
            }
            assert!(
                store
                    .merge_insight_candidates(
                        &id,
                        first.revision,
                        &[invalid],
                        &source,
                        &InferenceCancellation::new()
                    )
                    .is_err()
            );
        }
        assert_eq!(
            fs::read(store.dir_of(&id).join(INSIGHTS_FILE)).unwrap(),
            before
        );
    }
}
