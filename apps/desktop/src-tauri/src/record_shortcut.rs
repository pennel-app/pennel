//! Windows one-press registration and shared listening state. X11 keyed registration is deliberately unavailable.
use crate::state::Shared;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

static LISTENING: AtomicBool = AtomicBool::new(false);
static NEXT_MODEL: AtomicBool = AtomicBool::new(false);
pub fn next_model_enabled() -> bool {
    NEXT_MODEL.load(Ordering::SeqCst)
}
#[allow(dead_code)] // Consumed by the pill metadata integration; never infer live readiness from an idle row.
pub fn listening_action_ready(shared: &Shared, action: u32) -> bool {
    let id = match action {
        3 => "cancel",
        4 => "next_model",
        _ => return false,
    };
    if !is_listening()
        || crate::shortcut_capture::capturing()
        || (action == 4 && !next_model_enabled())
        || shared
            .hotkey
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none()
        || action_error(id).is_some()
    {
        return false;
    }
    let platform = crate::shortcuts::platform();
    let Some(binding) = pennel_core::effective_shortcut(&shared.settings(), id, platform) else {
        return false;
    };
    if !pennel_core::shortcut_capability(id, Some(&binding), platform).available {
        return false;
    }
    #[cfg(target_os = "macos")]
    if crate::macos::permissions()
        .get("accessibility")
        .and_then(serde_json::Value::as_str)
        != Some("granted")
    {
        return false;
    }
    #[cfg(target_os = "linux")]
    if binding.code.is_some() {
        return false;
    }
    true
}
pub fn set_next_model_enabled(enabled: bool) {
    NEXT_MODEL.store(enabled, Ordering::SeqCst);
}
pub fn is_listening() -> bool {
    LISTENING.load(Ordering::SeqCst)
}
fn failure() -> &'static Mutex<Option<String>> {
    static FAILURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    FAILURE.get_or_init(|| Mutex::new(None))
}
fn presentation() -> &'static Mutex<(String, bool)> {
    static VALUE: OnceLock<Mutex<(String, bool)>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new((String::new(), false)))
}
fn record_failure_message() -> String {
    let (label, dutch) = presentation()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let language = if dutch {
        pennel_core::InterfaceLanguage::Dutch
    } else {
        pennel_core::InterfaceLanguage::English
    };
    let key = if label.is_empty() {
        "backend.recording_shortcut_unavailable"
    } else {
        "backend.recording_shortcut_unavailable_named"
    };
    pennel_core::ui_text(language, key, &[("label", &label)])
}
pub fn error() -> Option<String> {
    if action_error("record").is_some()
        || failure()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    {
        Some(record_failure_message())
    } else {
        None
    }
}

pub fn action_error(action: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let number = match action {
            "record" => 1,
            "open_pennel" => 2,
            "cancel" => 3,
            "next_model" => 4,
            _ => return None,
        };
        portable::store()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .errors
            .get(&number)
            .cloned()
    }
    #[cfg(target_os = "linux")]
    {
        let _ = action;
        None
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        if matches!(
            action,
            "record" | "open_pennel" | "cancel" | "next_model" | "dictate"
        ) && failure()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            Some("native_shortcut_unavailable".into())
        } else {
            None
        }
    }
}
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    if !crate::portable_shortcuts::uses_registration_backend(crate::shortcuts::platform()) {
        return Ok(());
    }
    #[cfg(windows)]
    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(portable::event)
            .build(),
    )?;
    #[cfg(not(windows))]
    let _ = app;
    Ok(())
}

