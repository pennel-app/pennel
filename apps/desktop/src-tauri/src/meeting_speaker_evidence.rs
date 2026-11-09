//! Samples one explicitly associated Teams call on the recording timeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pennel_core::{
    AppEvidenceAvailability, AppEvidenceBinding, AppSpeakerEvidence, SpeakingSample,
};

pub const TEAMS: &str = "com.microsoft.teams2";
const INTERVAL: Duration = Duration::from_millis(500);
type Consent = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct EvidenceSampler {
    app_id: String,
    recording_uid: String,
    recording_started_at: String,
    stop: Arc<AtomicBool>,
    disabled: Arc<AtomicBool>,
    consent: Consent,
    thread: Option<JoinHandle<Sampled>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CallWindow {
    process_id: u32,
    process_started_at_ms: u64,
    window_id: String,
}

#[derive(Default)]
struct Reading {
    available: bool,
    participants: Vec<String>,
    speaking: Vec<String>,
    window: Option<CallWindow>,
    unavailable_reason: Option<String>,
    duplicate_names: bool,
}

#[derive(Default)]
struct Sampled {
    samples: Vec<SpeakingSample>,
    participants: Vec<String>,
    window: Option<CallWindow>,
    availability: AppEvidenceAvailability,
    unavailable_reason: Option<String>,
    duplicate_names: bool,
    invalidated: bool,
}

impl Sampled {
    fn revoke_consent(&mut self) {
        self.samples.clear();
        self.participants.clear();
        self.window = None;
        self.duplicate_names = false;
        self.availability = AppEvidenceAvailability::Disabled;
        self.unavailable_reason = Some("consent_revoked".into());
        self.invalidated = true;
    }

    fn observe(&mut self, at_ms: u64, reading: Reading) {
        if self.invalidated {
            return;
        }
        let reason = if !reading.available {
            Some(
                reading
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("unavailable"),
            )
        } else if reading.window.is_none() {
            Some("unbound_window")
        } else if self
            .window
            .as_ref()
            .is_some_and(|bound| Some(bound) != reading.window.as_ref())
        {
            Some("call_changed")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.availability = match reason {
                "permission_denied" => AppEvidenceAvailability::PermissionDenied,
                "unsupported" => AppEvidenceAvailability::Unsupported,
                _ => AppEvidenceAvailability::Unavailable,
            };
            self.unavailable_reason = Some(reason.to_owned());
            self.invalidated = true;
            self.samples.push(SpeakingSample {
                at_ms,
                available: false,
                speaking: Vec::new(),
            });
            return;
        }
        self.window = reading.window;
        self.availability = AppEvidenceAvailability::Available;
        self.duplicate_names |= reading.duplicate_names;
        for name in reading.participants {
            if !self.participants.contains(&name) {
                self.participants.push(name);
            }
        }
        self.samples.push(SpeakingSample {
            at_ms,
            available: true,
            speaking: reading.speaking,
        });
    }
}

/// Discarding a recording still stops its narrowly scoped observer.
impl Drop for EvidenceSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl EvidenceSampler {
    /// `detected_app` must come from the selected recording source, never just a running Teams app.
    #[cfg(target_os = "macos")]
    pub fn start(
        started: Instant,
        detected_app: Option<&str>,
        recording_uid: &str,
        recording_started_at: &str,
        consent: Consent,
    ) -> Option<Self> {
        if detected_app != Some(TEAMS)
            || recording_uid.is_empty()
            || recording_started_at.is_empty()
        {
            return None;
        }
        Self::spawn(
            TEAMS,
            started,
            recording_uid,
            recording_started_at,
            consent,
            || {
                let snapshot = crate::macos::meeting_speakers_snapshot(TEAMS);
                let window = match (
                    snapshot.process_id,
                    snapshot.process_started_at_ms,
                    snapshot.window_id,
                ) {
                    (Some(process_id), Some(process_started_at_ms), Some(window_id))
                        if process_id > 0 && process_started_at_ms > 0 && !window_id.is_empty() =>
                    {
                        Some(CallWindow {
                            process_id,
                            process_started_at_ms,
                            window_id,
                        })
                    }
                    _ => None,
                };
                Reading {
                    available: snapshot.available,
                    participants: snapshot.participants,
                    speaking: snapshot.speaking,
                    window,
                    unavailable_reason: snapshot.unavailable_reason,
                    duplicate_names: snapshot.duplicate_names,
                }
            },
        )
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(
        _started: Instant,
        _detected_app: Option<&str>,
        _recording_uid: &str,
        _recording_started_at: &str,
        _consent: Consent,
    ) -> Option<Self> {
        None
    }

    fn spawn(
        app_id: &str,
        started: Instant,
        recording_uid: &str,
        recording_started_at: &str,
        consent: Consent,
        read: impl Fn() -> Reading + Send + 'static,
    ) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let disabled = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("meeting-speaker-evidence".into())
            .spawn({
                let stop = stop.clone();
                let disabled = disabled.clone();
                let consent = consent.clone();
                move || sample_until(&stop, &disabled, started, INTERVAL, &*consent, read)
            })
            .ok()?;
        Some(Self {
            app_id: app_id.to_owned(),
            recording_uid: recording_uid.to_owned(),
            recording_started_at: recording_started_at.to_owned(),
            stop,
            disabled,
            consent,
            thread: Some(thread),
        })
    }

    /// Stop requests never wait for an in-flight AX read or extend the audio capture lifetime.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Revocation is sticky for this recording even if the setting is subsequently re-enabled.
    pub fn disable(&self) {
        self.disabled.store(true, Ordering::SeqCst);
        self.request_stop();
    }

    pub fn finish(mut self) -> AppSpeakerEvidence {
        self.request_stop();
        let mut sampled = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        if self.disabled.load(Ordering::SeqCst) || !(self.consent)() {
            sampled.revoke_consent();
        } else if sampled.samples.is_empty() {
            sampled.unavailable_reason = Some("sampler_unavailable".into());
        }
        AppSpeakerEvidence {
            app_id: std::mem::take(&mut self.app_id),
            samples: sampled.samples,
            participants: sampled.participants,
            binding: sampled.window.map(|window| AppEvidenceBinding {
                recording_uid: std::mem::take(&mut self.recording_uid),
                recording_started_at: std::mem::take(&mut self.recording_started_at),
                process_id: window.process_id,
                process_started_at_ms: window.process_started_at_ms,
                window_id: window.window_id,
            }),
            availability: sampled.availability,
            unavailable_reason: sampled.unavailable_reason,
            duplicate_names: sampled.duplicate_names,
        }
    }
}

fn sample_until(
    stop: &AtomicBool,
    disabled: &AtomicBool,
    started: Instant,
    interval: Duration,
    consent: &dyn Fn() -> bool,
    read: impl Fn() -> Reading,
) -> Sampled {
    let mut sampled = Sampled::default();
    while !stop.load(Ordering::SeqCst) {
        if disabled.load(Ordering::SeqCst) || !consent() {
            disabled.store(true, Ordering::SeqCst);
            sampled.revoke_consent();
            break;
        }
        let at = Instant::now();
        let reading = read();
        if disabled.load(Ordering::SeqCst) || !consent() {
            disabled.store(true, Ordering::SeqCst);
            sampled.revoke_consent();
            break;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        sampled.observe(
            at.saturating_duration_since(started).as_millis() as u64,
            reading,
        );
        // A new call can reuse the same Teams window. Once association is lost, an explicit new
        // recording must establish it again; never keep inspecting a replacement call.
        if sampled.invalidated {
            break;
        }
        std::thread::sleep(interval.saturating_sub(at.elapsed()));
    }
    if disabled.load(Ordering::SeqCst) {
        sampled.revoke_consent();
    }
    sampled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readable(window_id: &str) -> Reading {
        Reading {
            available: true,
            participants: vec!["Ann".into(), "Bert".into()],
            speaking: vec!["Ann".into()],
            window: Some(CallWindow {
                process_id: 42,
                process_started_at_ms: 1_000,
                window_id: window_id.into(),
            }),
            ..Reading::default()
        }
    }

    #[test]
    fn samples_on_the_recording_timeline_until_stopped() {
        let started = Instant::now() - Duration::from_secs(10);
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            started,
            "meeting-uid",
            "2026-11-09T10:00:00Z",
            Arc::new(|| true),
            || readable("call-window"),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let evidence = sampler.finish();

        assert_eq!(evidence.app_id, TEAMS);
        assert!(!evidence.samples.is_empty());
        assert!(evidence.samples[0].at_ms >= 10_000);
        assert_eq!(evidence.samples[0].speaking, ["Ann"]);
        assert_eq!(evidence.participants, ["Ann", "Bert"]);
        assert_eq!(evidence.availability, AppEvidenceAvailability::Available);
        let binding = evidence.binding.unwrap();
        assert_eq!(binding.recording_uid, "meeting-uid");
        assert_eq!(binding.recording_started_at, "2026-11-09T10:00:00Z");
        assert_eq!(binding.window_id, "call-window");
    }

    #[test]
    fn unavailable_permission_is_retained_without_any_readable_sample() {
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "meeting-uid",
            "2026-11-09T10:00:00Z",
            Arc::new(|| true),
            || Reading {
                unavailable_reason: Some("permission_denied".into()),
                ..Reading::default()
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let evidence = sampler.finish();
        assert_eq!(evidence.samples.len(), 1);
        assert!(!evidence.samples[0].available);
        assert!(evidence.binding.is_none());
        assert_eq!(
            evidence.availability,
            AppEvidenceAvailability::PermissionDenied
        );
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("permission_denied")
        );
    }

    #[test]
    fn a_changed_window_cannot_contribute_names_or_resume_the_old_call() {
        let mut sampled = Sampled::default();
        sampled.observe(0, readable("first"));
        let mut changed = readable("second");
        changed.participants = vec!["Unrelated".into()];
        changed.speaking = vec!["Unrelated".into()];
        sampled.observe(500, changed);
        sampled.observe(1_000, readable("first"));
        assert_eq!(sampled.samples.len(), 2);
        assert!(!sampled.samples[1].available);
        assert_eq!(sampled.participants, ["Ann", "Bert"]);
        assert_eq!(sampled.window.unwrap().window_id, "first");
        assert_eq!(sampled.unavailable_reason.as_deref(), Some("call_changed"));
    }

    #[test]
    fn process_replacement_and_permission_revocation_invalidate_binding() {
        let mut replacement = Sampled::default();
        replacement.observe(0, readable("first"));
        let mut changed = readable("first");
        changed.window.as_mut().unwrap().process_started_at_ms += 1;
        replacement.observe(500, changed);
        assert_eq!(
            replacement.availability,
            AppEvidenceAvailability::Unavailable
        );
        assert_eq!(
            replacement.unavailable_reason.as_deref(),
            Some("call_changed")
        );

        let mut revoked = Sampled::default();
        revoked.observe(0, readable("first"));
        revoked.observe(
            500,
            Reading {
                unavailable_reason: Some("permission_denied".into()),
                ..Reading::default()
            },
        );
        assert_eq!(
            revoked.availability,
            AppEvidenceAvailability::PermissionDenied
        );
        assert!(revoked.invalidated);
    }

    #[test]
    fn duplicate_names_remain_an_explicit_conflict() {
        let mut sampled = Sampled::default();
        let mut reading = readable("first");
        reading.duplicate_names = true;
        sampled.observe(0, reading);
        sampled.observe(500, readable("first"));
        assert!(sampled.duplicate_names);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unrelated_or_unselected_apps_never_start_the_reader() {
        for app in [None, Some("us.zoom.xos"), Some("com.google.Chrome")] {
            assert!(
                EvidenceSampler::start(
                    Instant::now(),
                    app,
                    "meeting-uid",
                    "2026-11-09T10:00:00Z",
                    Arc::new(|| true)
                )
                .is_none()
            );
        }
    }

    #[test]
    fn consent_revoked_during_a_read_discards_that_result_and_prior_names() {
        let allowed = AtomicBool::new(true);
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let sampled = sample_until(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            &|| allowed.load(Ordering::SeqCst),
            || {
                if reads.fetch_add(1, Ordering::SeqCst) == 1 {
                    allowed.store(false, Ordering::SeqCst);
                }
                readable("first")
            },
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(sampled.availability, AppEvidenceAvailability::Disabled);
        assert!(sampled.samples.is_empty());
        assert!(sampled.participants.is_empty());
        assert!(sampled.window.is_none());
    }

    #[test]
    fn initial_revocation_never_calls_the_native_reader() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let sampled = sample_until(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            &|| false,
            || panic!("revoked consent must not read Teams"),
        );
        assert_eq!(sampled.availability, AppEvidenceAvailability::Disabled);
        assert!(sampled.samples.is_empty());
    }

    #[test]
    fn explicit_disable_is_sticky_when_the_setting_is_reenabled() {
        let allowed = Arc::new(AtomicBool::new(true));
        let policy = allowed.clone();
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "meeting-uid",
            "2026-11-09T10:00:00Z",
            Arc::new(move || policy.load(Ordering::SeqCst)),
            || readable("first"),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        allowed.store(false, Ordering::SeqCst);
        sampler.disable();
        allowed.store(true, Ordering::SeqCst);
        let evidence = sampler.finish();
        assert_eq!(evidence.availability, AppEvidenceAvailability::Disabled);
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("consent_revoked")
        );
        assert!(evidence.samples.is_empty());
        assert!(evidence.participants.is_empty());
        assert!(evidence.binding.is_none());
    }
}
