use std::path::Path;

use pennel_speakers::{Identity, Turn as SpeakerTurn};

use crate::error::PennelError;
use crate::meetings::{Segment, Speaker, SpeakerName, VOICES};

/// Upper bound for a transcript chunk; the live session never cuts longer pieces.
#[cfg(test)]
const MAX_SEGMENT_MS: u64 = 30_000;
/// Generous speaking pace, so a short remark before a long silence doesn't borrow the next speaker's voice.
#[cfg(test)]
const MS_PER_WORD: u64 = 450;

/// A stretch of audio attributed to one voice: `(start_ms, end_ms, voice)`.
pub type Turn = (u64, u64, u32);

/// Runs the Sortformer model over a whole track (16 kHz mono). Kept for installations that only
/// have this model; it finds voices but no voiceprints.
#[cfg(feature = "local-models")]
pub fn diarize(model_path: &Path, samples: &[f32]) -> Result<Vec<Turn>, PennelError> {
    use transcribe_cpp::Model;

    let _physical = crate::live_speakers::offline_acoustic_permit();
    if !model_path.exists() {
        return Err(PennelError::ModelNotDownloaded);
    }
    crate::cpu_support::ensure_local_models_supported()?;
    let error = |e: transcribe_cpp::Error| PennelError::InvalidResponse {
        message: format!("speaker recognition: {e}"),
    };
    let model = Model::load(model_path).map_err(error)?;
    let mut session = model.diarize_session().map_err(error)?;
    let turns = session.run(samples, &Default::default()).map_err(error)?;
    Ok(turns
        .into_iter()
        .filter(|t| t.speaker_id > 0 && t.t1_ms > t.t0_ms)
        .map(|t| (t.t0_ms.max(0) as u64, t.t1_ms as u64, t.speaker_id as u32))
        .collect())
}

#[cfg(not(feature = "local-models"))]
pub fn diarize(_model_path: &Path, _samples: &[f32]) -> Result<Vec<Turn>, PennelError> {
    Err(PennelError::local_models_unavailable())
}

/// Sortformer turns as a diarization without voiceprints.
pub fn from_sortformer(turns: &[Turn]) -> pennel_speakers::Diarization {
    let (turns, count) = pennel_speakers::renumber(
        turns
            .iter()
            .map(|&(start, end, voice)| (start, end, i64::from(voice))),
    );
    let speakers = (0..count as u32)
        .map(|speaker| pennel_speakers::Speaker {
            speaker,
            speech_ms: pennel_speakers::speech_ms(&turns, speaker),
            embedding: Vec::new(),
        })
        .collect();
    pennel_speakers::Diarization { turns, speakers }
}

/// For each segment `include` accepts, the speaker that overlaps it most. Transcripts store start
/// times only, so a segment ends at the next included start, at 30 s, or when its words would have
/// been spoken, whichever is first.
#[cfg(test)]
fn dominant_speakers(
    segments: &[Segment],
    turns: &[SpeakerTurn],
    include: impl Fn(&Segment) -> bool,
) -> Vec<Option<u32>> {
    let starts: Vec<u64> = segments
        .iter()
        .filter(|s| include(s))
        .map(|s| s.start_ms)
        .collect();
    segments
        .iter()
        .map(|segment| {
            if !include(segment) || segment.failed || segment.text.trim().is_empty() {
                return None;
            }
            let next = starts
                .iter()
                .copied()
                .filter(|s| *s > segment.start_ms)
                .min();
            let words = segment.text.split_whitespace().count() as u64;
            let cap = segment.start_ms + MAX_SEGMENT_MS.min(1_000 + words * MS_PER_WORD);
            let end = next.map_or(cap, |n| n.min(cap)).max(segment.end_ms);
            let mut overlap: Vec<(u32, u64)> = Vec::new();
            for turn in turns {
                let shared = turn
                    .end_ms
                    .min(end)
                    .saturating_sub(turn.start_ms.max(segment.start_ms));
                if shared == 0 {
                    continue;
                }
                match overlap.iter_mut().find(|(s, _)| *s == turn.speaker) {
                    Some((_, total)) => *total += shared,
                    None => overlap.push((turn.speaker, shared)),
                }
            }
            overlap
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(speaker, _)| speaker)
        })
        .collect()
}

/// Gives each "Anderen" segment the voice that overlaps it most (Sortformer voices, 1-based).
#[cfg(test)]
fn assign_voices(segments: &mut [Segment], turns: &[Turn]) {
    let diarization = from_sortformer(turns);
    apply(
        segments,
        &diarization.turns,
        &vec![Identity::Unknown; diarization.speakers.len()],
        Speaker::Others,
    );
}

/// Which recording a diarization came from. A shared microphone holds everyone, so its result may
/// relabel "Ik" lines; the system-audio track only splits "Anderen".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    Microphone,
    System,
}

impl From<Speaker> for Track {
    fn from(speaker: Speaker) -> Self {
        match speaker {
            Speaker::Me => Track::Microphone,
            Speaker::Others => Track::System,
        }
    }
}

/// The outcome of [`apply`]: which diarization speaker each meeting voice is, and whether the user
/// was heard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    /// `(voice, speaker index)`, voices numbered 1.. in order of first line.
    pub voices: Vec<(u32, usize)>,
    pub me_found: bool,
}

