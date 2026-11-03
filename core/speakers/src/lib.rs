//! Who spoke when, and whose voice it was.
//!
//! The model-free half (voiceprint maths, joining chunked results, matching against known voices) is
//! always compiled, so a device without the native runtime can still match the result of a remote
//! diarization against voiceprints that never leave it. The `onnx` feature adds the models.

use serde::{Deserialize, Serialize};

#[cfg(feature = "onnx")]
mod pipeline;
#[cfg(feature = "onnx")]
pub use pipeline::{Diarizer, Embedder, Models, Options, diarize, embed};

pub const SAMPLE_RATE: u32 = 16_000;

/// Distance at which the diarizer stops merging clusters within one recording. Measured on Dutch and
/// English synthetic voices and public Mandarin test speech: same speaker ≥ 0.67, others ≤ 0.46 cosine.
pub const CLUSTER_THRESHOLD: f32 = 0.6;
/// Cosine similarity from which a meeting voice is the user. Enrollment is clean read speech and
/// meetings are not, so this sits below the threshold for other known voices.
pub const ME_THRESHOLD: f32 = 0.5;
pub const KNOWN_THRESHOLD: f32 = 0.55;
/// Joining the per-chunk clusters of one meeting: the same voice in another chunk.
pub const MERGE_THRESHOLD: f32 = 0.55;
/// Shorter enrollment audio gives an unstable voiceprint.
pub const MIN_ENROLLMENT_MS: u64 = 10_000;
/// Turns shorter than this make noisy embeddings and are skipped when a longer turn exists.
pub const MIN_VOICEPRINT_TURN_MS: u64 = 1_000;
/// Speech per speaker used for its voiceprint; more adds time, not accuracy.
pub const VOICEPRINT_BUDGET_MS: u64 = 60_000;
/// A known voiceprint keeps learning, but one meeting may not overwrite it.
const MAX_BLEND_WEIGHT: u32 = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Index into [`Diarization::speakers`].
    pub speaker: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    pub speaker: u32,
    pub speech_ms: u64,
    /// L2-normalised voiceprint; empty when the speaker never spoke long enough for one.
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diarization {
    pub turns: Vec<Turn>,
    pub speakers: Vec<Speaker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The model files are missing or could not be loaded.
    Model(String),
    /// The audio could not be processed.
    Audio(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Model(detail) => write!(f, "could not load speaker model: {detail}"),
            Error::Audio(detail) => write!(f, "speaker recognition failed: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

pub fn normalized(vector: &[f32]) -> Vec<f32> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Vec::new();
    }
    vector.iter().map(|x| x / norm).collect()
}

/// Cosine similarity; 0 for empty or mismatched vectors so they never match anything.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norms <= f32::EPSILON {
        0.0
    } else {
        dot / norms
    }
}

/// Weighted mean of voiceprints of the same dimension, normalised; empty when nothing usable.
pub fn weighted_mean<'a>(items: impl IntoIterator<Item = (&'a [f32], f32)>) -> Vec<f32> {
    let mut sum: Vec<f32> = Vec::new();
    for (vector, weight) in items {
        let vector = normalized(vector);
        if vector.is_empty() || weight <= 0.0 {
            continue;
        }
        if sum.is_empty() {
            sum = vec![0.0; vector.len()];
        }
        if sum.len() != vector.len() {
            continue;
        }
        for (total, value) in sum.iter_mut().zip(vector) {
            *total += value * weight;
        }
    }
    normalized(&sum)
}

/// Moves a stored voiceprint towards a newly confirmed one. `count` is how many meetings it already
/// learned from.
pub fn blend(stored: &[f32], count: u32, fresh: &[f32]) -> Vec<f32> {
    if stored.len() != fresh.len() {
        return normalized(fresh);
    }
    let weight = count.clamp(1, MAX_BLEND_WEIGHT) as f32;
    weighted_mean([(stored, weight), (fresh, 1.0)])
}

/// Sorts turns by start and numbers raw speaker labels 0.. in order of first appearance.
pub fn renumber(raw: impl IntoIterator<Item = (u64, u64, i64)>) -> (Vec<Turn>, usize) {
    let (turns, labels) = renumber_labels(raw);
    (turns, labels.len())
}

fn renumber_labels(raw: impl IntoIterator<Item = (u64, u64, i64)>) -> (Vec<Turn>, Vec<i64>) {
    let mut raw: Vec<(u64, u64, i64)> = raw
        .into_iter()
        .filter(|(start, end, _)| end > start)
        .collect();
    raw.sort_by_key(|(start, end, label)| (*start, *end, *label));
    let mut labels: Vec<i64> = Vec::new();
    let turns = raw
        .into_iter()
        .map(|(start_ms, end_ms, label)| {
            let speaker = labels.iter().position(|l| *l == label).unwrap_or_else(|| {
                labels.push(label);
                labels.len() - 1
            });
            Turn {
                start_ms,
                end_ms,
                speaker: speaker as u32,
            }
        })
        .collect();
    (turns, labels)
}

