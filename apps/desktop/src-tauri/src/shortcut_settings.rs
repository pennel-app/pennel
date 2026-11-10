//! Settings mutations use the core's single conflict/legacy writer and the shell refresh path.
use crate::{commands::AppInfo, state::Shared};
use pennel_core::{Settings, ShortcutBinding, ShortcutPlatform, ShortcutProblem};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, State, WebviewWindow};

#[derive(Serialize)]
pub struct WriteError {
    kind: String,
    reason: String,
    action: Option<String>,
}
impl From<String> for WriteError {
    fn from(reason: String) -> Self {
        Self {
            kind: "save".into(),
            reason,
            action: None,
        }
    }
}
impl From<ShortcutProblem> for WriteError {
    fn from(problem: ShortcutProblem) -> Self {
        Self {
            kind: problem.kind,
            reason: problem.reason,
            action: problem.action,
        }
    }
}
pub(crate) struct EditGuard<'a>(&'a AtomicBool);
impl Drop for EditGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
/// Shares the capture/listening admission protocol so new input cannot race a rebind.
pub(crate) fn editing(
    active: &AtomicBool,
    blocked: impl Fn() -> bool,
) -> Result<EditGuard<'_>, String> {
    active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "shortcut_input_active".to_owned())?;
    let guard = EditGuard(active);
    if blocked() {
        return Err("shortcut_input_active".into());
    }
    Ok(guard)
}
pub(crate) fn binding_fields_changed(previous: &Settings, next: &Settings) -> bool {
    previous.hotkey != next.hotkey
        || previous.record_shortcut_enabled != next.record_shortcut_enabled
        || previous.shortcuts != next.shortcuts
}
fn workspace(window: &WebviewWindow) -> Result<(), WriteError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Only the workspace can change shortcuts".to_owned().into())
    }
}
fn set(
    settings: &mut Settings,
    action: &str,
    binding: Option<ShortcutBinding>,
    platform: ShortcutPlatform,
    replace: bool,
) -> Result<(), WriteError> {
    pennel_core::set_shortcut(settings, action, binding, platform, replace)
        .map_err(WriteError::from)
}
#[tauri::command]
pub async fn set_shortcut(
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
    action: String,
    binding: Option<ShortcutBinding>,
    replace: bool,
) -> Result<AppInfo, WriteError> {
    workspace(&window)?;
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::change_settings(&app, &shared, |settings| {
            set(
                settings,
                &action,
                binding,
                crate::shortcuts::platform(),
                replace,
            )
        })
    })
    .await
    .map_err(|_| WriteError::from("Shortcut update was interrupted".to_owned()))?
}
#[tauri::command]
pub async fn reset_shortcut(
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
    action: String,
) -> Result<AppInfo, WriteError> {
    workspace(&window)?;
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::change_settings(&app, &shared, |settings| {
            pennel_core::reset_shortcut(settings, &action, crate::shortcuts::platform())
                .map_err(WriteError::from)
        })
    })
    .await
    .map_err(|_| WriteError::from("Shortcut update was interrupted".to_owned()))?
}
#[tauri::command]
pub async fn reset_all_shortcuts(
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo, WriteError> {
    workspace(&window)?;
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::change_settings(&app, &shared, |settings| {
            pennel_core::reset_all_shortcuts(settings, crate::shortcuts::platform());
            Ok::<(), WriteError>(())
        })
    })
    .await
    .map_err(|_| WriteError::from("Shortcut update was interrupted".to_owned()))?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_preferences_do_not_require_input_admission_but_all_shortcut_fields_do() {
        let settings = pennel_core::default_settings();
        let mut next = settings.clone();
        next.live_text_enabled = !next.live_text_enabled;
        assert!(!binding_fields_changed(&settings, &next));
        next.hotkey = pennel_core::HotkeyTrigger::RightOption;
        assert!(binding_fields_changed(&settings, &next));
        next = settings.clone();
        next.record_shortcut_enabled = !next.record_shortcut_enabled;
        assert!(binding_fields_changed(&settings, &next));
        next = settings.clone();
        next.shortcuts.insert("dictate".into(), None);
        assert!(binding_fields_changed(&settings, &next));
    }
    #[test]
    fn editing_refuses_live_or_capturing_sessions_and_reserves_admission_until_drop() {
        let active = std::sync::atomic::AtomicBool::new(true);
        assert!(editing(&active, || false).is_err());
        assert!(active.load(std::sync::atomic::Ordering::SeqCst));
        active.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(editing(&active, || true).is_err());
        assert!(!active.load(std::sync::atomic::Ordering::SeqCst));
        let guard = editing(&active, || false).unwrap_or_else(|_| panic!("edit admission failed"));
        assert!(active.load(std::sync::atomic::Ordering::SeqCst));
        assert!(editing(&active, || false).is_err());
        drop(guard);
        assert!(!active.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn rejected_conflict_preserves_both_bindings_and_replace_is_one_change() {
        let mut settings = pennel_core::default_settings();
        let original = settings.clone();
        let binding = pennel_core::default_shortcut("find", ShortcutPlatform::Macos);
        let problem = set(
            &mut settings,
            "record",
            binding.clone(),
            ShortcutPlatform::Macos,
            false,
        )
        .err()
        .unwrap();
        assert_eq!(problem.kind, "conflict");
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        set(
            &mut settings,
            "record",
            binding,
            ShortcutPlatform::Macos,
            true,
        )
        .unwrap_or_else(|_| panic!("replacement failed"));
        assert!(
            pennel_core::effective_shortcut(&settings, "find", ShortcutPlatform::Macos).is_none()
        );
        assert!(!settings.record_shortcut_enabled);
    }
}