/// Labels transcript lines with the diarized speakers. On a microphone track the user's own speakers
/// become "Ik" and every other speaker an "Anderen N" voice; on the system track only "Anderen" lines
/// change. Lines no speaker overlaps keep their side and lose a stale voice.
#[cfg(test)]
pub fn apply(
    segments: &mut [Segment],
    turns: &[SpeakerTurn],
    identities: &[Identity],
    track: impl Into<Track>,
) -> Applied {
    let track = track.into();
    // On the microphone only "Ik" lines and lines an earlier microphone run gave a voice take part;
    // a plain "Anderen" line came from a system track and is not this recording's to relabel.
    let include = |s: &Segment| match track {
        Track::Microphone => s.speaker == Speaker::Me || s.voice.is_some(),
        Track::System => s.speaker == Speaker::Others,
    };
    let dominant = dominant_speakers(segments, turns, include);
    let is_me = |speaker: u32| {
        track == Track::Microphone && identities.get(speaker as usize) == Some(&Identity::Me)
    };
    let mut applied = Applied::default();
    for (segment, speaker) in segments.iter_mut().zip(dominant) {
        if !include(segment) || segment.failed || segment.text.trim().is_empty() {
            continue;
        }
        let Some(speaker) = speaker else {
            if segment.speaker == Speaker::Others {
                segment.voice = None;
            }
            continue;
        };
        if is_me(speaker) {
            segment.speaker = Speaker::Me;
            segment.voice = None;
            applied.me_found = true;
            continue;
        }
        let voice = match applied.voices.iter().find(|(_, s)| *s == speaker as usize) {
            Some((voice, _)) => Some(*voice),
            None => {
                let next = applied.voices.len() as u32 + 1;
                VOICES.contains(&next).then(|| {
                    applied.voices.push((next, speaker as usize));
                    next
                })
            }
        };
        segment.speaker = Speaker::Others;
        segment.voice = voice;
    }
    applied
}

fn includes_source(row: &crate::meeting_utterances::MeetingUtterance, track: Track) -> bool {
    matches!(
        (track, row.metadata.source_id.as_deref()),
        (Track::Microphone, Some("microphone" | "room_microphone"))
            | (Track::System, Some("system" | "system_audio"))
    )
}

#[derive(Clone, PartialEq, Eq)]
struct AttributionLabel {
    kind: crate::meeting_utterances::AttributionKind,
    speakers: Vec<u32>,
}

fn attribution_label(turns: &[SpeakerTurn], start: u64, end: u64) -> AttributionLabel {
    use crate::meeting_utterances::AttributionKind;
    let present: Vec<_> = turns
        .iter()
        .filter(|turn| turn.end_ms.min(end) > turn.start_ms.max(start))
        .collect();
    let mut speakers: Vec<_> = present.iter().map(|turn| turn.speaker).collect();
    speakers.sort_unstable();
    speakers.dedup();
    let overlap = present.iter().enumerate().any(|(index, a)| {
        present.iter().skip(index + 1).any(|b| {
            a.speaker != b.speaker
                && a.end_ms.min(b.end_ms).min(end) > a.start_ms.max(b.start_ms).max(start)
        })
    });
    AttributionLabel {
        kind: if overlap {
            AttributionKind::Overlap
        } else if speakers.len() > 1 {
            AttributionKind::Mixed
        } else if speakers.len() == 1 {
            AttributionKind::Single
        } else {
            AttributionKind::Unknown
        },
        speakers,
    }
}

fn usable_spans(row: &crate::meeting_utterances::MeetingUtterance) -> bool {
    if row.metadata.spans.is_empty() {
        return false;
    }
    let text = &row.segment.text;
    let mut previous_byte = 0;
    let mut previous_time = row.metadata.start_ms;
    for span in &row.metadata.spans {
        let (first, last) = (span.0 as usize, span.1 as usize);
        if first < previous_byte
            || last <= first
            || last > text.len()
            || !text.is_char_boundary(first)
            || !text.is_char_boundary(last)
            || text[previous_byte..first]
                .chars()
                .any(char::is_alphanumeric)
            || span.2 < previous_time
            || span.3 <= span.2
            || span.2 < row.metadata.start_ms
            || span.3 > row.metadata.end_ms
        {
            return false;
        }
        previous_byte = last;
        previous_time = span.2;
    }
    !text[previous_byte..].chars().any(char::is_alphanumeric)
}

fn provisional_voice_map(
    before: &[crate::meeting_utterances::MeetingUtterance],
    turns: &[SpeakerTurn],
    track: Track,
) -> std::collections::BTreeMap<u32, Option<u32>> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut candidates: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut reverse: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for old in before
        .iter()
        .filter(|row| includes_source(row, track) && row.metadata.attribution.provisional)
    {
        for prior in &old.metadata.attribution.turns {
            let Some(voice) = prior.voice else { continue };
            for fresh in turns
                .iter()
                .filter(|fresh| fresh.end_ms.min(prior.end_ms) > fresh.start_ms.max(prior.start_ms))
            {
                candidates.entry(fresh.speaker).or_default().insert(voice);
                reverse.entry(voice).or_default().insert(fresh.speaker);
            }
        }
    }
    candidates
        .into_iter()
        .map(|(speaker, voices)| {
            let voice = voices.first().copied().filter(|voice| {
                voices.len() == 1
                    && reverse
                        .get(voice)
                        .is_some_and(|speakers| speakers.len() == 1)
            });
            (speaker, voice)
        })
        .collect()
}

/// Apply actual acoustic turns to exact authoritative spans. Missing word alignment preserves the
/// intact utterance as mixed/unknown; acoustic duration never allocates words between speakers.
pub fn apply_utterances(
    before: &[crate::meeting_utterances::MeetingUtterance],
    turns: &[SpeakerTurn],
    identities: &[Identity],
    track: Track,
) -> (Vec<crate::meeting_utterances::MeetingUtterance>, Applied) {
    apply_utterances_inner(before, turns, identities, track, None, false)
}

/// The rolling acoustic worker owns the mapping; window order must never renumber session voices.
pub(crate) fn apply_utterances_with_voice_map(
    before: &[crate::meeting_utterances::MeetingUtterance],
    turns: &[SpeakerTurn],
    track: Track,
    voice_map: &[(u32, u32)],
    provisional: bool,
) -> (Vec<crate::meeting_utterances::MeetingUtterance>, Applied) {
    apply_utterances_inner(before, turns, &[], track, Some(voice_map), provisional)
}

