//! Port of `PennelKit/DictationStatus.swift`.

use std::time::{Duration, Instant};

use serde::Serialize;

/// What the dictation pill shows. Notices carry their parts separately, so no reader splits text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PillState {
    Hidden,
    Listening,
    HandsFree,
    Command,
    /// Listening, but the microphone has carried no signal for a while.
    NoSound,
    Working,
    /// Pasted into the app that had focus when the recording stopped.
    Inserted {
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<String>,
    },
    /// On the clipboard, waiting for the person to paste it.
    Copied {
        paste_key: String,
    },
    /// What failed and, when words were involved, where they were kept.
    Problem {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

const INSERTED_FOR: Duration = Duration::from_millis(900);
const COPIED_FOR: Duration = Duration::from_millis(2400);
/// Long enough to read a two-line problem, also in Dutch.
const PROBLEM_FOR: Duration = Duration::from_millis(3500);

fn notice_duration(notice: &PillState) -> Duration {
    match notice {
        PillState::Inserted { .. } => INSERTED_FOR,
        PillState::Copied { .. } => COPIED_FOR,
        _ => PROBLEM_FOR,
    }
}

/// Derives what the pill shows from everything in flight: an active recording wins,
/// then a short-lived notice, then pending dictations and deliveries.
#[derive(Debug, Default)]
pub struct DictationStatus {
    recording: Option<PillState>,
    no_sound: bool,
    in_flight: u32,
    delivering: u32,
    notice: Option<PillState>,
    notice_until: Option<Instant>,
}

impl DictationStatus {
    pub fn pill(&self) -> PillState {
        if let Some(recording) = &self.recording {
            if self.no_sound && *recording != PillState::Command {
                return PillState::NoSound;
            }
            return recording.clone();
        }
        if let Some(notice) = &self.notice {
            return notice.clone();
        }
        if self.in_flight + self.delivering > 0 {
            PillState::Working
        } else {
            PillState::Hidden
        }
    }

    pub fn recording_started(&mut self, hands_free: bool) {
        self.no_sound = false;
        self.recording = Some(if hands_free {
            PillState::HandsFree
        } else {
            PillState::Listening
        });
    }

    pub fn mark_command(&mut self) {
        if self.recording.is_some() {
            self.recording = Some(PillState::Command);
        }
    }

    /// Only informs: a silent microphone never ends the recording by itself.
    pub fn set_no_sound(&mut self, silent: bool) {
        if self.recording.is_some() {
            self.no_sound = silent;
        }
    }

    pub fn recording_cancelled(&mut self) {
        self.recording = None;
        self.no_sound = false;
    }

    pub fn recording_submitted(&mut self) {
        self.recording_cancelled();
        self.in_flight += 1;
    }

    pub fn dictation_finished(&mut self, notice: Option<PillState>, now: Instant) {
        self.in_flight = self.in_flight.saturating_sub(1);
        if let Some(notice) = notice {
            self.show_notice(notice, now);
        }
    }

    /// A finished result is on its way to its destination; the pill keeps working until it lands.
    pub fn delivery_started(&mut self) {
        self.delivering += 1;
    }

    pub fn delivery_finished(&mut self, notice: PillState, now: Instant) {
        self.delivering = self.delivering.saturating_sub(1);
        self.show_notice(notice, now);
    }

    /// A confirmation never hides a problem that is still on screen.
    pub fn show_notice(&mut self, notice: PillState, now: Instant) {
        let problem_showing = matches!(self.notice, Some(PillState::Problem { .. }))
            && self.notice_until.is_some_and(|until| now < until);
        if problem_showing && !matches!(notice, PillState::Problem { .. }) {
            return;
        }
        self.notice_until = Some(now + notice_duration(&notice));
        self.notice = Some(notice);
    }

    pub fn notice_deadline(&self) -> Option<Instant> {
        self.notice_until
    }

    /// Returns whether a notice ended, so the pill needs drawing again.
    pub fn expire_notice(&mut self, now: Instant) -> bool {
        if self.notice_until.is_none_or(|until| now < until) {
            return false;
        }
        self.notice = None;
        self.notice_until = None;
        true
    }
}

/// Below this a microphone carries no signal at all; even a quiet room measures far above it.
const NO_SIGNAL_LEVEL: f32 = 1e-4;
const NO_SOUND_AFTER: Duration = Duration::from_secs(2);

/// Watches the microphone level while listening: two seconds without signal means No sound,
/// and any signal clears it again.
#[derive(Debug, Default)]
pub struct NoSoundWatch {
    flat_since: Option<Instant>,
    silent: bool,
}

impl NoSoundWatch {
    /// Feeds one level sample and returns the new verdict when it changes.
    pub fn level(&mut self, level: f32, now: Instant) -> Option<bool> {
        if level >= NO_SIGNAL_LEVEL {
            self.flat_since = None;
            return std::mem::take(&mut self.silent).then_some(false);
        }
        let since = *self.flat_since.get_or_insert(now);
        if self.silent || now.saturating_duration_since(since) < NO_SOUND_AFTER {
            return None;
        }
        self.silent = true;
        Some(true)
    }
}

/// Whether the current recording is a spoken command: Shift pressed at any point while the key is held.
#[derive(Debug, Default)]
pub struct CommandIntent {
    is_command: bool,
}

impl CommandIntent {
    pub fn is_command(&self) -> bool {
        self.is_command
    }

    pub fn shift(&mut self, down: bool, key_held: bool) {
        if down && key_held {
            self.is_command = true;
        }
    }

    pub fn reset(&mut self) {
        self.is_command = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(title: &str) -> PillState {
        PillState::Problem {
            title: title.into(),
            detail: Some("Your words are kept in History".into()),
        }
    }

    fn inserted() -> PillState {
        PillState::Inserted {
            app: Some("Mail".into()),
        }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn idle_is_hidden() {
        assert_eq!(DictationStatus::default().pill(), PillState::Hidden);
    }

    #[test]
    fn recording_wins_over_work_and_notice() {
        let now = Instant::now();
        let mut s = DictationStatus::default();
        s.recording_started(false);
        s.recording_submitted();
        s.show_notice(problem("x"), now);
        s.recording_started(true);
        assert_eq!(s.pill(), PillState::HandsFree);
        s.mark_command();
        assert_eq!(s.pill(), PillState::Command);
    }

    #[test]
    fn notice_shows_until_expired_then_work_resumes() {
        let now = Instant::now();
        let mut s = DictationStatus::default();
        s.recording_started(false);
        s.recording_submitted();
        s.recording_started(false);
        s.recording_submitted();
        s.dictation_finished(Some(problem("Mislukt")), now);
        assert_eq!(s.pill(), problem("Mislukt"));
        assert!(s.expire_notice(now + PROBLEM_FOR));
        assert_eq!(s.pill(), PillState::Working);
        s.dictation_finished(None, now);
        assert_eq!(s.pill(), PillState::Hidden);
    }

    #[test]
    fn each_notice_stays_long_enough_to_read_it() {
        let now = Instant::now();
        for (notice, stays) in [
            (inserted(), ms(900)),
            (
                PillState::Copied {
                    paste_key: "⌘V".into(),
                },
                ms(2400),
            ),
            (problem("No connection"), ms(3500)),
        ] {
            let mut s = DictationStatus::default();
            s.show_notice(notice.clone(), now);
            assert_eq!(s.notice_deadline(), Some(now + stays));
            assert!(!s.expire_notice(now + stays - ms(1)));
            assert_eq!(s.pill(), notice);
            assert!(s.expire_notice(now + stays));
            assert_eq!(s.pill(), PillState::Hidden);
            assert_eq!(s.notice_deadline(), None);
            assert!(!s.expire_notice(now + stays));
        }
    }

    #[test]
    fn a_confirmation_never_hides_a_problem_that_is_still_readable() {
        let now = Instant::now();
        let mut s = DictationStatus::default();
        s.show_notice(problem("Cleanup failed"), now);
        s.show_notice(inserted(), now + ms(100));
        assert_eq!(s.pill(), problem("Cleanup failed"));
        assert_eq!(s.notice_deadline(), Some(now + PROBLEM_FOR));
        s.show_notice(problem("Not pasted"), now + ms(200));
        assert_eq!(s.pill(), problem("Not pasted"));
        assert!(s.expire_notice(now + ms(200) + PROBLEM_FOR));
        s.show_notice(inserted(), now + ms(5000));
        assert_eq!(s.pill(), inserted());
        s.show_notice(problem("Copying failed"), now + ms(5100));
        assert_eq!(s.pill(), problem("Copying failed"));
    }

    #[test]
    fn delivery_keeps_the_pill_working_until_the_words_land() {
        let now = Instant::now();
        let mut s = DictationStatus::default();
        s.recording_started(false);
        s.recording_submitted();
        s.delivery_started();
        s.dictation_finished(None, now);
        assert_eq!(s.pill(), PillState::Working);
        s.delivery_finished(inserted(), now);
        assert_eq!(s.pill(), inserted());
        assert!(s.expire_notice(now + INSERTED_FOR));
        assert_eq!(s.pill(), PillState::Hidden);
        s.delivery_finished(inserted(), now);
        assert!(s.expire_notice(now + INSERTED_FOR));
        assert_eq!(s.pill(), PillState::Hidden);
    }

    #[test]
    fn no_sound_shows_while_listening_but_never_over_a_command() {
        let mut s = DictationStatus::default();
        s.set_no_sound(true);
        assert_eq!(s.pill(), PillState::Hidden);
        s.recording_started(false);
        s.set_no_sound(true);
        assert_eq!(s.pill(), PillState::NoSound);
        s.set_no_sound(false);
        assert_eq!(s.pill(), PillState::Listening);
        s.set_no_sound(true);
        s.mark_command();
        assert_eq!(s.pill(), PillState::Command);
        s.recording_started(true);
        assert_eq!(s.pill(), PillState::HandsFree);
        s.set_no_sound(true);
        assert_eq!(s.pill(), PillState::NoSound);
        s.recording_submitted();
        assert_eq!(s.pill(), PillState::Working);
        s.recording_started(false);
        assert_eq!(s.pill(), PillState::Listening);
    }

    #[test]
    fn two_seconds_without_signal_means_no_sound_and_any_signal_clears_it() {
        let start = Instant::now();
        let mut watch = NoSoundWatch::default();
        assert_eq!(watch.level(0.0, start), None);
        assert_eq!(watch.level(0.0, start + ms(1999)), None);
        assert_eq!(watch.level(0.0, start + ms(2000)), Some(true));
        assert_eq!(watch.level(0.0, start + ms(2500)), None);
        assert_eq!(watch.level(0.05, start + ms(2600)), Some(false));
        assert_eq!(watch.level(0.05, start + ms(2700)), None);
        assert_eq!(watch.level(0.0, start + ms(2800)), None);
        assert_eq!(watch.level(0.0, start + ms(4799)), None);
        assert_eq!(watch.level(0.0, start + ms(4800)), Some(true));
    }

    #[test]
    fn a_quiet_room_is_not_mistaken_for_a_silent_microphone() {
        let start = Instant::now();
        let mut watch = NoSoundWatch::default();
        for tick in 0..100 {
            assert_eq!(watch.level(0.002, start + ms(tick * 50)), None);
        }
        let mut muted = NoSoundWatch::default();
        assert_eq!(muted.level(0.00002, start), None);
        assert_eq!(muted.level(0.00002, start + ms(2000)), Some(true));
    }

    #[test]
    fn mark_command_without_recording_is_ignored() {
        let mut s = DictationStatus::default();
        s.mark_command();
        assert_eq!(s.pill(), PillState::Hidden);
    }

    #[test]
    fn finishing_more_than_started_never_underflows() {
        let mut s = DictationStatus::default();
        s.dictation_finished(None, Instant::now());
        assert_eq!(s.pill(), PillState::Hidden);
    }

    #[test]
    fn shift_only_counts_while_key_held() {
        let mut c = CommandIntent::default();
        c.shift(true, false);
        assert!(!c.is_command());
        c.shift(true, true);
        c.shift(false, true);
        assert!(c.is_command());
        c.reset();
        assert!(!c.is_command());
    }

    #[test]
    fn pill_state_serializes_for_the_webview() {
        let json = |state: &PillState| serde_json::to_string(state).unwrap();
        assert_eq!(json(&PillState::Listening), r#"{"state":"listening"}"#);
        assert_eq!(json(&PillState::HandsFree), r#"{"state":"hands_free"}"#);
        assert_eq!(json(&PillState::NoSound), r#"{"state":"no_sound"}"#);
        assert_eq!(json(&inserted()), r#"{"state":"inserted","app":"Mail"}"#);
        assert_eq!(
            json(&PillState::Inserted { app: None }),
            r#"{"state":"inserted"}"#
        );
        assert_eq!(
            json(&PillState::Copied {
                paste_key: "Ctrl+V".into()
            }),
            r#"{"state":"copied","paste_key":"Ctrl+V"}"#
        );
        assert_eq!(
            json(&problem("No connection")),
            r#"{"state":"problem","title":"No connection","detail":"Your words are kept in History"}"#
        );
        assert_eq!(
            json(&PillState::Problem {
                title: "No microphone found".into(),
                detail: None
            }),
            r#"{"state":"problem","title":"No microphone found"}"#
        );
    }
}