pub fn set_listening(app: &AppHandle, shared: &Shared, listening: bool) {
    LISTENING.store(listening, Ordering::SeqCst);
    let listener = shared
        .hotkey
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if let Some(listener) = listener
        && let Err(error) = listener.set_listening(listening)
    {
        *failure().lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
    }
    refresh(app, shared);
}
pub fn refresh(app: &AppHandle, shared: &Shared) {
    let settings = shared.settings();
    let language = settings
        .ui_language
        .resolve(&[shared.system_interface_language.code().to_owned()]);
    let label = pennel_core::effective_shortcut(&settings, "record", crate::shortcuts::platform())
        .as_ref()
        .map(|binding| {
            pennel_core::format_shortcut(binding, crate::shortcuts::platform(), language).text
        })
        .unwrap_or_default();
    *presentation().lock().unwrap_or_else(|p| p.into_inner()) =
        (label, language == pennel_core::InterfaceLanguage::Dutch);
    #[cfg(target_os = "macos")]
    let result = {
        let listener = shared
            .hotkey
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        listener.map_or(Ok(()), |listener| {
            if crate::shortcut_capture::capturing() {
                listener.suspend_bindings()
            } else {
                listener.set_bindings_with_next_model(&shared.settings(), next_model_enabled())
            }
        })
    };
    #[cfg(windows)]
    let result = {
        let _ = shared;
        portable::refresh(app)
    };
    #[cfg(target_os = "linux")]
    let result = {
        let listener = shared
            .hotkey
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        listener.map_or(Ok(()), |listener| {
            listener.set_bindings_with_next_model(&settings, next_model_enabled())
        })
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let result: Result<(), String> = {
        let _ = (app, shared);
        Ok(())
    };
    let next = result.err();
    let changed = {
        let mut failure = failure().lock().unwrap_or_else(|p| p.into_inner());
        let changed = *failure != next;
        *failure = next;
        changed
    };
    if changed {
        let _ = app.emit("settings-changed", ());
    }
}

#[cfg(windows)]
mod portable {
    use super::*;
    use crate::portable_shortcuts::{Plan, Registration};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tauri::{Emitter, Manager};
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
    };
    #[derive(Clone)]
    struct Entry {
        shortcut: Shortcut,
        row: Registration,
        pressed: bool,
    }
    #[derive(Default)]
    pub(super) struct Store {
        entries: HashMap<u32, Entry>,
        pub(super) errors: HashMap<u32, String>,
    }
    pub(super) fn store() -> &'static Mutex<Store> {
        static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
        STORE.get_or_init(|| Mutex::new(Store::default()))
    }
    fn desired(shared: &Shared) -> Plan {
        if shared
            .hotkey
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none()
        {
            return Plan::default();
        }
        crate::portable_shortcuts::plan(
            &shared.settings(),
            crate::shortcuts::platform(),
            is_listening(),
            crate::shortcut_capture::capturing(),
            next_model_enabled(),
        )
    }
    fn shortcut(row: &Registration) -> Option<Shortcut> {
        let code = row.binding.code.as_deref()?.parse::<Code>().ok()?;
        let mut modifiers = Modifiers::empty();
        for modifier in &row.binding.mods {
            modifiers |= match modifier.as_str() {
                "ctrl" => Modifiers::CONTROL,
                "alt" => Modifiers::ALT,
                "shift" => Modifiers::SHIFT,
                "meta" => Modifiers::SUPER,
                _ => return None,
            };
        }
        Some(Shortcut::new(Some(modifiers), code))
    }
    pub(super) fn event(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
        let Some(shared) = app.try_state::<Arc<Shared>>() else {
            return;
        };
        let plan = desired(&shared);
        let action = {
            let mut state = store().lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry) = state.entries.get_mut(&shortcut.id()) else {
                return;
            };
            entry.pressed = event.state() == ShortcutState::Pressed;
            (entry.pressed && plan.registrations.contains(&entry.row)).then_some(entry.row.action)
        };
        if let Some(action) = action
            && !crate::shortcut_capture::capturing()
        {
            let _ = shared.dictation.send(crate::dictation::Msg::Hotkey(
                crate::hotkey::HotkeyEvent::Action(action),
            ));
        }
        if event.state() == ShortcutState::Released {
            // Native release notifications may arrive off-main. Never block the callback on unregister.
            let target = app.clone();
            let _ = app.run_on_main_thread(move || {
                let _ = refresh_now(&target);
            });
        }
    }
    pub(super) fn refresh(app: &AppHandle) -> Result<(), String> {
        let target = app.clone();
        let (send, receive) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = send.send(refresh_now(&target));
        })
        .map_err(|error| error.to_string())?;
        receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "Shortcut registration did not finish".to_owned())?
    }
    fn refresh_now(app: &AppHandle) -> Result<(), String> {
        let Some(shared) = app.try_state::<Arc<Shared>>() else {
            return Err("Shortcut registration is not ready".into());
        };
        let plan = desired(&shared);
        let before_errors = store()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .errors
            .clone();
        let existing: Vec<_> = store()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .values()
            .cloned()
            .collect();
        let mut errors = HashMap::new();
        for entry in existing {
            let wanted = plan.registrations.contains(&entry.row);
            if !crate::portable_shortcuts::retain_registration(wanted, entry.pressed) {
                match app.global_shortcut().unregister(entry.shortcut) {
                    Ok(()) => {
                        store()
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .entries
                            .remove(&entry.shortcut.id());
                    }
                    Err(_) => {
                        errors.insert(entry.row.action, "shortcut_release_failed".into());
                    }
                }
            }
        }
        for row in &plan.registrations {
            let Some(key) = shortcut(row) else {
                errors.insert(row.action, "native_key_not_supported".into());
                continue;
            };
            let current = store()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entries
                .get(&key.id())
                .cloned();
            if let Some(current) = current {
                if current.row != *row {
                    errors.insert(row.action, "shortcut_registration_failed".into());
                }
                continue;
            }
            match app.global_shortcut().register(key) {
                Ok(()) => {
                    store()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .entries
                        .insert(
                            key.id(),
                            Entry {
                                shortcut: key,
                                row: row.clone(),
                                pressed: false,
                            },
                        );
                }
                Err(_) => {
                    errors.insert(row.action, "shortcut_registration_failed".into());
                }
            }
        }
        let registered: Vec<_> = {
            let mut state = store().lock().unwrap_or_else(|p| p.into_inner());
            state.errors = errors;
            state
                .entries
                .values()
                .filter(|entry| {
                    plan.registrations.contains(&entry.row) && matches!(entry.row.action, 1 | 2)
                })
                .filter_map(|entry| {
                    crate::hotkey::TriggerBinding::resolve(
                        &entry.row.binding,
                        crate::portable_shortcuts::virtual_key,
                    )
                    .map(|binding| (entry.row.action, binding))
                })
                .collect()
        };
        let listener = shared
            .hotkey
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(listener) = listener {
            listener.configure_portable_bindings(
                &shared.settings(),
                next_model_enabled(),
                registered,
            );
        }
        if before_errors != store().lock().unwrap_or_else(|p| p.into_inner()).errors {
            let _ = app.emit("settings-changed", ());
        }
        Ok(())
    }
}
