//! macOS virtual key positions and masks. Swift receives numeric tables, never shortcut policy.
use pennel_core::{Settings, ShortcutBinding, ShortcutPlatform, effective_shortcut};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MacShortcut {
    pub action: u32,
    pub code: Option<u32>,
    pub mods: Vec<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MacShortcutTable {
    pub hold: Option<MacShortcut>,
    pub always: Vec<MacShortcut>,
    pub listening: Vec<MacShortcut>,
}

pub const MAC_CODES: &[(&str, u32)] = &[
    ("KeyA", 0),
    ("KeyS", 1),
    ("KeyD", 2),
    ("KeyF", 3),
    ("KeyH", 4),
    ("KeyG", 5),
    ("KeyZ", 6),
    ("KeyX", 7),
    ("KeyC", 8),
    ("KeyV", 9),
    ("IntlBackslash", 10),
    ("KeyB", 11),
    ("KeyQ", 12),
    ("KeyW", 13),
    ("KeyE", 14),
    ("KeyR", 15),
    ("KeyY", 16),
    ("KeyT", 17),
    ("Digit1", 18),
    ("Digit2", 19),
    ("Digit3", 20),
    ("Digit4", 21),
    ("Digit6", 22),
    ("Digit5", 23),
    ("Equal", 24),
    ("Digit9", 25),
    ("Digit7", 26),
    ("Minus", 27),
    ("Digit8", 28),
    ("Digit0", 29),
    ("BracketRight", 30),
    ("KeyO", 31),
    ("KeyU", 32),
    ("BracketLeft", 33),
    ("KeyI", 34),
    ("KeyP", 35),
    ("Enter", 36),
    ("KeyL", 37),
    ("KeyJ", 38),
    ("Quote", 39),
    ("KeyK", 40),
    ("Semicolon", 41),
    ("Backslash", 42),
    ("Comma", 43),
    ("Slash", 44),
    ("KeyN", 45),
    ("KeyM", 46),
    ("Period", 47),
    ("Tab", 48),
    ("Space", 49),
    ("Backquote", 50),
    ("Backspace", 51),
    ("Escape", 53),
    ("MetaRight", 54),
    ("MetaLeft", 55),
    ("ShiftLeft", 56),
    ("AltLeft", 58),
    ("ControlLeft", 59),
    ("ShiftRight", 60),
    ("AltRight", 61),
    ("ControlRight", 62),
    ("Fn", 63),
    ("F17", 64),
    ("F18", 79),
    ("F19", 80),
    ("F20", 90),
    ("F5", 96),
    ("F6", 97),
    ("F7", 98),
    ("F3", 99),
    ("F8", 100),
    ("F9", 101),
    ("F11", 103),
    ("F13", 105),
    ("F16", 106),
    ("F14", 107),
    ("F10", 109),
    ("F12", 111),
    ("F15", 113),
    ("Insert", 114),
    ("Home", 115),
    ("PageUp", 116),
    ("Delete", 117),
    ("F4", 118),
    ("End", 119),
    ("F2", 120),
    ("PageDown", 121),
    ("F1", 122),
    ("ArrowLeft", 123),
    ("ArrowRight", 124),
    ("ArrowDown", 125),
    ("ArrowUp", 126),
];
pub fn mac_keycode(code: &str) -> Option<u32> {
    MAC_CODES
        .iter()
        .find(|(name, _)| *name == code)
        .map(|(_, code)| *code)
}
pub fn mac_mod_mask(modifier: &str) -> Option<u64> {
    Some(match modifier {
        "ctrl" => 0x40000,
        "alt" => 0x80000,
        "meta" => 0x100000,
        "shift" => 0x20000,
        "fn" => 0x800000,
        "left_ctrl" => 0x1,
        "right_ctrl" => 0x2000,
        "left_alt" => 0x20,
        "right_alt" => 0x40,
        "left_meta" => 0x8,
        "right_meta" => 0x10,
        "left_shift" => 0x2,
        "right_shift" => 0x4,
        _ => return None,
    })
}
pub fn resolve_mac(action: u32, binding: &ShortcutBinding) -> Option<MacShortcut> {
    Some(MacShortcut {
        action,
        code: match binding.code.as_deref() {
            Some(code) => Some(mac_keycode(code)?),
            None => None,
        },
        mods: binding
            .mods
            .iter()
            .map(|modifier| mac_mod_mask(modifier))
            .collect::<Option<Vec<_>>>()?,
    })
}
pub fn mac_table(settings: &Settings, include_listening: bool) -> MacShortcutTable {
    let resolve = |id, action| {
        effective_shortcut(settings, id, ShortcutPlatform::Macos)
            .as_ref()
            .and_then(|binding| resolve_mac(action, binding))
    };
    MacShortcutTable {
        hold: resolve("dictate", 0),
        always: [("record", 1), ("open_pennel", 2)]
            .into_iter()
            .filter_map(|(id, action)| resolve(id, action))
            .collect(),
        listening: if include_listening {
            [("cancel", 3), ("next_model", 4)]
                .into_iter()
                .filter_map(|(id, action)| resolve(id, action))
                .collect()
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mac_keycodes_keep_position_separate_from_layout_characters() {
        assert_eq!(mac_keycode("KeyQ"), Some(12));
        assert_eq!(mac_keycode("Space"), Some(49));
        assert_eq!(mac_keycode("Escape"), Some(53));
        assert_eq!(mac_keycode("F20"), Some(90));
        assert_eq!(mac_keycode("Future"), None);
        let binding = pennel_core::ShortcutBinding {
            mods: vec!["ctrl".into(), "alt".into()],
            code: Some("KeyQ".into()),
            key: Some("a".into()),
        };
        let native = resolve_mac(1, &binding).unwrap();
        assert_eq!(native.code, Some(12));
        assert_eq!(native.mods, [0x40000, 0x80000]);
    }
    #[test]
    fn defaults_and_explicit_clear_build_the_expected_native_scopes() {
        let mut settings = pennel_core::default_settings();
        let table = mac_table(&settings, false);
        assert_eq!(table.hold.unwrap().mods, [0x800000]);
        assert_eq!(table.always.len(), 1);
        assert!(table.listening.is_empty());
        let table = mac_table(&settings, true);
        assert!(
            table
                .listening
                .iter()
                .any(|binding| binding.action == 3 && binding.code == Some(53))
        );
        settings.shortcuts.insert("dictate".into(), None);
        settings.record_shortcut_enabled = false;
        let table = mac_table(&settings, false);
        assert!(table.hold.is_none());
        assert!(table.always.is_empty());
    }
}
