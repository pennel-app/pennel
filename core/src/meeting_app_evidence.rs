//! Turns active-speaker observations read from a meeting app's interface into
//! name estimates for system-track voices. Applying an estimate is a separate opt-in policy;
//! manual corrections and remembered identities always take precedence.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::PennelEngine;
use crate::error::PennelError;
use crate::meeting_utterances::{AttributionKind, MeetingUtterance};
use crate::meetings::SpeakerName;
use crate::settings::MeetingAppNamePolicy;
use crate::voices::MeetingVoiceprint;

const TEAMS: &str = "com.microsoft.teams2";

/// A single observed call window tied to this recording, never an arbitrary foreground app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEvidenceBinding {
    pub recording_uid: String,
    pub recording_started_at: String,
    pub process_id: u32,
    pub process_started_at_ms: u64,
    pub window_id: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEvidenceAvailability {
    Available,
    Disabled,
    Unsupported,
    PermissionDenied,
    #[default]
    Unavailable,
}

/// Portable provenance for an automatic display name. The raw participant observations stay private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticNameAttribution {
    pub name: String,
    pub app_id: String,
    pub binding: AppEvidenceBinding,
    pub evidence_revision: String,
    pub source_fingerprint: String,
    pub attribution_revision: u64,
}

/// What a meeting app showed during one recording. Stored beside the meeting's voiceprints in the
/// private data folder, never in the synced meeting folder or exports.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSpeakerEvidence {
    pub app_id: String,
    pub samples: Vec<SpeakingSample>,
    /// Everyone the app showed during the call, including people who never spoke.
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub binding: Option<AppEvidenceBinding>,
    #[serde(default)]
    pub availability: AppEvidenceAvailability,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub duplicate_names: bool,
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
    pub automatic_names: Vec<NameSuggestion>,
    pub status: VoiceNamingStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceNamingStatus {
    Disabled,
    Unsupported,
    PermissionDenied,
    Unavailable,
    Unbound,
    InsufficientEvidence,
    Conflict,
    Suggested,
    AutomaticReady,
    Applied,
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

fn clear_system_voice(row: &MeetingUtterance) -> Option<u32> {
    if row.metadata.source_id.as_deref() != Some("system_audio")
        || row.segment.failed
        || row.segment.text.trim().is_empty()
        || row.metadata.attribution.kind != AttributionKind::Single
        || row.metadata.attribution.provisional
        || row.metadata.attribution.self_voice
    {
        return None;
    }
    let [voice] = row.metadata.attribution.voices.as_slice() else {
        return None;
    };
    (row.segment.voice == Some(*voice) && crate::meetings::VOICES.contains(voice)).then_some(*voice)
}

fn acoustic_intervals(
    row: &MeetingUtterance,
) -> impl Iterator<Item = (Option<u32>, u64, u64)> + '_ {
    row.metadata.attribution.turns.iter().filter_map(|turn| {
        let start = turn
            .start_ms
            .max(row.segment.start_ms)
            .max(row.metadata.start_ms);
        let end = turn.end_ms.min(row.segment.end_ms).min(row.metadata.end_ms);
        (turn.start_ms < turn.end_ms && start < end).then_some((
            turn.voice
                .filter(|voice| crate::meetings::VOICES.contains(voice)),
            start,
            end,
        ))
    })
}

/// Merge duplicate/adjacent fragments, without counting the silence between disjoint intervals.
fn union_voice_turns(mut turns: Vec<VoiceTurn>) -> Vec<VoiceTurn> {
    turns.sort_by_key(|turn| (turn.voice, turn.start_ms, turn.end_ms));
    let mut merged: Vec<VoiceTurn> = Vec::new();
    for turn in turns {
        if turn.end_ms <= turn.start_ms {
            continue;
        }
        if let Some(previous) = merged.last_mut()
            && previous.voice == turn.voice
            && turn.start_ms <= previous.end_ms
        {
            previous.end_ms = previous.end_ms.max(turn.end_ms);
        } else {
            merged.push(turn);
        }
    }
    merged
}

