use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::local_engine::CancelToken;
use crate::meetings::Speaker;
use crate::provider::ProviderKind;

const SAMPLE_RATE: u64 = 16_000;
const MAX_AUDIO_SAMPLES: usize = SAMPLE_RATE as usize * 6;
const INTERVAL_SAMPLES: u64 = SAMPLE_RATE * 3;
const MAX_CAPTION_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MeetingPreviewMode {
    Local,
    System,
    AfterRecording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MeetingPreviewState {
    Disabled,
    Warming,
    Ready,
    Busy,
    Failed,
    Unsupported,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct MeetingTrackPreview {
    /// Compatibility track identifiers: Me is the physical microphone, Others is system audio.
    pub source: Speaker,
    pub revision: u64,
    pub epoch: u64,
    pub chunk_start_ms: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub mode: MeetingPreviewMode,
    pub state: MeetingPreviewState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct MeetingPreviewSnapshot {
    pub session_id: String,
    pub revision: u64,
    pub transcript_revision: u64,
    pub tracks: Vec<MeetingTrackPreview>,
}

/// Accepted audio for optional platform preview. It is neither persisted nor diagnostic data.
pub struct MeetingPreviewAudio {
    pub session_id: String,
    pub source: Speaker,
    pub epoch: u64,
    pub chunk_start_ms: u64,
    pub start_ms: u64,
    pub samples: Vec<f32>,
}

/// Implementations enqueue bounded work and must not synchronously re-enter session mutation.
pub trait MeetingPreviewAudioObserver: Send + Sync {
    fn audio(&self, audio: MeetingPreviewAudio);
    fn reset(&self) {}
}

pub(crate) fn source_index(source: Speaker) -> usize {
    match source {
        Speaker::Me => 0,
        Speaker::Others => 1,
    }
}

pub(crate) struct PreviewDisplay {
    session_id: String,
    revision: u64,
    tracks: [MeetingTrackPreview; 2],
    accepted_end: [u64; 2],
    finalized_end: [u64; 2],
    paused: bool,
    stopped: bool,
}

impl Default for PreviewDisplay {
    fn default() -> Self {
        Self::new(None, false)
    }
}

impl PreviewDisplay {
    pub(crate) fn new(provider: Option<ProviderKind>, enabled: bool) -> Self {
        let mode = match provider {
            Some(ProviderKind::Local) => MeetingPreviewMode::Local,
            Some(ProviderKind::AppleSpeech) => MeetingPreviewMode::System,
            _ => MeetingPreviewMode::AfterRecording,
        };
        let state = if !enabled {
            MeetingPreviewState::Disabled
        } else if mode == MeetingPreviewMode::AfterRecording {
            MeetingPreviewState::Unsupported
        } else {
            MeetingPreviewState::Warming
        };
        Self {
            session_id: uuid::Uuid::now_v7().to_string(),
            revision: 0,
            tracks: [Speaker::Me, Speaker::Others].map(|source| MeetingTrackPreview {
                source,
                revision: 0,
                epoch: 0,
                chunk_start_ms: 0,
                start_ms: 0,
                end_ms: 0,
                text: String::new(),
                mode,
                state,
            }),
            accepted_end: [0, 0],
            finalized_end: [0, 0],
            paused: false,
            stopped: false,
        }
    }

    fn changed(&mut self, source: Speaker) {
        self.revision = self.revision.saturating_add(1);
        let track = &mut self.tracks[source_index(source)];
        track.revision = track.revision.saturating_add(1);
    }

    pub(crate) fn snapshot(&self, transcript_revision: u64) -> MeetingPreviewSnapshot {
        MeetingPreviewSnapshot {
            session_id: self.session_id.clone(),
            revision: self.revision,
            transcript_revision,
            tracks: self.tracks.to_vec(),
        }
    }

    pub(crate) fn allows_audio(&self, source: Speaker) -> bool {
        !self.paused
            && !self.stopped
            && !matches!(
                self.tracks[source_index(source)].state,
                MeetingPreviewState::Disabled
                    | MeetingPreviewState::Unsupported
                    | MeetingPreviewState::Failed
                    | MeetingPreviewState::Stopped
            )
    }

    pub(crate) fn advance(
        &mut self,
        source: Speaker,
        chunk_start_ms: u64,
        start_ms: u64,
        end_ms: u64,
    ) {
        let index = source_index(source);
        let track = &mut self.tracks[index];
        if track.chunk_start_ms != chunk_start_ms || start_ms > self.accepted_end[index] {
            track.epoch = track.epoch.saturating_add(1);
            track.chunk_start_ms = chunk_start_ms;
            track.start_ms = chunk_start_ms;
            track.end_ms = chunk_start_ms;
            track.text.clear();
            self.changed(source);
        }
        self.accepted_end[index] = end_ms;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish(
        &mut self,
        session: &str,
        source: Speaker,
        epoch: u64,
        chunk_start_ms: u64,
        start_ms: u64,
        end_ms: u64,
        text: &str,
    ) -> bool {
        let index = source_index(source);
        let track = &mut self.tracks[index];
        if session != self.session_id
            || self.stopped
            || self.paused
            || track.epoch != epoch
            || track.chunk_start_ms != chunk_start_ms
            || start_ms < chunk_start_ms
            || start_ms < self.finalized_end[index]
            || end_ms <= start_ms
            || end_ms > self.accepted_end[index]
            || end_ms < track.end_ms
            || matches!(
                track.state,
                MeetingPreviewState::Disabled
                    | MeetingPreviewState::Unsupported
                    | MeetingPreviewState::Failed
                    | MeetingPreviewState::Stopped
            )
        {
            return false;
        }
        let text: String = text
            .trim()
            .chars()
            .rev()
            .take(MAX_CAPTION_CHARS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if text.is_empty() {
            return false;
        }
        if track.text == text
            && track.start_ms == start_ms
            && track.end_ms == end_ms
            && track.state == MeetingPreviewState::Ready
        {
            return true;
        }
        track.text = text;
        track.start_ms = start_ms;
        track.end_ms = end_ms;
        track.state = MeetingPreviewState::Ready;
        self.changed(source);
        true
    }

    pub(crate) fn state(
        &mut self,
        session: &str,
        source: Speaker,
        epoch: u64,
        state: MeetingPreviewState,
    ) -> bool {
        let track = &mut self.tracks[source_index(source)];
        if session != self.session_id
            || self.stopped
            || self.paused
            || track.epoch != epoch
            || matches!(
                track.state,
                MeetingPreviewState::Disabled
                    | MeetingPreviewState::Unsupported
                    | MeetingPreviewState::Stopped
            )
        {
            return false;
        }
        if track.state != state {
            track.state = state;
            if matches!(
                state,
                MeetingPreviewState::Failed | MeetingPreviewState::Unsupported
            ) {
                track.text.clear();
            }
            self.changed(source);
        }
        true
    }

    pub(crate) fn complete(&mut self, source: Speaker, start_ms: u64, end_ms: u64) {
        let index = source_index(source);
        self.finalized_end[index] = self.finalized_end[index].max(end_ms);
        let track = &mut self.tracks[index];
        if track.chunk_start_ms == start_ms {
            track.epoch = track.epoch.saturating_add(1);
            track.text.clear();
            self.changed(source);
        }
    }

    pub(crate) fn exclude(&mut self, source: Speaker, start_ms: u64, end_ms: u64) {
        let track = &mut self.tracks[source_index(source)];
        if start_ms < track.end_ms && end_ms > track.start_ms {
            track.epoch = track.epoch.saturating_add(1);
            track.text.clear();
            track.start_ms = track.chunk_start_ms;
            track.end_ms = track.chunk_start_ms;
            self.changed(source);
        }
    }

    pub(crate) fn pause(&mut self, paused: bool) {
        if self.paused == paused || self.stopped {
            return;
        }
        self.paused = paused;
        for source in [Speaker::Me, Speaker::Others] {
            let track = &mut self.tracks[source_index(source)];
            track.epoch = track.epoch.saturating_add(1);
            track.text.clear();
            track.start_ms = track.chunk_start_ms;
            track.end_ms = track.chunk_start_ms;
            if !matches!(
                track.state,
                MeetingPreviewState::Disabled | MeetingPreviewState::Unsupported
            ) {
                track.state = if paused {
                    MeetingPreviewState::Busy
                } else {
                    MeetingPreviewState::Warming
                };
            }
            self.changed(source);
        }
    }

    pub(crate) fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        for source in [Speaker::Me, Speaker::Others] {
            let track = &mut self.tracks[source_index(source)];
            track.epoch = track.epoch.saturating_add(1);
            track.text.clear();
            track.state = MeetingPreviewState::Stopped;
            self.changed(source);
        }
    }
}

pub(crate) struct LocalPreviewAudio {
    pub(crate) source: Speaker,
    pub(crate) epoch: u64,
    pub(crate) chunk_start_ms: u64,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
    pub(crate) samples: Vec<f32>,
    pub(crate) cancel: Arc<CancelToken>,
}

#[derive(Default)]
struct Pending {
    rings: [VecDeque<f32>; 2],
    epochs: [Option<u64>; 2],
    ends: [u64; 2],
    submitted: [u64; 2],
    latest: [Option<LocalPreviewAudio>; 2],
    next_track: usize,
    active: Option<Arc<CancelToken>>,
    paused: bool,
    stopped: bool,
}

#[derive(Default)]
pub(crate) struct LocalPreviewQueue {
    pending: Mutex<Pending>,
    ready: Condvar,
}

impl LocalPreviewQueue {
    pub(crate) fn feed(
        &self,
        source: Speaker,
        epoch: u64,
        chunk_start_ms: u64,
        start: u64,
        samples: &[f32],
    ) {
        let mut pending = self.pending.lock().unwrap();
        if pending.stopped || pending.paused || samples.is_empty() {
            return;
        }
        let index = source_index(source);
        if pending.epochs[index] != Some(epoch) || pending.ends[index] != start {
            pending.rings[index].clear();
            pending.latest[index] = None;
            pending.submitted[index] = start;
        }
        pending.epochs[index] = Some(epoch);
        pending.ends[index] = start.saturating_add(samples.len() as u64);
        let ring = &mut pending.rings[index];
        if samples.len() >= MAX_AUDIO_SAMPLES {
            ring.clear();
        }
        ring.extend(
            samples[samples.len().saturating_sub(MAX_AUDIO_SAMPLES)..]
                .iter()
                .map(|value| if value.is_finite() { *value } else { 0.0 }),
        );
        let excess = ring.len().saturating_sub(MAX_AUDIO_SAMPLES);
        ring.drain(..excess);
        if pending.ends[index].saturating_sub(pending.submitted[index]) >= INTERVAL_SAMPLES {
            let end = pending.ends[index];
            let samples: Vec<f32> = pending.rings[index].iter().copied().collect();
            pending.latest[index] = Some(LocalPreviewAudio {
                source,
                epoch,
                chunk_start_ms,
                start_ms: end.saturating_sub(samples.len() as u64) * 1000 / SAMPLE_RATE,
                end_ms: end * 1000 / SAMPLE_RATE,
                samples,
                cancel: Arc::new(CancelToken::default()),
            });
            pending.submitted[index] = end;
            self.ready.notify_one();
        }
    }

    pub(crate) fn next(&self) -> Option<LocalPreviewAudio> {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if pending.stopped {
                return None;
            }
            if !pending.paused && pending.active.is_none() {
                for index in [pending.next_track, 1 - pending.next_track] {
                    if let Some(audio) = pending.latest[index].take() {
                        pending.next_track = 1 - index;
                        pending.active = Some(audio.cancel.clone());
                        return Some(audio);
                    }
                }
            }
            pending = self.ready.wait(pending).unwrap();
        }
    }

    pub(crate) fn finished(&self) {
        self.pending.lock().unwrap().active = None;
        self.ready.notify_one();
    }

    pub(crate) fn preempt(&self) {
        let mut pending = self.pending.lock().unwrap();
        if let Some(cancel) = &pending.active {
            cancel.cancel();
        }
        pending.latest = [None, None];
    }

    pub(crate) fn pause(&self, paused: bool) {
        self.preempt();
        let mut pending = self.pending.lock().unwrap();
        pending.paused = paused;
        pending.rings.iter_mut().for_each(VecDeque::clear);
        pending.epochs = [None, None];
        self.ready.notify_one();
    }

    pub(crate) fn stop(&self) {
        self.preempt();
        let mut pending = self.pending.lock().unwrap();
        pending.stopped = true;
        pending.rings.iter_mut().for_each(VecDeque::clear);
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalizing_an_old_chunk_does_not_clear_a_new_caption() {
        let mut display = PreviewDisplay::new(Some(ProviderKind::Local), true);
        display.advance(Speaker::Me, 0, 0, 6_000);
        let first = display.snapshot(0);
        assert!(display.publish(
            &first.session_id,
            Speaker::Me,
            first.tracks[0].epoch,
            0,
            0,
            6_000,
            "old"
        ));
        display.advance(Speaker::Me, 30_000, 30_000, 36_000);
        let current = display.snapshot(0);
        assert!(display.publish(
            &current.session_id,
            Speaker::Me,
            current.tracks[0].epoch,
            30_000,
            30_000,
            36_000,
            "new"
        ));
        display.complete(Speaker::Me, 0, 30_000);
        assert_eq!(display.snapshot(1).tracks[0].text, "new");
        assert_eq!(display.snapshot(1).transcript_revision, 1);
    }

    #[test]
    fn a_finalized_chunk_cannot_be_revived_by_a_late_partial() {
        let mut display = PreviewDisplay::new(Some(ProviderKind::AppleSpeech), true);
        display.advance(Speaker::Me, 0, 0, 6_000);
        let before = display.snapshot(0);
        display.complete(Speaker::Me, 0, 6_000);
        assert!(!display.publish(
            &before.session_id,
            Speaker::Me,
            before.tracks[0].epoch,
            0,
            0,
            6_000,
            "late"
        ));
        let after = display.snapshot(1);
        assert!(!display.publish(
            &after.session_id,
            Speaker::Me,
            after.tracks[0].epoch,
            0,
            0,
            6_000,
            "late with current epoch"
        ));
        assert!(display.snapshot(1).tracks[0].text.is_empty());
    }

    #[test]
    fn pause_stop_and_track_epochs_reject_late_publication() {
        let mut display = PreviewDisplay::new(Some(ProviderKind::AppleSpeech), true);
        display.advance(Speaker::Me, 0, 0, 6_000);
        display.advance(Speaker::Others, 0, 0, 6_000);
        let first = display.snapshot(0);
        display.pause(true);
        display.pause(false);
        assert!(!display.publish(
            &first.session_id,
            Speaker::Me,
            first.tracks[0].epoch,
            0,
            0,
            6_000,
            "stale"
        ));
        let next = display.snapshot(0);
        assert!(display.publish(
            &next.session_id,
            Speaker::Others,
            next.tracks[1].epoch,
            0,
            0,
            6_000,
            "system"
        ));
        assert!(display.snapshot(0).tracks[0].text.is_empty());
        display.stop();
        assert!(!display.publish(
            &next.session_id,
            Speaker::Others,
            next.tracks[1].epoch,
            0,
            0,
            6_000,
            "late"
        ));
        assert!(
            display
                .snapshot(0)
                .tracks
                .iter()
                .all(|track| track.text.is_empty())
        );
    }

    #[test]
    fn unavailable_and_disabled_routes_never_accept_preview_text() {
        for mut display in [
            PreviewDisplay::new(Some(ProviderKind::Groq), true),
            PreviewDisplay::new(Some(ProviderKind::Local), false),
        ] {
            display.advance(Speaker::Me, 0, 0, 6_000);
            let snapshot = display.snapshot(0);
            assert!(!display.publish(
                &snapshot.session_id,
                Speaker::Me,
                snapshot.tracks[0].epoch,
                0,
                0,
                6_000,
                "private"
            ));
            assert!(snapshot.tracks.iter().all(|track| track.text.is_empty()));
        }
    }

    #[test]
    fn local_audio_is_bounded_and_pending_snapshots_are_replaced() {
        let queue = LocalPreviewQueue::default();
        queue.feed(Speaker::Me, 1, 0, 0, &vec![0.2; 16_000 * 6]);
        let first = queue.next().unwrap();
        queue.feed(Speaker::Me, 1, 0, 96_000, &vec![0.3; 16_000 * 3]);
        queue.feed(Speaker::Me, 1, 0, 144_000, &vec![0.4; 16_000 * 3]);
        queue.feed(Speaker::Others, 1, 0, 0, &vec![0.5; 16_000 * 3]);
        let pending = queue.pending.lock().unwrap();
        assert_eq!(pending.rings[0].len(), 96_000);
        assert_eq!(pending.rings[1].len(), 48_000);
        assert_eq!(pending.latest[0].as_ref().unwrap().end_ms, 12_000);
        drop(pending);
        queue.preempt();
        assert!(first.cancel.is_cancelled());
        assert!(queue.pending.lock().unwrap().active.is_some());
        queue.finished();
        assert!(queue.pending.lock().unwrap().active.is_none());
        queue.stop();
        assert!(queue.next().is_none());
    }
}
