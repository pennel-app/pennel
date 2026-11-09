//! Turns active-speaker observations read from a meeting app's interface into
//! name suggestions for system-track voices.
//!
//! A highlighted name is display evidence, not proof of identity: the result is
//! only ever a suggestion the user confirms, and ambiguous evidence yields none.

use std::collections::HashMap;

/// One sample of the meeting app's interface on the recording timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct NameSuggestion {
    pub voice: u32,
    pub name: String,
    /// Share of this voice's attributable evidence that pointed at `name`.
    pub agreement: f32,
    /// Share of this voice's speech covered by attributable evidence.
    pub coverage: f32,
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
                    agreement,
                    coverage,
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

    #[test]
    fn unsorted_samples_are_ordered_before_use() {
        let mut samples = sampled(0, 10_000, &["Ann"]);
        samples.reverse();

        let suggestions = suggest_names(&[turn(1, 0, 10_000)], &samples, policy());

        assert_eq!(suggestions[0].name, "Ann");
    }
}
