//! Reconcile clean local windows without inheriting native speaker-centroid errors.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Diarization, Speaker, Turn, cosine, normalized, renumber_labels, speech_ms, voiceprint_spans,
    weighted_mean,
};

const WINDOW_MS: u64 = 5_000;
const EDGE_MS: u64 = 100;
const MIN_WINDOW_MS: u64 = 1_500;
const MIN_CLUSTER_MS: u64 = 10_000;
const AVERAGE_SIMILARITY: f32 = 0.55;
const WEAK_MATCH_SIMILARITY: f32 = 0.65;
const WEAK_MATCH_MARGIN: f32 = 0.15;
const OVERLAPPING_ALIAS_SIMILARITY: f32 = 0.70;
const MAX_EMBEDDED_WINDOWS: usize = 1_024;

struct Window {
    start_ms: u64,
    end_ms: u64,
    turn: usize,
    embedding: Vec<f32>,
}

impl Window {
    fn duration(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

fn windows(turns: &[Turn]) -> Vec<Window> {
    let speakers: BTreeSet<_> = turns.iter().map(|turn| turn.speaker).collect();
    let mut windows = Vec::new();
    for speaker in speakers {
        for (start, end) in voiceprint_spans(turns, speaker, u64::MAX) {
            let mut cursor = start;
            while cursor < end {
                let Some((turn, original)) = turns
                    .iter()
                    .enumerate()
                    .filter(|(_, turn)| {
                        turn.speaker == speaker && turn.start_ms <= cursor && turn.end_ms > cursor
                    })
                    .min_by_key(|(_, turn)| (turn.end_ms, turn.start_ms))
                else {
                    break;
                };
                // A union of adjacent/overlapping same-voice turns is clean, but windows still
                // need one original turn for vote lineage. Partition the union without reusing PCM.
                let section_end = end.min(original.end_ms);
                let mut at = cursor.saturating_add(EDGE_MS);
                while section_end.saturating_sub(at) >= MIN_WINDOW_MS + 2 * EDGE_MS {
                    let stop = at.saturating_add(WINDOW_MS).min(section_end - EDGE_MS);
                    windows.push(Window {
                        start_ms: at,
                        end_ms: stop,
                        turn,
                        embedding: Vec::new(),
                    });
                    at = stop;
                }
                cursor = section_end;
            }
        }
    }
    windows
}

fn overlapping_pairs(turns: &[Turn]) -> BTreeMap<(u32, u32), Vec<(u64, u64)>> {
    let mut pairs: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (index, left) in turns.iter().enumerate() {
        for right in &turns[index + 1..] {
            let start = left.start_ms.max(right.start_ms);
            let end = left.end_ms.min(right.end_ms);
            if left.speaker != right.speaker && end > start {
                pairs
                    .entry((
                        left.speaker.min(right.speaker),
                        left.speaker.max(right.speaker),
                    ))
                    .or_default()
                    .push((start, end));
            }
        }
    }
    pairs
}

fn cannot_link(turns: &[Turn], windows: &[Window]) -> BTreeSet<(u32, u32)> {
    let mut raw: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, window) in windows
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.embedding.is_empty())
    {
        raw.entry(turns[window.turn].speaker)
            .or_default()
            .push(index);
    }
    let prints: Vec<_> = raw
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().map(|i| windows[*i].duration()).sum::<u64>() >= 3_000
        })
        .map(|(speaker, members)| {
            let ms = members.iter().map(|i| windows[*i].duration()).sum::<u64>();
            let independent = members
                .iter()
                .map(|i| windows[*i].turn)
                .collect::<BTreeSet<_>>()
                .len()
                >= 2;
            (
                *speaker,
                weighted_mean(members.iter().map(|i| {
                    (
                        windows[*i].embedding.as_slice(),
                        windows[*i].duration() as f32,
                    )
                })),
                independent || ms >= MIN_CLUSTER_MS,
            )
        })
        .collect();
    let overlaps = overlapping_pairs(turns);
    let mut blocked = BTreeSet::new();
    for (index, (left, left_print, left_endpoint)) in prints.iter().enumerate() {
        for (right, right_print, right_endpoint) in &prints[index + 1..] {
            let pair = ((*left).min(*right), (*left).max(*right));
            let similarity = cosine(left_print, right_print);
            // Independent clean endpoints remain apart even when many intermediate windows
            // dominate average linkage. Raw means only restrict merges; they never grant identity.
            // An overlapping short monologue can veto a false merge without gaining identity
            // authority or the stronger non-overlap endpoint authority.
            if (*left_endpoint && *right_endpoint && similarity < AVERAGE_SIMILARITY)
                || (similarity < OVERLAPPING_ALIAS_SIMILARITY
                    && overlaps
                        .get(&pair)
                        .is_some_and(|spans| spans.iter().any(|(a, b)| b - a >= 300)))
            {
                blocked.insert(pair);
            }
        }
    }
    blocked
}

