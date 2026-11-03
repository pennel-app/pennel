use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::{Passage, PassageKind, Segment, Speaker};

/// Desktop request evidence; this is deliberately independent of the persisted/UniFFI records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
        assert_eq!(stored[0].end_ms, stored[0].start_ms);
        assert_eq!(stored[0].text, "Alpha Beta Gamma Delta");
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