/// Automatic identity requires acoustic speech, never the surrounding ASR chunk's duration.
fn clean_system_turns(rows: &[MeetingUtterance]) -> Vec<VoiceTurn> {
    let candidates = union_voice_turns(
        rows.iter()
            .flat_map(|row| {
                let voice = clear_system_voice(row);
                acoustic_intervals(row).filter_map(move |(observed, start_ms, end_ms)| {
                    voice
                        .filter(|voice| Some(*voice) == observed)
                        .map(|voice| VoiceTurn {
                            voice,
                            start_ms,
                            end_ms,
                        })
                })
            })
            .collect(),
    );
    // Mixed/unknown rows can contain a competing speaker even when their words could not be
    // split. Include their actual acoustic intervals when removing overlapping evidence.
    let mut observed: Vec<_> = rows
        .iter()
        .filter(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
        .flat_map(acoustic_intervals)
        .collect();
    observed.sort_by_key(|(_, start, end)| (*start, *end));
    let mut clean = Vec::new();
    for turn in candidates {
        let mut cursor = turn.start_ms;
        for (voice, start, end) in &observed {
            if *start >= turn.end_ms {
                break;
            }
            if *voice == Some(turn.voice) || *end <= cursor {
                continue;
            }
            if *start > cursor {
                clean.push(VoiceTurn {
                    voice: turn.voice,
                    start_ms: cursor,
                    end_ms: *start,
                });
            }
            cursor = cursor.max((*end).min(turn.end_ms));
            if cursor >= turn.end_ms {
                break;
            }
        }
        if cursor < turn.end_ms {
            clean.push(VoiceTurn {
                voice: turn.voice,
                start_ms: cursor,
                end_ms: turn.end_ms,
            });
        }
    }
    union_voice_turns(clean)
}

/// Older saved files can still offer a reviewable hint; coarse row timing never qualifies a name
/// for automatic application.
fn system_suggestion_turns(rows: &[MeetingUtterance]) -> Vec<VoiceTurn> {
    let mut turns = clean_system_turns(rows);
    turns.extend(
        rows.iter()
            .filter(|row| row.metadata.attribution.turns.is_empty())
            .filter_map(|row| {
                clear_system_voice(row).map(|voice| VoiceTurn {
                    voice,
                    start_ms: row.segment.start_ms,
                    end_ms: row.segment.end_ms,
                })
            }),
    );
    union_voice_turns(turns)
}

fn automatic_policy() -> FusionPolicy {
    FusionPolicy {
        min_evidence_ms: 10_000,
        min_coverage: 0.5,
        min_agreement: 0.9,
        ..FusionPolicy::default()
    }
}

fn automatic_suggestions(turns: &[VoiceTurn], samples: &[SpeakingSample]) -> Vec<NameSuggestion> {
    let policy = automatic_policy();
    let turns = union_voice_turns(turns.to_vec());
    let mut ordered: Vec<_> = samples.iter().collect();
    ordered.sort_by_key(|sample| sample.at_ms);
    let intervals = attributable_intervals(&ordered, policy);
    suggest_names(&turns, samples, policy)
        .into_iter()
        .filter(|suggestion| {
            let mut groups: Vec<(u64, u64)> = Vec::new();
            for turn in turns.iter().filter(|turn| turn.voice == suggestion.voice) {
                let support = intervals
                    .iter()
                    .filter(|(_, _, name)| same_name(name, &suggestion.name))
                    .map(|(start, end, _)| overlap_ms(turn.start_ms, turn.end_ms, *start, *end))
                    .sum::<u64>();
                // Nearby acoustic fragments belong to one evidence turn. Grouping them does not
                // add their intervening silence to agreement, coverage or supporting duration.
                if let Some((end, supported_ms)) = groups.last_mut()
                    && turn.start_ms <= end.saturating_add(300)
                {
                    *end = (*end).max(turn.end_ms);
                    *supported_ms += support;
                } else {
                    groups.push((turn.end_ms, support));
                }
            }
            groups
                .iter()
                .filter(|(_, support)| *support >= 1_000)
                .count()
                >= 3
        })
        .collect()
}

fn automatic_names_for_rows(
    rows: &[MeetingUtterance],
    samples: &[SpeakingSample],
) -> Vec<NameSuggestion> {
    let turns = clean_system_turns(rows);
    let mut ordered: Vec<_> = samples.iter().collect();
    ordered.sort_by_key(|sample| sample.at_ms);
    let intervals = attributable_intervals(&ordered, automatic_policy());
    automatic_suggestions(&turns, samples)
        .into_iter()
        .filter(|suggestion| {
            // A chunk split by ASR can leave disconnected fragments with one original lineage. They
            // provide useful speech evidence, but cannot manufacture three independent observations.
            let mut anchors: Vec<std::collections::BTreeSet<&str>> = Vec::new();
            for row in rows
                .iter()
                .filter(|row| clear_system_voice(row) == Some(suggestion.voice))
            {
                let support = turns
                    .iter()
                    .filter(|turn| turn.voice == suggestion.voice)
                    .map(|turn| {
                        (
                            turn.start_ms.max(row.segment.start_ms),
                            turn.end_ms.min(row.segment.end_ms),
                        )
                    })
                    .filter(|(start, end)| start < end)
                    .flat_map(|(start, end)| {
                        intervals
                            .iter()
                            .filter(|(_, _, name)| same_name(name, &suggestion.name))
                            .map(move |(observed_start, observed_end, _)| {
                                overlap_ms(start, end, *observed_start, *observed_end)
                            })
                    })
                    .sum::<u64>();
                if support < 1_000 || row.metadata.attribution.turns.is_empty() {
                    continue;
                }
                let mut anchor: std::collections::BTreeSet<&str> = row
                    .metadata
                    .lineage
                    .iter()
                    .map(String::as_str)
                    .chain([
                        row.metadata.utterance_id.as_str(),
                        row.metadata.source_fingerprint.as_str(),
                    ])
                    .collect();
                let mut index = 0;
                while index < anchors.len() {
                    if anchor.is_disjoint(&anchors[index]) {
                        index += 1;
                    } else {
                        anchor.extend(anchors.swap_remove(index));
                        index = 0;
                    }
                }
                anchors.push(anchor);
            }
            anchors.len() >= 3
        })
        .collect()
}

impl AppEvidenceBinding {
    pub(crate) fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.recording_uid).is_ok()
            && chrono::DateTime::parse_from_rfc3339(&self.recording_started_at).is_ok()
            && self.process_id > 0
            && self.process_started_at_ms > 0
            && !self.window_id.trim().is_empty()
            && self.window_id.len() <= 256
    }

    fn matches_recording(&self, recording: &crate::MeetingRecordingMetadata) -> bool {
        self.valid()
            && self.recording_uid == recording.meeting_uid
            && chrono::DateTime::parse_from_rfc3339(&self.recording_started_at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(&recording.started_at).ok())
                .is_some_and(|(observed, actual)| observed == actual)
    }
}

