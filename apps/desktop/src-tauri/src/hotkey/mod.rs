//! System-wide listening for the dictation key. Platform hooks translate raw key events into [`Key`]s;
//! [`TriggerTracker`] turns those into edges for the gesture, interrupts and Shift changes.
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use pennel_core::HotkeyTrigger;

use crate::gesture::KeyEdge;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    LeftSuper,
    RightSuper,
    LeftShift,
    RightShift,
    Fn,
    /// Caps/Num Lock and other modifier-only events never interrupt a hold.
    OtherModifier,
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyEvent {
    Edge(KeyEdge, f64),
    Interrupt,
    Shift(bool),
    /// Numeric ids cross the native boundary: 1 Record, 2 Open, 3 Cancel, 4 Next model.
    Action(u32),
    MeetingShortcut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerBinding {
    pub mods: Vec<String>,
    pub key: Option<u32>,
}
impl TriggerBinding {
    pub fn resolve(
        binding: &pennel_core::ShortcutBinding,
        code: impl FnOnce(&str) -> Option<u32>,
    ) -> Option<Self> {
        Some(Self {
            mods: binding.mods.clone(),
            key: match binding.code.as_deref() {
                Some(key) => Some(code(key)?),
                None => None,
            },
        })
    }
}
#[derive(Debug, Default)]
pub struct RoutedKey {
    pub events: Vec<HotkeyEvent>,
    pub withhold: bool,
}
impl RoutedKey {
    pub fn iter(&self) -> std::slice::Iter<'_, HotkeyEvent> {
        self.events.iter()
    }
}

