//! Private, anonymous clean-window clustering diagnostic; never writes embeddings.
#[cfg(feature = "onnx")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    diagnostic::run()
}
#[cfg(not(feature = "onnx"))]
fn main() {
    panic!("requires onnx");
}

#[cfg(feature = "onnx")]
mod diagnostic {
    use pennel_speakers::{
        Diarization, Diarizer, Embedder, FRAGMENT_THRESHOLD, Models, Options, Speaker, Turn,
        cosine, reconcile_fragments, speech_ms, voiceprint_spans, weighted_mean,
    };
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeSet, HashMap},
        error::Error,
        path::Path,
        time::Instant,
    };
    struct Point {
        start: u64,
        end: u64,
        raw: u32,
        turn: usize,
        v: Vec<f32>,
    }
    fn quantile(mut values: Vec<f32>, fraction: f32) -> f32 {
        values.sort_by(f32::total_cmp);
        values
            .get(((values.len().saturating_sub(1)) as f32 * fraction) as usize)
            .copied()
            .unwrap_or(0.)
    }
    fn summary(points: &[Point], members: &[usize]) -> Value {
        let voices: BTreeSet<_> = members.iter().map(|i| points[*i].raw).collect();
        let turns: BTreeSet<_> = members.iter().map(|i| points[*i].turn).collect();
        let ms: u64 = members
            .iter()
            .map(|i| points[*i].end - points[*i].start)
            .sum();
        let pairs = members
            .iter()
            .enumerate()
            .flat_map(|(a, i)| {
                members[a + 1..]
                    .iter()
                    .map(move |j| cosine(&points[*i].v, &points[*j].v))
            })
            .collect::<Vec<_>>();
        json!({"members":members,"raw_speakers":voices,"raw_turn_count":turns.len(),"clean_ms":ms,"point_count":members.len(),"pair_similarity_min":quantile(pairs.clone(),0.),"pair_similarity_p10":quantile(pairs.clone(),0.1),"pair_similarity_median":quantile(pairs,0.5),"reliable":ms>=10000&&(turns.len()>=2||members.len()>=2)})
    }
    fn cluster(points: &[Point], raw: &[Turn], threshold: f32, mode: &str) -> Value {
        let mut groups: Vec<Vec<usize>> = (0..points.len()).map(|i| vec![i]).collect();
        let mut matrix = vec![vec![0f32; points.len()]; points.len()];
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                let score = cosine(&points[a].v, &points[b].v);
                matrix[a][b] = score;
                matrix[b][a] = score;
            }
        }
        let mut links = matrix.clone();
        // Linkage over independent clean windows: no original speaker-centroid inheritance.
        loop {
            let mut best: Option<(usize, usize, f32)> = None;
            for a in 0..groups.len() {
                for b in a + 1..groups.len() {
                    let score = if mode == "p10" {
                        let scores: Vec<f32> = groups[a]
                            .iter()
                            .flat_map(|i| groups[b].iter().map(|j| matrix[*i][*j]))
                            .collect();
                        quantile(scores, 0.1)
                    } else {
                        links[a][b]
                    };
                    if score >= threshold && best.is_none_or(|(_, _, prior)| score > prior) {
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
                .map(|(a, b)| {
                    if mode == "complete" {
                        a.min(*b)
                    } else {
                        (a * left + b * right) / (left + right)
                    }
                })
                .collect();
            for (c, joined) in updates.into_iter().enumerate() {
                if c == a || c == b {
                    continue;
                }
                links[a][c] = joined;
                links[c][a] = joined;
            }
            links.remove(b);
            for row in &mut links {
                row.remove(b);
            }
            let next = groups.remove(b);
            groups[a].extend(next);
        }
        let summaries: Vec<Value> = groups.iter().map(|g| summary(points, g)).collect();
        let reliable: Vec<usize> = summaries
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (s["reliable"] == true).then_some(i))
            .collect();
        let centroids: Vec<Vec<f32>> = groups
            .iter()
            .map(|g| {
                weighted_mean(g.iter().map(|i| {
                    (
                        points[*i].v.as_slice(),
                        (points[*i].end - points[*i].start) as f32,
                    )
                }))
            })
            .collect();
        let mut assigned = Vec::new();
        let mut abstained_ms = 0u64;
        let mut stable_ms = 0u64;
        let mut point_to_group = HashMap::new();
        for (g, members) in groups.iter().enumerate() {
            for i in members {
                point_to_group.insert(*i, g);
            }
        }
        for (index, t) in raw.iter().enumerate() {
            let own: Vec<usize> = points
                .iter()
                .enumerate()
                .filter_map(|(i, p)| (p.turn == index).then_some(i))
                .collect();
            let observed: u64 = own.iter().map(|i| points[*i].end - points[*i].start).sum();
            let mut votes = HashMap::<usize, u64>::new();
            for i in own {
                let p = &points[i];
                let g = point_to_group[&i];
                let assignment = if reliable.contains(&g) {
                    Some(g)
                } else {
                    let mut nearest: Vec<_> = reliable
                        .iter()
                        .map(|r| (*r, cosine(&p.v, &centroids[*r])))
                        .collect();
                    nearest.sort_by(|a, b| b.1.total_cmp(&a.1));
                    nearest.first().and_then(|(r, s)| {
                        (*s >= 0.65
                            && nearest
                                .get(1)
                                .is_none_or(|(_, second)| *s - *second >= 0.15))
                        .then_some(*r)
                    })
                };
                if let Some(g) = assignment {
                    *votes.entry(g).or_default() += p.end - p.start;
                }
            }
            let best = votes.into_iter().max_by_key(|(_, v)| *v);
            let group = best
                .filter(|(_, ms)| *ms as f64 >= observed as f64 * 0.8)
                .map(|(g, _)| g);
            if group.is_some() {
                stable_ms += t.end_ms - t.start_ms;
            } else {
                abstained_ms += t.end_ms - t.start_ms;
            }
            // Isolated abstentions retain distinct labels, without becoming identified voices.
            assigned.push(Turn {
                start_ms: t.start_ms,
                end_ms: t.end_ms,
                speaker: group.unwrap_or(groups.len() + index) as u32,
            });
        }
        json!({"mode":mode,"threshold":threshold,"group_count":groups.len(),"reliable_group_count":reliable.len(),"groups":summaries,"stable_raw_turn_ms":stable_ms,"abstained_raw_turn_ms":abstained_ms,"turns":assigned})
    }
    fn production(manifest: &Value) -> Result<(), Box<dyn Error>> {
        let segmentation = Path::new(
            manifest["segmentation_model"]
                .as_str()
                .ok_or("segmentation_model required")?,
        );
        let embedding = Path::new(
            manifest["embedding_model"]
                .as_str()
                .ok_or("embedding_model required")?,
        );
        let options = Options::default();
        let began = Instant::now();
        let diarizer = Diarizer::load(
            &Models {
                segmentation,
                embedding,
            },
            &options,
        )?;
        let load_ms = began.elapsed().as_millis();
        let mut datasets = Vec::new();
        for data in manifest["datasets"].as_array().ok_or("datasets required")? {
            let audio = super::pcm16_wave(&std::fs::read(
                data["audio"].as_str().ok_or("audio required")?,
            )?)?;
            let began = Instant::now();
            let result = diarizer.process(&audio)?;
            datasets.push(json!({"id":data["id"],"inference_ms":began.elapsed().as_millis(),"sample_count":audio.len(),"sample_rate":pennel_speakers::SAMPLE_RATE,"qualified_speakers":result.speakers.iter().filter(|speaker|!speaker.embedding.is_empty()).map(|speaker|speaker.speaker).collect::<Vec<_>>(),"speakers":result.speakers.iter().map(|speaker|json!({"speaker":speaker.speaker,"speech_ms":speaker.speech_ms,"qualified":!speaker.embedding.is_empty(),"embedding_dimensions":speaker.embedding.len()})).collect::<Vec<_>>(),"turns":result.turns}));
        }
        println!(
            "{}",
            json!({"schema_version":1,"actual_production_path":true,"embeddings_retained":false,"model_load_ms":load_ms,"datasets":datasets})
        );
        Ok(())
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let manifest: Value = serde_json::from_slice(&std::fs::read(
            std::env::args().nth(1).ok_or("manifest required")?,
        )?)?;
        if std::env::args().nth(2).as_deref() == Some("--production") {
            return production(&manifest);
        }
        let embedder = Embedder::load(Path::new(manifest["embedding_model"].as_str().unwrap()), 4)?;
        let mut datasets = Vec::new();
        for data in manifest["datasets"].as_array().unwrap() {
            let began = Instant::now();
            let audio = super::pcm16_wave(&std::fs::read(data["audio"].as_str().unwrap())?)?;
            let turns: Vec<Turn> =
                serde_json::from_slice(&std::fs::read(data["turns"].as_str().unwrap())?)?;
            let count = turns.iter().map(|t| t.speaker).max().map_or(0, |v| v + 1);
            let mut speakers = Vec::new();
            let mut points = Vec::new();
            for id in 0..count {
                let spans = voiceprint_spans(&turns, id, u64::MAX);
                let budget = voiceprint_spans(&turns, id, 60000);
                speakers.push(Speaker {
                    speaker: id,
                    speech_ms: speech_ms(&turns, id),
                    embedding: embedder.embed_spans(&audio, &budget)?,
                });
                for (start, end) in spans {
                    let mut at = start + 100;
                    while end.saturating_sub(at) >= 1700 {
                        let stop = (at + 5000).min(end - 100);
                        if stop - at < 1500 {
                            break;
                        }
                        let turn = turns
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| {
                                t.speaker == id && t.start_ms <= at && t.end_ms >= stop
                            })
                            .map(|(i, _)| i)
                            .next();
                        if let Some(turn) = turn {
                            let v = embedder.embed_spans(&audio, &[(at, stop)])?;
                            if !v.is_empty() {
                                points.push(Point {
                                    start: at,
                                    end: stop,
                                    raw: id,
                                    turn,
                                    v,
                                });
                            }
                        }
                        at = stop;
                    }
                }
            }
            let inference_ms = began.elapsed().as_millis();
            let raw = Diarization {
                turns: turns.clone(),
                speakers,
            };
            let baseline = reconcile_fragments(raw.clone(), FRAGMENT_THRESHOLD);
            let mut blocked = Vec::new();
            for (i, a) in raw.speakers.iter().enumerate() {
                for b in &raw.speakers[i + 1..] {
                    let sim = cosine(&a.embedding, &b.embedding);
                    if sim < 0.65 {
                        continue;
                    }
                    let mut overlaps = Vec::new();
                    for x in raw.turns.iter().filter(|t| t.speaker == a.speaker) {
                        for y in raw.turns.iter().filter(|t| t.speaker == b.speaker) {
                            let start = x.start_ms.max(y.start_ms);
                            let end = x.end_ms.min(y.end_ms);
                            if end > start {
                                overlaps.push((start, end));
                            }
                        }
                    }
                    blocked.push(json!({"left":a.speaker,"right":b.speaker,"centroid_similarity":sim,"overlap_ms":overlaps.iter().map(|(a,b)|b-a).sum::<u64>(),"overlap_intervals":overlaps}));
                }
            }
            let originals: Vec<Value> = (0..count)
                .map(|id| {
                    let members: Vec<usize> = points
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| (p.raw == id).then_some(i))
                        .collect();
                    json!({"speaker":id,"summary":summary(&points,&members)})
                })
                .collect();
            let mut variants = Vec::new();
            for mode in ["complete", "average"] {
                for threshold in [0.55, 0.60, 0.65, 0.70] {
                    variants.push(cluster(&points, &turns, threshold, mode));
                }
            }
            datasets.push(json!({"id":data["id"],"inference_ms":inference_ms,"raw_speakers":count,"clean_points":points.len(),"points":points.iter().map(|p|json!({"start_ms":p.start,"end_ms":p.end,"raw_speaker":p.raw,"raw_turn":p.turn})).collect::<Vec<_>>(),"original_group_coherence":originals,"high_similarity_centroid_pairs":blocked,"production_baseline":{"speaker_count":baseline.speakers.len(),"turns":baseline.turns},"variants":variants}));
        }
        println!(
            "{}",
            json!({"schema_version":1,"prototype_only":true,"embeddings_retained":false,"datasets":datasets})
        );
        Ok(())
    }
}

