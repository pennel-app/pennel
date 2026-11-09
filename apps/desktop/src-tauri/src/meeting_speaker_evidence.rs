//! Samples the active speaker shown by a supported meeting app while a meeting records.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pennel_core::{AppSpeakerEvidence, SpeakingSample};

pub const TEAMS: &str = "com.microsoft.teams2";
const INTERVAL: Duration = Duration::from_millis(500);

pub struct EvidenceSampler {
    app_id: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Sampled>>,
}

#[derive(Default)]
struct Sampled {
    samples: Vec<SpeakingSample>,
    participants: Vec<String>,
}

/// One reading of the meeting app: whether it was readable, who it showed and who was speaking.
type Reading = (bool, Vec<String>, Vec<String>);

/// A recording that ends without `finish` (discarded, failed) still stops sampling.
impl Drop for EvidenceSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl EvidenceSampler {
    /// `started` is the recorder's origin, so samples share the transcript's timeline.
    #[cfg(target_os = "macos")]
    pub fn start(started: Instant) -> Option<Self> {
        Self::spawn(TEAMS, started, || {
            let snapshot = crate::macos::meeting_speakers_snapshot(TEAMS);
            (snapshot.available, snapshot.participants, snapshot.speaking)
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(_started: Instant) -> Option<Self> {
        None
    }

    fn spawn(
        app_id: &str,
        started: Instant,
        read: impl Fn() -> Reading + Send + 'static,
    ) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("meeting-speaker-evidence".into())
            .spawn({
                let stop = stop.clone();
                move || sample_until(&stop, started, INTERVAL, read)
            })
            .ok()?;
        Some(Self {
            app_id: app_id.to_owned(),
            stop,
            thread: Some(thread),
        })
    }

    /// Evidence without a single readable sample is empty, which stores nothing.
    pub fn finish(mut self) -> AppSpeakerEvidence {
        self.stop.store(true, Ordering::SeqCst);
        let sampled = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        let readable = sampled.samples.iter().any(|sample| sample.available);
        AppSpeakerEvidence {
            app_id: std::mem::take(&mut self.app_id),
            samples: if readable {
                sampled.samples
            } else {
                Vec::new()
            },
            participants: if readable {
                sampled.participants
            } else {
                Vec::new()
            },
        }
    }
}

fn sample_until(
    stop: &AtomicBool,
    started: Instant,
    interval: Duration,
    read: impl Fn() -> Reading,
) -> Sampled {
    let mut sampled = Sampled::default();
    while !stop.load(Ordering::SeqCst) {
        let at = Instant::now();
        let (available, participants, speaking) = read();
        for name in participants {
            if !sampled.participants.contains(&name) {
                sampled.participants.push(name);
            }
        }
        sampled.samples.push(SpeakingSample {
            at_ms: at.saturating_duration_since(started).as_millis() as u64,
            available,
            speaking,
        });
        std::thread::sleep(interval.saturating_sub(at.elapsed()));
    }
    sampled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_on_the_recording_timeline_until_stopped() {
        let started = Instant::now() - Duration::from_secs(10);
        let sampler = EvidenceSampler::spawn(TEAMS, started, || {
            (true, vec!["Ann".into(), "Bert".into()], vec!["Ann".into()])
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let evidence = sampler.finish();

        assert_eq!(evidence.app_id, TEAMS);
        assert!(!evidence.samples.is_empty());
        assert!(evidence.samples[0].at_ms >= 10_000);
        assert_eq!(evidence.samples[0].speaking, ["Ann"]);
        assert_eq!(evidence.participants, ["Ann", "Bert"]);
    }

    #[test]
    fn a_meeting_app_that_was_never_readable_leaves_no_evidence() {
        let sampler =
            EvidenceSampler::spawn(TEAMS, Instant::now(), || (false, Vec::new(), Vec::new()))
                .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let evidence = sampler.finish();
        assert!(evidence.samples.is_empty());
        assert!(evidence.participants.is_empty());
    }
}
