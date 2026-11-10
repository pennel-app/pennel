//! Port of `PennelKit/HotkeyGesture.swift`: hold to talk, double-tap for hands-free.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdge {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureAction {
    Start,
    Submit,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Manual {
        started_at: f64,
    },
    Holding {
        pressed_at: f64,
    },
    HandsFree {
        started_at: f64,
        awaiting_release: bool,
    },
    AwaitingRelease,
}

#[derive(Debug, Clone)]
pub struct HotkeyGesture {
    tap_max: f64,
    double_tap_window: f64,
    max_duration: f64,
    phase: Phase,
    last_tap_up: Option<f64>,
    hands_free: bool,
    trigger_down: bool,
}

impl Default for HotkeyGesture {
    fn default() -> Self {
        Self::new(0.25, 0.35, 600.0)
    }
}

impl HotkeyGesture {
    pub fn new(tap_max: f64, double_tap_window: f64, max_duration: f64) -> Self {
        Self {
            tap_max,
            double_tap_window,
            max_duration,
            phase: Phase::Idle,
            last_tap_up: None,
            hands_free: false,
            trigger_down: false,
        }
    }

    pub fn is_hands_free(&self) -> bool {
        self.hands_free
    }

    pub fn is_manual(&self) -> bool {
        matches!(self.phase, Phase::Manual { .. })
    }

    /// Explicit UI/desktop shortcuts do not depend on observing a physical modifier release.
    pub fn manual_toggle(&mut self, t: f64) -> Vec<GestureAction> {
        self.last_tap_up = None;
        match self.phase {
            Phase::Idle | Phase::AwaitingRelease => {
                self.phase = Phase::Manual { started_at: t };
                self.hands_free = true;
                vec![GestureAction::Start]
            }
            Phase::Holding { .. }
            | Phase::HandsFree {
                awaiting_release: true,
                ..
            } => {
                self.phase = Phase::AwaitingRelease;
                self.hands_free = false;
                vec![GestureAction::Submit]
            }
            Phase::Manual { .. }
            | Phase::HandsFree {
                awaiting_release: false,
                ..
            } => {
                self.phase = Phase::Idle;
                self.hands_free = false;
                vec![GestureAction::Submit]
            }
        }
    }

    pub fn handle(&mut self, edge: KeyEdge, t: f64) -> Vec<GestureAction> {
        self.trigger_down = edge == KeyEdge::Down;
        match (edge, self.phase) {
            (KeyEdge::Down, Phase::Idle) => {
                if self
                    .last_tap_up
                    .is_some_and(|up| t - up <= self.double_tap_window)
                {
                    self.last_tap_up = None;
                    self.hands_free = true;
                    self.phase = Phase::HandsFree {
                        started_at: t,
                        awaiting_release: true,
                    };
                } else {
                    self.phase = Phase::Holding { pressed_at: t };
                }
                vec![GestureAction::Start]
            }
            (
                KeyEdge::Down,
                Phase::HandsFree {
                    awaiting_release: false,
                    ..
                },
            ) => {
                self.hands_free = false;
                self.phase = Phase::AwaitingRelease;
                vec![GestureAction::Submit]
            }
            (KeyEdge::Up, Phase::Holding { pressed_at }) => {
                self.phase = Phase::Idle;
                if t - pressed_at >= self.tap_max {
                    return vec![GestureAction::Submit];
                }
                self.last_tap_up = Some(t);
                vec![GestureAction::Cancel]
            }
            (
                KeyEdge::Up,
                Phase::HandsFree {
                    started_at,
                    awaiting_release: true,
                },
            ) => {
                self.phase = Phase::HandsFree {
                    started_at,
                    awaiting_release: false,
                };
                vec![]
            }
            (KeyEdge::Up, Phase::AwaitingRelease) => {
                self.phase = Phase::Idle;
                vec![]
            }
            _ => vec![],
        }
    }

    /// Cancels listening, including hands-free and button sessions. Processing has no active phase.
    pub fn cancel(&mut self) -> Vec<GestureAction> {
        if matches!(self.phase, Phase::Idle | Phase::AwaitingRelease) {
            return vec![];
        }
        self.phase = if self.trigger_down {
            Phase::AwaitingRelease
        } else {
            Phase::Idle
        };
        self.hands_free = false;
        self.last_tap_up = None;
        vec![GestureAction::Cancel]
    }

    /// Another key pressed while holding means the hotkey is used as a modifier (Ctrl+C), not to dictate.
    pub fn interrupt(&mut self) -> Vec<GestureAction> {
        if !matches!(self.phase, Phase::Holding { .. }) {
            return vec![];
        }
        self.phase = Phase::AwaitingRelease;
        vec![GestureAction::Cancel]
    }

    pub fn tick(&mut self, t: f64) -> Vec<GestureAction> {
        match self.phase {
            Phase::Manual { started_at } if t - started_at >= self.max_duration => {
                self.hands_free = false;
                self.phase = Phase::Idle;
            }
            Phase::Holding { pressed_at } if t - pressed_at >= self.max_duration => {
                self.phase = Phase::AwaitingRelease;
            }
            Phase::HandsFree {
                started_at,
                awaiting_release,
            } if t - started_at >= self.max_duration => {
                self.hands_free = false;
                self.phase = if awaiting_release {
                    Phase::AwaitingRelease
                } else {
                    Phase::Idle
                };
            }
            _ => return vec![],
        }
        vec![GestureAction::Submit]
    }
}

#[cfg(test)]
mod tests {
    use super::GestureAction::*;
    use super::KeyEdge::*;
    use super::*;

