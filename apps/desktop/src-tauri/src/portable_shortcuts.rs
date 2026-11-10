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
fn generic(modifier: &str) -> String {
    modifier
        .strip_prefix("left_")
        .or_else(|| modifier.strip_prefix("right_"))
        .unwrap_or(modifier)
        .into()
}
pub fn plan(
    settings: &Settings,
    platform: ShortcutPlatform,
    listening: bool,
    capturing: bool,
    next_model: bool,
) -> Plan {
    plan_with_hold(settings, platform, listening, capturing, next_model, false)
}
pub fn plan_with_hold(
    settings: &Settings,
    platform: ShortcutPlatform,
    listening: bool,
    capturing: bool,
    next_model: bool,
    held: bool,
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
            && let Some(trigger) = TriggerBinding::resolve(&binding, virtual_key)
        {
            if platform == ShortcutPlatform::LinuxX11 && listening && binding.code.is_some() {
                let mut variants = vec![binding.clone()];
                if held && let Some(hold) = &result.hold {
                    let mut with_hold = binding.clone();
                    with_hold.mods.extend(hold.mods.iter().map(|m| generic(m)));
                    variants.push(with_hold);
                    let mut shift = binding.clone();
                    shift.mods.push("shift".into());
                    variants.push(shift);
                    let mut both = binding.clone();
                    both.mods.extend(hold.mods.iter().map(|m| generic(m)));
                    both.mods.push("shift".into());
                    variants.push(both);
                }
                for mut binding in variants {
                    binding.mods.sort();
                    binding.mods.dedup();
                    let row = Registration { action, binding };
                    if !result.registrations.contains(&row) {
                        result.registrations.push(row);
                    }
                }
            }
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
        assert_eq!(plan.registrations.len(), 1);
        assert_eq!(plan.registrations[0].action, 1);
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
        assert!(listening.registrations.iter().any(|row| row.action == 3));
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
    fn x11_listening_grabs_include_only_the_current_hold_and_command_aliases() {
        let settings = pennel_core::default_settings();
        let held = plan_with_hold(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            true,
            false,
            false,
            true,
        );
        let modifiers: Vec<_> = held
            .registrations
            .iter()
            .filter(|row| row.action == 3)
            .map(|row| row.binding.mods.clone())
            .collect();
        assert!(modifiers.contains(&vec![]));
        assert!(modifiers.contains(&vec!["ctrl".into()]));
        assert!(modifiers.contains(&vec!["shift".into()]));
        assert!(modifiers.contains(&vec!["ctrl".into(), "shift".into()]));
        let idle_hold = plan_with_hold(
            &settings,
            pennel_core::ShortcutPlatform::LinuxX11,
            true,
            false,
            false,
            false,
        );
        assert_eq!(
            idle_hold
                .registrations
                .iter()
                .filter(|row| row.action == 3)
                .count(),
            1
        );
    }
    #[test]
    fn a_scoped_grab_drains_its_release_before_unregistering() {
        assert!(retain_registration(false, true));
        assert!(!retain_registration(false, false));
        assert!(retain_registration(true, false));
    }
}
