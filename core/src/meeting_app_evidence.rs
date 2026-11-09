//! Turns active-speaker observations read from a meeting app's interface into
//! name suggestions for system-track voices.
//!
//! A highlighted name is display evidence, not proof of identity: the result is
//! only ever a suggestion the user confirms, and ambiguous evidence yields none.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::PennelEngine;
use crate::error::PennelError;
use crate::meetings::Speaker;

/// What a meeting app showed during one recording. Stored beside the meeting's voiceprints in the
/// private data folder, never in the synced meeting folder or exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSpeakerEvidence {
    pub app_id: String,
    pub samples: Vec<SpeakingSample>,
    /// Everyone the app showed during the call, including people who never spoke.
    #[serde(default)]
    pub participants: Vec<String>,
}

/// One sample of the meeting app's interface on the recording timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakingSample {
    pub at_ms: u64,
    /// `false` when the window was hidden, unreadable or belonged to another call.
    pub available: bool,
    /// Remote participants the app highlighted as speaking. The local user is excluded
    /// by the reader because their speech is on the microphone track.
    pub speaking: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceTurn {
    pub voice: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NameSuggestion {
    pub voice: u32,
    pub name: String,
    pub app_id: String,
    /// Share of this voice's attributable evidence that pointed at `name`.
    pub agreement: f32,
    /// Share of this voice's speech covered by attributable evidence.
    pub coverage: f32,
    /// The highlighted participant is a room or a device several people spoke through, so the name
    /// identifies where the voice came from, not who it is.
    pub shared_device: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    Invited,
    MeetingApp,
    KnownVoice,
}

/// A name the user can pick for a voice: who was invited, who the meeting app showed, or a
/// remembered voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameCandidate {
    pub name: String,
    pub source: CandidateSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceNaming {
    pub suggestions: Vec<NameSuggestion>,
    pub candidates: Vec<NameCandidate>,
}

#[derive(Debug, Clone, Copy)]
pub struct FusionPolicy {
    /// Meeting apps highlight a speaker shortly after the audio; observations are shifted back.
    pub highlight_latency_ms: u64,
    /// A sample stops counting this long after it was taken, so gaps in sampling are not filled.
    pub sample_validity_ms: u64,
    pub min_evidence_ms: u64,
    pub min_coverage: f32,
    pub min_agreement: f32,
}

impl Default for FusionPolicy {
    fn default() -> Self {
        Self {
            highlight_latency_ms: 400,
            sample_validity_ms: 1_000,
            min_evidence_ms: 3_000,
            min_coverage: 0.3,
            min_agreement: 0.75,
        }
    }
}

#[derive(Default)]
struct Tally {
    speech_ms: u64,
    by_name: HashMap<String, u64>,
}

impl Tally {
    fn evidence_ms(&self) -> u64 {
        self.by_name.values().sum()
    }

    fn leader(&self) -> Option<(&str, u64)> {
        let mut entries: Vec<_> = self.by_name.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        entries.first().map(|(name, ms)| (name.as_str(), **ms))
    }
}

pub fn suggest_names(
    turns: &[VoiceTurn],
    samples: &[SpeakingSample],
    policy: FusionPolicy,
) -> Vec<NameSuggestion> {
    let mut samples: Vec<&SpeakingSample> = samples.iter().collect();
    samples.sort_by_key(|sample| sample.at_ms);
    let intervals = attributable_intervals(&samples, policy);

    let mut tallies: HashMap<u32, Tally> = HashMap::new();
    for turn in turns.iter().filter(|turn| turn.end_ms > turn.start_ms) {
        let tally = tallies.entry(turn.voice).or_default();
        tally.speech_ms += turn.end_ms - turn.start_ms;
        for (start, end, name) in &intervals {
            let overlap = overlap_ms(turn.start_ms, turn.end_ms, *start, *end);
            if overlap > 0 {
                *tally.by_name.entry((*name).to_owned()).or_default() += overlap;
            }
        }
    }

    let mut suggestions: Vec<NameSuggestion> = tallies
        .into_iter()
        .filter_map(|(voice, tally)| {
            let evidence = tally.evidence_ms();
            let (name, leader_ms) = tally.leader()?;
            let agreement = leader_ms as f32 / evidence as f32;
            let coverage = evidence as f32 / tally.speech_ms as f32;
            (evidence >= policy.min_evidence_ms
                && coverage >= policy.min_coverage
                && agreement >= policy.min_agreement)
                .then(|| NameSuggestion {
                    voice,
                    name: name.to_owned(),
                    app_id: String::new(),
                    agreement,
                    coverage,
                    shared_device: false,
                })
        })
        .collect();
    suggestions.sort_by_key(|suggestion| suggestion.voice);
    suggestions
}

/// Intervals on the audio timeline where exactly one remote participant was highlighted.
/// Several simultaneous highlights or none at all give no evidence.
fn attributable_intervals<'a>(
    samples: &[&'a SpeakingSample],
    policy: FusionPolicy,
) -> Vec<(u64, u64, &'a str)> {
    samples
        .iter()
        .enumerate()
        .filter_map(|(index, sample)| {
            let [name] = sample.speaking.as_slice() else {
                return None;
            };
            if !sample.available || name.trim().is_empty() {
                return None;
            }
            let observed_until = samples
                .get(index + 1)
                .map_or(u64::MAX, |next| next.at_ms)
                .min(sample.at_ms.saturating_add(policy.sample_validity_ms));
            let start = sample.at_ms.saturating_sub(policy.highlight_latency_ms);
            let end = observed_until.saturating_sub(policy.highlight_latency_ms);
            (end > start).then_some((start, end, name.as_str()))
        })
        .collect()
}

impl PennelEngine {
    /// Replaces the evidence for a meeting; an empty sample list removes it.
    pub fn set_meeting_app_speaker_evidence(
        &self,
        id: &str,
        evidence: AppSpeakerEvidence,
    ) -> Result<(), PennelError> {
        self.voice_store()
            .set_app_speaker_evidence(&self.meeting_key(id), &evidence)
    }

    /// Name suggestions for unnamed voices plus every name the user can choose from.
    pub fn meeting_voice_naming(&self, id: &str) -> VoiceNaming {
        let key = self.meeting_key(id);
        let evidence = self.voice_store().app_speaker_evidence(&key);
        let people = self.meeting_people_for_voices(id);
        let store = self.meeting_store();
        let mut invited = store.attendees(id);
        invited.extend(people.names);
        let candidates = name_candidates(
            &invited,
            evidence.as_ref().map_or(&[][..], |e| &e.participants),
            &self
                .known_voices()
                .into_iter()
                .map(|voice| voice.name)
                .collect::<Vec<_>>(),
            &people.rooms,
        );
        let Some(evidence) = evidence else {
            return VoiceNaming {
                suggestions: Vec::new(),
                candidates,
            };
        };
        let named: Vec<u32> = store.speaker_names(id).iter().map(|n| n.voice).collect();
        let Ok(snapshot) = store.read_transcript_snapshot(id) else {
            return VoiceNaming {
                suggestions: Vec::new(),
                candidates,
            };
        };
        let turns: Vec<VoiceTurn> = snapshot
            .utterances
            .iter()
            .filter(|row| row.segment.speaker == Speaker::Others && !row.segment.failed)
            .filter_map(|row| {
                let voice = row.segment.voice.filter(|voice| !named.contains(voice))?;
                Some(VoiceTurn {
                    voice,
                    start_ms: row.segment.start_ms,
                    end_ms: row.segment.end_ms,
                })
            })
            .collect();
        let prints: Vec<(u32, Vec<f32>)> = self
            .voice_store()
            .meeting_voiceprints(&key, Some(&snapshot.transcript_digest))
            .into_iter()
            .map(|print| (print.voice, print.embedding))
            .collect();
        let suggestions = suggest_names(&turns, &evidence.samples, FusionPolicy::default())
            .into_iter()
            .map(|suggestion| NameSuggestion {
                name: crate::voices::clean_name(&suggestion.name),
                app_id: evidence.app_id.clone(),
                ..suggestion
            })
            .filter(|suggestion| !suggestion.name.is_empty())
            .collect();
        VoiceNaming {
            suggestions: mark_shared_devices(suggestions, &prints, &people.rooms),
            candidates,
        }
    }
}

/// One participant name on several voices is either one person split by diarization or several
/// people behind one device. Voiceprints decide: clearly different voices mean a shared device.
/// Without voiceprints to compare, the safe reading is a shared device.
fn mark_shared_devices(
    mut suggestions: Vec<NameSuggestion>,
    prints: &[(u32, Vec<f32>)],
    rooms: &[String],
) -> Vec<NameSuggestion> {
    let print = |voice: u32| {
        prints
            .iter()
            .find(|(v, embedding)| *v == voice && !embedding.is_empty())
            .map(|(_, embedding)| embedding.as_slice())
    };
    let shared: Vec<String> = suggestions
        .iter()
        .map(|suggestion| suggestion.name.clone())
        .filter(|name| {
            if rooms.iter().any(|room| same_name(room, name)) {
                return true;
            }
            let voices: Vec<u32> = suggestions
                .iter()
                .filter(|s| same_name(&s.name, name))
                .map(|s| s.voice)
                .collect();
            voices.iter().enumerate().any(|(i, a)| {
                voices[i + 1..]
                    .iter()
                    .any(|b| match (print(*a), print(*b)) {
                        (Some(a), Some(b)) => {
                            pennel_speakers::cosine(a, b) < pennel_speakers::MERGE_THRESHOLD
                        }
                        _ => true,
                    })
            })
        })
        .collect();
    for suggestion in &mut suggestions {
        suggestion.shared_device = shared.iter().any(|name| same_name(name, &suggestion.name));
    }
    suggestions
}

fn name_candidates(
    invited: &[String],
    meeting_app: &[String],
    known_voices: &[String],
    rooms: &[String],
) -> Vec<NameCandidate> {
    let mut candidates: Vec<NameCandidate> = Vec::new();
    let sources = [
        (invited, CandidateSource::Invited),
        (meeting_app, CandidateSource::MeetingApp),
        (known_voices, CandidateSource::KnownVoice),
    ];
    for (names, source) in sources {
        for name in names {
            let name = crate::voices::clean_name(name);
            if name.is_empty()
                || rooms.iter().any(|room| same_name(room, &name))
                || candidates.iter().any(|c| same_name(&c.name, &name))
            {
                continue;
            }
            candidates.push(NameCandidate {
                name,
                source: source.clone(),
            });
        }
    }
    candidates
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn overlap_ms(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> u64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> FusionPolicy {
        FusionPolicy {
            highlight_latency_ms: 0,
            ..FusionPolicy::default()
        }
    }

    fn sampled(from_ms: u64, to_ms: u64, speaking: &[&str]) -> Vec<SpeakingSample> {
        (from_ms..to_ms)
            .step_by(250)
            .map(|at_ms| SpeakingSample {
                at_ms,
                available: true,
                speaking: speaking.iter().map(|name| (*name).to_owned()).collect(),
            })
            .collect()
    }

    fn turn(voice: u32, start_ms: u64, end_ms: u64) -> VoiceTurn {
        VoiceTurn {
            voice,
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn names_each_voice_after_its_highlighted_participant() {
        let mut samples = sampled(0, 10_000, &["Ann Peeters"]);
        samples.extend(sampled(10_000, 20_000, &["Bert Jacobs"]));
        let turns = [turn(1, 0, 10_000), turn(2, 10_000, 20_000)];

        let suggestions = suggest_names(&turns, &samples, policy());

        let names: Vec<_> = suggestions
            .iter()
            .map(|s| (s.voice, s.name.as_str()))
            .collect();
        assert_eq!(names, [(1, "Ann Peeters"), (2, "Bert Jacobs")]);
        assert!(suggestions.iter().all(|s| s.agreement > 0.99));
    }

    #[test]
    fn corrects_for_highlight_latency() {
        let mut samples = sampled(0, 400, &[]);
        samples.extend(sampled(400, 6_400, &["Ann"]));
        samples.extend(sampled(6_400, 12_400, &["Bert"]));
        let turns = [turn(1, 0, 6_000), turn(2, 6_000, 12_000)];

        let suggestions = suggest_names(&turns, &samples, FusionPolicy::default());

        assert_eq!(suggestions.len(), 2);
        assert!(suggestions.iter().all(|s| s.agreement > 0.99));
    }

    #[test]
    fn overlapping_highlights_give_no_evidence() {
        let samples = sampled(0, 10_000, &["Ann", "Bert"]);

        assert!(suggest_names(&[turn(1, 0, 10_000)], &samples, policy()).is_empty());
    }

    #[test]
    fn disagreeing_evidence_gives_no_suggestion() {
        let mut samples = sampled(0, 6_000, &["Ann"]);
        samples.extend(sampled(6_000, 10_000, &["Bert"]));

        assert!(suggest_names(&[turn(1, 0, 10_000)], &samples, policy()).is_empty());
    }

    #[test]
    fn minority_noise_still_allows_a_clear_leader() {
        let mut samples = sampled(0, 9_000, &["Ann"]);
        samples.extend(sampled(9_000, 10_000, &["Bert"]));

        let suggestions = suggest_names(&[turn(1, 0, 10_000)], &samples, policy());

        assert_eq!(suggestions[0].name, "Ann");
        assert!((suggestions[0].agreement - 0.9).abs() < 0.01);
    }

    #[test]
    fn unavailable_samples_and_sampling_gaps_are_not_evidence() {
        let mut samples = sampled(0, 2_000, &["Ann"]);
        samples.push(SpeakingSample {
            at_ms: 2_000,
            available: false,
            speaking: vec!["Ann".into()],
        });
        samples.push(SpeakingSample {
            at_ms: 30_000,
            available: true,
            speaking: vec!["Ann".into()],
        });

        assert!(suggest_names(&[turn(1, 0, 30_000)], &samples, policy()).is_empty());
    }

    #[test]
    fn too_little_evidence_gives_no_suggestion() {
        let samples = sampled(0, 2_000, &["Ann"]);

        assert!(suggest_names(&[turn(1, 0, 2_000)], &samples, policy()).is_empty());
    }

    #[test]
    fn low_coverage_gives_no_suggestion() {
        let mut samples = sampled(0, 4_000, &["Ann"]);
        samples.extend(sampled(4_000, 20_000, &[]));

        assert!(suggest_names(&[turn(1, 0, 20_000)], &samples, policy()).is_empty());
    }

    #[test]
    fn evidence_outside_a_voices_turns_is_ignored() {
        let mut samples = sampled(0, 10_000, &["Ann"]);
        samples.extend(sampled(10_000, 20_000, &["Bert"]));
        let turns = [turn(1, 0, 4_000), turn(1, 10_000, 10_500)];

        let suggestions = suggest_names(&turns, &samples, policy());

        assert_eq!(suggestions[0].name, "Ann");
        assert!(suggestions[0].agreement > 0.85);
    }

    fn suggestion(voice: u32, name: &str) -> NameSuggestion {
        NameSuggestion {
            voice,
            name: name.into(),
            app_id: "com.microsoft.teams2".into(),
            agreement: 1.0,
            coverage: 1.0,
            shared_device: false,
        }
    }

    fn shared(suggestions: &[NameSuggestion]) -> Vec<(u32, bool)> {
        suggestions
            .iter()
            .map(|s| (s.voice, s.shared_device))
            .collect()
    }

    #[test]
    fn different_voices_behind_one_participant_are_a_shared_device() {
        let prints = [
            (1, vec![1.0, 0.0]),
            (2, vec![0.0, 1.0]),
            (3, vec![0.6, 0.8]),
        ];
        let marked = mark_shared_devices(
            vec![
                suggestion(1, "Ann"),
                suggestion(2, "Ann"),
                suggestion(3, "Bert"),
            ],
            &prints,
            &[],
        );

        assert_eq!(shared(&marked), [(1, true), (2, true), (3, false)]);
    }

    #[test]
    fn one_person_split_into_similar_voices_keeps_the_name() {
        let prints = [(1, vec![1.0, 0.1]), (2, vec![0.95, 0.15])];
        let marked = mark_shared_devices(
            vec![suggestion(1, "Ann"), suggestion(2, "ann ")],
            &prints,
            &[],
        );

        assert_eq!(shared(&marked), [(1, false), (2, false)]);
    }

    #[test]
    fn repeated_names_without_voiceprints_are_treated_as_shared() {
        let marked =
            mark_shared_devices(vec![suggestion(1, "Ann"), suggestion(2, "Ann")], &[], &[]);

        assert_eq!(shared(&marked), [(1, true), (2, true)]);
    }

    #[test]
    fn a_calendar_room_is_never_a_person_even_with_one_voice() {
        let marked = mark_shared_devices(
            vec![suggestion(1, "Meeting room 0.7")],
            &[(1, vec![1.0])],
            &["Meeting room 0.7".into()],
        );

        assert_eq!(shared(&marked), [(1, true)]);
    }

    #[test]
    fn candidates_combine_invitees_meeting_participants_and_known_voices_without_rooms() {
        let candidates = name_candidates(
            &[
                "Ann Peeters".into(),
                "Meeting room 0.7".into(),
                "ann peeters".into(),
            ],
            &[
                "Bert Jacobs".into(),
                "Ann Peeters".into(),
                "Meeting room 0.7".into(),
            ],
            &["Cas".into()],
            &["Meeting room 0.7".into()],
        );

        assert_eq!(
            candidates,
            [
                NameCandidate {
                    name: "Ann Peeters".into(),
                    source: CandidateSource::Invited
                },
                NameCandidate {
                    name: "Bert Jacobs".into(),
                    source: CandidateSource::MeetingApp
                },
                NameCandidate {
                    name: "Cas".into(),
                    source: CandidateSource::KnownVoice
                },
            ]
        );
    }

    #[test]
    fn unsorted_samples_are_ordered_before_use() {
        let mut samples = sampled(0, 10_000, &["Ann"]);
        samples.reverse();

        let suggestions = suggest_names(&[turn(1, 0, 10_000)], &samples, policy());

        assert_eq!(suggestions[0].name, "Ann");
    }
}
