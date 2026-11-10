use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::{Passage, PassageKind, Segment, Speaker};

pub(crate) const AUTOMATIC_NAME_ESTIMATE_RULE: &str = "Names marked Teams estimate are revisable estimates, not confirmed identities or action owners. A first-person commitment under an estimated label alone does not establish who owns it. Keep an owner unknown unless the source independently supports the assignment. Preserve estimate qualifications in intermediate and final summaries or answers.";

enum UtteranceSpeakerLabel {
    Name(String),
    TeamsEstimate(String),
    Number(u32),
    SelfVoice,
    Unknown,
    NeedsReview,
    Mixed,
    Overlap,
}

pub(crate) fn automatic_utterance_speaker_name(row: &crate::MeetingUtterance) -> Option<&str> {
    row.metadata
        .automatic_name
        .as_ref()
        .filter(|proof| {
            row.metadata.manual.is_none()
                && row.metadata.unresolved_manual.is_empty()
                && row.metadata.attribution.kind == crate::AttributionKind::Single
                && row.metadata.attribution.voices.len() == 1
                && !row.metadata.attribution.self_voice
                && !row.metadata.attribution.provisional
                && proof.source_fingerprint == row.metadata.source_fingerprint
                && proof.attribution_revision == row.metadata.attribution_revision
        })
        .map(|proof| proof.name.as_str())
}

fn utterance_speaker_identity(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
) -> UtteranceSpeakerLabel {
    let manual = row.metadata.manual.as_ref().filter(|manual| {
        !manual.review_required && manual.source_fingerprint == row.metadata.source_fingerprint
    });
    if let Some(name) = manual
        .and_then(|manual| manual.name.as_deref())
        .filter(|name| !name.trim().is_empty())
    {
        return UtteranceSpeakerLabel::Name(name.into());
    }
    if (row.metadata.manual.is_some() && manual.is_none())
        || !row.metadata.unresolved_manual.is_empty()
    {
        return UtteranceSpeakerLabel::NeedsReview;
    }
    if let Some(name) = automatic_utterance_speaker_name(row) {
        return UtteranceSpeakerLabel::TeamsEstimate(name.into());
    }
    let voice = manual.and_then(|manual| manual.voice).or_else(|| {
        (row.metadata.attribution.kind == crate::AttributionKind::Single
            && row.metadata.attribution.voices.len() == 1)
            .then(|| row.metadata.attribution.voices[0])
    });
    if let Some(voice) = voice {
        return names
            .iter()
            .find(|name| name.voice == voice)
            .map(|name| UtteranceSpeakerLabel::Name(name.name.clone()))
            .unwrap_or(UtteranceSpeakerLabel::Number(voice));
    }
    if row.metadata.attribution.kind == crate::AttributionKind::Single
        && row.metadata.attribution.self_voice
    {
        return UtteranceSpeakerLabel::SelfVoice;
    }
    match row.metadata.attribution.kind {
        crate::AttributionKind::Mixed => UtteranceSpeakerLabel::Mixed,
        crate::AttributionKind::Overlap => UtteranceSpeakerLabel::Overlap,
        _ => UtteranceSpeakerLabel::Unknown,
    }
}

/// A single settled identity; estimates and unresolved source corrections never name a person.
pub(crate) fn settled_utterance_speaker_label(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
) -> Option<String> {
    settled_utterance_speaker_identity(row, names).map(|(_, label)| label)
}

pub(crate) fn settled_utterance_speaker_identity(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
) -> Option<(String, String)> {
    if provisional_speaker_estimate(row) {
        return None;
    }
    match utterance_speaker_identity(row, names) {
        UtteranceSpeakerLabel::Name(name) => {
            let manual = row.metadata.manual.as_ref().filter(|manual| {
                !manual.review_required
                    && manual.source_fingerprint == row.metadata.source_fingerprint
            });
            let key = if let Some(explicit) = manual
                .and_then(|manual| manual.name.as_ref())
                .filter(|name| !name.trim().is_empty())
            {
                format!(
                    "manual-name:{:?}:{explicit}",
                    manual.and_then(|manual| manual.voice)
                )
            } else {
                let voice = manual
                    .and_then(|manual| manual.voice)
                    .or_else(|| row.metadata.attribution.voices.first().copied())?;
                format!("voice:{voice}")
            };
            Some((key, name))
        }
        UtteranceSpeakerLabel::Number(voice) => {
            Some((format!("voice:{voice}"), format!("Speaker {voice}")))
        }
        UtteranceSpeakerLabel::SelfVoice => Some(("self".into(), "Me".into())),
        _ => None,
    }
}