/// The spans of a speaker to build its voiceprint from: its longest turns, at most `budget_ms` in
/// total, preferring turns of at least [`MIN_VOICEPRINT_TURN_MS`].
pub fn voiceprint_spans(turns: &[Turn], speaker: u32, budget_ms: u64) -> Vec<(u64, u64)> {
    let mut own: Vec<(u64, u64)> = turns
        .iter()
        .filter(|t| t.speaker == speaker && t.end_ms > t.start_ms)
        .map(|t| (t.start_ms, t.end_ms))
        .collect();
    own.sort_by_key(|(start, end)| (std::cmp::Reverse(end - start), *start));
    if own
        .iter()
        .any(|(start, end)| end - start >= MIN_VOICEPRINT_TURN_MS)
    {
        own.retain(|(start, end)| end - start >= MIN_VOICEPRINT_TURN_MS);
    } else {
        own.truncate(1);
    }
    let mut spans = Vec::new();
    let mut left = budget_ms;
    for (start, end) in own {
        if left == 0 {
            break;
        }
        let end = end.min(start + left);
        left -= end - start;
        spans.push((start, end));
    }
    spans
}

/// Total speech per speaker, counting overlapping turns of the same speaker once.
pub fn speech_ms(turns: &[Turn], speaker: u32) -> u64 {
    let mut own: Vec<(u64, u64)> = turns
        .iter()
        .filter(|t| t.speaker == speaker)
        .map(|t| (t.start_ms, t.end_ms))
        .collect();
    own.sort_unstable();
    let (mut total, mut reached) = (0, 0);
    for (start, end) in own {
        let start = start.max(reached);
        if end > start {
            total += end - start;
            reached = end;
        }
    }
    total
}

/// Joins diarizations of consecutive chunks of one recording (offsets in ms). Speakers of different
/// chunks become one when their voiceprints are alike; speakers within one chunk stay apart, as that
/// chunk's diarizer already told them apart.
pub fn merge_chunks(chunks: Vec<(u64, Diarization)>, threshold: f32) -> Diarization {
    struct Group {
        chunks: Vec<usize>,
        members: Vec<(usize, u32)>,
        centroid: Vec<f32>,
        speech_ms: u64,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (index, (_, diarization)) in chunks.iter().enumerate() {
        for speaker in &diarization.speakers {
            groups.push(Group {
                chunks: vec![index],
                members: vec![(index, speaker.speaker)],
                centroid: normalized(&speaker.embedding),
                speech_ms: speaker.speech_ms,
            });
        }
    }
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                if groups[a]
                    .chunks
                    .iter()
                    .any(|c| groups[b].chunks.contains(c))
                {
                    continue;
                }
                let score = cosine(&groups[a].centroid, &groups[b].centroid);
                if score >= threshold && best.is_none_or(|(_, _, s)| score > s) {
                    best = Some((a, b, score));
                }
            }
        }
        let Some((a, b, _)) = best else { break };
        let absorbed = groups.remove(b);
        let target = &mut groups[a];
        target.centroid = weighted_mean([
            (target.centroid.as_slice(), target.speech_ms.max(1) as f32),
            (
                absorbed.centroid.as_slice(),
                absorbed.speech_ms.max(1) as f32,
            ),
        ]);
        target.speech_ms += absorbed.speech_ms;
        target.chunks.extend(absorbed.chunks);
        target.members.extend(absorbed.members);
    }
    let group_of = |chunk: usize, speaker: u32| {
        groups
            .iter()
            .position(|g| g.members.contains(&(chunk, speaker)))
    };
    let raw: Vec<(u64, u64, i64)> = chunks
        .iter()
        .enumerate()
        .flat_map(|(index, (offset, diarization))| {
            diarization.turns.iter().filter_map(move |t| {
                Some((
                    offset + t.start_ms,
                    offset + t.end_ms,
                    group_of(index, t.speaker)? as i64,
                ))
            })
        })
        .collect();
    let (turns, order) = renumber_labels(raw);
    let speakers = order
        .iter()
        .enumerate()
        .map(|(speaker, label)| {
            let group = &groups[*label as usize];
            Speaker {
                speaker: speaker as u32,
                speech_ms: group.speech_ms,
                embedding: group.centroid.clone(),
            }
        })
        .collect();
    Diarization { turns, speakers }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Identity {
    Me,
    /// Index into the known voiceprints, with its similarity.
    Known(usize, f32),
    Unknown,
}