#[cfg(any(feature = "onnx", test))]
fn pcm16_wave(bytes: &[u8]) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("audio must be a RIFF PCM16 mono 16000 Hz WAVE file".into());
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into()?) as usize + 8;
    if declared > bytes.len() {
        return Err("truncated RIFF file".into());
    }
    let mut cursor = 12usize;
    let mut format = false;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= declared) {
        let length = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into()?) as usize;
        let start = cursor + 8;
        let end = start.checked_add(length).ok_or("WAVE chunk overflow")?;
        if end > declared {
            return Err("truncated WAVE chunk".into());
        }
        match &bytes[cursor..cursor + 4] {
            b"fmt " => {
                if length < 16 {
                    return Err("truncated WAVE format".into());
                }
                let word = |offset| {
                    u16::from_le_bytes(
                        bytes[start + offset..start + offset + 2]
                            .try_into()
                            .unwrap(),
                    )
                };
                let rate = u32::from_le_bytes(bytes[start + 4..start + 8].try_into()?);
                if word(0) != 1 || word(2) != 1 || rate != 16_000 || word(12) != 2 || word(14) != 16
                {
                    return Err("audio must be PCM16 mono 16000 Hz".into());
                }
                format = true;
            }
            b"data" => {
                if data.is_some() {
                    return Err("multiple WAVE data chunks are unsupported".into());
                }
                data = Some(&bytes[start..end]);
            }
            _ => {}
        }
        cursor = end.checked_add(length % 2).ok_or("WAVE chunk overflow")?;
    }
    if !format {
        return Err("missing WAVE format".into());
    }
    let data = data.ok_or("missing WAVE data")?;
    if data.len() % 2 != 0 {
        return Err("incomplete PCM16 sample".into());
    }
    Ok(data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0)
        .collect())
}