pub(crate) fn utterance_speaker_label(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
) -> String {
    let label = match utterance_speaker_identity(row, names) {
        UtteranceSpeakerLabel::Name(name) => name,
        UtteranceSpeakerLabel::TeamsEstimate(name) => crate::ui_text(
            crate::InterfaceLanguage::English,
            "meetings.speaker_teams_estimate",
            &[("name", &name)],
        ),
        UtteranceSpeakerLabel::Number(voice) => format!("Speaker {voice}"),
        UtteranceSpeakerLabel::SelfVoice => "Me".into(),
        UtteranceSpeakerLabel::Unknown => "Unknown speaker".into(),
        UtteranceSpeakerLabel::NeedsReview => "Unknown speaker (correction needs review)".into(),
        UtteranceSpeakerLabel::Mixed => "Mixed speakers".into(),
        UtteranceSpeakerLabel::Overlap => "Overlapping speakers".into(),
    };
    if provisional_speaker_estimate(row) {
        format!("{label} (provisional)")
    } else {
        label
    }
}

fn provisional_speaker_estimate(row: &crate::MeetingUtterance) -> bool {
    row.metadata.attribution.provisional
        && !row.metadata.manual.as_ref().is_some_and(|manual| {
            !manual.review_required
                && manual.source_fingerprint == row.metadata.source_fingerprint
                && (manual.voice.is_some()
                    || manual
                        .name
                        .as_ref()
                        .is_some_and(|name| !name.trim().is_empty()))
        })
}

fn localized_settled_utterance_speaker_label(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
    language: crate::InterfaceLanguage,
) -> String {
    let key = match utterance_speaker_identity(row, names) {
        UtteranceSpeakerLabel::Name(name) => return name,
        UtteranceSpeakerLabel::TeamsEstimate(name) => {
            return crate::ui_text(
                language,
                "meetings.speaker_teams_estimate",
                &[("name", &name)],
            );
        }
        UtteranceSpeakerLabel::Number(voice) => {
            return crate::ui_text(
                language,
                "meetings.speaker_number",
                &[("number", &voice.to_string())],
            );
        }
        UtteranceSpeakerLabel::SelfVoice => "meetings.speaker_me",
        UtteranceSpeakerLabel::Unknown => "meetings.attribution_unknown",
        UtteranceSpeakerLabel::NeedsReview => {
            return format!(
                "{} ({})",
                crate::ui_text(language, "meetings.attribution_unknown", &[]),
                crate::ui_text(language, "meetings.attribution_correction_review", &[])
                    .trim_end_matches('.')
            );
        }
        UtteranceSpeakerLabel::Mixed => "meetings.attribution_mixed",
        UtteranceSpeakerLabel::Overlap => "meetings.attribution_overlap",
    };
    crate::ui_text(language, key, &[])
}

pub(crate) fn localized_utterance_speaker_label(
    row: &crate::MeetingUtterance,
    names: &[crate::SpeakerName],
    language: crate::InterfaceLanguage,
) -> String {
    let label = localized_settled_utterance_speaker_label(row, names, language);
    if provisional_speaker_estimate(row) {
        format!(
            "{label} ({})",
            crate::ui_text(language, "meetings.live_speakers_provisional_tag", &[])
        )
    } else {
        label
    }
}

/// Desktop request evidence; this is deliberately independent of the persisted/UniFFI records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeetingSourceAnchor {
    pub passage: Passage,
    pub segment_keys: Vec<String>,
    pub segment_fingerprints: Vec<String>,
    pub notes_start: Option<usize>,
    pub notes_end: Option<usize>,
    pub source_text: String,
}

#[derive(Debug, Clone)]
pub struct MeetingEvidence {
    pub sources: Vec<Passage>,
    pub anchors: Vec<MeetingSourceAnchor>,
    pub notes_revision: String,
    pub included_through_ms: u64,
}

pub fn evidence_revision(text: &str) -> String {
    crate::models::hex(&Sha256::digest(text.as_bytes()))
}

