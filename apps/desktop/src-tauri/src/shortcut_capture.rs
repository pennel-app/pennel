//! Shortcut capture is transient UI input, isolated from live actions and never logged.
use crate::state::Shared;
use pennel_core::{InterfaceLanguage, ShortcutBinding, ShortcutPlatform, ShortcutScope};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

static CAPTURING: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
pub fn capturing() -> bool {
    CAPTURING.load(Ordering::SeqCst)
}
fn session() -> &'static Mutex<Option<CaptureSession>> {
    static STATE: OnceLock<Mutex<Option<CaptureSession>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}
#[derive(Clone, Serialize)]
pub struct CaptureEvent {
    pub action: String,
    pub generation: u64,
    pub mods: Vec<String>,
    pub code: Option<String>,
    pub key: Option<String>,
    pub keycaps: Vec<String>,
    pub text: String,
    pub done: bool,
    pub cancelled: bool,
}
struct CaptureSession {
    action: String,
    generation: u64,
    peak: Vec<String>,
    done: bool,
}
impl CaptureSession {
    fn new(action: String, generation: u64) -> Self {
        Self {
            action,
            generation,
            peak: Vec::new(),
            done: false,
        }
    }
    fn input(
        &mut self,
        mut mods: Vec<String>,
        code: Option<String>,
        key: Option<String>,
        down: bool,
        platform: ShortcutPlatform,
        language: InterfaceLanguage,
    ) -> CaptureEvent {
        let keyed = code.is_some() && down;
        if keyed {
            for modifier in &mut mods {
                *modifier = modifier
                    .strip_prefix("left_")
                    .or_else(|| modifier.strip_prefix("right_"))
                    .unwrap_or(modifier)
                    .to_owned();
            }
        }
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
        mods.sort_by_key(|m| {
            ORDER
                .iter()
                .position(|name| name == m)
                .unwrap_or(usize::MAX)
        });
        mods.dedup();
        if !keyed && mods.len() >= self.peak.len() {
            self.peak = mods.clone();
        }
        let released_modifiers = code.is_none() && mods.is_empty() && !self.peak.is_empty();
        let cancelled = keyed && code.as_deref() == Some("Escape") && mods.is_empty();
        let done = keyed || released_modifiers;
        if released_modifiers {
            mods = self.peak.clone();
        }
        let code = keyed.then_some(code).flatten();
        let key = keyed.then_some(key).flatten();
        let label = pennel_core::format_shortcut(
            &ShortcutBinding {
                mods: mods.clone(),
                code: code.clone(),
                key: key.clone(),
            },
            platform,
            language,
        );
        self.done = done;
        CaptureEvent {
            action: self.action.clone(),
            generation: self.generation,
            mods,
            code,
            key,
            keycaps: label.keycaps,
            text: label.text,
            done,
            cancelled,
        }
    }
}
#[derive(Serialize)]
pub struct CaptureStarted {
    native: bool,
    generation: u64,
    reason: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
enum CaptureMode {
    Native,
    Dom(&'static str),
}
fn capture_mode(
    scope: ShortcutScope,
    has_listener: bool,
    accessibility: bool,
) -> Result<CaptureMode, String> {
    if has_listener && accessibility {
        return Ok(CaptureMode::Native);
    }
    if scope == ShortcutScope::Application {
        return Ok(CaptureMode::Dom(if has_listener && !accessibility {
            "accessibility_required"
        } else {
            "native_capture_unavailable"
        }));
    }
    Err("Native shortcut capture is unavailable on this system".into())
}
fn activate_with_focus<T>(
    focused: impl Fn() -> bool,
    activate: impl FnOnce() -> Result<T, String>,
    rollback: impl FnOnce(&T),
) -> Result<T, String> {
    if !focused() {
        return Err("Focus the Pennel workspace before recording a shortcut".into());
    }
    let value = activate()?;
    if !focused() {
        rollback(&value);
        return Err("Shortcut capture stopped because the workspace lost focus".into());
    }
    Ok(value)
}

#[tauri::command]
pub fn shortcut_capture_start(
    window: WebviewWindow,
    action: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<CaptureStarted, String> {
    if window.label() != "main" {
        return Err("Only the workspace can record a shortcut".into());
    }
    if !window.is_focused().unwrap_or(false) {
        return Err("Focus the Pennel workspace before recording a shortcut".into());
    }
    let app = window.app_handle().clone();
    let shared = shared.inner().clone();
    let (send, receive) = std::sync::mpsc::channel();
    // Admission, tap activation and the final focus check share the native UI serialization boundary.
    app.run_on_main_thread(move || {
        let result = activate_with_focus(
            || window.is_focused().unwrap_or(false),
            || start_capture(&window, action, &shared),
            |started| stop_generation(window.app_handle(), Some(started.generation)),
        );
        let _ = send.send(result);
    })
    .map_err(|error| error.to_string())?;
    receive
        .recv()
        .map_err(|_| "Shortcut capture activation was interrupted".to_owned())?
}
fn validate_capture_action(
    settings: &pennel_core::Settings,
    action: &str,
    platform: ShortcutPlatform,
) -> Result<(), String> {
    let binding = pennel_core::effective_shortcut(settings, action, platform);
    let candidate = settings
        .shortcuts
        .get(action)
        .and_then(Option::as_ref)
        .or(binding.as_ref());
    let capability = pennel_core::shortcut_capability(action, candidate, platform);
    if capability.available {
        Ok(())
    } else {
        Err(capability
            .reason
            .unwrap_or_else(|| "native_capture_unavailable".into()))
    }
}
fn start_capture(
    window: &WebviewWindow,
    action: String,
    shared: &Arc<Shared>,
) -> Result<CaptureStarted, String> {
    let scope = pennel_core::shortcut_scope(&action).ok_or("Unknown shortcut action")?;
    validate_capture_action(&shared.settings(), &action, crate::shortcuts::platform())?;
    if CAPTURING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A shortcut is already being recorded".into());
    }
    let app = window.app_handle();
    if shared.dictation_active.load(Ordering::SeqCst) {
        CAPTURING.store(false, Ordering::SeqCst);
        return Err("Finish dictating before recording a shortcut".into());
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    *session().lock().unwrap_or_else(|p| p.into_inner()) =
        Some(CaptureSession::new(action, generation));
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    if let Err(error) = crate::desktop_commands::refresh(app) {
        stop(app);
        return Err(error.to_string());
    }
    let result: Result<CaptureMode, String> = (|| {
        #[cfg(target_os = "macos")]
        {
            let listener = shared.hotkey.lock().unwrap().clone();
            let accessibility = crate::macos::permissions()
                .get("accessibility")
                .and_then(serde_json::Value::as_str)
                == Some("granted");
            let mode = capture_mode(scope, listener.is_some(), accessibility)?;
            if let Some(listener) = listener {
                // Clear the live table even for DOM fallback after Accessibility revocation.
                let suspended = listener.suspend_bindings();
                if mode == CaptureMode::Native {
                    suspended?;
                    let app = app.clone();
                    let shared = shared.clone();
                    listener.set_capture(Some(Box::new(move |event| {
                        native_mac_input(&app, &shared, generation, event)
                    })))?;
                }
            }
            Ok(mode)
        }
        #[cfg(any(windows, target_os = "linux"))]
        {
            #[cfg(target_os = "linux")]
            if crate::linux_input::is_wayland() {
                return capture_mode(scope, false, false);
            }
            let listener = shared
                .hotkey
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let mode = capture_mode(scope, listener.is_some(), true)?;
            // Release plugin grabs before raw capture; old pressed grabs remain only until their release.
            crate::record_shortcut::refresh(app, shared);
            if mode == CaptureMode::Native
                && let Some(listener) = listener
            {
                let app = app.clone();
                let shared = shared.clone();
                listener.set_capture(Some(Box::new(move |event| {
                    native_portable_input(&app, &shared, generation, event)
                })))?;
            }
            Ok(mode)
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        {
            capture_mode(scope, false, false)
        }
    })();
    match result {
        Ok(mode) => Ok(CaptureStarted {
            native: mode == CaptureMode::Native,
            generation,
            reason: match mode {
                CaptureMode::Native => None,
                CaptureMode::Dom(reason) => Some(reason.into()),
            },
        }),
        Err(error) => {
            stop(app);
            Err(error)
        }
    }
}
#[tauri::command]
pub fn shortcut_capture_stop(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the workspace can stop shortcut capture".into());
    }
    stop(window.app_handle());
    Ok(())
}
pub fn stop(app: &AppHandle) {
    stop_generation(app, None);
}
fn stop_generation(app: &AppHandle, generation: Option<u64>) {
    let stopped = {
        let mut session = session().lock().unwrap_or_else(|p| p.into_inner());
        if generation
            .is_some_and(|expected| session.as_ref().is_none_or(|s| s.generation != expected))
        {
            return;
        }
        session.take()
    };
    if !capturing() {
        return;
    }
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if let Some(shared) = app.try_state::<Arc<Shared>>() {
        let listener = shared.hotkey.lock().unwrap().clone();
        if let Some(listener) = listener {
            let _ = listener.set_capture(None);
        }
    }
    CAPTURING.store(false, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    if let Some(shared) = app.try_state::<Arc<Shared>>() {
        crate::record_shortcut::refresh(app, &shared);
    }
    let _ = crate::desktop_commands::refresh(app);
    if let Some(state) = stopped
        && !state.done
    {
        let _ = app.emit_to(
            "main",
            "shortcut-capture",
            CaptureEvent {
                action: state.action,
                generation: state.generation,
                mods: Vec::new(),
                code: None,
                key: None,
                keycaps: Vec::new(),
                text: String::new(),
                done: true,
                cancelled: true,
            },
        );
    }
}
#[cfg(target_os = "macos")]
fn native_mac_input(app: &AppHandle, shared: &Shared, generation: u64, event: serde_json::Value) {
    if !app
        .get_webview_window("main")
        .is_some_and(|window| window.is_focused().unwrap_or(false))
    {
        defer_stop(app, generation);
        return;
    }
    let Some(code) = event
        .get("code")
        .and_then(serde_json::Value::as_u64)
        .and_then(|code| u32::try_from(code).ok())
    else {
        return;
    };
    let Some(flags) = event.get("flags").and_then(serde_json::Value::as_u64) else {
        return;
    };
    let kind = event
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if kind == "down" && event.get("repeat").and_then(serde_json::Value::as_bool) == Some(true) {
        return;
    }
    let modifier = matches!(code, 54..=56 | 58..=63);
    let mut mods = Vec::new();
    for (family, left, right, generic, left_mask, right_mask) in [
        ("ctrl", "left_ctrl", "right_ctrl", 0x40000, 0x1, 0x2000),
        ("alt", "left_alt", "right_alt", 0x80000, 0x20, 0x40),
        ("shift", "left_shift", "right_shift", 0x20000, 0x2, 0x4),
        ("meta", "left_meta", "right_meta", 0x100000, 0x8, 0x10),
    ] {
        if flags & left_mask != 0 {
            mods.push(left.into());
        }
        if flags & right_mask != 0 {
            mods.push(right.into());
        }
        if flags & generic != 0 && flags & (left_mask | right_mask) == 0 {
            mods.push(family.into());
        }
    }
    // The fn flag is synthesized for arrows/function keys; record it only from its physical flags event.
    if modifier && flags & 0x800000 != 0 {
        mods.push("fn".into());
    }
    let key_code = if modifier {
        None
    } else {
        crate::native_shortcuts::MAC_CODES
            .iter()
            .find(|(_, key)| *key == code)
            .map(|(name, _)| (*name).to_owned())
    };
    if !modifier && key_code.is_none() {
        return;
    }
    let logical = key_code.as_deref().and_then(|name| {
        if name.starts_with('F')
            || matches!(
                name,
                "Escape"
                    | "Space"
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
        {
            Some(name.to_owned())
        } else {
            event
                .get("key")
                .and_then(serde_json::Value::as_str)
                .filter(|text| text.chars().count() == 1 && !text.chars().any(char::is_control))
                .map(str::to_owned)
        }
    });
    if !modifier && kind == "down" && logical.is_none() {
        return;
    }
    publish_input(
        app,
        shared,
        generation,
        mods,
        key_code,
        logical,
        kind == "down" || (modifier && !flags.eq(&0)),
    );
}
#[cfg(any(windows, target_os = "linux"))]
fn native_portable_input(
    app: &AppHandle,
    shared: &Shared,
    generation: u64,
    event: serde_json::Value,
) {
    if !app
        .get_webview_window("main")
        .is_some_and(|window| window.is_focused().unwrap_or(false))
    {
        defer_stop(app, generation);
        return;
    }
    let Some(mods) = event
        .get("mods")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
    else {
        return;
    };
    let Some(down) = event.get("down").and_then(serde_json::Value::as_bool) else {
        return;
    };
    let modifier = event.get("modifier").and_then(serde_json::Value::as_bool) == Some(true);
    let code = (!modifier).then(|| {
        event
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unidentified")
            .to_owned()
    });
    let key = (!modifier).then(|| {
        event
            .get("key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unidentified")
            .to_owned()
    });
    publish_input(app, shared, generation, mods, code, key, down);
}
fn publish_input(
    app: &AppHandle,
    shared: &Shared,
    generation: u64,
    mods: Vec<String>,
    code: Option<String>,
    key: Option<String>,
    down: bool,
) {
    let settings = shared.settings();
    let language = settings
        .ui_language
        .resolve(&[shared.system_interface_language.code().to_owned()]);
    let update = {
        let mut state = session().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = state
            .as_mut()
            .filter(|state| state.generation == generation && !state.done)
        else {
            return;
        };
        state.input(
            mods,
            code,
            key,
            down,
            crate::shortcuts::platform(),
            language,
        )
    };
    let done = update.done;
    let _ = app.emit_to("main", "shortcut-capture", update);
    if done {
        defer_stop(app, generation);
    }
}

fn defer_stop(app: &AppHandle, generation: u64) {
    let app = app.clone();
    // run_on_main_thread is inline on main. Originating on a worker truly queues teardown after the tap returns.
    let _ = std::thread::Builder::new()
        .name("pennel-shortcut-stop".into())
        .spawn(move || {
            let target = app.clone();
            let _ = app.run_on_main_thread(move || stop_generation(&target, Some(generation)));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_delayed_or_lost_focus_start_rolls_back_without_a_binding() {
        use std::cell::Cell;
        let focus = Cell::new(false);
        let active = Cell::new(false);
        let menus_suspended = Cell::new(false);
        let activate = || {
            active.set(true);
            menus_suspended.set(true);
            Ok(())
        };
        assert!(
            activate_with_focus(
                || focus.get(),
                activate,
                |_| {
                    active.set(false);
                    menus_suspended.set(false);
                }
            )
            .is_err()
        );
        assert!(!active.get());
        assert!(!menus_suspended.get());
        focus.set(true);
        assert!(
            activate_with_focus(
                || focus.get(),
                || {
                    active.set(true);
                    menus_suspended.set(true);
                    focus.set(false);
                    Ok(())
                },
                |_| {
                    active.set(false);
                    menus_suspended.set(false);
                }
            )
            .is_err()
        );
        assert!(!active.get());
        assert!(!menus_suspended.get());
    }
    #[test]
    fn an_existing_listener_without_accessibility_uses_only_application_dom_capture() {
        assert_eq!(
            capture_mode(ShortcutScope::Application, true, false).unwrap(),
            CaptureMode::Dom("accessibility_required")
        );
        assert!(capture_mode(ShortcutScope::Global, true, false).is_err());
        assert_eq!(
            capture_mode(ShortcutScope::Application, false, true).unwrap(),
            CaptureMode::Dom("native_capture_unavailable")
        );
        assert_eq!(
            capture_mode(ShortcutScope::Application, true, true).unwrap(),
            CaptureMode::Native
        );
    }

    #[test]
    fn x11_capture_entry_refuses_affected_keyed_actions_including_saved_bindings() {
        let mut settings = pennel_core::default_settings();
        for (id, code, key) in [
            ("record", "KeyQ", "a"),
            ("open_pennel", "KeyW", "z"),
            ("cancel", "Escape", "Escape"),
            ("next_model", "F5", "F5"),
        ] {
            settings.shortcuts.insert(
                id.into(),
                Some(ShortcutBinding {
                    mods: if matches!(id, "record" | "open_pennel") {
                        vec!["ctrl".into(), "alt".into()]
                    } else {
                        vec![]
                    },
                    code: Some(code.into()),
                    key: Some(key.into()),
                }),
            );
            assert_eq!(
                validate_capture_action(&settings, id, ShortcutPlatform::LinuxX11).unwrap_err(),
                "x11_keyed_shortcuts_unavailable"
            );
        }
        assert!(validate_capture_action(&settings, "dictate", ShortcutPlatform::LinuxX11).is_ok());
        assert!(validate_capture_action(&settings, "find", ShortcutPlatform::LinuxX11).is_ok());
        assert!(validate_capture_action(&settings, "record", ShortcutPlatform::Windows).is_ok());
    }

    #[test]
    fn modifier_only_capture_finishes_after_release_and_preserves_sides() {
        let mut state = CaptureSession::new("dictate".into(), 7);
        let press = state.input(
            vec!["right_alt".into()],
            None,
            None,
            true,
            pennel_core::ShortcutPlatform::Macos,
            pennel_core::InterfaceLanguage::English,
        );
        assert!(!press.done);
        assert_eq!(press.keycaps, ["right ⌥"]);
        let release = state.input(
            vec![],
            None,
            None,
            false,
            pennel_core::ShortcutPlatform::Macos,
            pennel_core::InterfaceLanguage::English,
        );
        assert!(release.done);
        assert_eq!(release.mods, ["right_alt"]);
    }
    #[test]
    fn keyed_capture_uses_current_generic_modifiers_and_esc_alone_cancels() {
        let mut state = CaptureSession::new("find".into(), 1);
        state.input(
            vec!["left_meta".into(), "right_alt".into()],
            None,
            None,
            true,
            pennel_core::ShortcutPlatform::Macos,
            pennel_core::InterfaceLanguage::English,
        );
        let event = state.input(
            vec!["left_meta".into(), "right_alt".into()],
            Some("KeyQ".into()),
            Some("a".into()),
            true,
            pennel_core::ShortcutPlatform::Macos,
            pennel_core::InterfaceLanguage::English,
        );
        assert!(event.done);
        assert!(!event.cancelled);
        assert_eq!(event.mods, ["alt", "meta"]);
        assert_eq!(event.key.as_deref(), Some("a"));
        let mut state = CaptureSession::new("cancel".into(), 2);
        assert!(
            state
                .input(
                    vec![],
                    Some("Escape".into()),
                    Some("Escape".into()),
                    true,
                    pennel_core::ShortcutPlatform::Macos,
                    pennel_core::InterfaceLanguage::English
                )
                .cancelled
        );
    }
}
