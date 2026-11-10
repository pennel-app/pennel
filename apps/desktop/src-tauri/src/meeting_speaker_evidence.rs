//! Samples one explicitly associated Teams call on the recording timeline.

#[cfg(any(target_os = "macos", test))]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pennel_core::{
    AppEvidenceAvailability, AppEvidenceBinding, AppSpeakerEvidence, SpeakingSample,
};

pub const TEAMS: &str = "com.microsoft.teams2";
const INTERVAL: Duration = Duration::from_millis(500);
type Consent = Arc<dyn Fn() -> bool + Send + Sync>;
pub(crate) type CaptureSourceCheck = Arc<dyn Fn() -> bool + Send + Sync>;
pub(crate) type CleanEndCallback = Arc<dyn Fn() + Send + Sync>;
type SharedHealth = Arc<Mutex<SamplerHealth>>;

#[derive(Default)]
struct SamplerHealth {
    sample_count: u64,
    retained_sample_count: usize,
    available_sample_count: usize,
    last_sample_at_ms: Option<u64>,
    last_sample_at: Option<Instant>,
    last_read_duration_ms: Option<u64>,
    last_sample_available: Option<bool>,
    last_read_reason: Option<String>,
    availability: AppEvidenceAvailability,
    reason: Option<String>,
    binding_verified: bool,
    producer_running: bool,
    stop_requested: bool,
    clean_end: bool,
    revoked: bool,
    panicked: bool,
}

fn health_reason(reason: Option<&str>) -> Option<String> {
    const SAFE: &[&str] = &[
        "unsupported",
        "permission_denied",
        "ambiguous_process",
        "app_unavailable",
        "app_lookup_transient",
        "window_hidden",
        "window_unavailable",
        "window_not_visible",
        "ambiguous_call",
        "layout_unavailable",
        "unavailable",
        "unbound_window",
        "call_changed",
        "unbound_call_end",
        "invalid_call_end",
        "audio_clock_unverified",
        "audio_capture_not_app_scoped",
        "consent_revoked",
        "speaking_state_unknown",
        "call_end_stop_unavailable",
        "sampler_unavailable",
        "sampler_starting",
        "sampler_failed",
    ];
    reason.map(|value| {
        SAFE.iter()
            .copied()
            .find(|known| *known == value)
            .unwrap_or("unavailable")
            .to_owned()
    })
}

impl SamplerHealth {
    fn observed(&mut self, at: Instant, started: Instant, reading: &Reading) {
        self.sample_count = self.sample_count.saturating_add(1);
        self.last_sample_at_ms = Some(at.saturating_duration_since(started).as_millis() as u64);
        self.last_sample_at = Some(at);
        self.last_read_duration_ms = Some(at.elapsed().as_millis() as u64);
        self.last_sample_available = Some(reading.available);
        self.last_read_reason = health_reason(reading.unavailable_reason.as_deref());
    }

    fn sampled(&mut self, sampled: &Sampled) {
        if self.revoked || self.panicked {
            return;
        }
        self.retained_sample_count = sampled.samples.len();
        self.available_sample_count = sampled
            .samples
            .iter()
            .filter(|sample| sample.available)
            .count();
        self.availability = sampled.availability;
        self.reason = health_reason(sampled.unavailable_reason.as_deref());
        self.binding_verified = sampled.window.is_some() && !sampled.invalidated;
        self.clean_end = sampled.ended && !sampled.invalidated;
    }

    fn revoke(&mut self) {
        self.revoked = true;
        self.availability = AppEvidenceAvailability::Disabled;
        self.reason = Some("consent_revoked".into());
        self.binding_verified = false;
        self.retained_sample_count = 0;
        self.available_sample_count = 0;
    }