pub fn segment_evidence_revision(segment: &Segment) -> String {
    let text = if segment.failed {
        String::new()
    } else {
        segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let voice = (segment.speaker == Speaker::Others)
        .then_some(segment.voice)
        .flatten();
    evidence_revision(
        &serde_json::json!([
            segment.start_ms,
            segment.speaker,
            text,
            segment.failed,
            voice
        ])
        .to_string(),
    )
}

pub fn utterance_evidence_revision(row: &crate::MeetingUtterance) -> String {
    let text = if row.segment.failed {
        String::new()
    } else {
        row.segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    evidence_revision(
        &serde_json::json!([
            row.metadata.utterance_id,
            row.metadata.source_id,
            row.segment.start_ms,
            row.segment.end_ms,
            text,
            row.segment.failed
        ])
        .to_string(),
    )
}

fn parts(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        if offset == text.len() {
            return None;
        }
        let start = offset;
        offset = (offset + 600).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        Some((start, &text[start..offset]))
    })
}

impl MeetingEvidence {
    /// Citation numbers follow the selected model context, including repeated equal passages.
    pub fn anchors_for_sources(
        &self,
        sources: &[Passage],
    ) -> Result<Vec<MeetingSourceAnchor>, crate::PennelError> {
        let mut remaining = self.anchors.iter().map(Some).collect::<Vec<_>>();
        sources
            .iter()
            .map(|source| {
                let index = remaining
                    .iter()
                    .position(|anchor| anchor.is_some_and(|anchor| anchor.passage == *source))
                    .ok_or_else(|| crate::PennelError::InvalidResponse {
                        message: "The question source could not be verified; it was not sent."
                            .into(),
                    })?;
                Ok(remaining[index].take().unwrap().clone())
            })
            .collect()
    }

    pub fn new(uid: &str, title: &str, start_iso: &str, notes: &str, segments: &[Segment]) -> Self {
        Self::new_with_speakers(uid, title, start_iso, notes, segments, &[])
    }

    pub fn new_with_speakers(
        uid: &str,
        title: &str,
        start_iso: &str,
        notes: &str,
        segments: &[Segment],
        names: &[crate::SpeakerName],
    ) -> Self {
        let mut anchors = Vec::new();
        let mut occurrences = HashMap::new();
        let passage = |kind, start_ms, text: String| Passage {
            meeting_id: uid.into(),
            title: title.into(),
            start_iso: start_iso.into(),
            kind,
            start_ms,
            snippet: text.chars().take(180).collect(),
            text,
            meeting_uid: None,
            utterance_ids: Vec::new(),
            utterance_fingerprints: Vec::new(),
        };
        let mut included_through_ms = 0;
        for segment in segments {
            let speaker = match segment.speaker {
                Speaker::Me => "me",
                Speaker::Others => "others",
            };
            let occurrence = occurrences
                .entry((speaker, segment.start_ms))
                .or_insert(0_u32);
            let key = format!("{speaker}:{}:{occurrence}", segment.start_ms);
            *occurrence += 1;
            if segment.failed || segment.text.trim().is_empty() {
                continue;
            }
            included_through_ms = included_through_ms.max(segment.end_ms);
            let label = crate::meetings::speaker_label(segment, names);
            for (_, text) in parts(&segment.text) {
                anchors.push(MeetingSourceAnchor {
                    passage: passage(
                        PassageKind::Transcript,
                        Some(segment.start_ms),
                        format!("{label}: {text}"),
                    ),
                    segment_keys: vec![key.clone()],
                    segment_fingerprints: vec![segment_evidence_revision(segment)],
                    notes_start: None,
                    notes_end: None,
                    source_text: text.into(),
                });
            }
        }
        let mut line_offset = 0;
        for line in notes.split_inclusive('\n') {
            let source = line.trim_end_matches(['\r', '\n']);
            if !source.trim().is_empty() {
                for (offset, text) in parts(source) {
                    let start = notes[..line_offset + offset].encode_utf16().count();
                    anchors.push(MeetingSourceAnchor {
                        passage: passage(PassageKind::Notes, None, text.into()),
                        segment_keys: Vec::new(),
                        segment_fingerprints: Vec::new(),
                        notes_start: Some(start),
                        notes_end: Some(start + text.encode_utf16().count()),
                        source_text: text.into(),
                    });
                }
            }
            line_offset += line.len();
        }
        Self {
            sources: anchors
                .iter()
                .map(|anchor| anchor.passage.clone())
                .collect(),
            anchors,
            notes_revision: evidence_revision(notes),
            included_through_ms,
        }
    }