fn clusters(turns: &[Turn], windows: &[Window]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = windows
        .iter()
        .enumerate()
        .filter_map(|(index, w)| (!w.embedding.is_empty()).then_some(vec![index]))
        .collect();
    let blocked = cannot_link(turns, windows);
    let mut links = vec![vec![0f32; groups.len()]; groups.len()];
    for a in 0..groups.len() {
        for b in a + 1..groups.len() {
            let left = &windows[groups[a][0]];
            let right = &windows[groups[b][0]];
            let ids = (
                turns[left.turn].speaker.min(turns[right.turn].speaker),
                turns[left.turn].speaker.max(turns[right.turn].speaker),
            );
            let similarity = if blocked.contains(&ids) {
                f32::NEG_INFINITY
            } else {
                cosine(&left.embedding, &right.embedding)
            };
            links[a][b] = similarity;
            links[b][a] = similarity;
        }
    }
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for (a, row) in links.iter().enumerate() {
            for (b, &score) in row.iter().enumerate().skip(a + 1) {
                if score >= AVERAGE_SIMILARITY
                    && best.is_none_or(|(_, _, previous)| score > previous)
                {
                    best = Some((a, b, score));
                }
            }
        }
        let Some((a, b, _)) = best else { break };
        let left = groups[a].len() as f32;
        let right = groups[b].len() as f32;
        let updates: Vec<_> = links[a]
            .iter()
            .zip(&links[b])
            .map(|(a, b)| (a * left + b * right) / (left + right))
            .collect();
        for (other, score) in updates.into_iter().enumerate() {
            if other == a || other == b {
                continue;
            }
            // A cannot-link survives every average-link update; an intermediate voice cannot erase it.
            links[a][other] = score;
            links[other][a] = score;
        }
        links.remove(b);
        for row in &mut links {
            row.remove(b);
        }
        let members = groups.remove(b);
        groups[a].extend(members);
    }
    groups
}

