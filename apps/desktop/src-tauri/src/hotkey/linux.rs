//! XRecord observes modifier-only dictation/listening and focused capture. Keyed global grabs stay disabled.
use super::{
    CaptureSlot, HotkeyEvent, INJECTING, Key, TriggerTracker, capture_callback, publish,
    route_shared,
};
use crate::gesture::KeyEdge;
use pennel_core::{Settings, ShortcutPlatform};
use rdev::KeyboardState;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
pub struct Hook {
    tracker: Arc<Mutex<TriggerTracker>>,
    capture: Arc<CaptureSlot>,
    sender: Sender<HotkeyEvent>,
}
fn modifier(key: rdev::Key) -> Option<Key> {
    use rdev::Key as K;
    Some(match key {
        K::ControlLeft => Key::LeftCtrl,
        K::ControlRight => Key::RightCtrl,
        K::Alt => Key::LeftAlt,
        K::AltGr => Key::RightAlt,
        K::MetaLeft => Key::LeftSuper,
        K::MetaRight => Key::RightSuper,
        K::ShiftLeft => Key::LeftShift,
        K::ShiftRight => Key::RightShift,
        K::CapsLock | K::NumLock | K::ScrollLock | K::Function => Key::OtherModifier,
        _ => return None,
    })
}
fn physical_code(key: rdev::Key) -> Option<String> {
    use rdev::Key as K;
    let code = match key {
        K::Num0 => "Digit0",
        K::Num1 => "Digit1",
        K::Num2 => "Digit2",
        K::Num3 => "Digit3",
        K::Num4 => "Digit4",
        K::Num5 => "Digit5",
        K::Num6 => "Digit6",
        K::Num7 => "Digit7",
        K::Num8 => "Digit8",
        K::Num9 => "Digit9",
        K::DownArrow => "ArrowDown",
        K::UpArrow => "ArrowUp",
        K::LeftArrow => "ArrowLeft",
        K::RightArrow => "ArrowRight",
        K::Return => "Enter",
        K::BackQuote => "Backquote",
        K::SemiColon => "Semicolon",
        K::BackSlash => "Backslash",
        K::LeftBracket => "BracketLeft",
        K::RightBracket => "BracketRight",
        K::Dot => "Period",
        _ => {
            return {
                let code = format!("{key:?}");
                pennel_core::supported_shortcut_code(&code).then_some(code)
            };
        }
    };
    Some(code.into())
}
impl Hook {
    pub fn start(settings: &Settings, sender: Sender<HotkeyEvent>) -> Result<Self, String> {
        if crate::linux_input::is_wayland() {
            return Err(crate::linux_input::SHORTCUT_HELP.into());
        }
        if !crate::linux_input::has_x11_display() {
            return Err("no X11 display (DISPLAY) found".into());
        }
        let tracker = Arc::new(Mutex::new(TriggerTracker::default()));
        let capture = Arc::new(Mutex::new(None));
        let hook = Self {
            tracker: tracker.clone(),
            capture: capture.clone(),
            sender: sender.clone(),
        };
        hook.set_bindings(settings, false);
        let (error, receive) = std::sync::mpsc::channel::<String>();
        std::thread::Builder::new().name("pennel-hotkey".into()).spawn(move || {
            // A separate state reader stays unmodified, so Shift/Caps/IME state is never mutated for capture.
            let mut keyboard = rdev::Keyboard::new();
            let mut pressed: HashMap<rdev::Key, (Key, Option<String>, Option<String>)> = HashMap::new();
            let result = rdev::listen(move |event| {
                if INJECTING.load(Ordering::SeqCst) { return; }
                if matches!(event.event_type, rdev::EventType::ButtonPress(_)) { tracker.lock().unwrap_or_else(|p| p.into_inner()).mouse_down(); return; }
                let (raw, down) = match event.event_type { rdev::EventType::KeyPress(key)=>(key,true),rdev::EventType::KeyRelease(key)=>(key,false),_=>return };
                let repeated = down && pressed.contains_key(&raw);
                let input = if down && repeated { pressed.get(&raw).cloned() } else if !down { pressed.remove(&raw) } else { None };
                let needs_names = tracker.lock().unwrap_or_else(|p| p.into_inner()).needs_key_names();
                let (key, code, logical) = input.unwrap_or_else(|| {
                    if let Some(key) = modifier(raw) { return (key,None,None); }
                    let physical = physical_code(raw);
                    if !needs_names {
                        let number = physical.as_deref().and_then(crate::portable_shortcuts::virtual_key).unwrap_or(u32::MAX);
                        return (Key::Other(number),physical,None);
                    }
                    let named = physical.as_deref().filter(|code| code.starts_with('F') || matches!(*code,"Escape"|"Space"|"Tab"|"Enter"|"Backspace"|"Delete"|"Insert"|"Home"|"End"|"PageUp"|"PageDown"|"ArrowUp"|"ArrowDown"|"ArrowLeft"|"ArrowRight")).map(str::to_owned);
                    let logical = named.or_else(|| keyboard.as_mut().and_then(|keyboard| { keyboard.reset(); keyboard.add(&rdev::EventType::KeyPress(raw)) })).unwrap_or_else(|| "Unidentified".into());
                    let code = crate::portable_shortcuts::code_for_character(&logical).or_else(|| physical.filter(|code| matches!(logical.as_str(),"Escape"|"Space"|"Tab"|"Enter"|"Backspace"|"Delete"|"Insert"|"Home"|"End"|"PageUp"|"PageDown"|"ArrowUp"|"ArrowDown"|"ArrowLeft"|"ArrowRight") || code.starts_with('F'))).unwrap_or_else(|| "Unidentified".into());
                    let number = crate::portable_shortcuts::virtual_key(&code).unwrap_or(u32::MAX);
                    (Key::Other(number),Some(code),Some(logical))
                });
                if down { pressed.entry(raw).or_insert_with(|| (key,code.clone(),logical.clone())); }
                let routed = route_shared(&tracker, key, down);
                let (capturing, mods) = { let state = tracker.lock().unwrap_or_else(|p| p.into_inner()); (state.capture_active(), state.capture_modifiers()) };
                if capturing && !repeated { capture_callback(&capture, serde_json::json!({"mods":mods,"code":code,"key":logical,"down":down,"modifier":!matches!(key,Key::Other(_))})); }
                for event in routed.events {
                    // XRecord cannot withhold a key; never dispatch a keyed action without a safe owned grab backend.
                    if matches!(key, Key::Other(_)) && matches!(event, HotkeyEvent::Action(_)) { continue; }
                    let _ = sender.send(event);
                }
            });
            if let Err(failure) = result { let _ = error.send(format!("X11 keyboard unavailable: {failure:?}")); }
        }).map_err(|error| error.to_string())?;
        match receive.recv_timeout(std::time::Duration::from_millis(300)) {
            Ok(error) => Err(error),
            Err(_) => Ok(hook),
        }
    }
    pub fn set_bindings(&self, settings: &Settings, next_model: bool) {
        let plan = crate::portable_shortcuts::plan(
            settings,
            ShortcutPlatform::LinuxX11,
            false,
            false,
            next_model,
        );
        let mut tracker = self.tracker.lock().unwrap_or_else(|p| p.into_inner());
        let reset = tracker.is_active() && tracker.binding != plan.hold;
        tracker.set_binding(plan.hold);
        tracker.set_listening_bindings(plan.listening);
        publish(&tracker);
        drop(tracker);
        if reset {
            let _ = self.sender.send(HotkeyEvent::Interrupt);
            let _ = self
                .sender
                .send(HotkeyEvent::Edge(KeyEdge::Up, super::now()));
        }
    }
    pub fn set_listening(&self, listening: bool) -> Result<(), String> {
        self.tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_listening(listening);
        Ok(())
    }
    pub fn set_capture(
        &self,
        callback: Option<Box<dyn FnMut(serde_json::Value) + Send>>,
    ) -> Result<(), String> {
        let start = callback.is_some();
        if start {
            *self.capture.lock().unwrap_or_else(|p| p.into_inner()) =
                callback.map(|callback| Arc::new(Mutex::new(callback)));
        }
        let mut tracker = self.tracker.lock().unwrap_or_else(|p| p.into_inner());
        tracker.set_capture(start);
        publish(&tracker);
        drop(tracker);
        if !start {
            *self.capture.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
        Ok(())
    }
}