    pub fn new_with_utterances(
        uid: &str,
        title: &str,
        start_iso: &str,
        notes: &str,
        rows: &[crate::MeetingUtterance],
        names: &[crate::SpeakerName],
    ) -> Result<Self, crate::PennelError> {
        let mut identities = std::collections::HashSet::new();
        if uid.trim().is_empty()
            || rows.iter().any(|row| {
                row.metadata.utterance_id.trim().is_empty()
                    || !identities.insert(row.metadata.utterance_id.as_str())
            })
        {
            return Err(crate::PennelError::InvalidResponse {
                message: "The question's utterance sources are ambiguous; it was not sent.".into(),
            });
        }
        let mut result = Self::new(uid, title, start_iso, notes, &[]);
        for anchor in &mut result.anchors {
            anchor.passage.meeting_uid = Some(uid.into());
        }
        let mut anchors = Vec::new();
        for row in rows
            .iter()
            .filter(|row| !row.segment.failed && !row.segment.text.trim().is_empty())
        {
            result.included_through_ms = result.included_through_ms.max(row.segment.end_ms);
            let label = utterance_speaker_label(row, names);
            let fingerprint = utterance_evidence_revision(row);
            for (_, text) in parts(&row.segment.text) {
                let labeled = format!("{label}: {text}");
                anchors.push(MeetingSourceAnchor {
                    passage: Passage {
                        meeting_id: uid.into(),
                        title: title.into(),
                        start_iso: start_iso.into(),
                        kind: PassageKind::Transcript,
                        start_ms: Some(row.segment.start_ms),
                        snippet: labeled.chars().take(180).collect(),
                        text: labeled,
                        meeting_uid: Some(uid.into()),
                        utterance_ids: vec![row.metadata.utterance_id.clone()],
                        utterance_fingerprints: vec![fingerprint.clone()],
                    },
                    segment_keys: vec![format!("utterance:{}", row.metadata.utterance_id)],
                    segment_fingerprints: vec![fingerprint.clone()],
                    notes_start: None,
                    notes_end: None,
                    source_text: text.into(),
                });
            }
        }
        anchors.extend(result.anchors);
        result.sources = anchors
            .iter()
            .map(|anchor| anchor.passage.clone())
            .collect();
        result.anchors = anchors;
        Ok(result)
    }