fn named_automatically(row: &MeetingUtterance, name: &SpeakerName) -> bool {
    row.segment.voice == Some(name.voice)
        && row.metadata.manual.is_none()
        && row.metadata.automatic_name.as_ref().is_some_and(|proof| {
            same_name(&proof.name, &name.name)
                && proof.source_fingerprint == row.metadata.source_fingerprint
                && proof.attribution_revision == row.metadata.attribution_revision
        })
}

impl PennelEngine {
    /// Replaces the evidence for a meeting, retaining availability when no samples were readable.
    pub fn set_meeting_app_speaker_evidence(
        &self,
        id: &str,
        evidence: AppSpeakerEvidence,
    ) -> Result<(), PennelError> {
        let settings = self.settings();
        let _policy = self.speaker_commit_policy_guard((
            settings.meeting_app_speakers,
            settings.meeting_app_name_policy,
        ))?;
        self.voice_store()
            .set_app_speaker_evidence(&self.meeting_key(id), &evidence)
    }

    fn app_naming_for_rows(
        &self,
        id: &str,
        rows: &[MeetingUtterance],
        names: &[SpeakerName],
        prints: &[MeetingVoiceprint],
        evidence: Option<AppSpeakerEvidence>,
    ) -> VoiceNaming {
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
        let mut result = VoiceNaming {
            suggestions: Vec::new(),
            candidates,
            automatic_names: Vec::new(),
            status: VoiceNamingStatus::Unavailable,
            reason: Some("no_meeting_app_evidence".into()),
        };
        let Some(evidence) = evidence else {
            if !self.settings().meeting_app_speakers {
                result.status = VoiceNamingStatus::Disabled;
                result.reason = Some("meeting_app_speakers_disabled".into());
            }
            return result;
        };
        let turns = system_suggestion_turns(rows);
        let embeddings: Vec<_> = prints
            .iter()
            .map(|print| (print.voice, print.embedding.clone()))
            .collect();
        let decorate = |suggestion: NameSuggestion| NameSuggestion {
            name: crate::voices::clean_name(&suggestion.name),
            app_id: evidence.app_id.clone(),
            ..suggestion
        };
        // Include already named voices in the shared-device assessment. A remembered voice must
        // not let another person behind the same endpoint acquire that person's name.
        let mut all_suggestions: Vec<_> =
            suggest_names(&turns, &evidence.samples, FusionPolicy::default())
                .into_iter()
                .map(decorate)
                .filter(|suggestion| !suggestion.name.is_empty())
                .collect();
        for name in names {
            if !all_suggestions
                .iter()
                .any(|s| s.voice == name.voice && same_name(&s.name, &name.name))
            {
                all_suggestions.push(NameSuggestion {
                    voice: name.voice,
                    name: name.name.clone(),
                    app_id: String::new(),
                    agreement: 1.0,
                    coverage: 1.0,
                    shared_device: false,
                });
            }
        }
        let all_suggestions = mark_shared_devices(all_suggestions, &embeddings, &people.rooms);
        let protected = |voice: u32| {
            names.iter().any(|name| {
                name.voice == voice && !rows.iter().any(|row| named_automatically(row, name))
            }) || rows.iter().any(|row| {
                row.segment.voice == Some(voice) && row.metadata.manual.is_some()
                    || row
                        .metadata
                        .unresolved_manual
                        .iter()
                        .any(|manual| manual.voice == Some(voice))
            })
        };
        result.suggestions = all_suggestions
            .iter()
            .filter(|suggestion| suggestion.app_id == evidence.app_id)
            .filter(|suggestion| !protected(suggestion.voice))
            .cloned()
            .collect();
        result.status = if result.suggestions.is_empty() {
            VoiceNamingStatus::InsufficientEvidence
        } else {
            VoiceNamingStatus::Suggested
        };
        result.reason = result
            .suggestions
            .is_empty()
            .then(|| "insufficient_speaking_evidence".into());

        if !self.settings().meeting_app_speakers {
            result.status = VoiceNamingStatus::Disabled;
            result.reason = Some("meeting_app_speakers_disabled".into());
            return result;
        }
        if evidence.app_id != TEAMS {
            result.status = VoiceNamingStatus::Unsupported;
            result.reason = Some("unsupported_meeting_app".into());
            return result;
        }
        let mut displayed_names = std::collections::HashSet::new();
        let ambiguous_display_names = evidence.participants.iter().any(|name| {
            let name = crate::voices::clean_name(name).to_lowercase();
            !name.is_empty() && !displayed_names.insert(name)
        });
        if evidence.duplicate_names || ambiguous_display_names {
            result.status = VoiceNamingStatus::Conflict;
            result.reason = Some("duplicate_participant_names".into());
            return result;
        }
        if evidence.availability != AppEvidenceAvailability::Available {
            result.status = match evidence.availability {
                AppEvidenceAvailability::Disabled => VoiceNamingStatus::Disabled,
                AppEvidenceAvailability::Unsupported => VoiceNamingStatus::Unsupported,
                AppEvidenceAvailability::PermissionDenied => VoiceNamingStatus::PermissionDenied,
                _ => VoiceNamingStatus::Unavailable,
            };
            result.reason = evidence
                .unavailable_reason
                .clone()
                .or_else(|| Some("meeting_app_unavailable".into()));
            return result;
        }
        let bound = evidence.binding.as_ref().is_some_and(|binding| {
            store
                .recording_metadata(id)
                .ok()
                .flatten()
                .is_some_and(|recording| binding.matches_recording(&recording))
        });
        if !bound {
            result.status = VoiceNamingStatus::Unbound;
            result.reason = Some("recording_call_binding_missing_or_stale".into());
            return result;
        }
        if self.settings().meeting_app_name_policy != MeetingAppNamePolicy::Automatic {
            return result;
        }
        let automatic = automatic_names_for_rows(rows, &evidence.samples);
        let mut conflict = false;
        result.automatic_names = automatic
            .into_iter()
            .map(decorate)
            .filter(|candidate| {
                if protected(candidate.voice) {
                    conflict |= names.iter().any(|name| {
                        name.voice == candidate.voice && !same_name(&name.name, &candidate.name)
                    }) || rows.iter().any(|row| {
                        row.segment.voice == Some(candidate.voice)
                            && row
                                .metadata
                                .manual
                                .as_ref()
                                .is_some_and(|manual| manual.review_required)
                            || row
                                .metadata
                                .unresolved_manual
                                .iter()
                                .any(|manual| manual.voice == Some(candidate.voice))
                    });
                    return false;
                }
                let shared = all_suggestions.iter().any(|suggestion| {
                    same_name(&suggestion.name, &candidate.name) && suggestion.shared_device
                });
                let reused_source = rows.iter().any(|row| {
                    row.metadata.source_id.as_deref() != Some("system_audio")
                        && (row.segment.voice == Some(candidate.voice)
                            || row.metadata.attribution.voices.contains(&candidate.voice))
                });
                conflict |= shared || reused_source;
                !shared && !reused_source && !candidate.name.is_empty()
            })
            .collect();
        if conflict {
            result.status = VoiceNamingStatus::Conflict;
            result.reason = Some("speaker_name_conflict_or_shared_device".into());
        } else if !result.automatic_names.is_empty() {
            result.status = if result.automatic_names.iter().all(|candidate| {
                names.iter().any(|name| {
                    name.voice == candidate.voice
                        && same_name(&name.name, &candidate.name)
                        && rows.iter().any(|row| named_automatically(row, name))
                })
            }) {
                VoiceNamingStatus::Applied
            } else {
                VoiceNamingStatus::AutomaticReady
            };
            result.reason = None;
        }
        result
    }