fn assignments(
    turns: &[Turn],
    windows: &[Window],
    groups: &[Vec<usize>],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let reliable: Vec<_> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, members)| {
            (members.iter().map(|i| windows[*i].duration()).sum::<u64>() >= MIN_CLUSTER_MS)
                .then_some(index)
        })
        .collect();
    let centroids: Vec<_> = groups
        .iter()
        .map(|members| {
            weighted_mean(members.iter().map(|i| {
                (
                    windows[*i].embedding.as_slice(),
                    windows[*i].duration() as f32,
                )
            }))
        })
        .collect();
    let blocked = cannot_link(turns, windows);
    let raw_members: Vec<BTreeSet<u32>> = groups
        .iter()
        .map(|members| {
            members
                .iter()
                .map(|i| turns[windows[*i].turn].speaker)
                .collect()
        })
        .collect();
    let mut strong = vec![false; windows.len()];
    let mut point_groups = vec![None; windows.len()];
    for (group, members) in groups.iter().enumerate() {
        for &index in members {
            point_groups[index] = if reliable.contains(&group) {
                strong[index] = true;
                Some(group)
            } else {
                let mut nearest: Vec<_> = reliable
                    .iter()
                    .map(|id| (*id, cosine(&windows[index].embedding, &centroids[*id])))
                    .collect();
                nearest.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                nearest.first().and_then(|&(id, similarity)| {
                    let source = turns[windows[index].turn].speaker;
                    let allowed = raw_members[id]
                        .iter()
                        .all(|other| !blocked.contains(&(source.min(*other), source.max(*other))));
                    // Rank every reliable voice before applying constraints. A blocked nearest
                    // voice must not make a weaker runner-up look uniquely supported.
                    (allowed
                        && similarity >= WEAK_MATCH_SIMILARITY
                        && nearest.get(1).is_none_or(|(_, runner_up)| {
                            similarity - *runner_up >= WEAK_MATCH_MARGIN
                        }))
                    .then_some(id)
                })
            };
        }
    }
    // Independently weak groups can each match a strong centroid while contradicting one
    // another. Reject both weak expansions together, without order-dependent transitive merges.
    let mut proposed = BTreeMap::<(usize, u32), Vec<usize>>::new();
    for (index, group) in point_groups.iter().enumerate() {
        if let Some(group) = group {
            proposed
                .entry((*group, turns[windows[index].turn].speaker))
                .or_default()
                .push(index);
        }
    }
    let mut rejected = BTreeSet::new();
    for &(left, right) in &blocked {
        for &group in &reliable {
            if let (Some(a), Some(b)) =
                (proposed.get(&(group, left)), proposed.get(&(group, right)))
            {
                rejected.extend(a.iter().chain(b).copied().filter(|i| !strong[*i]));
            }
        }
    }
    for index in rejected {
        point_groups[index] = None;
    }
    let mut assignments = Vec::with_capacity(turns.len());
    for index in 0..turns.len() {
        let own: Vec<_> = windows
            .iter()
            .enumerate()
            .filter(|(_, window)| window.turn == index)
            .collect();
        let observed = own.iter().map(|(_, window)| window.duration()).sum::<u64>();
        let mut votes = BTreeMap::<usize, u64>::new();
        for (index, window) in own {
            if let Some(group) = point_groups[index] {
                *votes.entry(group).or_default() += window.duration();
            }
        }
        let best = votes
            .into_iter()
            .max_by(|(a, aw), (b, bw)| aw.cmp(bw).then(b.cmp(a)));
        // Invalid, unembedded and ambiguous windows still count as observed evidence.
        assignments.push(
            best.filter(|(_, support)| u128::from(*support) * 5 >= u128::from(observed) * 4)
                .map(|(id, _)| id),
        );
    }
    (assignments, point_groups)
}

fn overlap_masks(
    turns: &[Turn],
    assignments: &[Option<usize>],
) -> (Vec<Vec<(u64, u64)>>, BTreeSet<usize>) {
    let mut masks = vec![Vec::new(); turns.len()];
    let mut pairs: BTreeMap<(usize, u32, u32), Vec<(u64, u64)>> = BTreeMap::new();
    for (index, left) in turns.iter().enumerate() {
        let Some(group) = assignments[index] else {
            continue;
        };
        for (other, right) in turns.iter().enumerate().skip(index + 1) {
            if left.speaker == right.speaker || assignments[other] != Some(group) {
                continue;
            }
            let start = left.start_ms.max(right.start_ms);
            let end = left.end_ms.min(right.end_ms);
            if end > start {
                masks[index].push((start, end));
                masks[other].push((start, end));
                pairs
                    .entry((
                        group,
                        left.speaker.min(right.speaker),
                        left.speaker.max(right.speaker),
                    ))
                    .or_default()
                    .push((start, end));
            }
        }
    }
    let mut conflicted = BTreeSet::new();
    for ((group, _, _), spans) in pairs {
        let spans = crate::union_spans(spans);
        // Strong acoustic similarity cannot prove that sustained or repeated simultaneous voices are aliases.
        if spans.iter().any(|(start, end)| end - start >= 2_000)
            || spans
                .iter()
                .filter(|(start, end)| end - start >= 1_000)
                .count()
                >= 2
        {
            conflicted.insert(group);
        }
    }
    (masks, conflicted)
}