    pub fn selected_sources(&self, question: &str, apple: bool) -> Vec<Passage> {
        let size = self
            .sources
            .iter()
            .map(|source| source.text.len())
            .sum::<usize>();
        if size <= crate::meeting_chat::WHOLE_MEETING_CHARS && !apple {
            return self.sources.clone();
        }
        let terms = crate::meeting_chat::search_terms(question);
        let mut scored = self
            .sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let text = source.text.to_lowercase();
                let score = terms
                    .iter()
                    .filter(|term| text.contains(term.as_str()))
                    .count();
                (score, index, source)
            })
            .collect::<Vec<_>>();
        scored.sort_by_key(|(score, index, _)| {
            (std::cmp::Reverse(*score), std::cmp::Reverse(*index))
        });
        scored
            .into_iter()
            .take(crate::meeting_chat::MEETING_PASSAGES as usize)
            .map(|(_, _, source)| source.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PassageKind, Segment, Speaker};

    fn segment(text: &str, start_ms: u64) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 3_000,
            speaker: Speaker::Me,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    #[test]
    fn canonical_anchors_survive_attribution_changes_but_not_source_edits() {
        let mut row = crate::MeetingUtterance::new(
            segment("Ik stuur de offerte morgen.", 1_250),
            Some("microphone".into()),
        );
        row.metadata.attribution = crate::AutomaticAttribution {
            kind: crate::AttributionKind::Single,
            voices: vec![1],
            turns: vec![],
            self_voice: false,
            provisional: false,
        };
        let before =
            MeetingEvidence::new_with_utterances("uid", "M", "", "", &[row.clone()], &[]).unwrap();
        row.segment.speaker = Speaker::Others;
        row.segment.voice = Some(2);
        row.metadata.attribution.voices = vec![2];
        row.metadata.attribution_revision += 1;
        let after =
            MeetingEvidence::new_with_utterances("uid", "Renamed", "", "", &[row.clone()], &[])
                .unwrap();
        assert_eq!(
            before.anchors[0].segment_keys,
            vec![format!("utterance:{}", row.metadata.utterance_id)]
        );
        assert_eq!(
            before.anchors[0].segment_keys,
            after.anchors[0].segment_keys
        );
        assert_eq!(
            before.anchors[0].segment_fingerprints,
            after.anchors[0].segment_fingerprints
        );
        assert_eq!(
            after.sources[0].utterance_ids,
            vec![row.metadata.utterance_id.clone()]
        );
        assert_eq!(after.sources[0].meeting_uid.as_deref(), Some("uid"));
        row.segment.text.push_str(" Changed.");
        let changed =
            MeetingEvidence::new_with_utterances("uid", "M", "", "", &[row], &[]).unwrap();
        assert_ne!(
            before.anchors[0].segment_fingerprints,
            changed.anchors[0].segment_fingerprints
        );
        assert!(before.anchors_for_sources(&changed.sources).is_err());
    }

    #[test]
    fn canonical_sources_preserve_unicode_uncertainty_and_valid_manual_names() {
        let text = "😀 één échte zin\u{0085}met\u{a0}ruimte. ".repeat(40);
        let mut row = crate::MeetingUtterance::new(segment(&text, 100), Some("microphone".into()));
        for (kind, expected) in [
            (crate::AttributionKind::Unknown, "Unknown speaker"),
            (crate::AttributionKind::Mixed, "Mixed speakers"),
            (crate::AttributionKind::Overlap, "Overlapping speakers"),
        ] {
            row.metadata.attribution.kind = kind;
            let evidence = MeetingEvidence::new_with_utterances(
                "uid",
                "M",
                "",
                "😀 notes",
                &[row.clone()],
                &[],
            )
            .unwrap();
            assert!(evidence.sources[0].text.starts_with(expected));
            assert_eq!(
                evidence
                    .anchors
                    .iter()
                    .filter(|anchor| anchor.passage.kind == PassageKind::Transcript)
                    .map(|anchor| anchor.source_text.as_str())
                    .collect::<String>(),
                text
            );
        }
        row.metadata.manual = Some(crate::ManualCorrection {
            self_voice: false,
            name: Some("Ada".into()),
            voice: Some(3),
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        let corrected =
            MeetingEvidence::new_with_utterances("uid", "M", "", "", &[row.clone()], &[]).unwrap();
        assert!(corrected.sources[0].text.starts_with("Ada:"));
        row.metadata.manual.as_mut().unwrap().source_fingerprint = "different-source".into();
        assert_eq!(
            utterance_speaker_label(&row, &[]),
            "Unknown speaker (correction needs review)"
        );
        row.metadata.manual.as_mut().unwrap().review_required = true;
        let uncertain =
            MeetingEvidence::new_with_utterances("uid", "M", "", "", &[row], &[]).unwrap();
        assert!(!uncertain.sources[0].text.starts_with("Ada:"));
    }

    #[test]
    fn canonical_evidence_refuses_ambiguous_duplicate_utterance_ids() {
        let row = crate::MeetingUtterance::new(segment("Ja.", 100), Some("microphone".into()));
        assert!(
            MeetingEvidence::new_with_utterances("uid", "M", "", "", &[row.clone(), row], &[])
                .is_err()
        );
    }

    #[test]
    fn enrolled_self_attribution_is_explicit_and_never_inferred_from_the_track() {
        let mut row = crate::MeetingUtterance::new(segment("Ja.", 100), Some("microphone".into()));
        assert_eq!(utterance_speaker_label(&row, &[]), "Unknown speaker");
        row.metadata.attribution.kind = crate::AttributionKind::Single;
        row.metadata.attribution.self_voice = true;
        row.segment.speaker = Speaker::Others;
        row.metadata.source_id = Some("system_audio".into());
        assert_eq!(utterance_speaker_label(&row, &[]), "Me");
    }

    #[test]
    fn live_speaker_estimates_are_qualified_without_changing_durable_evidence() {
        let mut row = crate::MeetingUtterance::new(
            segment("Finalized words", 100),
            Some("microphone".into()),
        );
        row.metadata.attribution.kind = crate::AttributionKind::Single;
        row.metadata.attribution.voices = vec![1];
        row.metadata.attribution.provisional = true;
        let provisional = MeetingEvidence::new_with_utterances(
            "uid",
            "M",
            "",
            "",
            std::slice::from_ref(&row),
            &[],
        )
        .unwrap();
        assert_eq!(
            provisional.sources[0].text,
            "Speaker 1 (provisional): Finalized words"
        );
        let fingerprint = utterance_evidence_revision(&row);
        row.metadata.attribution.provisional = false;
        let final_evidence = MeetingEvidence::new_with_utterances(
            "uid",
            "M",
            "",
            "",
            std::slice::from_ref(&row),
            &[],
        )
        .unwrap();
        assert_eq!(final_evidence.sources[0].text, "Speaker 1: Finalized words");
        assert_eq!(
            provisional.anchors[0].segment_keys,
            final_evidence.anchors[0].segment_keys
        );
        assert_eq!(
            provisional.anchors[0].segment_fingerprints,
            final_evidence.anchors[0].segment_fingerprints
        );
        assert_eq!(fingerprint, utterance_evidence_revision(&row));
        row.metadata.attribution.provisional = true;
        row.metadata.manual = Some(crate::ManualCorrection {
            self_voice: false,
            name: Some("Ada".into()),
            voice: None,
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        assert_eq!(utterance_speaker_label(&row, &[]), "Ada");
        row.metadata.manual.as_mut().unwrap().name = None;
        row.metadata.manual.as_mut().unwrap().voice = Some(2);
        assert_eq!(utterance_speaker_label(&row, &[]), "Speaker 2");
        row.metadata.manual = None;
        assert_eq!(
            utterance_speaker_label(
                &row,
                &[crate::SpeakerName {
                    voice: 1,
                    name: "Ada".into()
                }]
            ),
            "Ada (provisional)"
        );
        for (kind, label) in [
            (crate::AttributionKind::Unknown, "Unknown speaker"),
            (crate::AttributionKind::Mixed, "Mixed speakers"),
            (crate::AttributionKind::Overlap, "Overlapping speakers"),
        ] {
            row.metadata.attribution.kind = kind;
            assert_eq!(
                utterance_speaker_label(&row, &[]),
                format!("{label} (provisional)")
            );
        }
    }

    #[test]
    fn snapshot_owns_finalized_evidence_and_raw_note_ranges() {
        let mut segments = vec![segment("Approved budget", 4_000)];
        let notes = "# Notes\n😀 exact Markdown\n";
        let frozen = MeetingEvidence::new("uid", "Meeting", "2026-11-02", notes, &segments);
        segments.push(segment("Later secret", 20_000));
        assert_eq!(frozen.included_through_ms, 7_000);
        assert_eq!(frozen.sources.len(), 3);
        assert!(
            frozen
                .sources
                .iter()
                .all(|source| source.meeting_id == "uid")
        );
        assert!(
            !frozen
                .sources
                .iter()
                .any(|source| source.text.contains("Later secret"))
        );
        let note = frozen
            .anchors
            .iter()
            .find(|anchor| anchor.source_text.starts_with('😀'))
            .unwrap();
        assert_eq!(note.notes_start, Some(8));
        assert_eq!(note.notes_end, Some(25));
        assert_eq!(frozen.notes_revision, evidence_revision(notes));
    }

    #[test]
    fn duplicate_timestamp_rows_have_stable_keys_and_separate_content_checks() {
        let segments = [segment("first", 4_000), segment("second", 4_000)];
        let before = MeetingEvidence::new("uid", "M", "", "", &segments);
        let mut extended = segments.to_vec();
        extended.push(segment("new", 9_000));
        let after = MeetingEvidence::new("uid", "renamed", "", "", &extended);
        assert_eq!(before.anchors[0].segment_keys, ["me:4000:0"]);
        assert_eq!(before.anchors[1].segment_keys, ["me:4000:1"]);
        assert_eq!(
            before.anchors[1].segment_keys,
            after.anchors[1].segment_keys
        );
        assert_ne!(
            before.anchors[0].segment_fingerprints,
            before.anchors[1].segment_fingerprints
        );
    }

    #[test]
    fn failed_and_empty_rows_are_not_answer_evidence() {
        let mut failed = segment("Not recognized", 1_000);
        failed.failed = true;
        let evidence = MeetingEvidence::new("uid", "M", "", "", &[failed, segment(" ", 2_000)]);
        assert!(evidence.sources.is_empty());
        assert_eq!(evidence.included_through_ms, 0);
    }

    #[test]
    fn bounded_selection_uses_frozen_candidates_only() {
        let segments = (0..150)
            .map(|i| {
                segment(
                    &format!(
                        "{} {}",
                        if i == 20 { "budget" } else { "other" },
                        "x".repeat(500)
                    ),
                    i * 5_000,
                )
            })
            .collect::<Vec<_>>();
        let frozen = MeetingEvidence::new("uid", "M", "", "", &segments);
        let sources = frozen.selected_sources("budget", false);
        assert!(sources.len() <= 25);
        assert!(sources.iter().any(|source| source.text.contains("budget")));
        assert!(
            sources
                .iter()
                .all(|source| source.kind == PassageKind::Transcript)
        );
    }

    #[test]
    fn numbered_anchors_keep_repeated_note_occurrences_in_citation_order() {
        let evidence = MeetingEvidence::new("uid", "M", "", "Repeated note\nRepeated note\n", &[]);
        let anchors = evidence.anchors_for_sources(&evidence.sources).unwrap();
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].notes_start, Some(0));
        assert_eq!(anchors[1].notes_start, Some(14));
        let cited = crate::meeting_chat::cited("The second occurrence [2].", &evidence.sources);
        assert_eq!(anchors[cited[0].number as usize - 1].notes_start, Some(14));
    }

    #[test]
    fn numbered_anchors_follow_retrieval_and_reduced_context_source_order() {
        let evidence = MeetingEvidence::new("uid", "M", "", "First\nSecond\nThird\n", &[]);
        let reordered = vec![evidence.sources[2].clone(), evidence.sources[0].clone()];
        let anchors = evidence.anchors_for_sources(&reordered).unwrap();
        assert_eq!(anchors[0].source_text, "Third");
        assert_eq!(anchors[1].source_text, "First");
        let (_, selected) =
            crate::meeting_chat::apple_chat_messages("Question", &[], &reordered).unwrap();
        let apple_anchors = evidence.anchors_for_sources(&selected).unwrap();
        assert_eq!(apple_anchors[0].passage, selected[0]);
        assert_eq!(apple_anchors[1].passage, selected[1]);
    }

    #[test]
    fn live_segment_anchor_survives_the_actual_markdown_storage_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::meetings::MeetingStore::new(root.path());
        let id = store.create("M", chrono::Local::now()).unwrap();
        let mut live = segment("  Alpha\n\tBeta\u{0085}Gamma\u{a0}Delta  ", 4_321);
        live.speaker = Speaker::Others;
        live.voice = Some(2);
        let frozen = MeetingEvidence::new("uid", "M", "", "", &[live.clone()]);
        store.write_transcript(&id, &[live.clone()]).unwrap();
        let stored = store.read_transcript(&id);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].end_ms, live.end_ms);
        assert_eq!(stored[0].text, "Alpha Beta Gamma Delta");
        let canonical = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(canonical.utterances[0].segment.text, live.text);
        let canonical_evidence = MeetingEvidence::new_with_utterances(
            &canonical.meeting_uid,
            "M",
            "",
            "",
            &canonical.utterances,
            &[],
        )
        .unwrap();
        assert_eq!(canonical_evidence.anchors[0].source_text, live.text);
        assert_eq!(
            segment_evidence_revision(&stored[0]),
            frozen.anchors[0].segment_fingerprints[0]
        );
        assert_eq!(frozen.included_through_ms, live.end_ms);
        let mut corrected = stored[0].clone();
        corrected.text.push_str(" changed");
        assert_ne!(
            segment_evidence_revision(&corrected),
            frozen.anchors[0].segment_fingerprints[0]
        );
    }

    #[test]
    fn source_passages_keep_existing_voice_numbers_and_saved_speaker_names() {
        let mut other = segment("I will prepare it", 1_000);
        other.speaker = Speaker::Others;
        other.voice = Some(2);
        let unnamed = MeetingEvidence::new("uid", "M", "", "", &[other.clone()]);
        assert_eq!(unnamed.sources[0].text, "Anderen 2: I will prepare it");
        let named = MeetingEvidence::new_with_speakers(
            "uid",
            "M",
            "",
            "",
            &[other],
            &[crate::SpeakerName {
                voice: 2,
                name: "Ada".into(),
            }],
        );
        assert_eq!(named.sources[0].text, "Ada: I will prepare it");
    }
}