    /// Name estimates plus their availability and explicit automatic-policy eligibility.
    pub fn meeting_voice_naming(&self, id: &str) -> VoiceNaming {
        let store = self.meeting_store();
        let Ok(snapshot) = store.read_transcript_snapshot(id) else {
            return VoiceNaming {
                suggestions: Vec::new(),
                candidates: Vec::new(),
                automatic_names: Vec::new(),
                status: VoiceNamingStatus::Unavailable,
                reason: Some("transcript_unavailable".into()),
            };
        };
        let prints = self
            .voice_store()
            .meeting_voiceprints(&self.meeting_key(id), Some(&snapshot.transcript_digest));
        self.app_naming_for_rows(
            id,
            &snapshot.utterances,
            &store.speaker_names(id),
            &prints,
            self.voice_store()
                .app_speaker_evidence(&self.meeting_key(id)),
        )
    }

    /// Called with the same edit/sync guards as the final speaker commit.
    pub(crate) fn prepare_meeting_automatic_names(
        &self,
        id: &str,
        rows: &mut [MeetingUtterance],
        names: &mut Vec<SpeakerName>,
        prints: &[MeetingVoiceprint],
    ) -> Result<(), PennelError> {
        let evidence = self
            .voice_store()
            .app_speaker_evidence(&self.meeting_key(id));
        let naming = self.app_naming_for_rows(id, rows, names, prints, evidence.clone());
        let binding = evidence.as_ref().and_then(|e| e.binding.as_ref());
        let evidence_revision = evidence
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(PennelError::storage)?
            .map(|bytes| crate::meeting_utterances::digest(&bytes));
        names.retain(|name| !rows.iter().any(|row| named_automatically(row, name)));
        for row in rows.iter_mut() {
            row.metadata.automatic_name = None;
        }
        let Some((binding, evidence_revision)) = binding.zip(evidence_revision) else {
            return Ok(());
        };
        for candidate in naming.automatic_names {
            names.retain(|name| name.voice != candidate.voice);
            names.push(SpeakerName {
                voice: candidate.voice,
                name: candidate.name.clone(),
            });
            for row in rows.iter_mut().filter(|row| {
                row.metadata.source_id.as_deref() == Some("system_audio")
                    && row.segment.voice == Some(candidate.voice)
                    && row.metadata.manual.is_none()
                    && !row.segment.failed
                    && row.metadata.attribution.kind == AttributionKind::Single
                    && !row.metadata.attribution.provisional
                    && !row.metadata.attribution.self_voice
            }) {
                row.metadata.refresh_fingerprints(&row.segment);
                row.metadata.automatic_name = Some(AutomaticNameAttribution {
                    name: candidate.name.clone(),
                    app_id: candidate.app_id.clone(),
                    binding: binding.clone(),
                    evidence_revision: evidence_revision.clone(),
                    source_fingerprint: row.metadata.source_fingerprint.clone(),
                    attribution_revision: row.metadata.attribution_revision,
                });
            }
        }
        names.sort_by_key(|name| name.voice);
        Ok(())
    }

