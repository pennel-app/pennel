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
    rows
}