fn apply_utterances_inner(
    before: &[crate::meeting_utterances::MeetingUtterance],
    turns: &[SpeakerTurn],
    identities: &[Identity],
    track: Track,
    voice_map: Option<&[(u32, u32)]>,
    provisional: bool,
) -> (Vec<crate::meeting_utterances::MeetingUtterance>, Applied) {
    use crate::meeting_utterances::{
        AcousticTurn, AttributionKind, AutomaticAttribution, TimedTextSpan,
    };
    use std::collections::BTreeSet;

    let reserved: BTreeSet<_> = before
        .iter()
        .filter(|row| !includes_source(row, track))
        .flat_map(|row| {
            row.segment
                .voice
                .into_iter()
                .chain(row.metadata.attribution.voices.iter().copied())
                .chain(row.metadata.manual.iter().filter_map(|manual| manual.voice))
                .chain(
                    row.metadata
                        .unresolved_manual
                        .iter()
                        .filter_map(|manual| manual.voice),
                )
        })
        .chain(
            before
                .iter()
                .filter(|row| row.metadata.attribution.provisional)
                .flat_map(|row| {
                    row.metadata.attribution.voices.iter().copied().chain(
                        row.metadata
                            .attribution
                            .turns
                            .iter()
                            .filter_map(|turn| turn.voice),
                    )
                }),
        )
        .collect();
    let previous_provisional = provisional_voice_map(before, turns, track);
    let manual_voices: BTreeSet<_> = before
        .iter()
        .filter_map(|row| row.metadata.manual.as_ref())
        .filter_map(|manual| manual.voice)
        .collect();
    let mut ordered: Vec<_> = turns
        .iter()
        .filter(|turn| {
            turn.end_ms > turn.start_ms
                && before.iter().any(|row| {
                    includes_source(row, track)
                        && !row.segment.failed
                        && !row.segment.text.trim().is_empty()
                        && turn.end_ms.min(row.metadata.end_ms)
                            > turn.start_ms.max(row.metadata.start_ms)
                })
        })
        .collect();
    ordered.sort_by_key(|turn| (turn.start_ms, turn.end_ms, turn.speaker));
    let mut seen = BTreeSet::new();
    let mut applied = Applied::default();
    if let Some(voice_map) = voice_map {
        for (speaker, voice) in voice_map {
            if VOICES.contains(voice)
                && !applied
                    .voices
                    .iter()
                    .any(|(old, model)| old == voice || *model == *speaker as usize)
                && ordered.iter().any(|turn| turn.speaker == *speaker)
            {
                applied.voices.push((*voice, *speaker as usize));
            }
        }
    } else {
        for turn in ordered {
            if !seen.insert(turn.speaker) {
                continue;
            }
            if track == Track::Microphone
                && identities.get(turn.speaker as usize) == Some(&Identity::Me)
            {
                continue;
            }
            if let Some(previous) = previous_provisional.get(&turn.speaker) {
                if let Some(voice) = previous.filter(|voice| VOICES.contains(voice))
                    && !applied.voices.iter().any(|(used, _)| *used == voice)
                {
                    applied.voices.push((voice, turn.speaker as usize));
                }
                // Ambiguous old-to-final mappings stay unknown instead of claiming another old ID.
                continue;
            }
            let preferred: BTreeSet<_> = before
                .iter()
                .filter(|row| includes_source(row, track))
                .filter(|row| {
                    attribution_label(turns, row.metadata.start_ms, row.metadata.end_ms).speakers
                        == [turn.speaker]
                })
                .filter_map(|row| {
                    row.metadata.manual.as_ref().filter(|manual| {
                        !manual.review_required
                            && manual.source_fingerprint == row.metadata.source_fingerprint
                    })
                })
                .filter_map(|manual| manual.voice)
                .collect();
            let preferred = preferred
                .iter()
                .next()
                .copied()
                .filter(|_| preferred.len() == 1);
            let voice = preferred
                .filter(|voice| {
                    VOICES.contains(voice)
                        && !reserved.contains(voice)
                        && !applied.voices.iter().any(|(used, _)| used == voice)
                })
                .or_else(|| {
                    VOICES.clone().find(|voice| {
                        !reserved.contains(voice)
                            && !manual_voices.contains(voice)
                            && !applied.voices.iter().any(|(used, _)| used == voice)
                    })
                });
            if let Some(voice) = voice {
                applied.voices.push((voice, turn.speaker as usize));
            }
        }
    }
    let voice_of = |speaker: u32| {
        applied
            .voices
            .iter()
            .find(|(_, index)| *index == speaker as usize)
            .map(|(voice, _)| *voice)
    };
    let is_me = |speaker: u32| {
        track == Track::Microphone && identities.get(speaker as usize) == Some(&Identity::Me)
    };
    let mut result = Vec::new();
    for original in before {
        if !includes_source(original, track)
            || original.segment.failed
            || original.segment.text.trim().is_empty()
        {
            result.push(original.clone());
            continue;
        }
        let groups = if usable_spans(original) {
            let mut groups: Vec<(usize, usize, AttributionLabel)> = Vec::new();
            for (index, span) in original.metadata.spans.iter().enumerate() {
                let label = attribution_label(turns, span.2, span.3);
                if let Some(last) = groups.last_mut().filter(|last| last.2 == label) {
                    last.1 = index + 1;
                } else {
                    groups.push((index, index + 1, label));
                }
            }
            groups
        } else {
            vec![(
                0,
                0,
                attribution_label(turns, original.metadata.start_ms, original.metadata.end_ms),
            )]
        };
        let split = groups.len() > 1;
        for (first, last, label) in &groups {
            let mut row = original.clone();
            if *last > 0 {
                let byte_start = if *first == 0 {
                    0
                } else {
                    original.metadata.spans[*first].0 as usize
                };
                let byte_end = if *last == original.metadata.spans.len() {
                    original.segment.text.len()
                } else {
                    original.metadata.spans[*last].0 as usize
                };
                row.segment.text = original.segment.text[byte_start..byte_end].to_owned();
                row.segment.start_ms = original.metadata.spans[*first].2;
                row.segment.end_ms = original.metadata.spans[*last - 1].3;
                row.metadata.spans = original.metadata.spans[*first..*last]
                    .iter()
                    .map(|span| {
                        TimedTextSpan(
                            span.0 - byte_start as u32,
                            span.1 - byte_start as u32,
                            span.2,
                            span.3,
                        )
                    })
                    .collect();
            }
            if split {
                row.metadata.utterance_id = uuid::Uuid::now_v7().to_string();
                if !row
                    .metadata
                    .lineage
                    .contains(&original.metadata.utterance_id)
                {
                    row.metadata
                        .lineage
                        .push(original.metadata.utterance_id.clone());
                }
            }
            let mut acoustic = Vec::new();
            for turn in turns {
                let start_ms = turn.start_ms.max(row.segment.start_ms);
                let end_ms = turn.end_ms.min(row.segment.end_ms);
                if end_ms > start_ms {
                    acoustic.push(AcousticTurn {
                        start_ms,
                        end_ms,
                        voice: voice_of(turn.speaker),
                    });
                }
            }
            let mut voices: Vec<_> = label
                .speakers
                .iter()
                .filter_map(|speaker| voice_of(*speaker))
                .collect();
            voices.sort_unstable();
            voices.dedup();
            let self_voice = label.speakers.iter().any(|speaker| is_me(*speaker));
            let kind = if label.kind == AttributionKind::Single && voices.is_empty() && !self_voice
            {
                AttributionKind::Unknown
            } else {
                label.kind
            };
            let automatic = AutomaticAttribution {
                kind,
                voices,
                turns: acoustic,
                self_voice,
                provisional,
            };
            if row.metadata.attribution != automatic || split {
                row.metadata.attribution_revision =
                    row.metadata.attribution_revision.saturating_add(1);
            }
            row.metadata.attribution = automatic;
            if label.kind == AttributionKind::Single
                && label
                    .speakers
                    .first()
                    .is_some_and(|speaker| is_me(*speaker))
            {
                row.segment.speaker = Speaker::Me;
                row.segment.voice = None;
                applied.me_found = true;
            } else {
                row.segment.speaker = Speaker::Others;
                row.segment.voice = (label.kind == AttributionKind::Single)
                    .then(|| {
                        label
                            .speakers
                            .first()
                            .and_then(|speaker| voice_of(*speaker))
                    })
                    .flatten();
            }
            row.metadata.refresh_fingerprints(&row.segment);
            if let Some(mut manual) = row.metadata.manual.take() {
                if split
                    || manual.review_required
                    || manual.source_fingerprint != row.metadata.source_fingerprint
                {
                    manual.review_required = true;
                    if !row.metadata.unresolved_manual.contains(&manual) {
                        row.metadata.unresolved_manual.push(manual);
                    }
                } else {
                    if let Some(voice) = manual.voice.filter(|voice| VOICES.contains(voice)) {
                        row.segment.speaker = Speaker::Others;
                        row.segment.voice = Some(voice);
                    }
                    row.metadata.manual = Some(manual);
                }
            }
            result.push(row);
        }
    }
    (result, applied)
}