    /// A separate, guarded naming refresh; it never enrolls a voice or fabricates a manual edit.
    pub fn apply_meeting_automatic_names(&self, id: &str) -> Result<VoiceNaming, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(id)?;
        let _sync = self
            .sync_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let store = self.meeting_store();
        let snapshot = store.read_transcript_snapshot(id)?;
        crate::meeting_utterances::assert_writable(&snapshot)?;
        let original_metadata = snapshot.metadata_digest.clone();
        let settings = self.settings();
        let original_policy = (
            settings.meeting_app_speakers,
            settings.meeting_app_name_policy,
        );
        let evidence_revision = || {
            serde_json::to_vec(
                &self
                    .voice_store()
                    .app_speaker_evidence(&self.meeting_key(id)),
            )
            .map(|bytes| crate::meeting_utterances::digest(&bytes))
            .map_err(PennelError::storage)
        };
        let original_evidence = evidence_revision()?;
        let before = snapshot.utterances;
        let previous_names = store.speaker_names(id);
        let mut names: Vec<_> = previous_names
            .iter()
            .filter(|name| !before.iter().any(|row| named_automatically(row, name)))
            .cloned()
            .collect();
        let mut after = before.clone();
        let prints = self
            .voice_store()
            .meeting_voiceprints(&self.meeting_key(id), Some(&snapshot.transcript_digest));
        self.prepare_meeting_automatic_names(id, &mut after, &mut names, &prints)?;
        if after != before || names != previous_names {
            let _policy = self.speaker_commit_policy_guard(original_policy)?;
            let current_settings = self.settings();
            if store.transcript_fingerprint(id)? != snapshot.transcript_digest
                || store.read_transcript_snapshot(id)?.metadata_digest != original_metadata
                || crate::speaker_jobs::speaker_names_digest(&store.speaker_names(id))?
                    != crate::speaker_jobs::speaker_names_digest(&previous_names)?
                || original_policy
                    != (
                        current_settings.meeting_app_speakers,
                        current_settings.meeting_app_name_policy,
                    )
                || original_evidence != evidence_revision()?
            {
                return Err(PennelError::Cancelled);
            }
            self.commit_speaker_utterances(id, &after, &names, prints, None)?;
        }
        Ok(self.meeting_voice_naming(id))
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

    fn clean_row(source: &str, voice: u32, start_ms: u64, end_ms: u64) -> MeetingUtterance {
        let mut row = MeetingUtterance::new(
            crate::Segment {
                start_ms,
                end_ms,
                speaker: crate::Speaker::Others,
                text: "Clear speech.".into(),
                failed: false,
                voice: Some(voice),
            },
            Some(source.into()),
        );
        row.metadata.attribution.kind = AttributionKind::Single;
        row.metadata.attribution.voices = vec![voice];
        row.metadata.attribution.turns = vec![crate::meeting_utterances::AcousticTurn {
            start_ms,
            end_ms,
            voice: Some(voice),
        }];
        row
    }