    #[test]
    fn explicit_cancel_covers_hold_hands_free_and_manual_and_waits_for_release() {
        let mut g = HotkeyGesture::default();
        assert_eq!(g.cancel(), []);
        g.handle(Down, 0.0);
        assert_eq!(g.cancel(), [Cancel]);
        assert_eq!(g.handle(Down, 0.1), []);
        assert_eq!(g.handle(Up, 0.2), []);
        g.handle(Down, 1.0);
        g.handle(Up, 1.1);
        g.handle(Down, 1.2);
        g.handle(Up, 1.3);
        assert!(g.is_hands_free());
        assert_eq!(g.cancel(), [Cancel]);
        assert!(!g.is_hands_free());
        assert_eq!(g.handle(Down, 1.4), [Start]);
        assert!(!g.is_hands_free());
        g.handle(Up, 2.0);
        g.manual_toggle(3.0);
        g.handle(Down, 3.1);
        assert_eq!(g.cancel(), [Cancel]);
        assert_eq!(g.handle(Down, 3.2), []);
        assert_eq!(g.handle(Up, 3.3), []);
        assert_eq!(g.handle(Down, 4.0), [Start]);
    }

    #[test]
    fn hold_then_release_submits() {
        let mut g = HotkeyGesture::default();
        assert_eq!(g.handle(Down, 0.0), [Start]);
        assert_eq!(g.handle(Up, 1.0), [Submit]);
    }

    #[test]
    fn short_tap_cancels() {
        let mut g = HotkeyGesture::default();
        assert_eq!(g.handle(Down, 0.0), [Start]);
        assert_eq!(g.handle(Up, 0.1), [Cancel]);
    }

    #[test]
    fn double_tap_enters_hands_free_and_next_press_submits() {
        let mut g = HotkeyGesture::default();
        g.handle(Down, 0.0);
        g.handle(Up, 0.1);
        assert_eq!(g.handle(Down, 0.3), [Start]);
        assert!(g.is_hands_free());
        assert_eq!(g.handle(Up, 0.4), []);
        assert_eq!(g.handle(Down, 5.0), [Submit]);
        assert!(!g.is_hands_free());
        assert_eq!(g.handle(Up, 5.1), []);
        assert_eq!(g.handle(Down, 9.0), [Start]);
        assert!(!g.is_hands_free());
    }

    #[test]
    fn slow_second_tap_is_a_new_hold() {
        let mut g = HotkeyGesture::default();
        g.handle(Down, 0.0);
        g.handle(Up, 0.1);
        g.handle(Down, 1.0);
        assert!(!g.is_hands_free());
        assert_eq!(g.handle(Up, 2.0), [Submit]);
    }

    #[test]
    fn other_key_while_holding_cancels_until_release() {
        let mut g = HotkeyGesture::default();
        g.handle(Down, 0.0);
        assert_eq!(g.interrupt(), [Cancel]);
        assert_eq!(g.handle(Up, 2.0), []);
        assert_eq!(g.handle(Down, 3.0), [Start]);
    }

    #[test]
    fn interrupt_does_not_stop_hands_free() {
        let mut g = HotkeyGesture::default();
        g.handle(Down, 0.0);
        g.handle(Up, 0.1);
        g.handle(Down, 0.2);
        g.handle(Up, 0.3);
        assert_eq!(g.interrupt(), []);
    }

    #[test]
    fn max_duration_submits() {
        let mut g = HotkeyGesture::new(0.25, 0.35, 10.0);
        g.handle(Down, 0.0);
        assert_eq!(g.tick(5.0), []);
        assert_eq!(g.tick(10.0), [Submit]);
        assert_eq!(g.handle(Up, 11.0), []);
    }

    #[test]
    fn hands_free_max_duration_submits_and_resets() {
        let mut g = HotkeyGesture::new(0.25, 0.35, 10.0);
        g.handle(Down, 0.0);
        g.handle(Up, 0.1);
        g.handle(Down, 0.2);
        g.handle(Up, 0.3);
        assert_eq!(g.tick(10.2), [Submit]);
        assert!(!g.is_hands_free());
        assert_eq!(g.handle(Down, 12.0), [Start]);
    }

    #[test]
    fn manual_toggle_starts_and_submits_without_key_edges() {
        let mut g = HotkeyGesture::default();
        assert_eq!(g.manual_toggle(1.0), [Start]);
        assert!(g.is_hands_free());
        assert!(g.is_manual());
        assert_eq!(g.manual_toggle(5.0), [Submit]);
        assert!(!g.is_hands_free());
        assert!(!g.is_manual());
        assert_eq!(g.manual_toggle(6.0), [Start]);
    }

    #[test]
    fn manual_recording_ignores_physical_keys_and_interrupts() {
        let mut g = HotkeyGesture::default();
        g.manual_toggle(1.0);
        assert_eq!(g.handle(Down, 2.0), []);
        assert_eq!(g.interrupt(), []);
        assert_eq!(g.handle(Up, 2.1), []);
        assert!(g.is_manual());
        assert_eq!(g.manual_toggle(3.0), [Submit]);
    }

    #[test]
    fn manual_recording_keeps_the_duration_limit() {
        let mut g = HotkeyGesture::new(0.25, 0.35, 10.0);
        g.manual_toggle(1.0);
        assert_eq!(g.tick(10.0), []);
        assert_eq!(g.tick(11.0), [Submit]);
        assert!(!g.is_hands_free());
        assert_eq!(g.manual_toggle(12.0), [Start]);
    }

    #[test]
    fn manual_stop_of_a_held_key_waits_for_release() {
        let mut g = HotkeyGesture::default();
        g.handle(Down, 1.0);
        assert_eq!(g.manual_toggle(2.0), [Submit]);
        assert_eq!(g.handle(Up, 2.1), []);
        assert_eq!(g.handle(Down, 3.0), [Start]);
    }
}