pub(crate) fn reconcile<E>(
    turns: &[Turn],
    mut embed: impl FnMut(u64, u64) -> Result<Vec<f32>, E>,
) -> Result<Diarization, E> {
    let mut windows = windows(turns);
    let selected = windows.len().min(MAX_EMBEDDED_WINDOWS);
    for index in 0..selected {
        let chosen = if selected == 1 {
            0
        } else {
            index * (windows.len() - 1) / (selected - 1)
        };
        let window = &mut windows[chosen];
        window.embedding = normalized(&embed(window.start_ms, window.end_ms)?);
    }
    let groups = clusters(turns, &windows);
    let (mut assigned, point_groups) = assignments(turns, &windows, &groups);
    let (masks, conflicted) = overlap_masks(turns, &assigned);
    for group in &mut assigned {
        if group.is_some_and(|id| conflicted.contains(&id)) {
            *group = None;
        }
    }
    let mut prints = Vec::with_capacity(groups.len());
    for group in 0..groups.len() {
        let members: Vec<_> = windows
            .iter()
            .enumerate()
            .filter(|(i, w)| assigned[w.turn] == Some(group) && point_groups[*i] == Some(group))
            .collect();
        let ms = members.iter().map(|(_, w)| w.duration()).sum::<u64>();
        prints.push(if ms >= MIN_CLUSTER_MS {
            weighted_mean(
                members
                    .iter()
                    .map(|(_, w)| (w.embedding.as_slice(), w.duration() as f32)),
            )
        } else {
            Vec::new()
        });
    }
    for group in &mut assigned {
        if group.is_some_and(|id| prints[id].is_empty()) {
            *group = None;
        }
    }
    let mut output = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        let label = assigned[index].map_or(-i64::from(turn.speaker) - 1, |id| id as i64);
        let mut at = turn.start_ms;
        for (start, end) in crate::union_spans(masks[index].clone()) {
            if at < start {
                output.push((at, start, label));
            }
            output.push((start, end, -i64::from(turn.speaker) - 1));
            at = end;
        }
        if at < turn.end_ms {
            output.push((at, turn.end_ms, label));
        }
    }
    let (turns, order) = renumber_labels(output);
    let speakers = order
        .iter()
        .enumerate()
        .map(|(index, label)| Speaker {
            speaker: index as u32,
            speech_ms: speech_ms(&turns, index as u32),
            embedding: if *label >= 0 {
                prints[*label as usize].clone()
            } else {
                Vec::new()
            },
        })
        .collect();
    Ok(Diarization { turns, speakers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cosine, normalized};
    use std::convert::Infallible;

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }
    fn direction(index: usize) -> Vec<f32> {
        let mut vector = vec![0.; 8];
        vector[index] = 1.;
        vector
    }
    fn replay(turns: &[Turn], voices: &[(u32, Vec<f32>)]) -> Diarization {
        reconcile(turns, |start, end| {
            let speaker = turns
                .iter()
                .find(|turn| turn.start_ms <= start && turn.end_ms >= end)
                .unwrap()
                .speaker;
            Ok::<_, Infallible>(
                voices
                    .iter()
                    .find(|(id, _)| *id == speaker)
                    .map_or_else(Vec::new, |(_, v)| v.clone()),
            )
        })
        .unwrap()
    }
    fn qualified(result: &Diarization) -> usize {
        result
            .speakers
            .iter()
            .filter(|speaker| !speaker.embedding.is_empty())
            .count()
    }
    fn identity(result: &Diarization, start: u64) -> u32 {
        result
            .turns
            .iter()
            .find(|turn| turn.start_ms == start)
            .unwrap()
            .speaker
    }
    fn qualified_turn(result: &Diarization, start: u64) -> bool {
        let id = identity(result, start);
        result
            .speakers
            .iter()
            .find(|s| s.speaker == id)
            .is_some_and(|s| !s.embedding.is_empty())
    }
    fn times(result: &Diarization) -> Vec<(u64, u64)> {
        result
            .turns
            .iter()
            .map(|t| (t.start_ms, t.end_ms))
            .collect()
    }
    fn repeated() -> Vec<Turn> {
        vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 1),
            turn(14000, 20000, 2),
            turn(21000, 27000, 0),
            turn(28000, 34000, 1),
            turn(35000, 41000, 2),
        ]
    }

    #[test]
    fn repeated_fragments_join_without_inheriting_contaminated_native_centroids() {
        let raw = repeated();
        let result = replay(
            &raw,
            &[
                (0, direction(0)),
                (1, direction(1)),
                (2, normalized(&[0.95, 0.05, 0., 0., 0., 0., 0., 0.])),
            ],
        );
        assert_eq!(qualified(&result), 2);
        assert_eq!(identity(&result, 0), identity(&result, 14000));
        assert_ne!(identity(&result, 0), identity(&result, 7000));
        assert_eq!(
            times(&result),
            raw.iter()
                .map(|t| (t.start_ms, t.end_ms))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn similar_distinct_voices_do_not_require_a_fixed_speaker_count() {
        let raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 1),
            turn(14000, 20000, 0),
            turn(21000, 27000, 1),
        ];
        let other = normalized(&[0.54, (1f32 - 0.54 * 0.54).sqrt(), 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, other)]);
        assert_eq!(qualified(&result), 2);
        assert_ne!(identity(&result, 0), identity(&result, 7000));
    }
    #[test]
    fn an_intermediate_bridge_does_not_join_distinct_endpoints() {
        let mut raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 0),
            turn(14000, 20000, 1),
            turn(21000, 27000, 1),
        ];
        raw.push(turn(28000, 34000, 2));
        let bridge = normalized(&[1., 1., 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, direction(1)), (2, bridge)]);
        assert_eq!(qualified(&result), 2);
        assert_ne!(identity(&result, 0), identity(&result, 14000));
    }
    #[test]
    fn acoustic_evidence_preserves_real_overlap_between_similar_voices() {
        let raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 1),
            turn(14000, 20000, 0),
            turn(21000, 27000, 1),
            turn(28000, 34000, 0),
            turn(33000, 39000, 1),
        ];
        let other = normalized(&[0.65, (1f32 - 0.65 * 0.65).sqrt(), 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, other)]);
        assert_eq!(qualified(&result), 2);
        assert_ne!(identity(&result, 28000), identity(&result, 33000));
        assert_eq!(result.turns.len(), raw.len());
    }
    #[test]
    fn raw_overlap_alone_does_not_block_strong_same_voice_aliases() {
        let raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 1),
            turn(14000, 20000, 0),
            turn(21000, 27000, 1),
            turn(28000, 34000, 0),
            turn(33500, 39500, 1),
        ];
        let result = replay(
            &raw,
            &[
                (0, direction(0)),
                (1, normalized(&[0.95, 0.05, 0., 0., 0., 0., 0., 0.])),
            ],
        );
        assert_eq!(qualified(&result), 1);
        assert_eq!(identity(&result, 0), identity(&result, 7000));
        let ambiguous: Vec<_> = result
            .turns
            .iter()
            .filter(|turn| turn.start_ms == 33500 && turn.end_ms == 34000)
            .collect();
        assert_eq!(ambiguous.len(), 2);
        assert_ne!(ambiguous[0].speaker, ambiguous[1].speaker);
        assert!(
            ambiguous
                .iter()
                .all(|turn| result.speakers[turn.speaker as usize].embedding.is_empty())
        );
        assert_eq!(
            result
                .turns
                .iter()
                .map(|t| t.end_ms - t.start_ms)
                .sum::<u64>(),
            raw.iter().map(|t| t.end_ms - t.start_ms).sum::<u64>()
        );
    }
    #[test]
    fn empty_nonfinite_and_incoherent_outliers_remain_unqualified() {
        let raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 0),
            turn(14000, 20000, 1),
            turn(21000, 27000, 2),
            turn(28000, 34000, 3),
        ];
        let result = replay(
            &raw,
            &[
                (0, vec![0.; 8]),
                (1, vec![f32::NAN; 8]),
                (2, direction(2)),
                (3, direction(3)),
            ],
        );
        assert_eq!(qualified(&result), 0);
        assert_eq!(result.turns.len(), raw.len());
    }
    #[test]
    fn abstained_windows_are_in_the_turn_agreement_denominator() {
        let raw = vec![turn(0, 12000, 0), turn(14000, 30000, 1)];
        let result = reconcile(&raw, |start, _| {
            Ok::<_, Infallible>(if start < 20000 {
                direction(0)
            } else {
                Vec::new()
            })
        })
        .unwrap();
        assert!(qualified_turn(&result, 0));
        assert!(!qualified_turn(&result, 14000));
        assert_eq!(result.turns.len(), 2);
    }
    #[test]
    fn a_mixed_raw_turn_does_not_acquire_a_single_identity() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 26000, 1),
            turn(28000, 44000, 2),
        ];
        let result = reconcile(&raw, |start, _| {
            Ok::<_, Infallible>(if start < 12000 || (28000..33000).contains(&start) {
                direction(0)
            } else {
                direction(1)
            })
        })
        .unwrap();
        assert_eq!(qualified(&result), 2);
        assert!(!qualified_turn(&result, 28000));
        assert_eq!(
            times(&result),
            vec![(0, 12000), (14000, 26000), (28000, 44000)]
        );
    }
    #[test]
    fn short_live_windows_keep_speech_without_admitting_a_voice() {
        let raw = vec![turn(0, 600, 0), turn(800, 1400, 1), turn(1600, 8000, 2)];
        let result = replay(
            &raw,
            &[(0, direction(0)), (1, direction(1)), (2, direction(2))],
        );
        assert_eq!(qualified(&result), 0);
        assert_eq!(
            times(&result),
            raw.iter()
                .map(|t| (t.start_ms, t.end_ms))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn a_long_clean_monologue_has_a_normalized_compatible_voiceprint() {
        let result = replay(&[turn(0, 12000, 7)], &[(7, direction(0))]);
        assert_eq!(qualified(&result), 1);
        assert_eq!(result.speakers[0].embedding.len(), 8);
        assert!(cosine(&result.speakers[0].embedding, &direction(0)) > 0.999);
    }
    #[test]
    fn more_than_four_actual_voices_are_preserved() {
        let mut raw = Vec::new();
        let voices: Vec<_> = (0..7).map(|id| (id, direction(id as usize))).collect();
        for pass in 0..2 {
            for id in 0..7 {
                let at = (pass * 7 + id) as u64 * 7000;
                raw.push(turn(at, at + 6000, id));
            }
        }
        let result = replay(&raw, &voices);
        assert_eq!(qualified(&result), 7);
        assert_eq!(result.speakers.len(), 7);
    }
    #[test]
    fn output_ids_follow_first_appearance_and_are_repeatable() {
        let raw = repeated();
        let voices = [(0, direction(0)), (1, direction(1)), (2, direction(2))];
        let first = replay(&raw, &voices);
        let mut reversed = raw.clone();
        reversed.reverse();
        let second = replay(&reversed, &voices);
        assert_eq!(first, second);
        assert_eq!(first.turns[0].speaker, 0);
        assert_eq!(first.turns[1].speaker, 1);
        assert_eq!(first.turns[2].speaker, 2);
    }
    #[test]
    fn no_turns_or_no_clean_evidence_never_invents_identity() {
        assert_eq!(replay(&[], &[]), Diarization::default());
        let raw = vec![turn(0, 30000, 0), turn(0, 30000, 1)];
        let result = replay(&raw, &[(0, direction(0)), (1, direction(1))]);
        assert_eq!(qualified(&result), 0);
        assert_eq!(result.turns, raw);
    }
    #[test]
    fn embedding_errors_are_reported_without_an_identity_fallback() {
        let result = reconcile(&[turn(0, 12000, 0)], |_, _| Err::<Vec<f32>, _>("failed"));
        assert_eq!(result, Err("failed"));
    }
    #[test]
    fn highly_similar_sustained_overlap_rejects_confident_collapsed_identity() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 26000, 1),
            turn(28000, 40000, 0),
            turn(37000, 49000, 1),
        ];
        for second in [
            direction(0),
            normalized(&[0.95, 0.05, 0., 0., 0., 0., 0., 0.]),
        ] {
            let result = replay(&raw, &[(0, direction(0)), (1, second)]);
            assert_eq!(qualified(&result), 0);
            assert!(
                result
                    .turns
                    .iter()
                    .all(|t| result.speakers[t.speaker as usize].embedding.is_empty())
            );
            assert_eq!(
                result
                    .turns
                    .iter()
                    .map(|t| t.end_ms - t.start_ms)
                    .sum::<u64>(),
                raw.iter().map(|t| t.end_ms - t.start_ms).sum::<u64>()
            );
        }
    }
    #[test]
    fn repeated_substantial_self_conflicts_do_not_authorize_a_shared_identity() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 26000, 1),
            turn(28000, 40000, 0),
            turn(39000, 51000, 1),
            turn(53000, 65000, 0),
            turn(64000, 76000, 1),
        ];
        let result = replay(&raw, &[(0, direction(0)), (1, direction(0))]);
        assert_eq!(qualified(&result), 0);
    }
    #[test]
    fn clean_windows_trim_edges_and_never_embed_raw_overlap() {
        let raw = vec![turn(0, 12000, 0), turn(10000, 22000, 1)];
        let mut called = 0;
        reconcile(&raw, |start, end| {
            called += 1;
            assert!(end - start >= MIN_WINDOW_MS && end - start <= WINDOW_MS);
            assert!(!(start < 12000 && end > 10000));
            assert!(start >= EDGE_MS && end <= 22000 - EDGE_MS);
            Ok::<_, Infallible>(direction(0))
        })
        .unwrap();
        assert!(called > 0);
    }
    #[test]
    fn embedding_budget_abstains_without_dropping_unprocessed_speech() {
        let raw: Vec<_> = (0..1100)
            .map(|index| turn(index * 7000, index * 7000 + 6000, 0))
            .collect();
        let mut called = 0;
        let result = reconcile(&raw, |_, _| {
            called += 1;
            Ok::<_, Infallible>(Vec::new())
        })
        .unwrap();
        assert_eq!(called, MAX_EMBEDDED_WINDOWS);
        assert_eq!(qualified(&result), 0);
        assert_eq!(
            times(&result),
            raw.iter()
                .map(|t| (t.start_ms, t.end_ms))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn weak_reassignment_keeps_a_hard_raw_cannot_link_outside_the_overlap() {
        let raw = vec![
            turn(0, 6000, 0),
            turn(7000, 13000, 0),
            turn(20000, 23000, 1),
            turn(28000, 31000, 1),
            turn(30500, 36500, 0),
        ];
        let other = normalized(&[0.68, (1f32 - 0.68 * 0.68).sqrt(), 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, other)]);
        assert!(qualified_turn(&result, 0));
        assert!(!qualified_turn(&result, 20000));
        assert!(!qualified_turn(&result, 28000));
    }
    #[test]
    fn weak_reassignment_checks_every_destination_raw_member() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 26000, 0),
            turn(28000, 40000, 0),
            turn(42000, 54000, 0),
            turn(56000, 62000, 2),
            turn(64000, 67000, 1),
            turn(68500, 74500, 2),
            turn(74000, 77000, 1),
        ];
        let endpoint = normalized(&[0.8660254, -0.5, 0., 0., 0., 0., 0., 0.]);
        let weak = normalized(&[0.76604444, 0.64278764, 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, weak), (2, endpoint)]);
        assert_eq!(identity(&result, 0), identity(&result, 56000));
        assert!(!qualified_turn(&result, 64000));
        assert!(!qualified_turn(&result, 74000));
    }
    fn artificial_assignments(
        raw: &[Turn],
        voices: &[(u32, Vec<f32>)],
        grouping: &[Vec<u32>],
    ) -> Vec<Option<usize>> {
        let mut sample_windows = windows(raw);
        for window in &mut sample_windows {
            window.embedding = voices
                .iter()
                .find(|(id, _)| *id == raw[window.turn].speaker)
                .unwrap()
                .1
                .clone();
        }
        let groups: Vec<Vec<usize>> = grouping
            .iter()
            .map(|ids| {
                sample_windows
                    .iter()
                    .enumerate()
                    .filter_map(|(i, w)| ids.contains(&raw[w.turn].speaker).then_some(i))
                    .collect()
            })
            .collect();
        assignments(raw, &sample_windows, &groups).0
    }
    #[test]
    fn mutually_blocked_weak_clusters_cannot_transitively_share_a_reliable_voice() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 17000, 1),
            turn(20000, 23000, 1),
            turn(22500, 25500, 2),
            turn(27000, 30000, 2),
        ];
        let plus = normalized(&[0.8660254, 0.5, 0., 0., 0., 0., 0., 0.]);
        let minus = normalized(&[0.8660254, -0.5, 0., 0., 0., 0., 0., 0.]);
        let result = artificial_assignments(
            &raw,
            &[(0, direction(0)), (1, plus), (2, minus)],
            &[vec![0], vec![1], vec![2]],
        );
        assert_eq!(result[0], Some(0));
        assert!(result[1..].iter().all(Option::is_none));
    }
    #[test]
    fn a_blocked_nearest_voice_does_not_make_the_runner_up_uniquely_plausible() {
        let mut raw: Vec<_> = (0..6)
            .map(|i| turn(i * 14000, i * 14000 + 12000, 0))
            .collect();
        raw.extend([
            turn(86000, 92000, 2),
            turn(94000, 97000, 1),
            turn(98000, 104000, 2),
            turn(103500, 106500, 1),
            turn(108000, 120000, 3),
        ]);
        let member = normalized(&[0.76604444, -0.64278764, 0., 0., 0., 0., 0., 0.]);
        let weak = normalized(&[0.9396926, 0.34202015, 0., 0., 0., 0., 0., 0.]);
        let runner = normalized(&[0.42261827, 0.9063078, 0., 0., 0., 0., 0., 0.]);
        let result = artificial_assignments(
            &raw,
            &[(0, direction(0)), (1, weak), (2, member), (3, runner)],
            &[vec![0, 2], vec![1], vec![3]],
        );
        assert!(
            result
                .iter()
                .enumerate()
                .filter(|(i, _)| raw[*i].speaker == 1)
                .all(|(_, g)| g.is_none())
        );
    }
    #[test]
    fn nearly_equal_reliable_candidates_leave_a_weak_voice_unassigned() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(14000, 26000, 1),
            turn(28000, 31000, 2),
        ];
        let result = artificial_assignments(
            &raw,
            &[
                (0, direction(0)),
                (1, direction(1)),
                (2, normalized(&[1., 1., 0., 0., 0., 0., 0., 0.])),
            ],
            &[vec![0], vec![1], vec![2]],
        );
        assert_eq!(result[2], None);
    }
    #[test]
    fn adjacent_native_turns_do_not_make_a_clean_monologue_disappear() {
        let raw: Vec<_> = (0..8).map(|i| turn(i * 2000, (i + 1) * 2000, 0)).collect();
        let mut calls = 0;
        let result = reconcile(&raw, |_, _| {
            calls += 1;
            Ok::<_, Infallible>(direction(0))
        })
        .unwrap();
        assert_eq!(calls, 8);
        assert_eq!(qualified(&result), 1);
        assert_eq!(
            times(&result),
            raw.iter()
                .map(|t| (t.start_ms, t.end_ms))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn a_dominant_bridge_cannot_erase_independently_supported_distinct_endpoints() {
        for endpoint in [
            direction(1),
            normalized(&[0.45, (1f32 - 0.45 * 0.45).sqrt(), 0., 0., 0., 0., 0., 0.]),
        ] {
            let mut raw = vec![
                turn(0, 6000, 0),
                turn(7000, 13000, 0),
                turn(14000, 20000, 1),
                turn(21000, 27000, 1),
            ];
            raw.extend((0..20).map(|i| turn(28000 + i * 7000, 34000 + i * 7000, 2)));
            let bridge = normalized(
                &direction(0)
                    .iter()
                    .zip(&endpoint)
                    .map(|(a, b)| a + b)
                    .collect::<Vec<_>>(),
            );
            let result = replay(&raw, &[(0, direction(0)), (1, endpoint), (2, bridge)]);
            assert!(qualified_turn(&result, 0));
            assert!(qualified_turn(&result, 14000));
            assert_ne!(identity(&result, 0), identity(&result, 14000));
        }
    }
    #[test]
    fn overlapping_same_voice_turns_do_not_repeat_identity_audio() {
        let raw = vec![turn(0, 12000, 0), turn(6000, 18000, 0)];
        let mut spans = Vec::new();
        let result = reconcile(&raw, |start, end| {
            spans.push((start, end));
            Ok::<_, Infallible>(direction(0))
        })
        .unwrap();
        spans.sort_unstable();
        assert!(spans.windows(2).all(|pair| pair[0].1 <= pair[1].0));
        assert_eq!(qualified(&result), 1);
    }
    #[test]
    fn a_short_overlapping_monologue_remains_negative_evidence_without_identity_authority() {
        let raw = vec![
            turn(0, 12000, 0),
            turn(22500, 30500, 1),
            turn(30000, 42000, 0),
        ];
        let other = normalized(&[0.68, (1f32 - 0.68 * 0.68).sqrt(), 0., 0., 0., 0., 0., 0.]);
        let result = replay(&raw, &[(0, direction(0)), (1, other)]);
        assert!(qualified_turn(&result, 0));
        assert!(!qualified_turn(&result, 22500));
    }
}