    fn automatic_fixture() -> (tempfile::TempDir, std::sync::Arc<PennelEngine>, String) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new(directory.path().join("data").to_string_lossy().into()).unwrap();
        let mut settings = engine.settings();
        settings.meetings_dir = directory.path().join("meetings").to_string_lossy().into();
        settings.meeting_app_speakers = true;
        settings.meeting_app_name_policy = MeetingAppNamePolicy::Automatic;
        engine.update_settings(settings).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Owned naming fixture", chrono::Local::now())
            .unwrap();
        store
            .prepare_recording(
                &id,
                crate::RecordingEnvironment {
                    app_version: "test".into(),
                    build: "test".into(),
                    platform: "macos".into(),
                    os_version: "test".into(),
                },
                crate::RecordingRecognition {
                    provider_kind: "local".into(),
                    model: "fixture".into(),
                    requested_language: None,
                    local_model_sha256: None,
                },
                false,
            )
            .unwrap();
        let rows = vec![
            clean_row("system_audio", 1, 0, 4_000),
            clean_row("system_audio", 1, 6_000, 10_000),
            clean_row("system_audio", 1, 12_000, 16_000),
            clean_row("microphone", 2, 0, 16_000),
        ];
        store
            .write_transcript_snapshot(&id, Some(0), &rows)
            .unwrap();
        let recording = store.recording_metadata(&id).unwrap().unwrap();
        engine
            .set_meeting_app_speaker_evidence(
                &id,
                AppSpeakerEvidence {
                    app_id: TEAMS.into(),
                    samples: sampled(0, 20_000, &["Ann"]),
                    participants: vec!["Ann".into()],
                    binding: Some(AppEvidenceBinding {
                        recording_uid: recording.meeting_uid,
                        recording_started_at: recording.started_at,
                        process_id: 42,
                        process_started_at_ms: 1,
                        window_id: "owned-window".into(),
                    }),
                    availability: AppEvidenceAvailability::Available,
                    ..AppSpeakerEvidence::default()
                },
            )
            .unwrap();
        (directory, engine, id)
    }

    #[test]
    fn physical_source_and_final_attribution_are_required_for_name_evidence() {
        let system = clean_row("system_audio", 1, 0, 5_000);
        let microphone = clean_row("microphone", 2, 0, 5_000);
        let room = clean_row("room_microphone", 3, 0, 5_000);
        let mut provisional = clean_row("system_audio", 4, 10_000, 15_000);
        provisional.metadata.attribution.provisional = true;
        let mut mixed = clean_row("system_audio", 5, 20_000, 25_000);
        mixed.metadata.attribution.kind = AttributionKind::Mixed;
        assert_eq!(
            clean_system_turns(&[system, microphone, room, provisional, mixed]),
            [turn(1, 0, 5_000)]
        );
    }

    #[test]
    fn automatic_names_require_three_independent_clean_turns_and_ten_seconds() {
        let samples = sampled(0, 30_000, &["Ann"]);
        assert!(automatic_suggestions(&[turn(1, 0, 20_000)], &samples).is_empty());
        let short = [
            turn(1, 0, 2_000),
            turn(1, 4_000, 6_000),
            turn(1, 8_000, 10_000),
        ];
        assert!(automatic_suggestions(&short, &samples).is_empty());
        let enough = [
            turn(1, 0, 4_000),
            turn(1, 6_000, 10_000),
            turn(1, 12_000, 16_000),
        ];
        assert_eq!(automatic_suggestions(&enough, &samples)[0].name, "Ann");
        let split_one_turn = [
            clean_row("system_audio", 1, 0, 4_000),
            clean_row("system_audio", 1, 4_000, 8_000),
            clean_row("system_audio", 1, 8_000, 12_000),
        ];
        assert!(automatic_suggestions(&clean_system_turns(&split_one_turn), &samples).is_empty());
    }

    #[test]
    fn automatic_names_reject_conflicting_or_low_coverage_observations() {
        let turns = [
            turn(1, 0, 4_000),
            turn(1, 6_000, 10_000),
            turn(1, 12_000, 16_000),
        ];
        let mut conflicting = sampled(0, 8_000, &["Ann"]);
        conflicting.extend(sampled(8_000, 20_000, &["Bert"]));
        assert!(automatic_suggestions(&turns, &conflicting).is_empty());
        assert!(automatic_suggestions(&turns, &sampled(0, 4_000, &["Ann"])).is_empty());
    }

    #[test]
    fn automatic_application_is_guarded_portable_and_never_a_manual_correction() {
        let (_directory, engine, id) = automatic_fixture();
        assert_eq!(
            engine.meeting_voice_naming(&id).status,
            VoiceNamingStatus::AutomaticReady
        );
        let result = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(result.status, VoiceNamingStatus::Applied);
        assert_eq!(
            engine.meeting_speakers(id.clone()),
            [SpeakerName {
                voice: 1,
                name: "Ann".into()
            }]
        );
        let store = engine.meeting_store();
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            snapshot.metadata_status,
            crate::TranscriptMetadataStatus::Valid
        );
        assert!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
                .all(|row| row.metadata.automatic_name.is_some() && row.metadata.manual.is_none())
        );
        assert!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.metadata.source_id.as_deref() == Some("microphone"))
                .all(|row| row.metadata.automatic_name.is_none())
        );
        assert!(engine.known_voices().is_empty());
        assert!(engine.voice_store().profile().is_none());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(store.dir_of(&id).join("transcript.meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schema_version"], 5);
        let first_revision = snapshot.revision;
        engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().revision,
            first_revision
        );
        store.rename_speaker(&id, 1, "Owner corrected").unwrap();
        engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(store.speaker_names(&id)[0].name, "Owner corrected");
        assert!(
            store
                .read_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .iter()
                .all(|row| row.metadata.automatic_name.is_none())
        );
    }

    #[test]
    fn stale_calls_duplicate_names_and_source_reuse_never_apply_names() {
        let (_directory, engine, id) = automatic_fixture();
        let key = engine.meeting_key(&id);
        let mut evidence = engine.voice_store().app_speaker_evidence(&key).unwrap();
        evidence.binding.as_mut().unwrap().recording_uid = uuid::Uuid::now_v7().to_string();
        engine
            .set_meeting_app_speaker_evidence(&id, evidence.clone())
            .unwrap();
        assert_eq!(
            engine.meeting_voice_naming(&id).status,
            VoiceNamingStatus::Unbound
        );
        assert!(
            engine
                .apply_meeting_automatic_names(&id)
                .unwrap()
                .automatic_names
                .is_empty()
        );
        evidence.duplicate_names = true;
        engine
            .set_meeting_app_speaker_evidence(&id, evidence)
            .unwrap();
        assert_eq!(
            engine.meeting_voice_naming(&id).status,
            VoiceNamingStatus::Conflict
        );
        assert!(engine.meeting_speakers(id.clone()).is_empty());

        let (_other, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        let microphone = snapshot
            .utterances
            .iter_mut()
            .find(|row| row.metadata.source_id.as_deref() == Some("microphone"))
            .unwrap();
        microphone.segment.voice = Some(1);
        microphone.metadata.attribution.voices = vec![1];
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        assert!(
            engine
                .apply_meeting_automatic_names(&id)
                .unwrap()
                .automatic_names
                .is_empty()
        );
        assert!(store.speaker_names(&id).is_empty());
    }

    #[test]
    fn unbound_historical_evidence_is_suggestion_only() {
        let (_directory, engine, id) = automatic_fixture();
        let mut evidence = engine
            .voice_store()
            .app_speaker_evidence(&engine.meeting_key(&id))
            .unwrap();
        evidence.binding = None;
        engine
            .set_meeting_app_speaker_evidence(&id, evidence)
            .unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(naming.status, VoiceNamingStatus::Unbound);
        assert!(!naming.suggestions.is_empty());
        assert!(naming.automatic_names.is_empty());
        assert!(engine.meeting_speakers(id).is_empty());
    }

    #[test]
    fn an_already_named_shared_endpoint_blocks_automatic_person_identity() {
        let (_directory, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        snapshot
            .utterances
            .push(clean_row("system_audio", 3, 18_000, 19_000));
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        store.rename_speaker(&id, 3, "Ann").unwrap();
        engine
            .voice_store()
            .set_meeting_voiceprints(
                &engine.meeting_key(&id),
                vec![
                    MeetingVoiceprint {
                        voice: 1,
                        embedding: vec![1.0, 0.0],
                        known_id: None,
                    },
                    MeetingVoiceprint {
                        voice: 3,
                        embedding: vec![0.0, 1.0],
                        known_id: None,
                    },
                ],
                Some(&store.transcript_fingerprint(&id).unwrap()),
            )
            .unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(naming.status, VoiceNamingStatus::Conflict);
        assert!(naming.automatic_names.is_empty());
        assert_eq!(
            store.speaker_names(&id),
            [SpeakerName {
                voice: 3,
                name: "Ann".into()
            }]
        );
    }

    #[test]
    fn lost_permissions_and_suggestion_policy_never_apply_automatic_names() {
        let (_directory, engine, id) = automatic_fixture();
        let mut settings = engine.settings();
        settings.meeting_app_name_policy = MeetingAppNamePolicy::Suggest;
        engine.update_settings(settings).unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(naming.status, VoiceNamingStatus::Suggested);
        assert!(naming.automatic_names.is_empty());
        assert!(engine.meeting_speakers(id.clone()).is_empty());
        let mut settings = engine.settings();
        settings.meeting_app_name_policy = MeetingAppNamePolicy::Automatic;
        engine.update_settings(settings).unwrap();
        let mut evidence = engine
            .voice_store()
            .app_speaker_evidence(&engine.meeting_key(&id))
            .unwrap();
        evidence.availability = AppEvidenceAvailability::PermissionDenied;
        evidence.unavailable_reason = Some("accessibility_permission_denied".into());
        engine
            .set_meeting_app_speaker_evidence(&id, evidence)
            .unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(naming.status, VoiceNamingStatus::PermissionDenied);
        assert!(naming.automatic_names.is_empty());
        assert!(engine.meeting_speakers(id).is_empty());
    }

    #[test]
    fn retained_manual_work_requiring_review_is_not_overwritten_by_an_app_name() {
        let (_directory, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        let row = &mut snapshot.utterances[0];
        row.metadata.manual = Some(crate::ManualCorrection {
            name: Some("Retained correction".into()),
            voice: Some(1),
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: true,
            self_voice: false,
        });
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert_eq!(naming.status, VoiceNamingStatus::Conflict);
        assert!(naming.automatic_names.is_empty());
        assert!(store.speaker_names(&id).is_empty());
        assert!(
            store.read_transcript_snapshot(&id).unwrap().utterances[0]
                .metadata
                .manual
                .is_some()
        );
    }

    #[test]
    fn automatic_names_never_count_unspoken_parts_of_coarse_rows() {
        let mut rows = vec![
            clean_row("system_audio", 1, 0, 15_000),
            clean_row("system_audio", 1, 30_000, 45_000),
            clean_row("system_audio", 1, 60_000, 75_000),
        ];
        for row in &mut rows {
            row.metadata.attribution.turns[0].start_ms = row.segment.end_ms - 1_000;
        }
        let turns = clean_system_turns(&rows);
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn.end_ms - turn.start_ms)
                .sum::<u64>(),
            3_000
        );
        assert!(automatic_suggestions(&turns, &sampled(0, 80_000, &["Ann"])).is_empty());

        let (_directory, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        for row in snapshot
            .utterances
            .iter_mut()
            .filter(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
        {
            row.metadata.attribution.turns[0].start_ms = row.segment.end_ms - 1_000;
        }
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        assert!(
            engine
                .apply_meeting_automatic_names(&id)
                .unwrap()
                .automatic_names
                .is_empty()
        );
        assert!(store.speaker_names(&id).is_empty());
    }

    #[test]
    fn automatic_acoustic_evidence_requires_real_turns_and_clips_to_the_owned_row() {
        let mut missing = clean_row("system_audio", 1, 0, 15_000);
        missing.metadata.attribution.turns.clear();
        assert!(clean_system_turns(&[missing]).is_empty());
        let mut first = clean_row("system_audio", 1, 0, 5_000);
        first.metadata.attribution.turns[0].start_ms = 1_000;
        first.metadata.attribution.turns[0].end_ms = 4_000;
        let mut second = clean_row("system_audio", 1, 5_000, 10_000);
        second.metadata.attribution.turns[0].start_ms = 3_000;
        second.metadata.attribution.turns[0].end_ms = 6_000;
        assert_eq!(
            clean_system_turns(&[first, second]),
            [turn(1, 1_000, 4_000), turn(1, 5_000, 6_000)]
        );
    }

    #[test]
    fn automatic_acoustic_evidence_unions_duplicates_and_subtracts_other_or_unknown_voices() {
        let first = clean_row("system_audio", 1, 0, 15_000);
        let duplicate = first.clone();
        let mut overlap = clean_row("system_audio", 2, 4_000, 8_000);
        overlap.segment.voice = None;
        overlap.metadata.attribution.kind = AttributionKind::Overlap;
        overlap.metadata.attribution.voices = vec![1, 2];
        let mut unknown = clean_row("system_audio", 3, 9_000, 10_000);
        unknown.segment.voice = None;
        unknown.metadata.attribution.kind = AttributionKind::Unknown;
        unknown.metadata.attribution.voices.clear();
        unknown.metadata.attribution.turns[0].voice = None;
        assert_eq!(
            clean_system_turns(&[first, duplicate, overlap, unknown]),
            [
                turn(1, 0, 4_000),
                turn(1, 8_000, 9_000),
                turn(1, 10_000, 15_000)
            ]
        );
    }

    #[test]
    fn automatic_names_do_not_treat_shared_asr_lineage_as_independent_turns() {
        let (_directory, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        let source = uuid::Uuid::now_v7().to_string();
        for row in snapshot
            .utterances
            .iter_mut()
            .filter(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
        {
            row.metadata.lineage = vec![source.clone()];
        }
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        assert!(engine.meeting_voice_naming(&id).automatic_names.is_empty());
    }

    #[test]
    fn short_gaps_do_not_add_evidence_or_manufacture_independent_turns() {
        let rows = [
            clean_row("system_audio", 1, 0, 4_000),
            clean_row("system_audio", 1, 4_100, 8_100),
            clean_row("system_audio", 1, 8_200, 12_200),
        ];
        let turns = clean_system_turns(&rows);
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn.end_ms - turn.start_ms)
                .sum::<u64>(),
            12_000
        );
        assert!(automatic_suggestions(&turns, &sampled(0, 20_000, &["Ann"])).is_empty());
    }

    #[test]
    fn legacy_rows_without_acoustic_turns_can_only_offer_reviewable_suggestions() {
        let (_directory, engine, id) = automatic_fixture();
        let store = engine.meeting_store();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        for row in &mut snapshot.utterances {
            row.metadata.attribution.turns.clear();
        }
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
            .unwrap();
        let naming = engine.apply_meeting_automatic_names(&id).unwrap();
        assert!(!naming.suggestions.is_empty());
        assert!(naming.automatic_names.is_empty());
        assert!(store.speaker_names(&id).is_empty());
    }
}
