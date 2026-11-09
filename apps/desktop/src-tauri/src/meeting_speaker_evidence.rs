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
    thread: Option<JoinHandle<Vec<SpeakingSample>>>,
}

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
            (snapshot.available, snapshot.speaking)
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(_started: Instant) -> Option<Self> {
        None
    }

    fn spawn(
        app_id: &str,
        started: Instant,
        read: impl Fn() -> (bool, Vec<String>) + Send + 'static,
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
        let samples = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        let samples = if samples.iter().any(|sample| sample.available) {
            samples
        } else {
            Vec::new()
        };
        AppSpeakerEvidence {
            app_id: std::mem::take(&mut self.app_id),
            samples,
        }
    }
}

fn sample_until(
    stop: &AtomicBool,
    started: Instant,
    interval: Duration,
    read: impl Fn() -> (bool, Vec<String>),
) -> Vec<SpeakingSample> {
    let mut samples = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let at = Instant::now();
        let (available, speaking) = read();
        samples.push(SpeakingSample {
            at_ms: at.saturating_duration_since(started).as_millis() as u64,
            available,
            speaking,
        });
        std::thread::sleep(interval.saturating_sub(at.elapsed()));
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_on_the_recording_timeline_until_stopped() {
        let started = Instant::now() - Duration::from_secs(10);
        let sampler =
            EvidenceSampler::spawn(TEAMS, started, || (true, vec!["Ann".into()])).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let evidence = sampler.finish();

        assert_eq!(evidence.app_id, TEAMS);
        assert!(!evidence.samples.is_empty());
        assert!(evidence.samples[0].at_ms >= 10_000);
        assert_eq!(evidence.samples[0].speaking, ["Ann"]);
    }

    #[test]
    fn a_meeting_app_that_was_never_readable_leaves_no_evidence() {
        let sampler =
            EvidenceSampler::spawn(TEAMS, Instant::now(), || (false, Vec::new())).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        assert!(sampler.finish().samples.is_empty());
    }
}