pub(crate) fn preserve_manual_names(
    rows: &mut [crate::meeting_utterances::MeetingUtterance],
    names: &[SpeakerName],
) {
    use crate::meeting_utterances::ManualCorrection;
    for row in rows {
        if row.metadata.manual.is_none()
            && let Some(name) = row
                .segment
                .voice
                .and_then(|voice| names.iter().find(|name| name.voice == voice))
        {
            row.metadata.manual = Some(ManualCorrection {
                name: Some(name.name.clone()),
                voice: Some(name.voice),
                source_fingerprint: row.metadata.source_fingerprint.clone(),
                review_required: false,
            });
        }
    }
}

pub(crate) fn carry_utterance_names(
    before: &[crate::meeting_utterances::MeetingUtterance],
    after: &[crate::meeting_utterances::MeetingUtterance],
    names_before: &[SpeakerName],
    known: &[(u32, String)],
) -> Vec<SpeakerName> {
    let blocked: Vec<_> = after
        .iter()
        .filter(|row| {
            !row.metadata.unresolved_manual.is_empty()
                || row
                    .metadata
                    .manual
                    .as_ref()
                    .is_some_and(|manual| manual.review_required)
        })
        .flat_map(|row| {
            row.segment
                .voice
                .into_iter()
                .chain(row.metadata.attribution.voices.iter().copied())
        })
        .collect();
    let known: Vec<_> = known
        .iter()
        .filter(|(voice, _)| !blocked.contains(voice))
        .cloned()
        .collect();
    let confirmed_names: Vec<_> = names_before
        .iter()
        .filter(|name| {
            before.iter().any(|row| {
                row.segment.voice == Some(name.voice)
                    && row.metadata.manual.as_ref().is_some_and(|manual| {
                        !manual.review_required
                            && manual.source_fingerprint == row.metadata.source_fingerprint
                            && manual.name.as_deref() == Some(name.name.as_str())
                    })
            })
        })
        .cloned()
        .collect();
    let mut names = carry_names(
        &before
            .iter()
            .map(|row| row.segment.clone())
            .collect::<Vec<_>>(),
        &after
            .iter()
            .map(|row| row.segment.clone())
            .collect::<Vec<_>>(),
        &confirmed_names,
        &known,
    );
    let present: std::collections::BTreeSet<_> = after
        .iter()
        .flat_map(|row| {
            row.segment
                .voice
                .into_iter()
                .chain(row.metadata.attribution.voices.iter().copied())
        })
        .collect();
    for (voice, name) in &known {
        if present.contains(voice)
            && !names
                .iter()
                .any(|old| old.voice == *voice || old.name == *name)
        {
            names.push(SpeakerName {
                voice: *voice,
                name: name.clone(),
            });
        }
    }
    for row in after {
        if let Some(manual) = row.metadata.manual.as_ref().filter(|manual| {
            !manual.review_required && manual.source_fingerprint == row.metadata.source_fingerprint
        }) && let (Some(voice), Some(name)) = (row.segment.voice, manual.name.as_ref())
        {
            names.retain(|previous| previous.voice != voice && previous.name != *name);
            names.push(SpeakerName {
                voice,
                name: name.clone(),
            });
        }
    }
    names.sort_by_key(|name| name.voice);
    names
}

