//! Shell runtime facts decorate the core shortcut policy; bindings and labels stay in core.
use crate::state::Shared;
use pennel_core::{ShortcutPlatform, ShortcutRow};

pub fn platform() -> ShortcutPlatform {
    #[cfg(target_os = "macos")]
    {
        ShortcutPlatform::Macos
    }
    #[cfg(windows)]
    {
        ShortcutPlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        if crate::linux_input::is_wayland() {
            ShortcutPlatform::LinuxWayland
        } else {
            ShortcutPlatform::LinuxX11
        }
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        ShortcutPlatform::LinuxWayland
    }
}
fn next_model_reason(
    enabled: bool,
    models: usize,
    bound: bool,
    handler: bool,
) -> Option<&'static str> {
    if !enabled {
        Some("next_model_switch_disabled")
    } else if models < 2 {
        Some("next_model_second_model_required")
    } else if !bound {
        Some("next_model_shortcut_cleared")
    } else if !handler {
        Some("next_model_hook_unavailable")
    } else {
        None
    }
}

pub fn rows(shared: &Shared) -> Vec<ShortcutRow> {
    let settings = shared.settings();
    let language = settings
        .ui_language
        .resolve(&[shared.system_interface_language.code().to_owned()]);
    let mut rows = pennel_core::shortcut_rows(&settings, platform(), language);
    if let Some(error) = shared.hotkey_error.lock().unwrap().as_ref() {
        for row in &mut rows {
            if row.scope != pennel_core::ShortcutScope::Application && row.capability.available {
                row.capability.available = false;
                row.capability.reason = Some(error.clone());
            }
        }
    }
    for row in &mut rows {
        let scoped = matches!(row.id.as_str(), "cancel" | "next_model");
        let keyed_dictate = row.id == "dictate"
            && row
                .binding
                .as_ref()
                .is_some_and(|binding| binding.code.is_some());
        if row.capability.available
            && (!scoped || crate::record_shortcut::is_listening())
            && (row.id != "dictate" || keyed_dictate)
            && let Some(reason) = crate::record_shortcut::action_error(&row.id)
        {
            row.capability.available = false;
            row.capability.reason = Some(reason);
        }
        if row.id == "next_model" && row.capability.available {
            let reason = next_model_reason(
                settings.pill_model_switch_enabled,
                shared
                    .engine
                    .configured_dictation_models(shared.keys())
                    .len(),
                row.binding.is_some(),
                crate::record_shortcut::next_model_enabled()
                    && shared
                        .hotkey
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_some(),
            );
            if let Some(reason) = reason {
                row.capability.available = false;
                row.capability.reason = Some(reason.into());
            }
        }
    }
    #[cfg(target_os = "macos")]
    if crate::macos::permissions()
        .get("accessibility")
        .and_then(serde_json::Value::as_str)
        != Some("granted")
    {
        for row in &mut rows {
            if row.scope != pennel_core::ShortcutScope::Application {
                row.capability.available = false;
                row.capability.reason = Some("accessibility_required".into());
            }
        }
    }
    #[cfg(target_os = "macos")]
    for row in &mut rows {
        if row.scope != pennel_core::ShortcutScope::Application
            && row
                .binding
                .as_ref()
                .is_some_and(|binding| crate::native_shortcuts::resolve_mac(0, binding).is_none())
        {
            row.capability.available = false;
            row.capability.reason = Some("native_key_not_supported".into());
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn next_model_readiness_explains_each_missing_requirement_without_enabling_a_fake_handler() {
        assert_eq!(
            next_model_reason(false, 2, true, true),
            Some("next_model_switch_disabled")
        );
        assert_eq!(
            next_model_reason(true, 1, true, true),
            Some("next_model_second_model_required")
        );
        assert_eq!(
            next_model_reason(true, 2, false, true),
            Some("next_model_shortcut_cleared")
        );
        assert_eq!(
            next_model_reason(true, 2, true, false),
            Some("next_model_hook_unavailable")
        );
        assert_eq!(next_model_reason(true, 2, true, true), None);
    }
}