#[derive(Debug, Default)]
pub struct TriggerTracker {
    // Retained for the legacy platform adapters until their binding migration.
    trigger: Option<HotkeyTrigger>,
    binding: Option<TriggerBinding>,
    pressed: HashSet<Key>,
    withheld: HashSet<Key>,
    active: bool,
    listening: bool,
    listening_bindings: Vec<(u32, TriggerBinding)>,
    armed: HashMap<u32, bool>,
    capturing: bool,
}
impl TriggerTracker {
    pub fn new(trigger: HotkeyTrigger) -> Self {
        let binding = pennel_core::legacy_dictate(trigger, pennel_core::ShortcutPlatform::Windows);
        Self {
            trigger: Some(trigger),
            binding: TriggerBinding::resolve(&binding, |_| None),
            ..Self::default()
        }
    }
    pub fn from_binding(binding: Option<TriggerBinding>) -> Self {
        Self {
            binding,
            ..Self::default()
        }
    }
    pub fn set_trigger(&mut self, trigger: HotkeyTrigger) {
        let withheld = std::mem::take(&mut self.withheld);
        *self = Self::new(trigger);
        self.withheld = withheld;
    }
    pub fn set_binding(&mut self, binding: Option<TriggerBinding>) {
        self.binding = binding;
        self.trigger = None;
        self.active = false;
        self.armed.clear();
    }
    pub fn set_listening_bindings(&mut self, bindings: Vec<(u32, TriggerBinding)>) {
        self.listening_bindings = bindings;
        self.armed.clear();
    }
    pub fn set_listening(&mut self, listening: bool) {
        if self.listening != listening {
            self.armed.clear();
        }
        self.listening = listening;
    }
    pub fn set_capture(&mut self, capturing: bool) {
        self.capturing = capturing;
        self.active = false;
        self.armed.clear();
        if !capturing {
            self.pressed.clear();
        }
    }
    pub fn mouse_down(&mut self) {
        self.armed.clear();
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn is_shift_down(&self) -> bool {
        self.pressed.contains(&Key::LeftShift) || self.pressed.contains(&Key::RightShift)
    }
    fn modifier(&self, name: &str) -> bool {
        let held = |key| self.pressed.contains(&key);
        match name {
            "ctrl" => held(Key::LeftCtrl) || held(Key::RightCtrl),
            "left_ctrl" => held(Key::LeftCtrl),
            "right_ctrl" => held(Key::RightCtrl),
            "alt" => held(Key::LeftAlt) || held(Key::RightAlt),
            "left_alt" => held(Key::LeftAlt),
            "right_alt" => held(Key::RightAlt),
            "meta" => held(Key::LeftSuper) || held(Key::RightSuper),
            "left_meta" => held(Key::LeftSuper),
            "right_meta" => held(Key::RightSuper),
            "shift" => self.is_shift_down(),
            "left_shift" => held(Key::LeftShift),
            "right_shift" => held(Key::RightShift),
            "fn" => held(Key::Fn),
            _ => false,
        }
    }
    fn matches(&self, binding: &TriggerBinding, exact: bool) -> bool {
        if !binding.mods.iter().all(|m| self.modifier(m)) {
            return false;
        }
        if binding
            .key
            .is_some_and(|key| !self.pressed.contains(&Key::Other(key)))
        {
            return false;
        }
        if !exact {
            return true;
        }
        ["ctrl", "alt", "meta", "shift", "fn"].iter().all(|family| {
            !self.modifier(family)
                || binding
                    .mods
                    .iter()
                    .any(|m| m == family || m.ends_with(&format!("_{family}")))
                || (self.active
                    && self.binding.as_ref().is_some_and(|hold| {
                        hold.mods
                            .iter()
                            .any(|m| m == family || m.ends_with(&format!("_{family}")))
                    }))
                || (self.active && *family == "shift")
        })
    }
    pub fn key(&mut self, key: Key, down: bool, time: f64) -> Vec<HotkeyEvent> {
        self.route(key, down, time).events
    }
    pub fn route(&mut self, key: Key, down: bool, time: f64) -> RoutedKey {
        let before: Vec<_> = self
            .listening_bindings
            .iter()
            .map(|(id, b)| (*id, self.matches(b, true)))
            .collect();
        let changed = if down {
            self.pressed.insert(key)
        } else {
            self.pressed.remove(&key)
        };
        let mut withhold = self.withheld.contains(&key);
        if !down {
            self.withheld.remove(&key);
        }
        if self.capturing {
            if down {
                self.withheld.insert(key);
            }
            return RoutedKey {
                events: Vec::new(),
                withhold: true,
            };
        }
        let mut events = Vec::new();
        let mut action_key = false;
        if self.listening {
            // Any ordinary key or click invalidates a pending modifier-only action.
            if matches!(key, Key::Other(_)) && down {
                self.armed.clear();
            }
            for (id, binding) in &self.listening_bindings {
                let engaged = self.matches(binding, true);
                if let Some(code) = binding.key {
                    if key == Key::Other(code) && down && engaged {
                        action_key = true;
                        withhold = true;
                        self.withheld.insert(key);
                        if changed {
                            events.push(HotkeyEvent::Action(*id));
                        }
                    }
                } else {
                    let was = before
                        .iter()
                        .find(|(old, _)| old == id)
                        .is_some_and(|(_, engaged)| *engaged);
                    if engaged && !was && changed {
                        self.armed.insert(*id, true);
                    }
                    if was && !engaged && self.armed.remove(id) == Some(true) && !down {
                        events.push(HotkeyEvent::Action(*id));
                    }
                }
            }
        }
        if matches!(key, Key::LeftShift | Key::RightShift) && changed {
            events.push(HotkeyEvent::Shift(self.is_shift_down()));
        }
        let own_key = self
            .binding
            .as_ref()
            .and_then(|b| b.key)
            .is_some_and(|code| key == Key::Other(code));
        if own_key
            && down
            && self
                .binding
                .as_ref()
                .is_some_and(|b| self.matches(b, false))
        {
            withhold = true;
            self.withheld.insert(key);
        }
        if matches!(key, Key::Other(_))
            && down
            && self.active
            && self.binding.as_ref().is_some_and(|b| b.key.is_none())
            && !action_key
        {
            events.push(HotkeyEvent::Interrupt);
        }
        let engaged = self
            .binding
            .as_ref()
            .is_some_and(|b| self.matches(b, false));
        if engaged != self.active {
            self.active = engaged;
            events.push(HotkeyEvent::Edge(
                if engaged { KeyEdge::Down } else { KeyEdge::Up },
                time,
            ));
        }
        RoutedKey { events, withhold }
    }
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

pub fn now() -> f64 {
    epoch().elapsed().as_secs_f64()
}

static HELD: AtomicBool = AtomicBool::new(false);
static SHIFT: AtomicBool = AtomicBool::new(false);

pub fn trigger_held() -> bool {
    HELD.load(Ordering::SeqCst)
}

pub fn shift_held() -> bool {
    SHIFT.load(Ordering::SeqCst)
}

fn publish(tracker: &TriggerTracker) {
    HELD.store(tracker.is_active(), Ordering::SeqCst);
    SHIFT.store(tracker.is_shift_down(), Ordering::SeqCst);
}

/// Synthetic Ctrl+V/Ctrl+C must not reach the gesture as user input.
pub static INJECTING: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
pub struct Listener {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    inner: std::sync::Arc<PlatformListener>,
}

#[cfg(windows)]
type PlatformListener = windows::Hook;
#[cfg(target_os = "linux")]
type PlatformListener = linux::Hook;
#[cfg(target_os = "macos")]
type PlatformListener = crate::macos::Hotkey;

impl Listener {
    /// Starts the platform hook on its own thread; events arrive on `sender`.
    pub fn start(trigger: HotkeyTrigger, sender: Sender<HotkeyEvent>) -> Result<Self, String> {
        epoch();
        #[cfg(target_os = "macos")]
        {
            use crate::macos::NativeHotkeyEvent as Native;
            let inner = PlatformListener::start(trigger, move |native| {
                let event = match native {
                    Native::Down => {
                        HELD.store(true, Ordering::SeqCst);
                        HotkeyEvent::Edge(KeyEdge::Down, now())
                    }
                    Native::Up => {
                        HELD.store(false, Ordering::SeqCst);
                        HotkeyEvent::Edge(KeyEdge::Up, now())
                    }
                    Native::Shift(down) => {
                        SHIFT.store(down, Ordering::SeqCst);
                        HotkeyEvent::Shift(down)
                    }
                    Native::Interrupt => HotkeyEvent::Interrupt,
                    Native::MeetingShortcut => HotkeyEvent::MeetingShortcut,
                };
                let _ = sender.send(event);
            })?;
            HELD.store(inner.trigger_held(), Ordering::SeqCst);
            SHIFT.store(inner.shift_held(), Ordering::SeqCst);
            Ok(Self {
                inner: std::sync::Arc::new(inner),
            })
        }
        #[cfg(any(windows, target_os = "linux"))]
        {
            PlatformListener::start(trigger, sender).map(|inner| Self {
                inner: std::sync::Arc::new(inner),
            })
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            let _ = (trigger, sender);
            Err("shortcuts are not supported on this platform".into())
        }
    }

    pub fn set_trigger(&self, trigger: HotkeyTrigger) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        self.inner.set_trigger(trigger);
        #[cfg(target_os = "macos")]
        {
            HELD.store(false, Ordering::SeqCst);
            SHIFT.store(false, Ordering::SeqCst);
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let _ = trigger;
    }

    #[cfg(target_os = "macos")]
    pub fn set_record_shortcut_enabled(&self, enabled: bool) -> Result<(), String> {
        self.inner.set_record_shortcut_enabled(enabled)
    }
}

/// Shared by the platform hooks: feeds one key event through the tracker and forwards the results.
fn dispatch(
    tracker: &std::sync::Mutex<TriggerTracker>,
    sender: &Sender<HotkeyEvent>,
    key: Key,
    down: bool,
) -> RoutedKey {
    let mut tracker = tracker.lock().unwrap_or_else(|p| p.into_inner());
    let routed = tracker.route(key, down, now());
    publish(&tracker);
    for event in &routed.events {
        let _ = sender.send(*event);
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::HotkeyEvent::*;
    use super::*;

    fn press(t: &mut TriggerTracker, key: Key) -> Vec<HotkeyEvent> {
        t.key(key, true, 1.0)
    }

    fn release(t: &mut TriggerTracker, key: Key) -> Vec<HotkeyEvent> {
        t.key(key, false, 2.0)
    }

    fn keyed_hold() -> TriggerBinding {
        TriggerBinding {
            mods: vec!["ctrl".into(), "alt".into()],
            key: Some(32),
        }
    }

    #[test]
    fn keyed_hold_withholds_repeats_and_release_after_modifier_release() {
        let mut t = TriggerTracker::from_binding(Some(keyed_hold()));
        press(&mut t, Key::LeftCtrl);
        press(&mut t, Key::RightAlt);
        let first = t.route(Key::Other(32), true, 1.0);
        assert_eq!(first.events, [Edge(KeyEdge::Down, 1.0)]);
        assert!(first.withhold);
        let repeat = t.route(Key::Other(32), true, 1.1);
        assert!(repeat.withhold);
        assert!(repeat.events.is_empty());
        assert_eq!(release(&mut t, Key::LeftCtrl), [Edge(KeyEdge::Up, 2.0)]);
        assert!(t.route(Key::Other(32), false, 3.0).withhold);
        assert!(!t.route(Key::Other(32), false, 3.1).withhold);
    }

    #[test]
    fn keyed_hold_keeps_foreign_keys_and_listening_actions_out_of_interrupts() {
        let mut t = TriggerTracker::from_binding(Some(keyed_hold()));
        press(&mut t, Key::LeftCtrl);
        press(&mut t, Key::LeftAlt);
        press(&mut t, Key::Other(32));
        assert_eq!(press(&mut t, Key::Other(65)), []);
        t.set_listening_bindings(vec![(
            3,
            TriggerBinding {
                mods: vec![],
                key: Some(27),
            },
        )]);
        assert!(!t.route(Key::Other(27), true, 2.0).withhold);
        release(&mut t, Key::Other(27));
        t.set_listening(true);
        let cancel = t.route(Key::Other(27), true, 3.0);
        assert!(cancel.withhold);
        assert_eq!(cancel.events, [Action(3)]);
        t.set_listening(false);
        assert!(t.route(Key::Other(27), false, 3.1).withhold);
    }

    #[test]
    fn bare_listening_modifier_fires_on_clean_release_and_capture_suspends_every_action() {
        let mut t = TriggerTracker::from_binding(None);
        t.set_listening_bindings(vec![(
            4,
            TriggerBinding {
                mods: vec!["left_alt".into()],
                key: None,
            },
        )]);
        t.set_listening(true);
        assert_eq!(press(&mut t, Key::LeftAlt), []);
        assert_eq!(release(&mut t, Key::LeftAlt), [Action(4)]);
        press(&mut t, Key::LeftAlt);
        press(&mut t, Key::Other(65));
        assert_eq!(release(&mut t, Key::LeftAlt), []);
        release(&mut t, Key::Other(65));
        press(&mut t, Key::LeftAlt);
        t.mouse_down();
        assert_eq!(release(&mut t, Key::LeftAlt), []);
        t.set_capture(true);
        assert!(t.route(Key::Other(27), true, 1.0).withhold);
        assert!(press(&mut t, Key::RightCtrl).is_empty());
        t.set_capture(false);
        assert!(t.route(Key::Other(27), false, 2.0).withhold);
    }

    #[test]
    fn compatibility_constructor_matches_all_legacy_presets() {
        for trigger in [
            HotkeyTrigger::Fn,
            HotkeyTrigger::RightControl,
            HotkeyTrigger::RightOption,
            HotkeyTrigger::RightCommand,
            HotkeyTrigger::ControlSuper,
        ] {
            let mut legacy = TriggerTracker::new(trigger);
            let binding =
                pennel_core::legacy_dictate(trigger, pennel_core::ShortcutPlatform::Windows);
            let mut table =
                TriggerTracker::from_binding(TriggerBinding::resolve(&binding, |_| None));
            for (key, down) in [
                (Key::LeftCtrl, true),
                (Key::RightCtrl, true),
                (Key::LeftSuper, true),
                (Key::RightAlt, true),
                (Key::RightSuper, true),
                (Key::LeftShift, true),
                (Key::Other(65), true),
                (Key::Other(65), false),
                (Key::RightCtrl, false),
                (Key::RightSuper, false),
                (Key::RightAlt, false),
                (Key::LeftCtrl, false),
                (Key::LeftSuper, false),
            ] {
                assert_eq!(
                    legacy.key(key, down, 1.0),
                    table.key(key, down, 1.0),
                    "{trigger:?} {key:?}"
                );
            }
        }
    }

    #[test]
    fn disabled_hold_has_no_edges_and_reconfiguration_keeps_withheld_release() {
        let mut t = TriggerTracker::from_binding(Some(keyed_hold()));
        press(&mut t, Key::LeftCtrl);
        press(&mut t, Key::LeftAlt);
        assert!(t.route(Key::Other(32), true, 1.0).withhold);
        t.set_binding(None);
        assert!(t.route(Key::Other(32), false, 2.0).withhold);
        assert!(press(&mut t, Key::RightCtrl).is_empty());
    }

    #[test]
    fn right_ctrl_produces_edges_and_ignores_auto_repeat() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightControl);
        assert_eq!(press(&mut t, Key::RightCtrl), [Edge(KeyEdge::Down, 1.0)]);
        assert_eq!(press(&mut t, Key::RightCtrl), []);
        assert_eq!(release(&mut t, Key::RightCtrl), [Edge(KeyEdge::Up, 2.0)]);
    }

    #[test]
    fn left_ctrl_does_not_trigger_right_ctrl() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightControl);
        assert_eq!(press(&mut t, Key::LeftCtrl), []);
        assert_eq!(release(&mut t, Key::LeftCtrl), []);
    }

    #[test]
    fn other_key_while_held_interrupts() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightOption);
        press(&mut t, Key::RightAlt);
        assert_eq!(press(&mut t, Key::Other(99)), [Interrupt]);
        assert_eq!(press(&mut t, Key::LeftCtrl), []);
        assert_eq!(press(&mut t, Key::OtherModifier), []);
    }

    #[test]
    fn other_key_without_trigger_is_ignored() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightControl);
        assert_eq!(press(&mut t, Key::Other(99)), []);
    }

    #[test]
    fn shift_is_reported_separately() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightControl);
        press(&mut t, Key::RightCtrl);
        assert_eq!(press(&mut t, Key::LeftShift), [Shift(true)]);
        assert_eq!(press(&mut t, Key::RightShift), [Shift(true)]);
        assert_eq!(release(&mut t, Key::LeftShift), [Shift(true)]);
        assert_eq!(release(&mut t, Key::RightShift), [Shift(false)]);
        assert!(t.is_active());
    }

    #[test]
    fn chord_needs_ctrl_and_super_in_any_order() {
        let mut t = TriggerTracker::new(HotkeyTrigger::ControlSuper);
        assert_eq!(press(&mut t, Key::LeftSuper), []);
        assert_eq!(press(&mut t, Key::LeftCtrl), [Edge(KeyEdge::Down, 1.0)]);
        assert_eq!(release(&mut t, Key::LeftSuper), [Edge(KeyEdge::Up, 2.0)]);
        assert_eq!(release(&mut t, Key::LeftCtrl), []);
        assert_eq!(press(&mut t, Key::RightCtrl), []);
        assert_eq!(press(&mut t, Key::RightSuper), [Edge(KeyEdge::Down, 1.0)]);
    }

    #[test]
    fn changing_trigger_resets_state() {
        let mut t = TriggerTracker::new(HotkeyTrigger::RightControl);
        press(&mut t, Key::RightCtrl);
        t.set_trigger(HotkeyTrigger::RightOption);
        assert!(!t.is_active());
        assert_eq!(release(&mut t, Key::RightCtrl), []);
    }
}
