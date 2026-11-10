//! Per-machine shortcut policy. Persist strings so future bindings never prevent startup.
use crate::{HotkeyTrigger, InterfaceLanguage, Settings};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ShortcutBinding {
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

pub(crate) fn lenient_shortcuts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<HashMap<String, Option<ShortcutBinding>>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(id, value)| {
                    if value.is_null() {
                        Some((id.clone(), None))
                    } else {
                        serde_json::from_value::<ShortcutBinding>(value.clone())
                            .ok()
                            .map(|binding| (id.clone(), Some(binding)))
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutPlatform {
    Macos,
    Windows,
    LinuxX11,
    LinuxWayland,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutScope {
    GlobalHold,
    Listening,
    Global,
    Application,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ShortcutLabel {
    pub keycaps: Vec<String>,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ShortcutCapability {
    pub available: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ShortcutRow {
    pub id: String,
    pub scope: ShortcutScope,
    pub binding: Option<ShortcutBinding>,
    pub default_binding: Option<ShortcutBinding>,
    pub label: ShortcutLabel,
    pub editable: bool,
    pub capability: ShortcutCapability,
    pub changed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutProblem {
    pub kind: String,
    pub reason: String,
    pub action: Option<String>,
}
impl std::fmt::Display for ShortcutProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.reason)
    }
}
impl std::error::Error for ShortcutProblem {}
fn problem(kind: &str, reason: &str) -> ShortcutProblem {
    ShortcutProblem {
        kind: kind.into(),
        reason: reason.into(),
        action: None,
    }
}
fn conflict(id: &str) -> ShortcutProblem {
    ShortcutProblem {
        kind: "conflict".into(),
        reason: "already_assigned".into(),
        action: Some(id.into()),
    }
}

/// Order also resolves conflicting hand-edited settings, independently of HashMap iteration order.
pub const SHORTCUT_ACTIONS: &[&str] = &[
    "dictate",
    "cancel",
    "next_model",
    "record",
    "open_pennel",
    "new_recording",
    "find",
    "dictation",
    "meetings",
    "history",
    "toggle_sidebar",
    "settings",
];
pub fn shortcut_scope(id: &str) -> Option<ShortcutScope> {
    Some(match id {
        "dictate" => ShortcutScope::GlobalHold,
        "cancel" | "next_model" => ShortcutScope::Listening,
        "record" | "open_pennel" => ShortcutScope::Global,
        "new_recording" | "find" | "dictation" | "meetings" | "history" | "toggle_sidebar"
        | "settings" => ShortcutScope::Application,
        _ => return None,
    })
}
fn binding(mods: &[&str], key: Option<(&str, &str)>) -> ShortcutBinding {
    ShortcutBinding {
        mods: mods.iter().map(|s| (*s).into()).collect(),
        code: key.map(|(c, _)| c.into()),
        key: key.map(|(_, k)| k.into()),
    }
}
pub fn legacy_dictate(trigger: HotkeyTrigger, platform: ShortcutPlatform) -> ShortcutBinding {
    binding(
        match trigger {
            HotkeyTrigger::Fn | HotkeyTrigger::ControlSuper
                if platform == ShortcutPlatform::Macos =>
            {
                &["fn"]
            }
            HotkeyTrigger::Fn | HotkeyTrigger::RightControl => &["right_ctrl"],
            HotkeyTrigger::RightOption => &["right_alt"],
            HotkeyTrigger::RightCommand => &["right_meta"],
            HotkeyTrigger::ControlSuper => &["ctrl", "meta"],
        },
        None,
    )
}
pub fn default_shortcut(id: &str, platform: ShortcutPlatform) -> Option<ShortcutBinding> {
    let primary = if platform == ShortcutPlatform::Macos {
        "meta"
    } else {
        "ctrl"
    };
    Some(match id {
        "dictate" => legacy_dictate(HotkeyTrigger::Fn, platform),
        "cancel" => binding(&[], Some(("Escape", "Escape"))),
        "next_model" if platform == ShortcutPlatform::Macos => binding(&["left_alt"], None),
        "record" => binding(&[primary, "shift"], Some(("KeyR", "r"))),
        "new_recording" => binding(&[primary], Some(("KeyN", "n"))),
        "find" => binding(&[primary], Some(("KeyF", "f"))),
        "dictation" => binding(&[primary], Some(("Digit1", "1"))),
        "meetings" => binding(&[primary], Some(("Digit2", "2"))),
        "history" => binding(&[primary], Some(("Digit3", "3"))),
        "toggle_sidebar" if platform == ShortcutPlatform::Macos => {
            binding(&["ctrl", "meta"], Some(("KeyS", "s")))
        }
        "toggle_sidebar" => binding(&["ctrl", "shift"], Some(("KeyS", "s"))),
        "settings" => binding(&[primary], Some(("Comma", ","))),
        _ => return None,
    })
}
fn fallback(settings: &Settings, id: &str, platform: ShortcutPlatform) -> Option<ShortcutBinding> {
    match id {
        "dictate" => Some(legacy_dictate(settings.hotkey, platform)),
        "record" if !settings.record_shortcut_enabled => None,
        _ => default_shortcut(id, platform),
    }
}
fn raw_effective(
    settings: &Settings,
    id: &str,
    platform: ShortcutPlatform,
) -> Option<ShortcutBinding> {
    match settings.shortcuts.get(id) {
        Some(None) => None,
        Some(Some(b)) if validate_shape(id, b, platform).is_ok() => Some(canonical(b.clone())),
        _ => fallback(settings, id, platform).map(canonical),
    }
}
pub fn effective_shortcut(
    settings: &Settings,
    id: &str,
    platform: ShortcutPlatform,
) -> Option<ShortcutBinding> {
    shortcut_scope(id)?;
    let mut accepted: Vec<(&str, ShortcutBinding)> = Vec::new();
    for action in SHORTCUT_ACTIONS {
        let candidate = raw_effective(settings, action, platform).filter(|b| {
            !accepted
                .iter()
                .any(|(_, other)| bindings_conflict(b, other))
                && (*action == "dictate"
                    || !accepted.iter().any(|(other_id, other)| {
                        *other_id == "dictate" && bindings_conflict(b, &command_binding(other))
                    }))
        });
        if *action == id {
            return candidate;
        }
        if let Some(b) = candidate {
            accepted.push((action, b));
        }
    }
    None
}
fn canonical(mut b: ShortcutBinding) -> ShortcutBinding {
    const ORDER: &[&str] = &[
        "ctrl",
        "left_ctrl",
        "right_ctrl",
        "alt",
        "left_alt",
        "right_alt",
        "shift",
        "left_shift",
        "right_shift",
        "meta",
        "left_meta",
        "right_meta",
        "fn",
    ];
    b.mods
        .sort_by_key(|m| ORDER.iter().position(|v| v == m).unwrap_or(usize::MAX));
    b.mods.dedup();
    b
}
fn family(modifier: &str) -> &str {
    modifier
        .strip_prefix("left_")
        .or_else(|| modifier.strip_prefix("right_"))
        .unwrap_or(modifier)
}
fn contains_mod(container: &str, contained: &str) -> bool {
    container == contained || container == family(contained)
}
fn mod_subset(a: &[String], b: &[String]) -> bool {
    a.iter().all(|x| b.iter().any(|y| contains_mod(x, y)))
}
fn same_mods(a: &[String], b: &[String]) -> bool {
    let mut a: Vec<_> = a.iter().map(|m| family(m)).collect();
    a.sort();
    a.dedup();
    let mut b: Vec<_> = b.iter().map(|m| family(m)).collect();
    b.sort();
    b.dedup();
    a == b
}
pub fn bindings_conflict(a: &ShortcutBinding, b: &ShortcutBinding) -> bool {
    match (&a.code, &b.code) {
        (Some(ac), Some(bc)) => {
            same_mods(&a.mods, &b.mods)
                && (ac == bc
                    || a.key
                        .as_ref()
                        .zip(b.key.as_ref())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)))
        }
        (None, None) => mod_subset(&a.mods, &b.mods) || mod_subset(&b.mods, &a.mods),
        _ => false,
    }
}
fn command_binding(b: &ShortcutBinding) -> ShortcutBinding {
    let mut b = b.clone();
    b.mods.push("shift".into());
    canonical(b)
}
fn function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}
pub fn supported_shortcut_code(code: &str) -> bool {
    (code.len() == 4 && code.starts_with("Key") && code.as_bytes()[3].is_ascii_uppercase())
        || (code.len() == 6 && code.starts_with("Digit") && code.as_bytes()[5].is_ascii_digit())
        || function_key(code)
        || matches!(
            code,
            "Space"
                | "Escape"
                | "Tab"
                | "Enter"
                | "Backspace"
                | "Delete"
                | "Insert"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "Comma"
                | "Period"
                | "Slash"
                | "Backslash"
                | "Semicolon"
                | "Quote"
                | "Backquote"
                | "Minus"
                | "Equal"
                | "BracketLeft"
                | "BracketRight"
                | "IntlBackslash"
        )
}
fn menu_key(key: &str) -> bool {
    key.chars().count() == 1
        || function_key(key)
        || matches!(
            key,
            "Space"
                | "Escape"
                | "Tab"
                | "Enter"
                | "Backspace"
                | "Delete"
                | "Insert"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
                | "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
        )
}
fn reserved(b: &ShortcutBinding, platform: ShortcutPlatform) -> bool {
    let Some(key) = b.key.as_deref() else {
        return false;
    };
    let key = key.to_lowercase();
    let mods: Vec<String> = b.mods.iter().map(|m| family(m).into()).collect();
    let exact = |expected: &[&str]| {
        same_mods(
            &mods,
            &expected.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
        )
    };
    let primary = if platform == ShortcutPlatform::Macos {
        "meta"
    } else {
        "ctrl"
    };
    (exact(&[primary]) && matches!(key.as_str(), "q" | "w" | "c" | "x" | "v" | "a" | "z"))
        || (exact(&[primary, "shift"]) && key == "z")
        || (platform == ShortcutPlatform::Macos
            && exact(&["meta"])
            && matches!(key.as_str(), "h" | "m" | "tab" | "space" | " "))
        || (platform != ShortcutPlatform::Macos
            && ((exact(&["alt"]) && matches!(key.as_str(), "tab" | "f4" | "space"))
                || (exact(&["meta"]) && key == "l")
                || (exact(&["ctrl", "alt"]) && key == "delete")))
}
fn validate_shape(
    id: &str,
    b: &ShortcutBinding,
    platform: ShortcutPlatform,
) -> Result<(), ShortcutProblem> {
    let scope = shortcut_scope(id).ok_or_else(|| problem("not_allowed", "unknown_action"))?;
    if b.mods.iter().any(|m| {
        !matches!(
            m.as_str(),
            "ctrl"
                | "alt"
                | "shift"
                | "meta"
                | "fn"
                | "left_ctrl"
                | "right_ctrl"
                | "left_alt"
                | "right_alt"
                | "left_shift"
                | "right_shift"
                | "left_meta"
                | "right_meta"
        )
    }) {
        return Err(problem("unsupported", "unknown_modifier"));
    }
    if b.mods.contains(&"fn".into()) && (platform != ShortcutPlatform::Macos || b.code.is_some()) {
        return Err(problem("not_allowed", "fn_modifier_only_macos"));
    }
    if b.code.is_some() != b.key.is_some() {
        return Err(problem("not_allowed", "key_and_code_required"));
    }
    if id == "dictate" && b.mods.iter().any(|m| family(m) == "shift") {
        return Err(problem("not_allowed", "shift_is_command"));
    }
    if let (Some(code), Some(key)) = (&b.code, &b.key) {
        if !supported_shortcut_code(code) || !menu_key(key) {
            return Err(problem("unsupported", "unknown_key"));
        }
        if b.mods
            .iter()
            .any(|m| m.starts_with("left_") || m.starts_with("right_"))
        {
            return Err(problem("not_allowed", "sides_require_modifier_only"));
        }
        if reserved(b, platform) {
            return Err(problem("reserved", "system_shortcut"));
        }
        let function = function_key(key) && b.mods.is_empty();
        let primary = if scope == ShortcutScope::Application {
            if platform == ShortcutPlatform::Macos {
                b.mods.contains(&"meta".into())
            } else {
                b.mods.contains(&"ctrl".into())
            }
        } else {
            b.mods
                .iter()
                .any(|m| matches!(m.as_str(), "ctrl" | "alt" | "meta"))
        };
        if !function
            && !primary
            && !(scope == ShortcutScope::Listening && (key == "Escape" || !b.mods.is_empty()))
        {
            return Err(problem("not_allowed", "modifier_required"));
        }
    } else {
        if !matches!(scope, ShortcutScope::GlobalHold | ShortcutScope::Listening) {
            return Err(problem("not_allowed", "key_required"));
        }
        if b.mods.is_empty() || b.mods.iter().all(|m| family(m) == "shift") {
            return Err(problem("not_allowed", "modifier_required"));
        }
    }
    let capability = shortcut_capability(id, Some(b), platform);
    if !capability.available {
        return Err(problem(
            "unsupported",
            capability.reason.as_deref().unwrap_or("unavailable"),
        ));
    }
    Ok(())
}
pub fn validate_shortcut(
    settings: &Settings,
    id: &str,
    b: &ShortcutBinding,
    platform: ShortcutPlatform,
) -> Result<(), ShortcutProblem> {
    validate_shape(id, b, platform)?;
    for other in SHORTCUT_ACTIONS {
        if let Some(existing) = effective_shortcut(settings, other, platform) {
            // A Shift chord belongs to Command before replacement can clear Dictate.
            if *other == "dictate"
                && id != "dictate"
                && b.mods.iter().any(|m| family(m) == "shift")
                && bindings_conflict(b, &command_binding(&existing))
            {
                return Err(conflict("command"));
            }
            if *other != id && bindings_conflict(b, &existing) {
                return Err(conflict(other));
            }
            if id == "dictate" && *other != id && bindings_conflict(&command_binding(b), &existing)
            {
                return Err(conflict(other));
            }
        }
    }
    Ok(())
}
fn write_shortcut(
    settings: &mut Settings,
    id: &str,
    b: Option<ShortcutBinding>,
    platform: ShortcutPlatform,
) {
    let b = b.map(canonical);
    if id == "dictate" {
        let preset = [
            HotkeyTrigger::Fn,
            HotkeyTrigger::RightControl,
            HotkeyTrigger::RightOption,
            HotkeyTrigger::RightCommand,
            HotkeyTrigger::ControlSuper,
        ]
        .into_iter()
        .find(|trigger| b.as_ref() == Some(&canonical(legacy_dictate(*trigger, platform))));
        settings.hotkey = preset.unwrap_or(if platform == ShortcutPlatform::Macos {
            HotkeyTrigger::Fn
        } else {
            HotkeyTrigger::RightControl
        });
        if preset.is_some() {
            settings.shortcuts.remove(id);
        } else {
            settings.shortcuts.insert(id.into(), b);
        }
    } else if id == "record" {
        settings.record_shortcut_enabled = b == default_shortcut(id, platform).map(canonical);
        if b.is_none() || settings.record_shortcut_enabled {
            settings.shortcuts.remove(id);
        } else {
            settings.shortcuts.insert(id.into(), b);
        }
    } else if b == default_shortcut(id, platform).map(canonical) {
        settings.shortcuts.remove(id);
    } else {
        settings.shortcuts.insert(id.into(), b);
    }
}
pub fn set_shortcut(
    settings: &mut Settings,
    id: &str,
    b: Option<ShortcutBinding>,
    platform: ShortcutPlatform,
    replace: bool,
) -> Result<(), ShortcutProblem> {
    shortcut_scope(id).ok_or_else(|| problem("not_allowed", "unknown_action"))?;
    let mut next = settings.clone();
    if let Some(binding) = b.as_ref() {
        validate_shape(id, binding, platform)?;
        loop {
            match validate_shortcut(&next, id, binding, platform) {
                Ok(()) => break,
                Err(error)
                    if replace
                        && error.kind == "conflict"
                        && error.action.as_deref() != Some("command") =>
                {
                    write_shortcut(
                        &mut next,
                        error.action.as_deref().expect("conflict action"),
                        None,
                        platform,
                    );
                }
                Err(error) => return Err(error),
            }
        }
    }
    write_shortcut(&mut next, id, b, platform);
    *settings = next;
    Ok(())
}
pub fn reset_shortcut(
    settings: &mut Settings,
    id: &str,
    platform: ShortcutPlatform,
) -> Result<(), ShortcutProblem> {
    set_shortcut(
        settings,
        id,
        default_shortcut(id, platform),
        platform,
        false,
    )
}
pub fn reset_all_shortcuts(settings: &mut Settings, platform: ShortcutPlatform) {
    settings.shortcuts.clear();
    settings.hotkey = if platform == ShortcutPlatform::Macos {
        HotkeyTrigger::Fn
    } else {
        HotkeyTrigger::RightControl
    };
    settings.record_shortcut_enabled = true;
}
pub fn reconcile_legacy_shortcuts(previous: &Settings, next: &mut Settings) {
    for (id, changed) in [
        ("dictate", previous.hotkey != next.hotkey),
        (
            "record",
            previous.record_shortcut_enabled != next.record_shortcut_enabled,
        ),
    ] {
        if changed && previous.shortcuts.get(id) == next.shortcuts.get(id) {
            next.shortcuts.remove(id);
        }
    }
}
pub fn shortcut_capability(
    id: &str,
    b: Option<&ShortcutBinding>,
    platform: ShortcutPlatform,
) -> ShortcutCapability {
    let scope = shortcut_scope(id);
    let reason = if scope.is_none() {
        Some("unknown_action")
    } else if scope == Some(ShortcutScope::Application) {
        None
    } else if platform == ShortcutPlatform::LinuxWayland {
        Some(match id {
            "dictate" => "wayland_toggle_dictation",
            "record" => "wayland_toggle_meeting",
            "open_pennel" => "wayland_open_pennel",
            _ => "wayland_global_shortcuts",
        })
    } else if platform == ShortcutPlatform::LinuxX11
        && id == "dictate"
        && b.is_some_and(|b| b.code.is_some())
    {
        Some("x11_dictate_modifier_only")
    } else {
        None
    };
    ShortcutCapability {
        available: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}
pub fn format_shortcut(
    b: &ShortcutBinding,
    platform: ShortcutPlatform,
    language: InterfaceLanguage,
) -> ShortcutLabel {
    let b = canonical(b.clone());
    let mac = platform == ShortcutPlatform::Macos;
    let mut keycaps: Vec<_> = b
        .mods
        .iter()
        .map(|m| {
            let name = match (family(m), mac) {
                ("ctrl", true) => "⌃",
                ("alt", true) => "⌥",
                ("meta", true) => "⌘",
                ("shift", true) => "⇧",
                ("ctrl", false) => "Ctrl",
                ("alt", false) => "Alt",
                ("shift", false) => "Shift",
                ("meta", false) if platform == ShortcutPlatform::Windows => "Win",
                ("meta", false) => "Super",
                _ => "fn",
            };
            let side = if m.starts_with("left_") {
                if language == InterfaceLanguage::Dutch {
                    "links "
                } else {
                    "left "
                }
            } else if m.starts_with("right_") {
                if language == InterfaceLanguage::Dutch {
                    "rechts "
                } else {
                    "right "
                }
            } else {
                ""
            };
            format!("{side}{name}")
        })
        .collect();
    if let Some(key) = b.key {
        keycaps.push(match key.as_str() {
            "Escape" => "Esc".into(),
            " " | "Space" => {
                if language == InterfaceLanguage::Dutch {
                    "Spatie".into()
                } else {
                    "Space".into()
                }
            }
            "ArrowUp" => "↑".into(),
            "ArrowDown" => "↓".into(),
            "ArrowLeft" => "←".into(),
            "ArrowRight" => "→".into(),
            _ => key.to_uppercase(),
        });
    }
    let text = keycaps.join(if mac { "" } else { "+" });
    ShortcutLabel { keycaps, text }
}
pub fn shortcut_rows(
    settings: &Settings,
    platform: ShortcutPlatform,
    language: InterfaceLanguage,
) -> Vec<ShortcutRow> {
    SHORTCUT_ACTIONS
        .iter()
        .map(|id| {
            let binding = effective_shortcut(settings, id, platform);
            let default_binding = default_shortcut(id, platform).map(canonical);
            ShortcutRow {
                id: (*id).into(),
                scope: shortcut_scope(id).expect("known action"),
                label: binding
                    .as_ref()
                    .map(|b| format_shortcut(b, platform, language))
                    .unwrap_or(ShortcutLabel {
                        keycaps: Vec::new(),
                        text: String::new(),
                    }),
                capability: shortcut_capability(id, binding.as_ref(), platform),
                changed: binding != default_binding,
                binding,
                default_binding,
                editable: true,
            }
        })
        .collect()
}
/// Native menus and DOM handlers match logical characters, while global hooks match `code`.
pub fn menu_accelerator(b: &ShortcutBinding) -> Option<String> {
    let key = b.key.as_deref()?;
    let mut parts: Vec<String> = b
        .mods
        .iter()
        .map(|m| {
            match family(m) {
                "ctrl" => "Ctrl",
                "alt" => "Alt",
                "meta" => "Cmd",
                "shift" => "Shift",
                _ => "",
            }
            .into()
        })
        .collect();
    if parts.iter().any(String::is_empty) {
        return None;
    }
    parts.push(match key {
        " " | "Space" => "Space".into(),
        "ArrowUp" => "Up".into(),
        "ArrowDown" => "Down".into(),
        "ArrowLeft" => "Left".into(),
        "ArrowRight" => "Right".into(),
        _ => key.to_uppercase(),
    });
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HotkeyTrigger, default_settings};

    fn binding(mods: &[&str], code: &str, key: &str) -> ShortcutBinding {
        ShortcutBinding {
            mods: mods.iter().map(|s| (*s).into()).collect(),
            code: Some(code.into()),
            key: Some(key.into()),
        }
    }

    #[test]
    fn sparse_settings_derive_legacy_and_preserve_explicit_clear() {
        let mut settings = default_settings();
        settings.hotkey = HotkeyTrigger::RightOption;
        assert_eq!(
            effective_shortcut(&settings, "dictate", ShortcutPlatform::Macos)
                .unwrap()
                .mods,
            ["right_alt"]
        );
        settings.shortcuts.insert("dictate".into(), None);
        assert_eq!(
            effective_shortcut(&settings, "dictate", ShortcutPlatform::Macos),
            None
        );
    }

    #[test]
    fn strict_settings_accept_future_and_malformed_shortcuts() {
        for shortcuts in [
            serde_json::json!(123),
            serde_json::json!({"dictate": 42}),
            serde_json::json!({"future": {"mods": ["hyper"], "code": "Future", "key": "future"}, "dictate": {"mods": ["hyper"]}}),
        ] {
            let mut value = serde_json::to_value(default_settings()).unwrap();
            value["shortcuts"] = shortcuts.clone();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let settings = crate::Settings::load_strict(&path).unwrap();
            assert!(effective_shortcut(&settings, "dictate", ShortcutPlatform::Macos).is_some());
            if shortcuts.get("future").is_some() {
                assert_eq!(
                    serde_json::to_value(&settings).unwrap()["shortcuts"]["future"],
                    shortcuts["future"]
                );
            }
        }
    }

    #[test]
    fn writer_preserves_downgrade_values_and_resets_sparsely() {
        let mut settings = default_settings();
        let custom = binding(&["ctrl", "alt"], "KeyR", "r");
        set_shortcut(
            &mut settings,
            "record",
            Some(custom.clone()),
            ShortcutPlatform::Macos,
            false,
        )
        .unwrap();
        assert!(!settings.record_shortcut_enabled);
        assert_eq!(settings.shortcuts["record"], Some(custom));
        reset_shortcut(&mut settings, "record", ShortcutPlatform::Macos).unwrap();
        assert!(settings.record_shortcut_enabled);
        assert!(!settings.shortcuts.contains_key("record"));
        set_shortcut(
            &mut settings,
            "dictate",
            None,
            ShortcutPlatform::Macos,
            false,
        )
        .unwrap();
        assert_eq!(settings.hotkey, HotkeyTrigger::Fn);
        assert_eq!(settings.shortcuts["dictate"], None);
        assert!(matches!(
            serde_json::to_value(&settings).unwrap()["hotkey"].as_str(),
            Some("fn" | "right_control" | "right_option" | "right_command" | "control_super")
        ));
    }

    #[test]
    fn conflict_replace_is_atomic_and_command_cannot_be_replaced() {
        let mut settings = default_settings();
        let find = default_shortcut("find", ShortcutPlatform::Macos).unwrap();
        let before = settings.clone();
        let error = set_shortcut(
            &mut settings,
            "record",
            Some(find.clone()),
            ShortcutPlatform::Macos,
            false,
        )
        .unwrap_err();
        assert_eq!(error.kind, "conflict");
        assert_eq!(error.action.as_deref(), Some("find"));
        assert_eq!(settings, before);
        set_shortcut(
            &mut settings,
            "record",
            Some(find),
            ShortcutPlatform::Macos,
            true,
        )
        .unwrap();
        assert_eq!(
            effective_shortcut(&settings, "find", ShortcutPlatform::Macos),
            None
        );
        let command = ShortcutBinding {
            mods: vec!["fn".into(), "shift".into()],
            code: None,
            key: None,
        };
        assert_eq!(
            set_shortcut(
                &mut settings,
                "cancel",
                Some(command),
                ShortcutPlatform::Macos,
                true
            )
            .unwrap_err()
            .action
            .as_deref(),
            Some("command")
        );
    }

    #[test]
    fn validation_rejects_reserved_sides_with_keys_and_unsupported_x11() {
        let settings = default_settings();
        for (action, b, platform, kind) in [
            (
                "find",
                binding(&["meta"], "KeyQ", "q"),
                ShortcutPlatform::Macos,
                "reserved",
            ),
            (
                "record",
                binding(&["right_ctrl"], "KeyR", "r"),
                ShortcutPlatform::Windows,
                "not_allowed",
            ),
            (
                "dictate",
                binding(&["ctrl"], "Space", "Space"),
                ShortcutPlatform::LinuxX11,
                "unsupported",
            ),
            (
                "find",
                binding(&["alt"], "KeyF", "f"),
                ShortcutPlatform::Macos,
                "not_allowed",
            ),
            (
                "record",
                binding(&[], "KeyR", "r"),
                ShortcutPlatform::Macos,
                "not_allowed",
            ),
        ] {
            assert_eq!(
                validate_shortcut(&settings, action, &b, platform)
                    .unwrap_err()
                    .kind,
                kind
            );
        }
    }

    #[test]
    fn key_conflicts_use_both_physical_code_and_logical_key() {
        assert!(bindings_conflict(
            &binding(&["alt", "ctrl"], "KeyQ", "a"),
            &binding(&["ctrl", "alt"], "KeyA", "a")
        ));
        assert!(bindings_conflict(
            &binding(&["ctrl"], "KeyQ", "a"),
            &binding(&["ctrl"], "KeyQ", "q")
        ));
        let modifier = |mods: &[&str]| ShortcutBinding {
            mods: mods.iter().map(|s| (*s).into()).collect(),
            code: None,
            key: None,
        };
        assert!(bindings_conflict(
            &modifier(&["alt"]),
            &modifier(&["right_alt"])
        ));
        assert!(!bindings_conflict(
            &modifier(&["left_alt"]),
            &modifier(&["right_alt"])
        ));
    }

    #[test]
    fn labels_and_capabilities_are_platform_data() {
        let b = binding(&["ctrl", "alt"], "KeyQ", "a");
        assert_eq!(
            format_shortcut(
                &b,
                ShortcutPlatform::Macos,
                crate::InterfaceLanguage::English
            )
            .keycaps,
            ["⌃", "⌥", "A"]
        );
        assert_eq!(
            format_shortcut(
                &b,
                ShortcutPlatform::Windows,
                crate::InterfaceLanguage::Dutch
            )
            .text,
            "Ctrl+Alt+A"
        );
        assert_eq!(
            shortcut_capability("record", None, ShortcutPlatform::LinuxWayland)
                .reason
                .as_deref(),
            Some("wayland_toggle_meeting")
        );
        assert!(shortcut_capability("find", None, ShortcutPlatform::LinuxWayland).available);
        assert!(default_shortcut("open_pennel", ShortcutPlatform::Macos).is_none());
        assert!(default_shortcut("next_model", ShortcutPlatform::Windows).is_none());
    }

    #[test]
    fn all_presets_are_resolved_on_all_platforms_and_reset_all_restores_defaults() {
        for platform in [
            ShortcutPlatform::Macos,
            ShortcutPlatform::Windows,
            ShortcutPlatform::LinuxX11,
            ShortcutPlatform::LinuxWayland,
        ] {
            for trigger in [
                HotkeyTrigger::Fn,
                HotkeyTrigger::RightControl,
                HotkeyTrigger::RightOption,
                HotkeyTrigger::RightCommand,
                HotkeyTrigger::ControlSuper,
            ] {
                let mut settings = default_settings();
                settings.hotkey = trigger;
                assert_eq!(
                    effective_shortcut(&settings, "dictate", platform),
                    Some(canonical(legacy_dictate(trigger, platform)))
                );
                settings.shortcuts.insert("future".into(), None);
                reset_all_shortcuts(&mut settings, platform);
                assert!(settings.shortcuts.is_empty());
                assert_eq!(
                    effective_shortcut(&settings, "dictate", platform),
                    default_shortcut("dictate", platform)
                );
                assert!(settings.record_shortcut_enabled);
            }
        }
    }

    #[test]
    fn invalid_overrides_fall_back_and_collision_order_is_stable() {
        let mut settings = default_settings();
        settings
            .shortcuts
            .insert("find".into(), Some(binding(&["meta"], "KeyQ", "q")));
        assert_eq!(
            effective_shortcut(&settings, "find", ShortcutPlatform::Macos),
            default_shortcut("find", ShortcutPlatform::Macos)
        );
        let b = binding(&["alt", "meta"], "KeyK", "k");
        settings.shortcuts.insert("find".into(), Some(b.clone()));
        settings.shortcuts.insert("record".into(), Some(b.clone()));
        assert_eq!(
            effective_shortcut(&settings, "record", ShortcutPlatform::Macos),
            Some(b)
        );
        assert_eq!(
            effective_shortcut(&settings, "find", ShortcutPlatform::Macos),
            None
        );
    }

    #[test]
    fn listening_combinations_and_side_labels_keep_the_accepted_scope() {
        let settings = default_settings();
        assert!(
            validate_shortcut(
                &settings,
                "cancel",
                &binding(&["shift"], "KeyK", "k"),
                ShortcutPlatform::Macos
            )
            .is_ok()
        );
        let b = ShortcutBinding {
            mods: vec!["right_alt".into()],
            code: None,
            key: None,
        };
        assert_eq!(
            format_shortcut(&b, ShortcutPlatform::Macos, crate::InterfaceLanguage::Dutch).keycaps,
            ["rechts ⌥"]
        );
        assert!(
            serde_json::to_value(default_settings())
                .unwrap()
                .get("shortcuts")
                .is_none()
        );
    }

    #[test]
    fn legacy_writers_drop_only_unchanged_matching_override() {
        let mut previous = default_settings();
        previous.shortcuts.insert(
            "record".into(),
            Some(binding(&["ctrl", "alt"], "KeyR", "r")),
        );
        previous.record_shortcut_enabled = false;
        let mut next = previous.clone();
        next.record_shortcut_enabled = true;
        reconcile_legacy_shortcuts(&previous, &mut next);
        assert!(!next.shortcuts.contains_key("record"));
        next.shortcuts.insert("record".into(), None);
        reconcile_legacy_shortcuts(&previous, &mut next);
        assert_eq!(next.shortcuts["record"], None);
    }
}