    fn snapshot(&self, handle_running: bool) -> serde_json::Value {
        serde_json::json!({
            "sample_count": self.sample_count,
            "retained_sample_count": self.retained_sample_count,
            "available_sample_count": self.available_sample_count,
            "last_sample_at_ms": self.last_sample_at_ms,
            "last_sample_age_ms": self.last_sample_at.map(|at| at.elapsed().as_millis() as u64),
            "last_read_duration_ms": self.last_read_duration_ms,
            "last_sample_available": self.last_sample_available,
            "last_read_reason": self.last_read_reason,
            "availability": self.availability,
            "reason": self.reason,
            "binding_verified": self.binding_verified,
            "producer_running": self.producer_running && handle_running,
            "stop_requested": self.stop_requested,
            "clean_end": self.clean_end,
        })
    }
}

struct ProducerHealth(SharedHealth);

impl Drop for ProducerHealth {
    fn drop(&mut self) {
        let mut health = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        health.producer_running = false;
        if std::thread::panicking() {
            health.panicked = true;
            health.availability = AppEvidenceAvailability::Unavailable;
            health.reason = Some("sampler_failed".into());
            health.binding_verified = false;
            health.retained_sample_count = 0;
            health.available_sample_count = 0;
        }
    }
}

fn publish_health(health: Option<&SharedHealth>, sampled: &Sampled) {
    if let Some(health) = health {
        health
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .sampled(sampled);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct EvidenceClock {
    pub(crate) started: Instant,
    pub(crate) verified: bool,
}

#[derive(Clone)]
struct SamplingPolicy {
    consent: Consent,
    capture_source: CaptureSourceCheck,
    clean_end: Option<CleanEndCallback>,
    clock_verified: bool,
}

struct SamplingChecks<'a> {
    consent: &'a dyn Fn() -> bool,
    capture_source: &'a dyn Fn() -> bool,
    clean_end: Option<&'a CleanEndCallback>,
    clock_verified: bool,
}

/// Updated synchronously before native capture can widen to global audio. A later app-scoped
/// restart cannot make earlier unscoped audio safe for participant-name attribution.
#[cfg(any(target_os = "macos", test))]
pub(crate) struct AppAudioSourceIntegrity {
    requested_app: Option<String>,
    state: AtomicU8,
}

#[cfg(any(target_os = "macos", test))]
impl AppAudioSourceIntegrity {
    pub(crate) fn new(requested_app: Option<&str>) -> Self {
        Self {
            requested_app: requested_app.map(str::to_owned),
            state: AtomicU8::new(0), // Unknown until the native tap reports its actual selection.
        }
    }

    pub(crate) fn observe(&self, app_scoped: bool) {
        if !app_scoped || self.requested_app.is_none() {
            self.state.store(2, Ordering::SeqCst);
        } else {
            let _ = self
                .state
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    pub(crate) fn check(self: &Arc<Self>, expected_app: &str) -> CaptureSourceCheck {
        let source = self.clone();
        let expected_app = expected_app.to_owned();
        Arc::new(move || {
            source.requested_app.as_deref() == Some(expected_app.as_str())
                && source.state.load(Ordering::SeqCst) == 1
        })
    }
}

pub struct EvidenceSampler {
    app_id: String,
    recording_uid: String,
    recording_started_at: String,
    stop: Arc<AtomicBool>,
    disabled: Arc<AtomicBool>,
    consent: Consent,
    capture_source: CaptureSourceCheck,
    clock_verified: bool,
    thread: Option<JoinHandle<Sampled>>,
    health: SharedHealth,
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
    call_ended: bool,
    call_active: bool,
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
    ended: bool,
}

impl Sampled {
    fn invalidate_clock(&mut self) {
        self.invalidate_capture_source();
        self.unavailable_reason = Some("audio_clock_unverified".into());
    }
    fn invalidate_capture_source(&mut self) {
        self.samples.clear();
        self.participants.clear();
        self.window = None;
        self.duplicate_names = false;
        self.availability = AppEvidenceAvailability::Unavailable;
        self.unavailable_reason = Some("audio_capture_not_app_scoped".into());
        self.invalidated = true;
    }

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
        if self.invalidated || self.ended {
            return;
        }
        if reading.call_active
            && !reading.available
            && !reading.call_ended
            && reading.window.is_some()
            && self
                .window
                .as_ref()
                .is_none_or(|bound| Some(bound) == reading.window.as_ref())
        {
            self.window = reading.window;
            self.duplicate_names |= reading.duplicate_names;
            self.unavailable_reason = Some("speaking_state_unknown".into());
            self.samples.push(SpeakingSample {
                at_ms,
                available: false,
                speaking: Vec::new(),
            });
            return;
        }
        let matching_end = reading.call_ended
            && !reading.call_active
            && !reading.available
            && reading.participants.is_empty()
            && reading.speaking.is_empty()
            && self
                .window
                .as_ref()
                .is_some_and(|bound| Some(bound) == reading.window.as_ref());
        let reason =
            if reading.call_ended && !matching_end {
                Some(if self.window.is_none() {
                    "unbound_call_end"
                } else if self.window.as_ref() != reading.window.as_ref() {
                    "call_changed"
                } else {
                    "invalid_call_end"
                })
            } else if self.window.as_ref().is_some_and(|bound| {
                reading.window.is_some() && Some(bound) != reading.window.as_ref()
            }) {
                Some("call_changed")
            } else if !reading.available && !matching_end {
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
        if matching_end {
            self.samples.push(SpeakingSample {
                at_ms,
                available: false,
                speaking: Vec::new(),
            });
            self.ended = true;
            return;
        }
        self.window = reading.window;
        self.availability = AppEvidenceAvailability::Available;
        self.unavailable_reason = None;
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
        self.request_stop();
    }
}

impl EvidenceSampler {
    pub(crate) fn live_status(&self) -> serde_json::Value {
        let handle_running = self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished());
        self.health
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .snapshot(handle_running)
    }

    /// `detected_app` must come from the selected recording source, never just a running Teams app.
    #[cfg(target_os = "macos")]
    pub(crate) fn start(
        clock: EvidenceClock,
        detected_app: Option<&str>,
        recording_uid: &str,
        recording_started_at: &str,
        consent: Consent,
        capture_source: CaptureSourceCheck,
        clean_end: Option<CleanEndCallback>,
    ) -> Option<Self> {
        if detected_app != Some(TEAMS)
            || recording_uid.is_empty()
            || recording_started_at.is_empty()
        {
            return None;
        }
        Self::spawn_with_policy(
            TEAMS,
            clock.started,
            recording_uid,
            recording_started_at,
            SamplingPolicy {
                consent,
                capture_source,
                clean_end,
                clock_verified: clock.verified,
            },
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
                    call_ended: snapshot.call_ended,
                    call_active: snapshot.call_active,
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
    pub(crate) fn start(
        _clock: EvidenceClock,
        _detected_app: Option<&str>,
        _recording_uid: &str,
        _recording_started_at: &str,
        _consent: Consent,
        _capture_source: CaptureSourceCheck,
        _clean_end: Option<CleanEndCallback>,
    ) -> Option<Self> {
        None
    }

    #[cfg(test)]
    fn spawn(
        app_id: &str,
        started: Instant,
        recording_uid: &str,
        recording_started_at: &str,
        consent: Consent,
        capture_source: CaptureSourceCheck,
        read: impl Fn() -> Reading + Send + 'static,
    ) -> Option<Self> {
        Self::spawn_with_policy(
            app_id,
            started,
            recording_uid,
            recording_started_at,
            SamplingPolicy {
                consent,
                capture_source,
                clean_end: None,
                clock_verified: true,
            },
            read,
        )
    }

    fn spawn_with_policy(
        app_id: &str,
        started: Instant,
        recording_uid: &str,
        recording_started_at: &str,
        policy: SamplingPolicy,
        read: impl Fn() -> Reading + Send + 'static,
    ) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let disabled = Arc::new(AtomicBool::new(false));
        let health = Arc::new(Mutex::new(SamplerHealth {
            producer_running: true,
            reason: Some("sampler_starting".into()),
            ..SamplerHealth::default()
        }));
        let thread = std::thread::Builder::new()
            .name("meeting-speaker-evidence".into())
            .spawn({
                let stop = stop.clone();
                let disabled = disabled.clone();
                let policy = policy.clone();
                let health = health.clone();
                move || {
                    let _producer = ProducerHealth(health.clone());
                    sample_until_with_end(
                        &stop,
                        &disabled,
                        started,
                        INTERVAL,
                        SamplingChecks {
                            consent: &*policy.consent,
                            capture_source: &*policy.capture_source,
                            clean_end: policy.clean_end.as_ref(),
                            clock_verified: policy.clock_verified,
                        },
                        read,
                        Some(&health),
                    )
                }
            })
            .ok()?;
        Some(Self {
            app_id: app_id.to_owned(),
            recording_uid: recording_uid.to_owned(),
            recording_started_at: recording_started_at.to_owned(),
            stop,
            disabled,
            consent: policy.consent,
            capture_source: policy.capture_source,
            clock_verified: policy.clock_verified,
            thread: Some(thread),
            health,
        })
    }

    /// Stop requests never wait for an in-flight AX read or extend the audio capture lifetime.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.health
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .stop_requested = true;
    }

    /// Revocation is sticky for this recording even if the setting is subsequently re-enabled.
    pub fn disable(&self) {
        self.disabled.store(true, Ordering::SeqCst);
        self.health
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .revoke();
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
        } else if !(self.capture_source)() {
            sampled.invalidate_capture_source();
        } else if !self.clock_verified {
            sampled.invalidate_clock();
        } else if sampled.samples.is_empty() && !sampled.invalidated {
            sampled.unavailable_reason = Some("sampler_unavailable".into());
        }
        publish_health(Some(&self.health), &sampled);
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

#[cfg(test)]
fn sample_until(
    stop: &AtomicBool,
    disabled: &AtomicBool,
    started: Instant,
    interval: Duration,
    consent: &dyn Fn() -> bool,
    capture_source: &dyn Fn() -> bool,
    read: impl Fn() -> Reading,
) -> Sampled {
    sample_until_with_end(
        stop,
        disabled,
        started,
        interval,
        SamplingChecks {
            consent,
            capture_source,
            clean_end: None,
            clock_verified: true,
        },
        read,
        None,
    )
}

fn dispatch_clean_end(callback: Option<&CleanEndCallback>) -> bool {
    let Some(callback) = callback.cloned() else {
        return false;
    };
    std::thread::Builder::new()
        .name("meeting-call-ended".into())
        .spawn(move || callback())
        .is_ok()
}

fn sample_until_with_end(
    stop: &AtomicBool,
    disabled: &AtomicBool,
    started: Instant,
    interval: Duration,
    checks: SamplingChecks<'_>,
    read: impl Fn() -> Reading,
    health: Option<&SharedHealth>,
) -> Sampled {
    let SamplingChecks {
        consent,
        capture_source,
        clean_end,
        clock_verified,
    } = checks;
    let mut sampled = Sampled::default();
    while !stop.load(Ordering::SeqCst) {
        if disabled.load(Ordering::SeqCst) || !consent() {
            disabled.store(true, Ordering::SeqCst);
            sampled.revoke_consent();
            break;
        }
        if !capture_source() {
            sampled.invalidate_capture_source();
            break;
        }
        if !clock_verified {
            sampled.invalidate_clock();
            break;
        }
        let at = Instant::now();
        let reading = read();
        if let Some(health) = health {
            health
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .observed(at, started, &reading);
        }
        if disabled.load(Ordering::SeqCst) || !consent() {
            disabled.store(true, Ordering::SeqCst);
            sampled.revoke_consent();
            break;
        }
        if !capture_source() {
            sampled.invalidate_capture_source();
            break;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        sampled.observe(
            at.saturating_duration_since(started).as_millis() as u64,
            reading,
        );
        if sampled.ended {
            if !dispatch_clean_end(clean_end) {
                sampled.invalidate_capture_source();
                sampled.unavailable_reason = Some("call_end_stop_unavailable".into());
            }
            publish_health(health, &sampled);
            break;
        }
        publish_health(health, &sampled);
        // A new call can reuse the same Teams window. Once association is lost, an explicit new
        // recording must establish it again; never keep inspecting a replacement call.
        if sampled.invalidated {
            break;
        }
        std::thread::sleep(interval.saturating_sub(at.elapsed()));
    }
    if disabled.load(Ordering::SeqCst) {
        sampled.revoke_consent();
    } else if !capture_source() {
        sampled.invalidate_capture_source();
    } else if !clock_verified {
        sampled.invalidate_clock();
    }
    publish_health(health, &sampled);
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

    fn clean_end(window_id: &str) -> Reading {
        Reading {
            call_ended: true,
            window: readable(window_id).window,
            ..Reading::default()
        }
    }

    fn unknown_speaking(window_id: &str) -> Reading {
        Reading {
            call_active: true,
            window: readable(window_id).window,
            ..Reading::default()
        }
    }

    fn wait_for_producer_exit(sampler: &EvidenceSampler) {
        let until = Instant::now() + Duration::from_secs(2);
        while !sampler.thread.as_ref().unwrap().is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(sampler.thread.as_ref().unwrap().is_finished());
    }

    #[test]
    fn live_health_reports_a_producer_that_stopped_mid_recording() {
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "private-recording-uuid",
            "private-started-at",
            Arc::new(|| true),
            Arc::new(|| true),
            move || {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    readable("private-window-uuid")
                } else {
                    Reading {
                        unavailable_reason: Some("app_unavailable".into()),
                        ..Reading::default()
                    }
                }
            },
        )
        .unwrap();
        wait_for_producer_exit(&sampler);
        let status = sampler.live_status();
        assert_eq!(status["sample_count"], 2);
        assert_eq!(status["retained_sample_count"], 2);
        assert_eq!(status["available_sample_count"], 1);
        assert_eq!(status["producer_running"], false);
        assert_eq!(status["availability"], "unavailable");
        assert_eq!(status["reason"], "app_unavailable");
        assert_eq!(status["binding_verified"], false);
        assert!(status["last_sample_at_ms"].as_u64().is_some());
        assert!(status["last_sample_age_ms"].as_u64().is_some());
        let encoded = status.to_string();
        for secret in [
            "Ann",
            "Bert",
            "private-window-uuid",
            "private-recording-uuid",
            "private-started-at",
        ] {
            assert!(!encoded.contains(secret));
        }
        sampler.finish();
    }

    #[test]
    fn live_health_does_not_echo_unrecognized_native_reason_text() {
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "private-recording-uuid",
            "private-started-at",
            Arc::new(|| true),
            Arc::new(|| true),
            || Reading {
                unavailable_reason: Some("private transcript or participant name".into()),
                ..Reading::default()
            },
        )
        .unwrap();
        wait_for_producer_exit(&sampler);
        let status = sampler.live_status();
        assert_eq!(status["reason"], "unavailable");
        assert!(!status.to_string().contains("private"));
        sampler.finish();
    }

    #[test]
    fn live_health_updates_immediately_when_consent_is_revoked_during_a_read() {
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let (blocked, waiting) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "private-recording-uuid",
            "private-started-at",
            Arc::new(|| true),
            Arc::new(|| true),
            move || {
                if reads.fetch_add(1, Ordering::SeqCst) == 1 {
                    blocked.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                readable("private-window-uuid")
            },
        )
        .unwrap();
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(sampler.live_status()["producer_running"], true);
        assert_eq!(sampler.live_status()["sample_count"], 1);
        sampler.disable();
        let revoked = sampler.live_status();
        assert_eq!(revoked["availability"], "disabled");
        assert_eq!(revoked["reason"], "consent_revoked");
        assert_eq!(revoked["stop_requested"], true);
        assert_eq!(revoked["binding_verified"], false);
        assert_eq!(revoked["retained_sample_count"], 0);
        release.send(()).unwrap();
        wait_for_producer_exit(&sampler);
        assert_eq!(sampler.live_status()["producer_running"], false);
        assert_eq!(
            sampler.finish().availability,
            AppEvidenceAvailability::Disabled
        );
    }

    #[test]
    fn dropping_the_sampler_requests_stop_and_thread_exit_updates_shared_health() {
        let (blocked, waiting) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "private-recording-uuid",
            "private-started-at",
            Arc::new(|| true),
            Arc::new(|| true),
            move || {
                blocked.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(2)).unwrap();
                readable("private-window-uuid")
            },
        )
        .unwrap();
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let health = sampler.health.clone();
        drop(sampler);
        assert!(health.lock().unwrap().stop_requested);
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while health.lock().unwrap().producer_running && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = health.lock().unwrap().snapshot(false);
        assert_eq!(status["producer_running"], false);
        assert_eq!(status["retained_sample_count"], 0);
    }

    #[test]
    fn strong_unknown_speaking_can_bind_then_recover_in_the_same_call() {
        let mut sampled = Sampled::default();
        sampled.observe(0, unknown_speaking("first"));
        assert_eq!(sampled.availability, AppEvidenceAvailability::Unavailable);
        assert!(sampled.participants.is_empty());
        assert!(!sampled.invalidated);
        sampled.observe(500, readable("first"));
        assert_eq!(sampled.availability, AppEvidenceAvailability::Available);
        assert_eq!(sampled.unavailable_reason, None);
        assert_eq!(sampled.participants, ["Ann", "Bert"]);
        assert_eq!(sampled.samples.len(), 2);
        assert!(!sampled.samples[0].available);
    }

    #[test]
    fn unknown_speaking_does_not_carry_a_highlight_or_accept_unverified_names() {
        let mut sampled = Sampled::default();
        sampled.observe(0, readable("first"));
        let mut unknown = unknown_speaking("first");
        unknown.participants = vec!["Unverified".into()];
        unknown.speaking = vec!["Unverified".into()];
        unknown.duplicate_names = true;
        sampled.observe(500, unknown);
        assert_eq!(sampled.availability, AppEvidenceAvailability::Available);
        assert_eq!(sampled.participants, ["Ann", "Bert"]);
        assert!(sampled.samples[1].speaking.is_empty());
        assert!(!sampled.samples[1].available);
        assert!(sampled.duplicate_names);
    }

    #[test]
    fn unknown_speaking_cannot_adopt_a_replacement_window() {
        let mut sampled = Sampled::default();
        sampled.observe(0, unknown_speaking("first"));
        sampled.observe(500, unknown_speaking("second"));
        assert!(sampled.invalidated);
        assert_eq!(sampled.unavailable_reason.as_deref(), Some("call_changed"));
        assert_eq!(sampled.window.as_ref().unwrap().window_id, "first");
    }

    #[test]
    fn unknown_only_call_can_seal_but_never_promotes_name_evidence() {
        let mut sampled = Sampled::default();
        sampled.observe(0, unknown_speaking("first"));
        sampled.observe(500, clean_end("first"));
        assert!(sampled.ended);
        assert_eq!(sampled.availability, AppEvidenceAvailability::Unavailable);
        assert!(sampled.participants.is_empty());
        assert!(
            sampled
                .samples
                .iter()
                .all(|sample| !sample.available && sample.speaking.is_empty())
        );
    }

    #[test]
    fn proven_clean_end_seals_matching_call_without_erasing_valid_evidence() {
        let mut sampled = Sampled::default();
        sampled.observe(0, readable("first"));
        sampled.observe(500, clean_end("first"));
        assert_eq!(sampled.availability, AppEvidenceAvailability::Available);
        assert_eq!(sampled.participants, ["Ann", "Bert"]);
        assert_eq!(sampled.window.as_ref().unwrap().window_id, "first");
        assert_eq!(sampled.samples.len(), 2);
        assert!(!sampled.samples[1].available);
        assert!(sampled.samples[1].speaking.is_empty());
        sampled.observe(1_000, readable("replacement"));
        assert_eq!(sampled.samples.len(), 2);
    }

    #[test]
    fn unbound_or_changed_clean_end_never_qualifies_as_a_clean_stop() {
        let mut unbound = Sampled::default();
        unbound.observe(0, clean_end("first"));
        assert_eq!(unbound.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            unbound.unavailable_reason.as_deref(),
            Some("unbound_call_end")
        );

        let mut changed = Sampled::default();
        changed.observe(0, readable("first"));
        changed.observe(500, clean_end("second"));
        assert_eq!(changed.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(changed.unavailable_reason.as_deref(), Some("call_changed"));
    }

    #[test]
    fn clean_end_dispatches_once_off_the_sampler_thread_and_stops_reading() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let callback: CleanEndCallback = Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            sent.send(std::thread::current().id()).unwrap();
        });
        let sampler_thread = std::thread::current().id();
        let sampled = sample_until_with_end(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            SamplingChecks {
                consent: &|| true,
                capture_source: &|| true,
                clean_end: Some(&callback),
                clock_verified: true,
            },
            || match reads.fetch_add(1, Ordering::SeqCst) {
                0 => readable("first"),
                1 => clean_end("first"),
                _ => panic!("a sealed call must not inspect a replacement call"),
            },
            None,
        );
        assert_ne!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            sampler_thread
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(sampled.availability, AppEvidenceAvailability::Available);
        assert!(sampled.ended);
        assert!(!sampled.samples.last().unwrap().available);
    }

    #[test]
    fn scope_loss_after_clean_end_still_erases_all_evidence_at_finish() {
        let scoped = Arc::new(AtomicBool::new(true));
        let check = scoped.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let sampler = EvidenceSampler::spawn_with_policy(
            TEAMS,
            Instant::now(),
            "meeting-uid",
            "2026-11-09T10:00:00Z",
            SamplingPolicy {
                consent: Arc::new(|| true),
                capture_source: Arc::new(move || check.load(Ordering::SeqCst)),
                clean_end: Some(Arc::new(move || {
                    sent.send(()).unwrap();
                })),
                clock_verified: true,
            },
            move || {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    readable("first")
                } else {
                    clean_end("first")
                }
            },
        )
        .unwrap();
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        scoped.store(false, Ordering::SeqCst);
        let evidence = sampler.finish();
        assert_eq!(evidence.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("audio_capture_not_app_scoped")
        );
        assert!(evidence.samples.is_empty());
        assert!(evidence.participants.is_empty());
        assert!(evidence.binding.is_none());
    }

    #[test]
    fn clean_end_without_an_owned_stop_handler_cannot_qualify_evidence() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let sampled = sample_until(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            &|| true,
            &|| true,
            || {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    readable("first")
                } else {
                    clean_end("first")
                }
            },
        );
        assert_eq!(sampled.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            sampled.unavailable_reason.as_deref(),
            Some("call_end_stop_unavailable")
        );
        assert!(sampled.samples.is_empty());
        assert!(sampled.participants.is_empty());
    }

    #[test]
    fn an_unverified_audio_clock_never_reads_or_labels_participants() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let sampled = sample_until_with_end(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            SamplingChecks {
                consent: &|| true,
                capture_source: &|| true,
                clean_end: None,
                clock_verified: false,
            },
            || panic!("an unverified timeline must not read participant names"),
            None,
        );
        assert_eq!(sampled.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            sampled.unavailable_reason.as_deref(),
            Some("audio_clock_unverified")
        );
        assert!(sampled.samples.is_empty());
        assert!(sampled.participants.is_empty());
        assert!(sampled.window.is_none());
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
                    EvidenceClock {
                        started: Instant::now(),
                        verified: true
                    },
                    app,
                    "meeting-uid",
                    "2026-11-09T10:00:00Z",
                    Arc::new(|| true),
                    Arc::new(|| true),
                    None,
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
            &|| true,
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
            &|| true,
            || panic!("revoked consent must not read Teams"),
        );
        assert_eq!(sampled.availability, AppEvidenceAvailability::Disabled);
        assert!(sampled.samples.is_empty());
    }

    #[test]
    fn a_global_capture_never_reads_participant_names() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let sampled = sample_until(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            &|| true,
            &|| false,
            || panic!("global audio must not read Teams"),
        );
        assert_eq!(sampled.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            sampled.unavailable_reason.as_deref(),
            Some("audio_capture_not_app_scoped")
        );
        assert!(sampled.samples.is_empty());
        assert!(sampled.participants.is_empty());
        assert!(sampled.window.is_none());
    }

    #[test]
    fn capture_scope_lost_during_a_read_erases_prior_and_current_names() {
        let stop = AtomicBool::new(false);
        let disabled = AtomicBool::new(false);
        let scoped = AtomicBool::new(true);
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let sampled = sample_until(
            &stop,
            &disabled,
            Instant::now(),
            Duration::ZERO,
            &|| true,
            &|| scoped.load(Ordering::SeqCst),
            || {
                if reads.fetch_add(1, Ordering::SeqCst) == 1 {
                    scoped.store(false, Ordering::SeqCst);
                    stop.store(true, Ordering::SeqCst);
                }
                readable("first")
            },
        );
        assert!(sampled.samples.is_empty());
        assert!(sampled.participants.is_empty());
        assert!(sampled.window.is_none());
        assert_eq!(
            sampled.unavailable_reason.as_deref(),
            Some("audio_capture_not_app_scoped")
        );
    }

    #[test]
    fn capture_scope_lost_after_stop_request_is_rechecked_at_finish() {
        let scoped = Arc::new(AtomicBool::new(true));
        let check = scoped.clone();
        let sampler = EvidenceSampler::spawn(
            TEAMS,
            Instant::now(),
            "meeting-uid",
            "2026-11-09T10:00:00Z",
            Arc::new(|| true),
            Arc::new(move || check.load(Ordering::SeqCst)),
            || readable("first"),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        sampler.request_stop();
        scoped.store(false, Ordering::SeqCst);
        let evidence = sampler.finish();
        assert_eq!(evidence.availability, AppEvidenceAvailability::Unavailable);
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("audio_capture_not_app_scoped")
        );
        assert!(evidence.samples.is_empty());
        assert!(evidence.participants.is_empty());
        assert!(evidence.binding.is_none());
    }

    #[test]
    fn native_scope_report_is_required_and_loss_is_sticky() {
        let source = Arc::new(AppAudioSourceIntegrity::new(Some(TEAMS)));
        let check = source.check(TEAMS);
        assert!(!check(), "an unreported tap must fail closed");
        source.observe(true);
        assert!(check());
        source.observe(false);
        assert!(!check());
        source.observe(true);
        assert!(
            !check(),
            "later scoped audio cannot repair earlier global audio"
        );
    }

    #[test]
    fn manual_global_and_other_app_sources_never_qualify_for_teams() {
        for requested in [None, Some("us.zoom.xos")] {
            let source = Arc::new(AppAudioSourceIntegrity::new(requested));
            let check = source.check(TEAMS);
            source.observe(true);
            assert!(!check());
        }
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
            Arc::new(|| true),
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
