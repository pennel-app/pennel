//! Portable policy projection, compiled and tested on macOS even while non-Mac builds are paused.
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
use crate::hotkey::TriggerBinding;
#[derive(Debug, Default)]
pub struct PhysicalKeys {
    down: std::collections::HashMap<u64, u32>,
}
impl PhysicalKeys {
    pub fn key(&mut self, identity: u64, key: u32, down: bool) -> (u32, bool) {
        if down {
            let repeated = self.down.contains_key(&identity);
            (*self.down.entry(identity).or_insert(key), repeated)
        } else {
            (self.down.remove(&identity).unwrap_or(key), false)
        }
    }
}
use pennel_core::{Settings, ShortcutBinding, ShortcutPlatform, effective_shortcut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub action: u32,
    pub binding: ShortcutBinding,
}
#[derive(Debug, Default)]
pub struct Plan {
    pub hold: Option<TriggerBinding>,
    pub listening: Vec<(u32, TriggerBinding)>,
    pub registrations: Vec<Registration>,
}
pub fn virtual_key(code: &str) -> Option<u32> {
    if code.len() == 4 && code.starts_with("Key") && code.as_bytes()[3].is_ascii_uppercase() {
        return Some(code.as_bytes()[3] as u32);
    }
    if code.len() == 6 && code.starts_with("Digit") && code.as_bytes()[5].is_ascii_digit() {
        return Some(code.as_bytes()[5] as u32);
    }
    if let Some(n) = code
        .strip_prefix('F')
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Some(0x6f + n);
    }
    Some(match code {
        "Escape" => 0x1b,
        "Space" => 0x20,
        "Tab" => 0x09,
        "Enter" => 0x0d,
        "Backspace" => 0x08,
        "Delete" => 0x2e,
        "Insert" => 0x2d,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ArrowUp" => 0x26,
        "ArrowDown" => 0x28,
        "ArrowLeft" => 0x25,
        "ArrowRight" => 0x27,
        "Comma" => 0xbc,
        "Period" => 0xbe,
        "Slash" => 0xbf,
        "Backslash" => 0xdc,
        "Semicolon" => 0xba,
        "Quote" => 0xde,
        "Backquote" => 0xc0,
        "Minus" => 0xbd,
        "Equal" => 0xbb,
        "BracketLeft" => 0xdb,
        "BracketRight" => 0xdd,
        "IntlBackslash" => 0xe2,
        _ => return None,
    })
}
pub fn virtual_code(key: u32) -> Option<String> {
    if (0x41..=0x5a).contains(&key) {
        return Some(format!("Key{}", char::from_u32(key)?));
    }
    if (0x30..=0x39).contains(&key) {
        return Some(format!("Digit{}", char::from_u32(key)?));
    }
    if (0x70..=0x87).contains(&key) {
        return Some(format!("F{}", key - 0x6f));
    }
    [
        "Escape",
        "Space",
        "Tab",
        "Enter",
        "Backspace",
        "Delete",
        "Insert",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "Comma",
        "Period",
        "Slash",
        "Backslash",
        "Semicolon",
        "Quote",
        "Backquote",
        "Minus",
        "Equal",
        "BracketLeft",
        "BracketRight",
        "IntlBackslash",
    ]
    .into_iter()
    .find(|code| virtual_key(code) == Some(key))
    .map(str::to_owned)
}
pub fn code_for_character(key: &str) -> Option<String> {
    if key.len() == 1 {
        let c = key.as_bytes()[0];
        if c.is_ascii_alphabetic() {
            return Some(format!("Key{}", (c as char).to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(format!("Digit{}", c as char));
        }
    }
    Some(
        match key {
            " " => "Space",
            "," => "Comma",
            "." => "Period",
            "/" => "Slash",
            "\\" => "Backslash",
            ";" => "Semicolon",
            "'" => "Quote",
            "`" => "Backquote",
            "-" => "Minus",
            "=" => "Equal",
            "[" => "BracketLeft",
            "]" => "BracketRight",
            _ => return None,
        }
        .into(),
    )
}
/// Only Windows uses this plugin. X11 needs an owned transport with acknowledged release first.
pub fn uses_registration_backend(platform: ShortcutPlatform) -> bool {
    platform == ShortcutPlatform::Windows
}
pub fn plan(
    settings: &Settings,
    platform: ShortcutPlatform,
    _listening: bool,
    capturing: bool,
    next_model: bool,
) -> Plan {
    if capturing || platform == ShortcutPlatform::LinuxWayland {
        return Plan::default();
    }
    let resolve = |id| effective_shortcut(settings, id, platform);
    let hold = resolve("dictate")
        .as_ref()
        .and_then(|binding| TriggerBinding::resolve(binding, virtual_key));
    let mut result = Plan {
        hold,
        ..Plan::default()
    };
    for (id, action) in [("record", 1), ("open_pennel", 2)] {
        if let Some(binding) = resolve(id)
            && uses_registration_backend(platform)
            && pennel_core::shortcut_capability(id, Some(&binding), platform).available
            && TriggerBinding::resolve(&binding, virtual_key).is_some()
        {
            result.registrations.push(Registration { action, binding });
        }
    }
    for (id, action) in [("cancel", 3), ("next_model", 4)] {
        if action == 4 && !next_model {
            continue;
        }
        if let Some(binding) = resolve(id)
            && pennel_core::shortcut_capability(id, Some(&binding), platform).available
            && let Some(trigger) = TriggerBinding::resolve(&binding, virtual_key)
        {
            result.listening.push((action, trigger));
        }
    }
    result
}
pub fn retain_registration(wanted: bool, pressed: bool) -> bool {
    wanted || pressed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_layout_change_keeps_the_original_physical_release_identity() {
        let mut keys = PhysicalKeys::default();
        assert_eq!(keys.key(12, 0x41, true), (0x41, false));
        assert_eq!(keys.key(12, 0x51, true), (0x41, true));
        assert_eq!(keys.key(12, 0x51, false), (0x41, false));
        assert_eq!(keys.key(12, 0x51, true), (0x51, false));
    }
    #[test]
    fn windows_keys_are_layout_named_virtual_keys_and_x11_hold_stays_modifier_only() {
        assert_eq!(virtual_key("KeyQ"), Some(0x51));
        assert_eq!(virtual_key("Space"), Some(0x20));
        assert_eq!(virtual_key("F24"), Some(0x87));
        assert_eq!(virtual_key("Unknown"), None);
        let settings = pennel_core::default_settings();
        let plan = plan(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            false,
            false,
            false,
        );
        assert!(plan.hold.unwrap().key.is_none());
        assert!(plan.registrations.is_empty());
    }
    #[test]
    fn scope_registration_and_capture_follow_the_core_capability_table() {
        let settings = pennel_core::default_settings();
        let idle = plan(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            false,
            false,
            false,
        );
        assert!(!idle.registrations.iter().any(|row| row.action == 3));
        let listening = plan(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            true,
            false,
            false,
        );
        assert!(listening.registrations.is_empty());
        assert!(!listening.listening.iter().any(|(id, _)| *id == 4));
        assert!(
            plan(
                &settings,
                pennel_core::ShortcutPlatform::LinuxWayland,
                true,
                false,
                true
            )
            .registrations
            .is_empty()
        );
        let captured = plan(
            &settings,
            pennel_core::ShortcutPlatform::Windows,
            true,
            true,
            true,
        );
        assert!(captured.registrations.is_empty());
        assert!(captured.hold.is_none());
    }
    #[test]
    fn x11_never_installs_keyed_grabs_from_defaults_saved_overrides_or_capture() {
        let mut settings = pennel_core::default_settings();
        for (id, code, key) in [
            ("record", "KeyQ", "a"),
            ("open_pennel", "KeyW", "z"),
            ("cancel", "Escape", "Escape"),
            ("next_model", "F5", "F5"),
        ] {
            settings.shortcuts.insert(
                id.into(),
                Some(pennel_core::ShortcutBinding {
                    mods: if matches!(id, "record" | "open_pennel") {
                        vec!["ctrl".into(), "alt".into()]
                    } else {
                        vec![]
                    },
                    code: Some(code.into()),
                    key: Some(key.into()),
                }),
            );
        }
        for listening in [false, true] {
            for capturing in [false, true] {
                {
                    let plan = plan(
                        &settings,
                        pennel_core::ShortcutPlatform::LinuxX11,
                        listening,
                        capturing,
                        true,
                    );
                    assert!(plan.registrations.is_empty());
                    assert!(
                        plan.listening
                            .iter()
                            .all(|(_, binding)| binding.key.is_none())
                    );
                }
            }
        }
        assert!(!uses_registration_backend(
            pennel_core::ShortcutPlatform::LinuxX11
        ));
        assert!(uses_registration_backend(
            pennel_core::ShortcutPlatform::Windows
        ));
        settings.shortcuts.insert(
            "cancel".into(),
            Some(pennel_core::ShortcutBinding {
                mods: vec!["right_alt".into()],
                code: None,
                key: None,
            }),
        );
        settings.shortcuts.insert(
            "next_model".into(),
            Some(pennel_core::ShortcutBinding {
                mods: vec!["left_alt".into()],
                code: None,
                key: None,
            }),
        );
        let safe = plan(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            true,
            false,
            true,
        );
        assert!(
            safe.listening
                .iter()
                .any(|(id, binding)| *id == 3 && binding.key.is_none())
        );
        assert!(
            safe.listening
                .iter()
                .any(|(id, binding)| *id == 4 && binding.key.is_none())
        );
        let disabled = plan(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            true,
            false,
            false,
        );
        assert!(!disabled.listening.iter().any(|(id, _)| *id == 4));
    }
    #[test]
    fn x11_raw_input_never_falls_back_to_unavailable_keyed_actions_in_hands_free() {
        use crate::hotkey::{HotkeyEvent, Key, TriggerTracker};
        use pennel_core::{
            GestureAction, HotkeyGesture, KeyEdge, ShortcutBinding, ShortcutPlatform,
        };
        let mut settings = pennel_core::default_settings();
        for (id, code, key, mods) in [
            ("record", "KeyR", "r", vec!["ctrl", "shift"]),
            ("open_pennel", "KeyO", "o", vec!["ctrl", "alt"]),
            ("cancel", "Escape", "Escape", vec![]),
            ("next_model", "F5", "F5", vec![]),
        ] {
            settings.shortcuts.insert(
                id.into(),
                Some(ShortcutBinding {
                    mods: mods.into_iter().map(str::to_owned).collect(),
                    code: Some(code.into()),
                    key: Some(key.into()),
                }),
            );
        }
        let projected = plan(&settings, ShortcutPlatform::LinuxX11, true, false, true);
        assert!(projected.registrations.is_empty());
        let mut tracker = TriggerTracker::from_binding(projected.hold);
        tracker.set_listening_bindings(projected.listening);
        tracker.set_listening(true);
        let mut gesture = HotkeyGesture::default();
        for (down, time) in [(true, 0.0), (false, 0.1), (true, 0.2), (false, 0.3)] {
            for event in tracker.key(Key::RightCtrl, down, time) {
                if let HotkeyEvent::Edge(edge, time) = event {
                    gesture.handle(edge, time);
                }
            }
        }
        assert!(gesture.is_hands_free());
        for code in ["Escape", "F5", "KeyR", "KeyO"] {
            let modifiers = match code {
                "KeyR" => vec![Key::LeftCtrl, Key::LeftShift],
                "KeyO" => vec![Key::LeftCtrl, Key::LeftAlt],
                _ => vec![],
            };
            for modifier in &modifiers {
                assert!(
                    !tracker
                        .key(*modifier, true, 0.9)
                        .iter()
                        .any(|event| matches!(event, HotkeyEvent::Action(_)))
                );
            }
            let down = tracker.route(Key::Other(virtual_key(code).unwrap()), true, 1.0);
            assert!(!down.withhold);
            assert!(down.events.is_empty(), "{code}");
            assert!(
                tracker
                    .key(Key::Other(virtual_key(code).unwrap()), false, 1.1)
                    .is_empty()
            );
            for modifier in modifiers.into_iter().rev() {
                assert!(
                    !tracker
                        .key(modifier, false, 1.2)
                        .iter()
                        .any(|event| matches!(event, HotkeyEvent::Action(_)))
                );
            }
            assert!(gesture.is_hands_free());
        }
        // Modifiers can still be used safely without installing or swallowing any X11 key grab.
        settings.shortcuts.insert(
            "next_model".into(),
            Some(ShortcutBinding {
                mods: vec!["left_alt".into()],
                code: None,
                key: None,
            }),
        );
        let projected = plan(&settings, ShortcutPlatform::LinuxX11, true, false, true);
        tracker.set_listening_bindings(projected.listening);
        assert!(tracker.key(Key::LeftAlt, true, 2.0).is_empty());
        assert_eq!(
            tracker.key(Key::LeftAlt, false, 2.1),
            [HotkeyEvent::Action(4)]
        );
        assert_eq!(gesture.handle(KeyEdge::Down, 3.0), [GestureAction::Submit]);
    }

    #[test]
    fn a_pressed_registration_drains_its_release_before_unregistering() {
        assert!(retain_registration(false, true));
        assert!(!retain_registration(false, false));
        assert!(retain_registration(true, false));
    }
}
