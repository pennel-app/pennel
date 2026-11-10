//! Conventional recording shortcuts use OS registration; dictation retains its raw modifier hook.

#[cfg(any(windows, target_os = "linux"))]
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;
#[cfg(any(windows, target_os = "linux"))]
use tauri::Manager;
#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::state::Shared;

fn failure() -> &'static Mutex<Option<String>> {
    static FAILURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    FAILURE.get_or_init(|| Mutex::new(None))
}

pub fn error() -> Option<String> {
    failure().lock().unwrap().clone()
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    if crate::linux_input::is_wayland() || !crate::linux_input::has_x11_display() {
        return Ok(());
    }
    #[cfg(any(windows, target_os = "linux"))]
    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _, event| {
                if event.state() == ShortcutState::Pressed
                    && let Some(shared) = app.try_state::<std::sync::Arc<Shared>>()
                {
                    let _ = shared.dictation.send(crate::dictation::Msg::Hotkey(
                        crate::hotkey::HotkeyEvent::MeetingShortcut,
                    ));
                }
            })
            .build(),
    )?;
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = app;
    Ok(())
}

pub fn refresh(app: &AppHandle, shared: &Shared) {
    #[cfg(not(target_os = "macos"))]
    let enabled = shared.settings().record_shortcut_enabled;
    #[cfg(target_os = "macos")]
    let result = {
        let listener = shared.hotkey.lock().unwrap().clone();
        listener.map_or(Ok(()), |listener| listener.set_bindings(&shared.settings()))
    };
    #[cfg(any(windows, target_os = "linux"))]
    let result = {
        static REQUESTED: AtomicU8 = AtomicU8::new(2);
        if REQUESTED.swap(u8::from(enabled), Ordering::SeqCst) == u8::from(enabled) {
            return;
        }
        #[cfg(target_os = "linux")]
        if crate::linux_input::is_wayland() || !crate::linux_input::has_x11_display() {
            *failure().lock().unwrap() = Some("Wayland: configure a recording shortcut for pennel --toggle-meeting in your desktop settings.".into());
            return;
        }
        if enabled { app.global_shortcut().register("Ctrl+Shift+R") }
        else { app.global_shortcut().unregister("Ctrl+Shift+R") }
            .map_err(|_| "The Ctrl+Shift+R recording shortcut is unavailable. Choose another binding in your desktop settings.".to_owned())
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let result: Result<(), String> = {
        let _ = enabled;
        Ok(())
    };
    #[cfg(target_os = "macos")]
    let _ = app;
    *failure().lock().unwrap() = result.err();
}