/// Names the speakers of one meeting. Every speaker that sounds like the user is the user (a
/// diarizer sometimes splits one voice); each known voice names at most one speaker, best match first.
pub fn identify(speakers: &[Speaker], me: Option<&[f32]>, known: &[&[f32]]) -> Vec<Identity> {
    identify_with(speakers, me, known, ME_THRESHOLD, KNOWN_THRESHOLD)
}

pub fn identify_with(
    speakers: &[Speaker],
    me: Option<&[f32]>,
    known: &[&[f32]],
    me_threshold: f32,
    known_threshold: f32,
) -> Vec<Identity> {
    let mut identities = vec![Identity::Unknown; speakers.len()];
    let best_known = |speaker: &Speaker| {
        known
            .iter()
            .map(|k| cosine(&speaker.embedding, k))
            .fold(0.0f32, f32::max)
    };
    if let Some(me) = me {
        for (identity, speaker) in identities.iter_mut().zip(speakers) {
            let score = cosine(&speaker.embedding, me);
            if score >= me_threshold && score >= best_known(speaker) {
                *identity = Identity::Me;
            }
        }
    }
    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
    for (s, speaker) in speakers.iter().enumerate() {
        if identities[s] == Identity::Me {
            continue;
        }
        for (k, voice) in known.iter().enumerate() {
            let score = cosine(&speaker.embedding, voice);
            if score >= known_threshold {
                pairs.push((s, k, score));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut used: Vec<usize> = Vec::new();
    for (s, k, score) in pairs {
        if identities[s] == Identity::Unknown && !used.contains(&k) {
            identities[s] = Identity::Known(k, score);
            used.push(k);
        }
    }
    identities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_use_english_context_and_preserve_provider_details() {
        assert_eq!(
            Error::Model("file missing".into()).to_string(),
            "could not load speaker model: file missing"
        );
        assert_eq!(
            Error::Audio("eigen foutbericht".into()).to_string(),
            "speaker recognition failed: eigen foutbericht"
        );
    }

    fn unit(direction: &[f32]) -> Vec<f32> {
        normalized(direction)
    }

    fn speaker(speaker: u32, embedding: &[f32]) -> Speaker {
        Speaker {
            speaker,
            speech_ms: 10_000,
            embedding: unit(embedding),
        }
    }

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    #[test]
    fn cosine_handles_mismatch_and_zero() {
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert!((cosine(&[1.0, 1.0], &[2.0, 2.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[-1.0, 0.0]) < -0.99);
    }

    #[test]
    fn weighted_mean_is_normalised_and_skips_unusable() {
        let mean = weighted_mean([
            (&[1.0f32, 0.0][..], 1.0),
            (&[0.0f32, 1.0][..], 1.0),
            (&[][..], 5.0),
        ]);
        assert!((mean[0] - mean[1]).abs() < 1e-6);
        assert!((mean.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(weighted_mean([(&[0.0f32, 0.0][..], 1.0)]).is_empty());
    }

    #[test]
    fn blend_moves_slowly_towards_new_meetings() {
        let stored = unit(&[1.0, 0.0]);
        let fresh = unit(&[0.0, 1.0]);
        let once = blend(&stored, 1, &fresh);
        let often = blend(&stored, 50, &fresh);
        assert!(cosine(&once, &fresh) > cosine(&often, &fresh));
        assert!(cosine(&often, &stored) > 0.99);
        assert_eq!(blend(&[1.0], 3, &fresh), fresh);
    }

    #[test]
    fn renumber_orders_by_first_appearance() {
        let (turns, count) = renumber([
            (5_000, 6_000, 7),
            (0, 1_000, 4),
            (2_000, 3_000, 7),
            (4_000, 4_000, 9),
        ]);
        assert_eq!(count, 2);
        assert_eq!(
            turns,
            vec![
                turn(0, 1_000, 0),
                turn(2_000, 3_000, 1),
                turn(5_000, 6_000, 1)
            ]
        );
    }

    #[test]
    fn voiceprint_spans_prefer_long_turns_within_budget() {
        let turns = [
            turn(0, 500, 0),
            turn(1_000, 4_000, 0),
            turn(5_000, 6_500, 0),
            turn(7_000, 9_000, 1),
        ];
        assert_eq!(
            voiceprint_spans(&turns, 0, 4_000),
            vec![(1_000, 4_000), (5_000, 6_000)]
        );
        assert_eq!(
            voiceprint_spans(&[turn(0, 400, 0), turn(500, 800, 0)], 0, 60_000),
            vec![(0, 400)]
        );
        assert!(voiceprint_spans(&turns, 3, 60_000).is_empty());
    }

    #[test]
    fn speech_counts_overlap_once() {
        let turns = [
            turn(0, 2_000, 0),
            turn(1_000, 3_000, 0),
            turn(5_000, 6_000, 0),
            turn(0, 9_000, 1),
        ];
        assert_eq!(speech_ms(&turns, 0), 4_000);
    }

    #[test]
    fn merge_joins_the_same_voice_across_chunks() {
        let anna = [1.0, 0.1, 0.0];
        let bram = [0.0, 1.0, 0.1];
        let first = Diarization {
            turns: vec![turn(0, 4_000, 0), turn(4_000, 9_000, 1)],
            speakers: vec![speaker(0, &anna), speaker(1, &bram)],
        };
        let second = Diarization {
            turns: vec![turn(0, 3_000, 0), turn(3_000, 8_000, 1)],
            speakers: vec![speaker(0, &[0.05, 1.0, 0.1]), speaker(1, &[0.0, 0.0, 1.0])],
        };
        let merged = merge_chunks(vec![(0, first), (600_000, second)], MERGE_THRESHOLD);
        assert_eq!(merged.speakers.len(), 3);
        assert_eq!(
            merged.turns,
            vec![
                turn(0, 4_000, 0),
                turn(4_000, 9_000, 1),
                turn(600_000, 603_000, 1),
                turn(603_000, 608_000, 2)
            ]
        );
        assert_eq!(merged.speakers[1].speech_ms, 20_000);
        assert!(cosine(&merged.speakers[1].embedding, &bram) > 0.99);
    }

    #[test]
    fn merge_keeps_speakers_of_one_chunk_apart() {
        let same = [1.0, 0.0];
        let only = Diarization {
            turns: vec![turn(0, 1_000, 0), turn(1_000, 2_000, 1)],
            speakers: vec![speaker(0, &same), speaker(1, &same)],
        };
        assert_eq!(
            merge_chunks(vec![(0, only)], MERGE_THRESHOLD)
                .speakers
                .len(),
            2
        );
    }

    #[test]
    fn merge_keeps_speakers_without_voiceprint_apart() {
        let a = Diarization {
            turns: vec![turn(0, 300, 0)],
            speakers: vec![Speaker {
                speaker: 0,
                speech_ms: 300,
                embedding: vec![],
            }],
        };
        let b = a.clone();
        assert_eq!(
            merge_chunks(vec![(0, a), (10_000, b)], MERGE_THRESHOLD)
                .speakers
                .len(),
            2
        );
    }

    #[test]
    fn identify_finds_me_and_known_voices_once() {
        let me = unit(&[1.0, 0.0, 0.0, 0.0]);
        let jan = unit(&[0.0, 1.0, 0.0, 0.0]);
        let piet = unit(&[0.0, 0.0, 1.0, 0.0]);
        let speakers = [
            speaker(0, &[1.0, 0.1, 0.0, 0.0]),
            speaker(1, &[0.0, 1.0, 0.2, 0.0]),
            speaker(2, &[0.1, 1.0, 0.0, 0.0]),
            speaker(3, &[0.0, 0.0, 0.0, 1.0]),
            speaker(4, &[0.95, 0.0, 0.0, 0.2]),
        ];
        let identities = identify(&speakers, Some(&me), &[&jan, &piet]);
        assert_eq!(identities[0], Identity::Me);
        assert_eq!(identities[4], Identity::Me);
        assert!(matches!(identities[2], Identity::Known(0, _)));
        assert_eq!(identities[1], Identity::Unknown);
        assert_eq!(identities[3], Identity::Unknown);
    }

    #[test]
    fn identify_prefers_a_closer_known_voice_over_me() {
        let me = unit(&[1.0, 0.6]);
        let jan = unit(&[1.0, 0.0]);
        let identities = identify(&[speaker(0, &[1.0, 0.0])], Some(&me), &[&jan]);
        assert!(matches!(identities[0], Identity::Known(0, _)));
    }

    #[test]
    fn identify_without_profiles_names_nobody() {
        let identities = identify(
            &[
                speaker(0, &[1.0, 0.0]),
                Speaker {
                    speaker: 1,
                    speech_ms: 0,
                    embedding: vec![],
                },
            ],
            None,
            &[],
        );
        assert_eq!(identities, vec![Identity::Unknown, Identity::Unknown]);
    }

    #[test]
    fn diarization_round_trips_as_json() {
        let diarization = Diarization {
            turns: vec![turn(0, 1_000, 0)],
            speakers: vec![speaker(0, &[1.0, 0.0])],
        };
        let json = serde_json::to_string(&diarization).unwrap();
        assert_eq!(
            serde_json::from_str::<Diarization>(&json).unwrap(),
            diarization
        );
    }
}