/// Preserve explicit meeting labels before remembered matches. A merge or split with more than
/// one established mapping is deliberately left unnamed; canonical metadata retains it for review.
pub fn carry_names(
    before: &[Segment],
    after: &[Segment],
    names_before: &[SpeakerName],
    known: &[(u32, String)],
) -> Vec<SpeakerName> {
    use std::collections::{BTreeMap, BTreeSet};

    let voices: BTreeSet<u32> = after.iter().filter_map(|segment| segment.voice).collect();
    let mut forward: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut reverse: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut reserved = BTreeSet::new();
    for new in after {
        let Some(voice) = new.voice else { continue };
        let candidates: Vec<_> = before
            .iter()
            .filter(|old| {
                old.start_ms == new.start_ms
                    && old.text == new.text
                    && old.failed == new.failed
                    && old.speaker == Speaker::Others
            })
            .collect();
        for old in &candidates {
            if old
                .voice
                .is_some_and(|voice| names_before.iter().any(|name| name.voice == voice))
            {
                reserved.insert(voice);
            }
        }
        if let [old] = candidates.as_slice()
            && let Some(previous) = old.voice
        {
            forward.entry(voice).or_default().insert(previous);
            reverse.entry(previous).or_default().insert(voice);
        }
    }
    let mut names = Vec::<SpeakerName>::new();
    for voice in &voices {
        let Some(previous) = forward
            .get(voice)
            .filter(|old| old.len() == 1)
            .and_then(|old| old.first())
        else {
            continue;
        };
        if reverse.get(previous).is_none_or(|new| new.len() != 1) {
            continue;
        }
        if let Some(name) = names_before.iter().find(|name| name.voice == *previous)
            && !names.iter().any(|saved| saved.name == name.name)
        {
            names.push(SpeakerName {
                voice: *voice,
                name: name.name.clone(),
            });
        }
    }
    for (voice, name) in known {
        if voices.contains(voice)
            && !reserved.contains(voice)
            && !names
                .iter()
                .any(|saved| saved.voice == *voice || saved.name == *name)
        {
            names.push(SpeakerName {
                voice: *voice,
                name: name.clone(),
            });
        }
    }
    names.sort_by_key(|name| name.voice);
    names
}

