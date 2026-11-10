//! Disjoint speaker intervals derived only from durable utterance timing and attribution.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerTimelineKind {
    Person,
    Unknown,
    Mixed,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SpeakerTimelineSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SpeakerTimelineLane {
    pub key: String,
    pub speaker: Option<String>,
    pub kind: SpeakerTimelineKind,
    pub duration_ms: u64,
    pub spans: Vec<SpeakerTimelineSpan>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingSpeakerTimeline {
    pub duration_ms: u64,
    pub duration_is_recorded: bool,
    pub lanes: Vec<SpeakerTimelineLane>,
}

fn unidentified(kind: SpeakerTimelineKind) -> SpeakerTimelineLane {
    SpeakerTimelineLane {
        key: match kind {
            SpeakerTimelineKind::Mixed => "mixed",
            SpeakerTimelineKind::Overlap => "overlap",
            _ => "unknown",
        }
        .into(),
        speaker: None,
        kind,
        duration_ms: 0,
        spans: Vec::new(),
    }
}

fn row_lane(row: &crate::MeetingUtterance, names: &[crate::SpeakerName]) -> SpeakerTimelineLane {
    if let Some(speaker) = crate::summary_structure::settled_speaker(row, names) {
        let manual = row.metadata.manual.as_ref();
        // Display names can coincide; acoustic voice identity must not be merged by a label.
        let key = if let Some(name) = manual
            .and_then(|manual| manual.name.as_ref())
            .filter(|name| !name.trim().is_empty())
        {
            format!(
                "manual-name:{:?}:{name}",
                manual.and_then(|manual| manual.voice)
            )
        } else if manual.is_some_and(|manual| manual.self_voice)
            || row.metadata.attribution.self_voice
        {
            "self".into()
        } else if let Some(voice) = manual
            .and_then(|manual| manual.voice)
            .or_else(|| row.metadata.attribution.voices.first().copied())
        {
            format!("voice:{voice}")
        } else {
            return unidentified(SpeakerTimelineKind::Unknown);
        };
        return SpeakerTimelineLane {
            key,
            speaker: Some(speaker),
            kind: SpeakerTimelineKind::Person,
            duration_ms: 0,
            spans: Vec::new(),
        };
    }
    let unresolved = !row.metadata.unresolved_manual.is_empty()
        || row.metadata.manual.as_ref().is_some_and(|manual| {
            manual.review_required || manual.source_fingerprint != row.metadata.source_fingerprint
        });
    unidentified(if unresolved {
        SpeakerTimelineKind::Unknown
    } else {
        match row.metadata.attribution.kind {
            crate::AttributionKind::Mixed => SpeakerTimelineKind::Mixed,
            crate::AttributionKind::Overlap => SpeakerTimelineKind::Overlap,
            _ => SpeakerTimelineKind::Unknown,
        }
    })
}

/// Equal boundaries are processed together. Every millisecond belongs to at most one lane.
pub(crate) fn speaker_timeline(
    rows: &[crate::MeetingUtterance],
    names: &[crate::SpeakerName],
    recorded_duration: Option<u64>,
) -> MeetingSpeakerTimeline {
    let valid = |row: &&crate::MeetingUtterance| {
        !row.segment.failed
            && !row.segment.text.trim().is_empty()
            && row.segment.end_ms > row.segment.start_ms
    };
    let duration_ms = recorded_duration.unwrap_or_else(|| {
        rows.iter()
            .filter(valid)
            .map(|row| row.segment.end_ms)
            .max()
            .unwrap_or(0)
    });
    let mut events: BTreeMap<u64, Vec<(String, bool)>> = BTreeMap::new();
    let mut definitions = BTreeMap::new();
    for row in rows.iter().filter(valid) {
        let start = row.segment.start_ms.min(duration_ms);
        let end = row.segment.end_ms.min(duration_ms);
        if start >= end {
            continue;
        }
        let lane = row_lane(row, names);
        let key = lane.key.clone();
        definitions.entry(key.clone()).or_insert(lane);
        events.entry(start).or_default().push((key.clone(), true));
        events.entry(end).or_default().push((key, false));
    }
    let mut active: BTreeMap<String, usize> = BTreeMap::new();
    let mut lanes: BTreeMap<String, SpeakerTimelineLane> = BTreeMap::new();
    let mut previous = 0;
    for (at, changes) in events {
        if previous < at && !active.is_empty() {
            let definition = if active.len() == 1 {
                definitions[active.keys().next().expect("one active lane")].clone()
            } else {
                unidentified(SpeakerTimelineKind::Overlap)
            };
            let lane = lanes.entry(definition.key.clone()).or_insert(definition);
            lane.duration_ms += at - previous;
            if let Some(last) = lane.spans.last_mut().filter(|last| last.end_ms == previous) {
                last.end_ms = at;
            } else {
                lane.spans.push(SpeakerTimelineSpan {
                    start_ms: previous,
                    end_ms: at,
                });
            }
        }
        for (key, start) in changes {
            if start {
                *active.entry(key).or_default() += 1;
            } else if let Some(count) = active.get_mut(&key) {
                *count -= 1;
                if *count == 0 {
                    active.remove(&key);
                }
            }
        }
        previous = at;
    }
    let mut lanes: Vec<_> = lanes.into_values().collect();
    lanes.sort_by(|a, b| {
        a.spans[0]
            .start_ms
            .cmp(&b.spans[0].start_ms)
            .then(a.key.cmp(&b.key))
    });
    MeetingSpeakerTimeline {
        duration_ms,
        duration_is_recorded: recorded_duration.is_some(),
        lanes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        start: u64,
        end: u64,
        voice: Option<u32>,
        kind: crate::AttributionKind,
    ) -> crate::MeetingUtterance {
        let mut row = crate::MeetingUtterance::new(
            crate::Segment {
                start_ms: start,
                end_ms: end,
                speaker: crate::Speaker::Others,
                text: "Recorded speech".into(),
                failed: false,
                voice,
            },
            Some("system".into()),
        );
        row.metadata.attribution.kind = kind;
        row.metadata.attribution.voices = voice.into_iter().collect();
        row
    }

    #[test]
    fn timeline_clips_boundaries_merges_duplicate_speech_and_keeps_equal_names_separate() {
        let rows = vec![
            row(0, 1000, Some(1), crate::AttributionKind::Single),
            row(500, 1500, Some(1), crate::AttributionKind::Single),
            row(1500, 2500, Some(2), crate::AttributionKind::Single),
        ];
        let names = vec![
            crate::SpeakerName {
                voice: 1,
                name: "Sam".into(),
            },
            crate::SpeakerName {
                voice: 2,
                name: "Sam".into(),
            },
        ];
        let result = speaker_timeline(&rows, &names, Some(2000));
        assert_eq!(result.duration_ms, 2000);
        assert!(result.duration_is_recorded);
        assert_eq!(result.lanes.len(), 2);
        assert_ne!(result.lanes[0].key, result.lanes[1].key);
        assert_eq!(result.lanes[0].speaker.as_deref(), Some("Sam"));
        assert_eq!(result.lanes[0].duration_ms, 1500);
        assert_eq!(result.lanes[0].spans.len(), 1);
        assert_eq!(result.lanes[1].duration_ms, 500);
    }

    #[test]
    fn actual_overlap_and_ambiguous_rows_are_never_credited_to_a_person() {
        let rows = vec![
            row(0, 1000, Some(1), crate::AttributionKind::Single),
            row(500, 1500, Some(2), crate::AttributionKind::Single),
            row(1500, 2000, Some(1), crate::AttributionKind::Mixed),
            row(2000, 2500, Some(2), crate::AttributionKind::Overlap),
            row(2500, 3000, None, crate::AttributionKind::Unknown),
        ];
        let result = speaker_timeline(&rows, &[], None);
        assert_eq!(result.duration_ms, 3000);
        assert!(!result.duration_is_recorded);
        let person_ms: u64 = result
            .lanes
            .iter()
            .filter(|lane| lane.kind == SpeakerTimelineKind::Person)
            .map(|lane| lane.duration_ms)
            .sum();
        assert_eq!(person_ms, 1000);
        assert_eq!(
            result
                .lanes
                .iter()
                .find(|lane| lane.kind == SpeakerTimelineKind::Overlap)
                .unwrap()
                .duration_ms,
            1000
        );
        assert_eq!(
            result
                .lanes
                .iter()
                .find(|lane| lane.kind == SpeakerTimelineKind::Mixed)
                .unwrap()
                .duration_ms,
            500
        );
        assert_eq!(
            result
                .lanes
                .iter()
                .find(|lane| lane.kind == SpeakerTimelineKind::Unknown)
                .unwrap()
                .duration_ms,
            500
        );
        assert_eq!(
            result
                .lanes
                .iter()
                .map(|lane| lane.duration_ms)
                .sum::<u64>(),
            3000
        );
    }

    #[test]
    fn zero_failed_empty_reversed_and_outside_rows_do_not_create_speaking_time() {
        let mut failed = row(0, 1000, Some(1), crate::AttributionKind::Single);
        failed.segment.failed = true;
        let mut empty = row(0, 1000, Some(1), crate::AttributionKind::Single);
        empty.segment.text.clear();
        let rows = vec![
            failed,
            empty,
            row(10, 10, Some(1), crate::AttributionKind::Single),
            row(900, 100, Some(2), crate::AttributionKind::Single),
            row(2000, 3000, Some(2), crate::AttributionKind::Single),
        ];
        assert!(speaker_timeline(&rows, &[], Some(1000)).lanes.is_empty());
        assert!(speaker_timeline(&rows, &[], Some(0)).lanes.is_empty());
        assert_eq!(speaker_timeline(&[], &[], None).duration_ms, 0);
    }

    #[test]
    fn unreviewed_or_provisional_identity_is_unknown_but_valid_manual_identity_wins() {
        let mut provisional = row(0, 1000, Some(1), crate::AttributionKind::Single);
        provisional.metadata.attribution.provisional = true;
        let mut reviewed = provisional.clone();
        reviewed.segment.start_ms = 1000;
        reviewed.segment.end_ms = 2000;
        reviewed.metadata.refresh_fingerprints(&reviewed.segment);
        reviewed.metadata.manual = Some(crate::ManualCorrection {
            name: Some("Noor".into()),
            voice: None,
            source_fingerprint: reviewed.metadata.source_fingerprint.clone(),
            review_required: false,
            self_voice: false,
        });
        let mut stale = row(2000, 3000, Some(1), crate::AttributionKind::Single);
        stale.metadata.manual = Some(crate::ManualCorrection {
            name: Some("Invented".into()),
            voice: None,
            source_fingerprint: "stale".into(),
            review_required: false,
            self_voice: false,
        });
        let result = speaker_timeline(&[provisional, reviewed, stale], &[], None);
        let known = result
            .lanes
            .iter()
            .find(|lane| lane.kind == SpeakerTimelineKind::Person)
            .unwrap();
        assert_eq!(known.speaker.as_deref(), Some("Noor"));
        assert_eq!(known.duration_ms, 1000);
        assert_eq!(
            result
                .lanes
                .iter()
                .find(|lane| lane.kind == SpeakerTimelineKind::Unknown)
                .unwrap()
                .duration_ms,
            2000
        );
    }

    #[test]
    fn timeline_is_order_independent_and_extreme_times_do_not_overflow() {
        let rows = vec![
            row(
                u64::MAX - 10,
                u64::MAX,
                Some(1),
                crate::AttributionKind::Single,
            ),
            row(0, 1, None, crate::AttributionKind::Unknown),
        ];
        let mut reversed = rows.clone();
        reversed.reverse();
        assert_eq!(
            speaker_timeline(&rows, &[], None),
            speaker_timeline(&reversed, &[], None)
        );
        assert_eq!(
            speaker_timeline(&rows, &[], None)
                .lanes
                .iter()
                .map(|lane| lane.duration_ms)
                .sum::<u64>(),
            11
        );
    }
}