/// Whether a track holds (almost) no sound: under 1% of 30 ms frames reach -40 dBFS.
pub fn mostly_silent(samples: &[f32]) -> bool {
    const FRAME: usize = 480;
    const LOUD: f32 = 0.01;
    let frames = samples.len() / FRAME;
    if frames == 0 {
        return true;
    }
    let loud = samples
        .as_chunks::<FRAME>()
        .0
        .iter()
        .filter(|frame| (frame.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt() >= LOUD)
        .count();
    loud * 100 < frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn others(start_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms,
            speaker: Speaker::Others,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    fn me(start_ms: u64, text: &str) -> Segment {
        Segment {
            speaker: Speaker::Me,
            ..others(start_ms, text)
        }
    }

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> SpeakerTurn {
        SpeakerTurn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn labels(segments: &[Segment]) -> Vec<(Speaker, Option<u32>)> {
        segments.iter().map(|s| (s.speaker, s.voice)).collect()
    }

    #[test]
    fn assign_voices_by_overlap() {
        let long = "woord ".repeat(30);
        let mut segments = vec![
            others(0, &long),
            me(3_000, "ik"),
            others(5_000, &long),
            others(15_000, &long),
        ];
        let turns = [
            (0, 4_600, 1),
            (5_100, 8_900, 2),
            (9_500, 14_200, 1),
            (14_600, 15_100, 2),
            (15_100, 18_500, 3),
        ];
        assign_voices(&mut segments, &turns);
        let voices: Vec<Option<u32>> = segments.iter().map(|s| s.voice).collect();
        assert_eq!(voices, [Some(1), None, Some(1), Some(2)]);
    }

    #[test]
    fn short_remark_before_silence_keeps_its_own_voice() {
        let mut segments = vec![others(0, "ja klopt"), others(28_000, "verder")];
        assign_voices(&mut segments, &[(0, 1_500, 1), (3_000, 27_000, 2)]);
        assert_eq!(segments[0].voice, Some(1));
    }

    #[test]
    fn ties_go_to_the_voice_heard_first() {
        let mut segments = vec![others(0, &"woord ".repeat(30)), others(10_000, "b")];
        assign_voices(&mut segments, &[(0, 5_000, 2), (5_000, 10_000, 1)]);
        assert_eq!(segments[0].voice, Some(1));
    }

    #[test]
    fn assign_voices_without_turns() {
        let mut segments = vec![others(0, "a"), others(40_000, "b")];
        assign_voices(&mut segments, &[]);
        assert!(segments.iter().all(|s| s.voice.is_none()));
        assign_voices(&mut segments, &[(100_000, 120_000, 1)]);
        assert!(segments.iter().all(|s| s.voice.is_none()));
    }

    #[test]
    fn failed_segments_get_no_voice() {
        let mut segments = vec![
            Segment {
                failed: true,
                ..others(0, "")
            },
            others(5_000, "b"),
        ];
        assign_voices(&mut segments, &[(0, 10_000, 1)]);
        assert_eq!(segments[0].voice, None);
        assert_eq!(segments[1].voice, Some(1));
    }

    #[test]
    fn system_track_never_touches_my_lines() {
        let mut segments = vec![me(0, "hallo"), others(2_000, "dag")];
        let applied = apply(
            &mut segments,
            &[turn(0, 4_000, 0)],
            &[Identity::Me],
            Speaker::Others,
        );
        assert_eq!(
            labels(&segments),
            [(Speaker::Me, None), (Speaker::Others, Some(1))]
        );
        assert!(!applied.me_found);
        assert_eq!(applied.voices, [(1, 0)]);
    }

    #[test]
    fn shared_microphone_splits_me_from_the_room() {
        let mut segments = vec![
            me(0, "goedemorgen allemaal"),
            me(4_000, "dank je"),
            me(8_000, "ik ben het niet eens"),
            Segment {
                voice: Some(3),
                ..others(12_000, "vroeger anders gelabeld")
            },
        ];
        let turns = [
            turn(0, 3_500, 0),
            turn(4_000, 7_500, 1),
            turn(8_000, 11_500, 2),
            turn(12_000, 15_000, 1),
        ];
        let applied = apply(
            &mut segments,
            &turns,
            &[Identity::Unknown, Identity::Me, Identity::Known(0, 0.8)],
            Speaker::Me,
        );
        assert_eq!(
            labels(&segments),
            [
                (Speaker::Others, Some(1)),
                (Speaker::Me, None),
                (Speaker::Others, Some(2)),
                (Speaker::Me, None),
            ]
        );
        assert!(applied.me_found);
        assert_eq!(applied.voices, [(1, 0), (2, 2)]);
    }

    #[test]
    fn voices_beyond_the_limit_stay_plain_anderen() {
        let mut segments: Vec<Segment> = (0..14).map(|i| others(i * 5_000, "a b")).collect();
        let turns: Vec<SpeakerTurn> = (0..14)
            .map(|i| turn(i as u64 * 5_000, i as u64 * 5_000 + 2_000, i))
            .collect();
        apply(
            &mut segments,
            &turns,
            &vec![Identity::Unknown; 14],
            Speaker::Others,
        );
        assert_eq!(segments[11].voice, Some(12));
        assert_eq!(segments[12].voice, None);
    }

    #[test]
    fn names_follow_known_voices_and_earlier_naming() {
        let before = vec![
            Segment {
                voice: Some(3),
                ..others(0, "a")
            },
            Segment {
                voice: Some(3),
                ..others(5_000, "b")
            },
            Segment {
                voice: Some(1),
                ..others(10_000, "c")
            },
            me(15_000, "d"),
        ];
        let mut after = before.clone();
        after[0].voice = Some(1);
        after[1].voice = Some(1);
        after[2].voice = Some(2);
        after[3] = Segment {
            voice: Some(3),
            ..others(15_000, "d")
        };
        let names_before = vec![
            SpeakerName {
                voice: 1,
                name: "Piet".into(),
            },
            SpeakerName {
                voice: 3,
                name: "Anna".into(),
            },
        ];
        let names = carry_names(&before, &after, &names_before, &[(2, "Jan".into())]);
        assert_eq!(
            names,
            [
                SpeakerName {
                    voice: 1,
                    name: "Anna".into()
                },
                SpeakerName {
                    voice: 2,
                    name: "Piet".into()
                },
            ]
        );
    }

    #[test]
    fn a_manual_name_wins_over_a_remembered_match_on_another_voice() {
        let before = vec![Segment {
            voice: Some(1),
            ..others(0, "a")
        }];
        let mut after = before.clone();
        after.push(Segment {
            voice: Some(2),
            ..others(5_000, "b")
        });
        let jan = vec![SpeakerName {
            voice: 1,
            name: "Jan".into(),
        }];
        let names = carry_names(&before, &after, &jan, &[(2, "Jan".into())]);
        assert_eq!(
            names,
            [SpeakerName {
                voice: 1,
                name: "Jan".into()
            }]
        );
    }

    #[test]
    fn a_manual_name_wins_over_a_different_remembered_match() {
        let before = vec![Segment {
            voice: Some(4),
            ..others(0, "A confirmed passage.")
        }];
        let after = vec![Segment {
            voice: Some(1),
            ..before[0].clone()
        }];
        assert_eq!(
            carry_names(
                &before,
                &after,
                &[SpeakerName {
                    voice: 4,
                    name: "Manual Anna".into()
                }],
                &[(1, "Remembered Ben".into())],
            ),
            [SpeakerName {
                voice: 1,
                name: "Manual Anna".into()
            }]
        );
    }

    #[test]
    fn ambiguous_voice_merge_does_not_choose_the_majority_manual_name() {
        let before = vec![
            Segment {
                voice: Some(1),
                ..others(0, "One")
            },
            Segment {
                voice: Some(1),
                ..others(1_000, "Two")
            },
            Segment {
                voice: Some(2),
                ..others(2_000, "Three")
            },
        ];
        let after: Vec<_> = before
            .iter()
            .cloned()
            .map(|mut segment| {
                segment.voice = Some(3);
                segment
            })
            .collect();
        assert!(
            carry_names(
                &before,
                &after,
                &[
                    SpeakerName {
                        voice: 1,
                        name: "Anna".into()
                    },
                    SpeakerName {
                        voice: 2,
                        name: "Ben".into()
                    }
                ],
                &[],
            )
            .is_empty()
        );
    }

    #[test]
    fn microphone_leaves_system_lines_and_unheard_lines_alone() {
        let mut segments = vec![
            me(0, "hallo"),
            others(5_000, "van de call"),
            me(20_000, "niemand gehoord"),
        ];
        apply(
            &mut segments,
            &[turn(0, 9_000, 0)],
            &[Identity::Unknown],
            Speaker::Me,
        );
        assert_eq!(
            labels(&segments),
            [
                (Speaker::Others, Some(1)),
                (Speaker::Others, None),
                (Speaker::Me, None)
            ]
        );
    }

    #[test]
    fn silence_detection() {
        assert!(mostly_silent(&[]));
        assert!(mostly_silent(&vec![0.0; 16_000 * 10]));
        assert!(mostly_silent(&vec![0.001; 16_000 * 10]));
        let mut speech = vec![0.0; 16_000 * 10];
        speech[..16_000].iter_mut().for_each(|x| *x = 0.1);
        assert!(!mostly_silent(&speech));
    }

    fn timed_row(
        text: &str,
        spans: &[(u32, u32, u64, u64)],
    ) -> crate::meeting_utterances::MeetingUtterance {
        use crate::meeting_utterances::{MeetingUtterance, TimedTextSpan};
        let mut row = MeetingUtterance::new(
            Segment {
                end_ms: 1_200,
                ..me(0, text)
            },
            Some("microphone".into()),
        );
        row.metadata.spans = spans
            .iter()
            .map(|&(a, b, c, d)| TimedTextSpan(a, b, c, d))
            .collect();
        row
    }

    #[test]
    fn actual_word_times_split_rapid_turns_without_losing_punctuation_or_spaces() {
        let row = timed_row(
            "Ja, nee. Yes!",
            &[(0, 3, 0, 250), (4, 8, 350, 700), (9, 13, 850, 1_200)],
        );
        let original = row.metadata.utterance_id.clone();
        let (rows, applied) = apply_utterances(
            &[row],
            &[turn(0, 300, 0), turn(300, 800, 1), turn(800, 1_200, 0)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.segment.text.as_str())
                .collect::<String>(),
            "Ja, nee. Yes!"
        );
        assert_eq!(
            rows.iter().map(|row| row.segment.voice).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(1)]
        );
        assert!(rows.iter().all(|row| row.segment.speaker == Speaker::Others
            && row.metadata.source_id.as_deref() == Some("microphone")));
        assert!(
            rows.iter()
                .all(|row| row.metadata.lineage.contains(&original))
        );
        assert!(!applied.me_found);
        let ids: Vec<_> = rows
            .iter()
            .map(|row| row.metadata.utterance_id.clone())
            .collect();
        let (again, _) = apply_utterances(
            &rows,
            &[turn(0, 300, 0), turn(300, 800, 1), turn(800, 1_200, 0)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(
            again
                .iter()
                .map(|row| row.metadata.utterance_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    }

    #[test]
    fn untimed_multi_turn_speech_is_preserved_once_as_mixed() {
        use crate::meeting_utterances::AttributionKind;
        let row = timed_row("First speaker. Short answer.", &[]);
        let (rows, _) = apply_utterances(
            std::slice::from_ref(&row),
            &[turn(0, 600, 0), turn(600, 1_200, 1)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].segment.text, row.segment.text);
        assert_eq!(rows[0].segment.voice, None);
        assert_eq!(rows[0].metadata.attribution.kind, AttributionKind::Mixed);
        assert_eq!(rows[0].metadata.attribution.turns.len(), 2);
        assert_eq!(rows[0].metadata.utterance_id, row.metadata.utterance_id);
    }

    #[test]
    fn overlapping_voices_remain_overlap_instead_of_dominant_speaker() {
        use crate::meeting_utterances::AttributionKind;
        let row = timed_row("Overlapping words", &[(0, 17, 0, 1_200)]);
        let (rows, _) = apply_utterances(
            &[row],
            &[turn(0, 900, 0), turn(500, 1_200, 1)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.attribution.kind, AttributionKind::Overlap);
        assert_eq!(rows[0].segment.voice, None);
    }

    #[test]
    fn invalid_word_mapping_keeps_unmapped_words_and_does_not_allocate_them() {
        use crate::meeting_utterances::AttributionKind;
        let row = timed_row("Yes unmapped no", &[(0, 3, 0, 300), (13, 14, 900, 1_200)]);
        let (rows, _) = apply_utterances(
            std::slice::from_ref(&row),
            &[turn(0, 600, 0), turn(600, 1_200, 1)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].segment.text, row.segment.text);
        assert_eq!(rows[0].metadata.attribution.kind, AttributionKind::Mixed);
    }

    #[test]
    fn manual_correction_survives_rerun_and_ambiguous_split_requests_review() {
        use crate::meeting_utterances::ManualCorrection;
        let mut row = timed_row("Yes no", &[(0, 3, 0, 500), (4, 6, 700, 1_200)]);
        row.metadata.manual = Some(ManualCorrection {
            name: Some("User confirmed".into()),
            voice: Some(4),
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        let (same, _) = apply_utterances(
            &[row.clone()],
            &[turn(0, 1_200, 0)],
            &[Identity::Known(0, 0.99)],
            Track::Microphone,
        );
        assert_eq!(same[0].segment.voice, Some(4));
        assert!(!same[0].metadata.manual.as_ref().unwrap().review_required);
        let (own, _) = apply_utterances(
            &[row.clone()],
            &[turn(0, 1_200, 0)],
            &[Identity::Me],
            Track::Microphone,
        );
        assert_eq!(own[0].segment.voice, Some(4));
        assert!(own[0].metadata.attribution.self_voice);
        let (split, _) = apply_utterances(
            &[row],
            &[turn(0, 600, 0), turn(600, 1_200, 1)],
            &[Identity::Unknown, Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(split.len(), 2);
        assert!(split.iter().all(|row| {
            row.metadata.manual.is_none()
                && row
                    .metadata
                    .unresolved_manual
                    .iter()
                    .all(|manual| manual.review_required)
        }));
        assert!(split.iter().all(|row| {
            row.metadata
                .unresolved_manual
                .iter()
                .any(|manual| manual.name.as_deref() == Some("User confirmed"))
        }));
    }

    #[test]
    fn physical_source_prevents_labels_bleeding_between_shared_microphone_and_call() {
        let mic = timed_row("Room", &[(0, 4, 0, 1_200)]);
        let mut call = timed_row("Call", &[(0, 4, 0, 1_200)]);
        call.metadata.source_id = Some("system".into());
        call.segment.speaker = Speaker::Others;
        call.segment.voice = Some(1);
        call.metadata.refresh_fingerprints(&call.segment);
        let (rows, _) = apply_utterances(
            &[mic, call.clone()],
            &[turn(0, 1_200, 0)],
            &[Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(rows[0].segment.voice, Some(2));
        assert_eq!(rows[1], call);
    }

    #[test]
    fn mixed_other_track_voices_are_reserved_before_new_microphone_clusters() {
        let mic = timed_row("Room", &[(0, 4, 0, 1_200)]);
        let mut call = timed_row("Call", &[]);
        call.metadata.source_id = Some("system_audio".into());
        call.metadata.refresh_fingerprints(&call.segment);
        let (system, _) = apply_utterances(
            &[mic, call],
            &[turn(0, 600, 0), turn(600, 1_200, 1)],
            &[Identity::Unknown, Identity::Unknown],
            Track::System,
        );
        assert_eq!(system[1].segment.voice, None);
        assert_eq!(system[1].metadata.attribution.voices, [1, 2]);
        let (hybrid, applied) = apply_utterances(
            &system,
            &[turn(0, 1_200, 0)],
            &[Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(hybrid[0].segment.voice, Some(3));
        assert_eq!(applied.voices, [(3, 0)]);
        assert_eq!(hybrid[1], system[1]);
    }

    #[test]
    fn prior_automatic_name_is_not_promoted_to_manual_during_regeneration() {
        let mut row = timed_row("Words", &[(0, 5, 0, 1_200)]);
        row.segment.speaker = Speaker::Others;
        row.segment.voice = Some(1);
        assert_eq!(
            carry_utterance_names(
                &[row.clone()],
                &[row],
                &[SpeakerName {
                    voice: 1,
                    name: "Old automatic".into()
                }],
                &[(1, "Current remembered".into())]
            ),
            [SpeakerName {
                voice: 1,
                name: "Current remembered".into()
            }]
        );
    }

    #[test]
    fn rolling_explicit_voice_map_never_renumbers_first_appearance() {
        let rows = vec![timed_row("Yes no", &[(0, 3, 0, 500), (4, 6, 700, 1_200)])];
        let (first, _) = apply_utterances_with_voice_map(
            &rows,
            &[turn(0, 600, 9), turn(600, 1_200, 2)],
            Track::Microphone,
            &[(9, 10), (2, 3)],
            true,
        );
        assert_eq!(
            first
                .iter()
                .map(|row| row.segment.voice)
                .collect::<Vec<_>>(),
            [Some(10), Some(3)]
        );
        assert!(first.iter().all(|row| row.metadata.attribution.provisional));
        let ids: Vec<_> = first
            .iter()
            .map(|row| row.metadata.utterance_id.clone())
            .collect();
        let (second, _) = apply_utterances_with_voice_map(
            &first,
            &[turn(700, 1_200, 2), turn(0, 600, 9)],
            Track::Microphone,
            &[(2, 3), (9, 10)],
            true,
        );
        assert_eq!(
            second
                .iter()
                .map(|row| row.segment.voice)
                .collect::<Vec<_>>(),
            [Some(10), Some(3)]
        );
        assert_eq!(
            second
                .iter()
                .map(|row| row.metadata.utterance_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        let (finalized, _) = apply_utterances_with_voice_map(
            &second,
            &[turn(0, 600, 9), turn(600, 1_200, 2)],
            Track::Microphone,
            &[(9, 10), (2, 3)],
            false,
        );
        assert!(
            finalized
                .iter()
                .all(|row| !row.metadata.attribution.provisional)
        );
    }

    #[test]
    fn final_analysis_reuses_unambiguous_live_voice_across_both_physical_sources() {
        let microphone = timed_row("Room", &[(0, 4, 0, 1_200)]);
        let mut system = timed_row("Call", &[(0, 4, 0, 1_200)]);
        system.metadata.source_id = Some("system_audio".into());
        system.metadata.refresh_fingerprints(&system.segment);
        let (rows, _) = apply_utterances_with_voice_map(
            &[microphone, system],
            &[turn(0, 1_200, 0)],
            Track::Microphone,
            &[(0, 1)],
            true,
        );
        let (rows, _) = apply_utterances_with_voice_map(
            &rows,
            &[turn(0, 1_200, 0)],
            Track::System,
            &[(0, 1)],
            true,
        );
        let ids: Vec<_> = rows
            .iter()
            .map(|row| row.metadata.utterance_id.clone())
            .collect();
        let (rows, system_applied) = apply_utterances(
            &rows,
            &[turn(0, 1_200, 7)],
            &[Identity::Unknown; 8],
            Track::System,
        );
        assert_eq!(system_applied.voices, [(1, 7)]);
        let (rows, microphone_applied) = apply_utterances(
            &rows,
            &[turn(0, 1_200, 3)],
            &[Identity::Unknown; 4],
            Track::Microphone,
        );
        assert_eq!(microphone_applied.voices, [(1, 3)]);
        assert!(
            rows.iter()
                .all(|row| row.segment.voice == Some(1) && !row.metadata.attribution.provisional)
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.metadata.utterance_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    }

    #[test]
    fn ambiguous_final_mapping_abstains_instead_of_recycling_live_voice_numbers() {
        let row = timed_row("Intact mixed words", &[]);
        let (provisional, _) = apply_utterances_with_voice_map(
            std::slice::from_ref(&row),
            &[turn(0, 600, 0), turn(600, 1_200, 1)],
            Track::Microphone,
            &[(0, 1), (1, 2)],
            true,
        );
        let (finalized, applied) = apply_utterances(
            &provisional,
            &[turn(0, 1_200, 7)],
            &[Identity::Unknown; 8],
            Track::Microphone,
        );
        assert!(applied.voices.is_empty());
        assert_eq!(finalized[0].segment.voice, None);
        assert_eq!(finalized[0].segment.text, row.segment.text);
        assert_eq!(
            finalized[0].metadata.utterance_id,
            provisional[0].metadata.utterance_id
        );
        assert!(!finalized[0].metadata.attribution.provisional);
    }

    /// Needs the Sortformer model in the app folder and `/tmp/diar/meeting.wav`; run with `--ignored`.
    #[test]
    #[ignore]
    fn real_diarization_splits_voices() {
        let home = std::env::var("HOME").unwrap();
        let model = std::path::PathBuf::from(home)
            .join("Library/Application Support/Pennel/models")
            .join(format!("{}.gguf", crate::models::DIARIZER_ID));
        let wav =
            std::fs::read(std::env::var("DIAR_WAV").unwrap_or("/tmp/diar/meeting.wav".into()))
                .unwrap();
        let (samples, _) = crate::wav::decode_wav(&wav).unwrap();
        let turns = diarize(&model, &samples).unwrap();
        let voices: std::collections::HashSet<u32> = turns.iter().map(|t| t.2).collect();
        eprintln!("{turns:?}");
        assert_eq!(voices.len(), 3);
    }

    #[cfg(not(feature = "local-models"))]
    #[test]
    fn diarize_without_feature_has_an_english_diagnostic() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            diarize(file.path(), &[0.0; 16_000]),
            Err(PennelError::local_models_unavailable())
        );
    }
}
