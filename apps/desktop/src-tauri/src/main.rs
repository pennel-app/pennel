#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_identity;
mod audio;
mod call_detection;
mod capabilities;
#[cfg(target_os = "macos")]
mod capture;
mod claude_access;
#[cfg(any(not(target_os = "macos"), test))]
mod clipboard;
#[cfg(not(target_os = "macos"))]
mod clipboard_system;
mod cloud;
mod commands;
mod desktop_commands;
mod desktop_environment;
mod diagnostics;
mod dictation;
mod external_paste;
mod foreground;
mod gesture;
mod hotkey;
mod inference;
mod keys;
#[cfg(any(target_os = "macos", test))]
mod legacy_migration;
#[cfg(target_os = "linux")]
mod linux_input;
#[cfg(target_os = "linux")]
mod linux_meetings;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_tray;
mod meeting;
mod meeting_runtime;
mod owned_dictation;
mod paste_queue;
mod pill;
mod platform_audio;
mod platform_clipboard;
mod platform_commands;
mod record_shortcut;
mod recording_metadata;
mod resample;
mod secrets;
mod speakers;
mod state;
mod status;
mod style_application;
mod ui;
#[cfg(windows)]
mod windows_meetings;
mod workspace;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pennel_core::PennelEngine;
use tauri::menu::MenuItem;
#[cfg(not(target_os = "macos"))]
use tauri::menu::{Menu, PredefinedMenuItem};
#[cfg(not(target_os = "macos"))]
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::diagnostics::Log;
use crate::hotkey::Listener;
use crate::state::Shared;

const LOG: Log = Log::new("app");

/// Models are large, so everything lives in the local (non-roaming) data folder.
/// On a Mac (development only) it stays apart from the native app's "Pennel" folder.
fn is_preview(app: &AppHandle) -> bool {
    cfg!(debug_assertions) || app.config().identifier.ends_with(".preview")
}

fn data_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(directory) = capture::data_directory(app).map_err(|error| {
        tauri::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    })? {
        return Ok(directory);
    }
    let name = if is_preview(app) {
        "Pennel Preview"
    } else {
        "Pennel"
    };
    app.path().local_data_dir().map(|dir| dir.join(name))
}

pub fn show_main(app: &AppHandle, tab: Option<&str>) {
    let kind = match tab {
        Some("meeting") => "live",
        Some("history") => "history",
        Some("pad" | "dictation-home") => "dictation",
        Some(
            "settings" | "overview" | "providers" | "models" | "dictation" | "dictionary"
            | "styles" | "meeting-settings" | "recording" | "appearance" | "account" | "local",
        ) => "settings",
        Some("meetings" | "library" | "chat") => "library",
        _ => "workspace",
    };
    let tab = match tab {
        Some("pad") => Some("pad"),
        Some("dictation-home") => Some("home"),
        Some("history" | "settings") => None,
        other => other,
    };
    show_window(app, kind, tab);
}

pub fn open_default(app: &AppHandle) {
    receive_launch(app, Vec::new());
}

fn receive_launch(app: &AppHandle, arguments: Vec<String>) {
    if let Err(arguments) = workspace::queue_launch(app, arguments) {
        dispatch_launch(app, &arguments);
    }
}

fn dispatch_launch(app: &AppHandle, arguments: &[String]) {
    if dispatch_cli(app, arguments) {
        return;
    }
    let status = app.state::<Arc<Shared>>().settings().onboarding.status;
    present(
        app,
        workspace::explicit_presentation(cfg!(target_os = "macos"), status),
    );
}

fn present(app: &AppHandle, presentation: workspace::Presentation) {
    match presentation {
        workspace::Presentation::Onboarding => show_window(app, "onboarding", None),
        workspace::Presentation::Workspace => show_main(app, None),
        workspace::Presentation::Background => {}
    }
}

pub fn notify_visibility(app: &AppHandle, label: &str, visible: bool) {
    #[cfg(target_os = "macos")]
    if matches!(label, "main" | "onboarding") {
        let product_visible = visible
            || app.webview_windows().iter().any(|(other, window)| {
                other != label
                    && matches!(other.as_str(), "main" | "onboarding")
                    && window.is_visible().unwrap_or(false)
            });
        macos::product_windows_visible(product_visible);
    }
    let _ = app.emit_to(
        label,
        "workspace-visibility-changed",
        serde_json::json!({ "visible": visible }),
    );
}

pub fn show_window(app: &AppHandle, kind: &str, tab: Option<&str>) {
    let spec = workspace::window_spec(kind);
    let (label, mode) = (spec.label, spec.mode);
    workspace::request(app, label, workspace::window_request_route(kind, tab));
    if kind == "onboarding" {
        workspace::request_onboarding(app, tab);
    }
    let url = workspace::window_url(kind, tab);
    let app = app.clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !workspace::requested(&handle, label) {
            return;
        }
        let window = handle.get_webview_window(label).or_else(|| {
            let (width, height) = spec.size;
            let fit = if mode == "prompt" {
                None
            } else {
                preferred_monitor(&handle).map(|monitor| {
                    let area = monitor.work_area();
                    workspace::fit_window_size(
                        spec.size,
                        spec.minimum,
                        (area.size.width, area.size.height),
                        monitor.scale_factor(),
                    )
                })
            };
            // The legacy URL alias preserves the requested section before pending route delivery.
            let mut builder = tauri::WebviewWindowBuilder::new(
                &handle,
                label,
                tauri::WebviewUrl::App(url.into()),
            )
            .title(ui::message(spec.title))
            .inner_size(
                fit.map_or(width, |size| size.width),
                fit.map_or(height, |size| size.height),
            )
            .visible(false);
            if mode != "prompt" {
                builder = native_title_bar(builder)
                    .min_inner_size(
                        fit.map_or(spec.minimum.0, |size| size.min_width),
                        fit.map_or(spec.minimum.1, |size| size.min_height),
                    )
                    .center();
            }
            #[cfg(target_os = "macos")]
            if label == "main" {
                builder = builder.transparent(true);
            }
            if mode == "prompt" {
                builder = builder
                    .decorations(false)
                    .transparent(true)
                    .shadow(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .focused(false)
                    .focusable(false)
                    .resizable(false);
            }
            builder.build().ok()
        });
        if let Some(window) = window {
            apply_appearance(&handle);
            #[cfg(target_os = "macos")]
            capture::prepare(&handle, &window);
            fit_window_to_work_area(&handle, &window);
            let _ = window.unminimize();
            if mode == "prompt" {
                #[cfg(target_os = "macos")]
                if let Ok(pointer) = window.ns_window() {
                    unsafe {
                        macos::configure_panel(pointer, true);
                        if let Err(error) = desktop_environment::attach(&window, true) {
                            LOG.error(format!("native prompt material unavailable: {error}"));
                        }
                        apply_appearance(&handle);
                        capture::prepare(&handle, &window);
                        macos::show_panel_at_top_right(
                            pointer,
                            spec.size.0,
                            spec.size.1,
                            workspace::PROMPT_MARGIN,
                        );
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Some(monitor) = preferred_monitor(&handle) {
                        let area = monitor.work_area();
                        let (x, y) = workspace::prompt_origin(
                            (area.position.x, area.position.y),
                            (area.size.width, area.size.height),
                            monitor.scale_factor(),
                            spec.size,
                        );
                        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                    crate::pill::show_without_focus(&window);
                }
            } else {
                if label == "main"
                    && let Err(error) = desktop_environment::attach(&window, false)
                {
                    LOG.error(format!("native workspace material unavailable: {error}"));
                }
                #[cfg(target_os = "macos")]
                capture::prepare(&handle, &window);
                let ready = handle
                    .state::<ExitState>()
                    .sync
                    .lock()
                    .unwrap()
                    .ready
                    .contains(label);
                if ready {
                    if label == "onboarding" {
                        workspace::deliver_onboarding(&handle);
                    } else {
                        workspace::deliver(&handle, label);
                    }
                }
                // Show the loading surface immediately; keep queued routes until listeners are ready.
                #[cfg(target_os = "macos")]
                macos::product_windows_visible(true);
                let _ = window.show();
                if workspace::take_focus(&handle, label) {
                    let _ = window.set_focus();
                }
                if ready {
                    notify_visibility(&handle, label, true);
                }
            }
        }
    });
}

/// Native controls remain in their system positions; the frontend reserves their measured inset.
fn native_title_bar<'a, R: tauri::Runtime, M: Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    builder
}

fn preferred_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
    let cursor = app.cursor_position().ok();
    let monitors = app.available_monitors().unwrap_or_default();
    cursor
        .and_then(|cursor| {
            monitors.into_iter().find(|monitor| {
                let (position, size) = (monitor.position(), monitor.size());
                cursor.x >= f64::from(position.x)
                    && cursor.x < f64::from(position.x) + f64::from(size.width)
                    && cursor.y >= f64::from(position.y)
                    && cursor.y < f64::from(position.y) + f64::from(size.height)
            })
        })
        .or_else(|| app.primary_monitor().ok().flatten())
}

fn fit_window_to_work_area(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Some(spec) = workspace::label_spec(window.label()).filter(|spec| spec.mode != "prompt")
    else {
        return;
    };
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }
    let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| preferred_monitor(app))
    else {
        return;
    };
    let (Ok(inner), Ok(outer)) = (window.inner_size(), window.outer_size()) else {
        return;
    };
    let area = monitor.work_area();
    if area.size.width == 0 || area.size.height == 0 {
        return;
    }
    let scale = monitor.scale_factor();
    let window_scale = window.scale_factor().unwrap_or(scale);
    let window_scale = if window_scale.is_finite() && window_scale > 0.0 {
        window_scale
    } else {
        1.0
    };
    let frame_width = outer.width.saturating_sub(inner.width);
    let frame_height = outer.height.saturating_sub(inner.height);
    let minimum = spec.minimum;
    let fit = workspace::fit_window_size(
        (
            f64::from(inner.width) / window_scale,
            f64::from(inner.height) / window_scale,
        ),
        minimum,
        (
            area.size.width.saturating_sub(frame_width),
            area.size.height.saturating_sub(frame_height),
        ),
        scale,
    );
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(fit.min_width, fit.min_height)));
    if f64::from(inner.width) / window_scale > fit.width + 0.5
        || f64::from(inner.height) / window_scale > fit.height + 0.5
    {
        let _ = window.set_size(tauri::LogicalSize::new(fit.width, fit.height));
    }
    let (Ok(position), Ok(outer)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let margin = (workspace::WINDOW_MARGIN * scale) as i64;
    let left = i64::from(area.position.x) + margin;
    let top = i64::from(area.position.y) + margin;
    let right =
        (i64::from(area.position.x) + i64::from(area.size.width) - i64::from(outer.width) - margin)
            .max(left);
    let bottom = (i64::from(area.position.y) + i64::from(area.size.height)
        - i64::from(outer.height)
        - margin)
        .max(top);
    let x = i64::from(position.x).clamp(left, right) as i32;
    let y = i64::from(position.y).clamp(top, bottom) as i32;
    if x != position.x || y != position.y {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

pub fn apply_appearance(app: &AppHandle) {
    let Some(shared) = app.try_state::<Arc<Shared>>() else {
        return;
    };
    let theme = match shared.settings().appearance {
        pennel_core::AppearancePreference::System => None,
        pennel_core::AppearancePreference::Light => Some(tauri::Theme::Light),
        pennel_core::AppearancePreference::Dark => Some(tauri::Theme::Dark),
    };
    for window in app.webview_windows().into_values() {
        let _ = window.set_theme(theme);
        desktop_environment::apply_appearance(&window, theme);
    }
}

fn hotkey_label(trigger: pennel_core::HotkeyTrigger) -> String {
    use pennel_core::HotkeyTrigger::*;
    crate::ui::message(match trigger {
        Fn if cfg!(target_os = "macos") => "fn",
        RightControl | Fn => "right Ctrl",
        RightOption if cfg!(target_os = "macos") => "right Option",
        RightCommand if cfg!(target_os = "macos") => "right Command",
        RightOption => "right Alt",
        RightCommand => "right Windows key",
        ControlSuper => "Ctrl + Windows key",
    })
}

pub struct TrayHint(pub MenuItem<tauri::Wry>);

#[derive(Default)]
struct ExitSync {
    sequence: u64,
    ready: std::collections::HashSet<String>,
    pending: std::collections::HashSet<String>,
    failed: bool,
}

#[derive(Default)]
pub struct ExitState {
    pub requested: AtomicBool,
    pub quit_attempts: std::sync::atomic::AtomicU64,
    pub transition_gate: Mutex<()>,
    approved: AtomicBool,
    sync: Mutex<ExitSync>,
    flush_gate: Mutex<()>,
    changed: std::sync::Condvar,
}

#[cfg(any(target_os = "macos", test))]
impl ExitState {
    fn permits_hide(&self, updating: &AtomicBool) -> bool {
        !self.requested.load(Ordering::SeqCst) && !updating.load(Ordering::SeqCst)
    }

    fn serialize_hide<T>(&self, updating: &AtomicBool, operation: impl FnOnce() -> T) -> Option<T> {
        let _flush = self.flush_gate.lock().unwrap();
        self.permits_hide(updating).then(operation)
    }
}

pub fn frontend_ready(app: &AppHandle, label: &str) {
    app.state::<ExitState>()
        .sync
        .lock()
        .unwrap()
        .ready
        .insert(label.to_owned());
    if label == "onboarding" {
        workspace::deliver_onboarding(app);
    } else {
        workspace::deliver(app, label);
    }
    if workspace::requested(app, label) {
        apply_appearance(app);
        if let Some(window) = app.get_webview_window(label) {
            fit_window_to_work_area(app, &window);
            #[cfg(target_os = "macos")]
            macos::product_windows_visible(true);
            let _ = window.show();
            if workspace::take_focus(app, label) {
                let _ = window.set_focus();
            }
            notify_visibility(app, label, true);
        }
    } else {
        notify_visibility(app, label, false);
    }
}

pub fn frontend_flushed(app: &AppHandle, label: &str, sequence: u64, success: bool) {
    let state = app.state::<ExitState>();
    let mut sync = state.sync.lock().unwrap();
    if sync.sequence == sequence && sync.pending.remove(label) {
        sync.failed |= !success;
        state.changed.notify_all();
    }
}

pub struct FrontendFlush {
    pub request_id: u64,
    pub result: Result<(), String>,
}

pub fn flush_frontends(app: &AppHandle, event: &str) -> FrontendFlush {
    let state = app.state::<ExitState>();
    let _operation = state.flush_gate.lock().unwrap();
    let mut flush = flush_frontend_requests(app, event, None);
    if flush.result.is_ok() && matches!(event, "prepare-exit" | "prepare-update") {
        flush.result = app
            .state::<Arc<Shared>>()
            .engine
            .retry_unsaved_history()
            .map_err(|_| {
                "Save or discard your unsaved dictations in History first. Pennel will stay open."
                    .into()
            });
    }
    flush
}

fn flush_frontend_requests(app: &AppHandle, event: &str, target: Option<&str>) -> FrontendFlush {
    let state = app.state::<ExitState>();
    let windows = app.webview_windows();
    let (sequence, labels) = {
        let mut sync = state.sync.lock().unwrap();
        sync.sequence = sync.sequence.wrapping_add(1);
        sync.failed = false;
        sync.pending = sync
            .ready
            .iter()
            .filter(|label| {
                windows.contains_key(*label) && target.is_none_or(|target| *label == target)
            })
            .cloned()
            .collect();
        (sync.sequence, sync.pending.clone())
    };
    for label in &labels {
        if app.emit_to(label, event, sequence).is_err() {
            return FrontendFlush {
                request_id: sequence,
                result: Err(ui::message("A window could not save its latest changes")),
            };
        }
    }
    let sync = state.sync.lock().unwrap();
    let (sync, _) = state
        .changed
        .wait_timeout_while(sync, Duration::from_secs(5), |sync| {
            !sync.pending.is_empty() && !sync.failed
        })
        .unwrap();
    let result = if sync.failed || !sync.pending.is_empty() {
        Err(ui::message(
            "Saving has not finished. Pennel will stay open to preserve your changes.",
        ))
    } else {
        Ok(())
    };
    FrontendFlush {
        request_id: sequence,
        result,
    }
}

#[cfg(target_os = "macos")]
fn finish_window_hide(app: &AppHandle, label: &str, revision: u64, mut result: Result<(), String>) {
    if result.is_ok() && workspace::hide_current(app, label, revision) && label == "onboarding" {
        let shared = app.state::<Arc<Shared>>();
        let mut settings = shared.settings();
        if settings.onboarding.status != pennel_core::OnboardingStatus::Completed {
            settings.onboarding.status = pennel_core::OnboardingStatus::Skipped;
            result = shared
                .engine
                .update_settings(settings)
                .map_err(|error| error.to_string());
        }
    }
    let hide = workspace::finish_hide(app, label, revision, result.is_ok());
    if let Some(window) = app.get_webview_window(label) {
        if hide {
            owned_dictation::close_owner(app, label);
            let _ = window.hide();
        } else if result.is_err() && workspace::requested(app, label) {
            let _ = window.show();
        }
        notify_visibility(app, label, window.is_visible().unwrap_or(!hide));
    }
    if let Err(error) = result {
        let _ = app.emit_to(label, "window-hide-error", error);
    }
}

#[cfg(target_os = "macos")]
fn prepare_window_hide(app: &AppHandle, label: &str) {
    let exit = app.state::<ExitState>();
    let shared = app.state::<Arc<Shared>>();
    if !exit.permits_hide(&shared.updating) {
        return;
    }
    let Some(revision) = workspace::begin_hide(app, label) else {
        return;
    };
    let ready = app
        .state::<ExitState>()
        .sync
        .lock()
        .unwrap()
        .ready
        .contains(label);
    if !ready {
        finish_window_hide(app, label, revision, Ok(()));
        return;
    }
    let app = app.clone();
    let label = label.to_owned();
    std::thread::spawn(move || {
        let exit = app.state::<ExitState>();
        let shared = app.state::<Arc<Shared>>();
        let flush = exit.serialize_hide(&shared.updating, || {
            flush_frontend_requests(&app, "prepare-hide", Some(&label))
        });
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let exit = handle.state::<ExitState>();
            let shared = handle.state::<Arc<Shared>>();
            if let Some(flush) = flush {
                if exit.permits_hide(&shared.updating) {
                    finish_window_hide(&handle, &label, revision, flush.result);
                } else {
                    workspace::finish_hide(&handle, &label, revision, false);
                }
                let _ = handle.emit_to(&label, "hide-flush-complete", flush.request_id);
            } else {
                workspace::finish_hide(&handle, &label, revision, false);
            }
        });
    });
}

pub fn clipboard_mode() -> bool {
    #[cfg(target_os = "linux")]
    return linux_input::is_wayland();
    #[cfg(not(target_os = "linux"))]
    false
}

fn dispatch_cli(app: &AppHandle, arguments: &[String]) -> bool {
    #[cfg(target_os = "macos")]
    if capture::dispatch(app, arguments) {
        return true;
    }
    let Some(shared) = app.try_state::<Arc<Shared>>() else {
        return false;
    };
    if arguments
        .iter()
        .any(|argument| argument == "--toggle-dictation")
    {
        let _ = shared.dictation.send(dictation::Msg::ManualToggle {
            copy_only: clipboard_mode(),
        });
        true
    } else if arguments
        .iter()
        .any(|argument| argument == "--toggle-meeting")
    {
        let app = app.clone();
        let shared = shared.inner().clone();
        std::thread::spawn(move || call_detection::toggle_recording(app, shared));
        true
    } else {
        false
    }
}

#[cfg(target_os = "macos")]
pub fn refresh_hotkey(app: &AppHandle, shared: &Arc<Shared>) {
    macos_tray::changed();
    let active = shared.hotkey.lock().unwrap().is_some();
    if active
        || macos::permissions()
            .get("accessibility")
            .and_then(|value| value.as_str())
            != Some("granted")
    {
        return;
    }
    static RETRYING: AtomicBool = AtomicBool::new(false);
    if RETRYING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    match Listener::start(shared.settings().hotkey, sender) {
        Ok(listener) => {
            let _ = listener.set_record_shortcut_enabled(shared.settings().record_shortcut_enabled);
            let previous = shared.hotkey.lock().unwrap().replace(listener);
            drop(previous);
            record_shortcut::refresh(app, shared);
            *shared.hotkey_error.lock().unwrap() = None;
            let tx = shared.dictation.clone();
            std::thread::spawn(move || {
                for event in receiver {
                    if tx.send(dictation::Msg::Hotkey(event)).is_err() {
                        break;
                    }
                }
            });
            if let Some(hint) = app.try_state::<TrayHint>() {
                let _ = hint.0.set_text(hotkey_hint(shared));
            }
        }
        Err(error) => *shared.hotkey_error.lock().unwrap() = Some(error),
    }
    RETRYING.store(false, Ordering::SeqCst);
    macos_tray::changed();
}

pub fn hotkey_hint(shared: &Shared) -> String {
    crate::ui::message(match shared.hotkey_error.lock().unwrap().as_ref() {
        None => format!(
            "Dictation — hold {}",
            hotkey_label(shared.settings().hotkey)
        ),
        Some(_) => "Shortcut unavailable (see log)".into(),
    })
}

#[cfg(target_os = "macos")]
fn build_tray(app: &AppHandle, shared: &Shared) -> tauri::Result<()> {
    macos_tray::build(app, shared)
}

#[cfg(not(target_os = "macos"))]
fn build_tray(app: &AppHandle, shared: &Shared) -> tauri::Result<()> {
    let hint = MenuItem::with_id(app, "hint", hotkey_hint(shared), false, None::<&str>)?;
    app.manage(TrayHint(hint.clone()));
    let menu = Menu::with_items(
        app,
        &[
            &hint,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "open",
                crate::ui::message("Open Pennel"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "dictation",
                crate::ui::message("Start/stop dictation"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "meeting",
                crate::ui::message("Record meeting…"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "pad",
                crate::ui::message("Dictation…"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "history",
                crate::ui::message("History…"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "settings",
                crate::ui::message("Settings…"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "log",
                crate::ui::message("Open log"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "quit",
                crate::ui::message("Quit Pennel"),
                true,
                None::<&str>,
            )?,
        ],
    )?;
    ui::register_tray(app, &menu)?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id("pennel")
        .icon(icon)
        .tooltip("Pennel")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "dictation" => {
                if let Some(shared) = app.try_state::<Arc<Shared>>() {
                    let _ = shared.dictation.send(dictation::Msg::ManualToggle {
                        copy_only: clipboard_mode(),
                    });
                }
            }
            "open" => open_default(app),
            "meeting" => show_main(app, Some("meeting")),
            "pad" => show_window(app, "dictation", None),
            "settings" => show_window(app, "settings", None),
            "history" => show_window(app, "history", None),
            "log" => {
                let _ = commands::open_log_file(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let dir = data_dir(&handle)?;
    let preview = is_preview(&handle);
    #[cfg(target_os = "macos")]
    let capture_profile = capture::enabled(&handle)
        && handle.config().identifier == "dev.pennel.Pennel.capture.preview";
    #[cfg(not(target_os = "macos"))]
    let capture_profile = false;
    app.manage(owned_dictation::Sessions::default());
    if !capture_profile {
        external_paste::start_tracking();
    }
    app.manage(call_detection::DetectionState::default());
    #[cfg(target_os = "macos")]
    {
        macos::set_accessory();
        if !preview && !macos::other_instances().is_empty() {
            return Err("Close the previous Pennel app before opening this version. Your data is unchanged.".into());
        }
        if !preview {
            legacy_migration::prepare(&dir, &handle.path().home_dir()?)?;
        }
    }
    let log = if cfg!(target_os = "macos") && !preview {
        handle
            .path()
            .home_dir()?
            .join("Library/Logs/Pennel/pennel.log")
    } else {
        dir.join("logs/pennel.log")
    };
    diagnostics::init(log);
    LOG.info(format!(
        "Pennel {} started on {}",
        handle.package_info().version,
        std::env::consts::OS
    ));
    let engine = if preview {
        PennelEngine::new_preview(dir.to_string_lossy().into_owned())?
    } else {
        PennelEngine::new(dir.to_string_lossy().into_owned())?
    };
    #[cfg(target_os = "macos")]
    {
        engine.set_system_transcriber(Arc::new(AppleTranscriber));
        engine.set_system_language_model(Arc::new(AppleLanguageModel));
    }
    let (tx, rx) = mpsc::channel();
    let secrets = Arc::new(if capture_profile {
        secrets::Secrets::for_capture()
    } else {
        secrets::Secrets::new(preview)
    });
    engine.set_secret_store(secrets.clone())?;
    #[cfg(target_os = "macos")]
    let migration_warning = if preview {
        None
    } else {
        match legacy_migration::finish(&engine, &secrets, &dir, &handle.path().home_dir()?) {
            Ok(()) => None,
            Err(legacy_migration::MigrationError::Credentials(message)) => Some(message),
            Err(error) => return Err(error.into()),
        }
    };
    #[cfg(not(target_os = "macos"))]
    let migration_warning = None;
    let system_languages = ui::system_languages();
    let system_interface_language =
        pennel_core::InterfaceLanguagePreference::System.resolve(&system_languages);
    ui::initialize(engine.settings().ui_language.resolve(&system_languages));
    ui::install_menu(&handle)?;
    let shared = Arc::new(Shared {
        system_interface_language,
        engine,
        secrets,
        data_dir: dir,
        version: handle.package_info().version.to_string(),
        migration_warning,
        preview,
        dictation: tx.clone(),
        hotkey: Mutex::new(None),
        hotkey_error: Mutex::new(None),
        meeting: Mutex::new(None),
        recording: Mutex::new(pennel_core::RecordingLifecycle::default()),
        meeting_context: Mutex::new(None),
        recording_cancel: AtomicBool::new(false),
        library_jobs: std::sync::atomic::AtomicUsize::new(0),
        updating: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
        dictation_active: AtomicBool::new(false),
        dictation_jobs: std::sync::atomic::AtomicUsize::new(0),
        paste_jobs: std::sync::atomic::AtomicUsize::new(0),
    });
    app.manage(shared.clone());
    app.manage(cloud::SyncRunner::start(shared.clone(), handle.clone()));
    app.manage(claude_access::ClaudeAccessRunner::start(
        shared.clone(),
        handle.clone(),
    ));
    app.manage(platform_commands::UpdateState::default());
    if handle
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|value| value.get("pubkey"))
        .and_then(|value| value.as_str())
        .is_some_and(|key| !key.is_empty())
    {
        handle.plugin(tauri_plugin_updater::Builder::new().build())?;
    }

    app.manage(pill::Snapshot::default());
    dictation::spawn(handle.clone(), shared.clone(), rx, tx.clone());

    let settings = shared.settings();
    if !capture_profile {
        let (hotkey_tx, hotkey_rx) = mpsc::channel();
        match Listener::start(settings.hotkey, hotkey_tx) {
            Ok(listener) => *shared.hotkey.lock().unwrap() = Some(listener),
            Err(e) => {
                LOG.error(format!("shortcut inactive: {e}"));
                *shared.hotkey_error.lock().unwrap() = Some(e);
            }
        }
        std::thread::spawn(move || {
            for event in hotkey_rx {
                if tx.send(dictation::Msg::Hotkey(event)).is_err() {
                    return;
                }
            }
        });
    }
    LOG.info(format!(
        "shortcut active={} · dictation {}/{} · cleanup {}/{} · language {} · mode {:?}",
        shared.hotkey_error.lock().unwrap().is_none(),
        settings.dictation_stt.provider_id,
        settings.dictation_stt.model,
        settings.llm.provider_id,
        settings.llm.model,
        settings.language.as_deref().unwrap_or("automatic"),
        settings.mode
    ));

    let idle_engine = shared.engine.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(60));
            if idle_engine.unload_idle_models() {
                Log::new("app").info("local model unloaded from memory");
            }
        }
    });

    build_tray(&handle, &shared)?;
    if !capture_profile {
        record_shortcut::install(&handle)?;
        record_shortcut::refresh(&handle, &shared);
        call_detection::spawn(handle.clone(), shared.clone());
    }
    capabilities::spawn_probe(&handle);
    platform_commands::spawn_update_checks(&handle);
    if shared.migration_warning.is_none() {
        let retained = shared.clone();
        std::thread::spawn(move || {
            if let Ok(_job) = meeting_runtime::LibraryJob::new(&retained) {
                let count = retained.engine.prune_meeting_audio();
                if count > 0 {
                    LOG.info(format!("Audio retention applied: {count} files"));
                }
            }
        });
    }
    let handled_cli = dispatch_cli(&handle, &std::env::args().collect::<Vec<_>>());
    if !handled_cli {
        present(
            &handle,
            workspace::startup_presentation(
                cfg!(target_os = "macos"),
                shared.settings().onboarding.status,
            ),
        );
    }
    for arguments in workspace::initialize_launches(&handle) {
        dispatch_launch(&handle, &arguments);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
struct AppleTranscriber;

#[cfg(target_os = "macos")]
struct AppleLanguageModel;
#[cfg(target_os = "macos")]
impl pennel_core::SystemLanguageModel for AppleLanguageModel {
    fn complete(
        &self,
        messages: Vec<pennel_core::LanguageModelMessage>,
        timeout_secs: u64,
    ) -> Result<String, pennel_core::PennelError> {
        macos::language_model_complete(messages, timeout_secs)
            .map_err(|message| pennel_core::PennelError::InvalidResponse { message })
    }
}

#[cfg(target_os = "macos")]
impl pennel_core::SystemTranscriber for AppleTranscriber {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        sample_rate: u32,
        language: Option<String>,
        vocabulary: Vec<String>,
    ) -> Result<String, pennel_core::PennelError> {
        macos::transcribe(&samples, sample_rate, language.as_deref(), &vocabulary)
            .map_err(|message| pennel_core::PennelError::InvalidResponse { message })
    }
}

fn main() {
    tauri::Builder::default()
        .manage(ExitState::default())
        .manage(workspace::Navigation::default())
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            receive_launch(app, args);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(setup)
        .on_menu_event(|app, event| desktop_commands::dispatch(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            if let WindowEvent::ScaleFactorChanged { .. } = event
                && let Some(webview) = window.app_handle().get_webview_window(window.label())
            {
                fit_window_to_work_area(window.app_handle(), &webview);
            }
            if let WindowEvent::Focused(false) = event {
                owned_dictation::focus_lost(window.app_handle(), window.label());
            }
            if let WindowEvent::Destroyed = event
                && let Some(webview) = window.app_handle().get_webview_window(window.label())
            {
                desktop_environment::teardown(&webview);
            }
            if let WindowEvent::CloseRequested { api, .. } = event
                && matches!(
                    window.label(),
                    "main" | "live" | "settings" | "history" | "dictation" | "onboarding"
                )
            {
                api.prevent_close();
                #[cfg(target_os = "macos")]
                {
                    prepare_window_hide(window.app_handle(), window.label());
                }
                #[cfg(not(target_os = "macos"))]
                {
                    workspace::cancel_presentation(window.app_handle(), window.label());
                    owned_dictation::close_owner(window.app_handle(), window.label());
                    if window.label() == "onboarding"
                        && let Some(shared) = window.app_handle().try_state::<Arc<Shared>>()
                    {
                        let mut settings = shared.settings();
                        if settings.onboarding.status != pennel_core::OnboardingStatus::Completed {
                            settings.onboarding.status = pennel_core::OnboardingStatus::Skipped;
                            let _ = shared.engine.update_settings(settings);
                        }
                    }
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_environment::desktop_environment,
            desktop_commands::set_desktop_command_state,
            #[cfg(target_os = "macos")]
            capture::capture_snapshot,
            commands::app_info,
            commands::save_settings,
            commands::add_provider,
            commands::remove_provider,
            commands::set_key,
            commands::provider_models,
            commands::local_models,
            commands::download_model,
            commands::join_model_download,
            commands::cancel_download,
            commands::delete_model,
            commands::history,
            commands::retry_history,
            commands::save_history,
            commands::copy_history,
            commands::test_provider_task,
            inference::inference_protocols,
            inference::inference_availability,
            inference::cleanup_verification_availability,
            inference::inference_decide,
            inference::inference_rerank,
            inference::verify_inference,
            inference::cancel_inference,
            commands::delete_history,
            commands::open_log,
            commands::open_path,
            commands::meeting_status,
            commands::meeting_runtime_status,
            commands::start_meeting,
            commands::stop_meeting,
            commands::retry_meeting_persistence,
            commands::set_meeting_notes,
            commands::set_meeting_title,
            commands::meetings,
            commands::meeting_detail,
            commands::meeting_templates,
            commands::summarize_meeting,
            commands::retranscribe_meeting,
            commands::retranscribe_meeting_audio,
            commands::rename_meeting,
            commands::delete_meeting,
            commands::meeting_conflict_files,
            commands::delete_conflict_copy,
            commands::search_library,
            commands::ask_meetings,
            commands::template_source,
            commands::save_template,
            commands::delete_template,
            commands::rename_speaker,
            commands::diarize_meeting,
            commands::speaker_setup,
            commands::start_voice_enrollment,
            commands::finish_voice_enrollment,
            commands::cancel_voice_enrollment,
            commands::forget_voice_profile,
            commands::rename_known_voice,
            commands::forget_known_voice,
            commands::forget_all_voices,
            commands::meeting_voices,
            commands::remember_speaker,
            commands::export_meeting,
            commands::save_meeting_notes,
            commands::set_meeting_attendees,
            commands::detect_cli_agents,
            commands::check_cli_agent,
            platform_commands::get_permissions,
            platform_commands::test_apple_intelligence,
            platform_commands::request_permission,
            platform_commands::calendar_snapshot,
            platform_commands::play_meeting_audio,
            platform_commands::stop_audio,
            platform_commands::copy_text,
            platform_commands::copy_secret_text,
            platform_commands::show_window,
            platform_commands::show_workspace,
            platform_commands::record_prompt,
            platform_commands::answer_record_prompt,
            platform_commands::check_for_updates,
            platform_commands::install_update,
            platform_commands::toggle_dictation,
            platform_commands::dictation_state,
            platform_commands::frontend_ready,
            platform_commands::frontend_flushed,
            platform_commands::mark_lines,
            platform_commands::share_meeting,
            platform_commands::choose_meetings_dir,
            style_application::pick_style_application,
            app_identity::app_identity,
            commands::model_state,
            commands::show_main,
            commands::hide_window,
            cloud::cloud_login_start,
            cloud::cloud_login_poll,
            cloud::cloud_logout,
            cloud::cloud_account,
            cloud::cloud_devices,
            cloud::cloud_rename_device,
            cloud::cloud_revoke_device,
            cloud::cloud_delete_account,
            cloud::cloud_sync_enable,
            cloud::cloud_sync_setup,
            cloud::cloud_sync_prepare,
            cloud::cloud_recovery_code_context,
            cloud::cloud_recovery_confirm,
            cloud::cloud_sync_activate,
            cloud::cloud_sync_resume,
            cloud::cloud_link_start,
            cloud::cloud_link_join,
            cloud::cloud_link_poll,
            cloud::cloud_link_view,
            cloud::cloud_link_approve,
            cloud::cloud_link_cancel,
            cloud::cloud_sync_unlock,
            cloud::cloud_sync_new_recovery_code,
            cloud::cloud_sync_reset,
            cloud::cloud_sync_disable,
            cloud::cloud_sync_now,
            cloud::cloud_sync_last,
            cloud::cloud_sync_confirm_deletes,
            cloud::meeting_sync_states,
            cloud::cloud_folder_warning,
            cloud::open_url,
            claude_access::claude_access,
            claude_access::set_claude_access,
            owned_dictation::onboarding_readiness,
            owned_dictation::pad_text,
            owned_dictation::save_pad_text,
            owned_dictation::open_dictation_session,
            owned_dictation::get_dictation_session,
            owned_dictation::toggle_dictation_session,
            owned_dictation::cancel_dictation_session,
            owned_dictation::close_dictation_session,
        ])
        .build(tauri::generate_context!())
        .expect("Pennel could not start")
        .run(|_app, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                if _app.state::<ExitState>().approved.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_exit();
                let exit_state = _app.state::<ExitState>();
                let transition = exit_state.transition_gate.lock().unwrap();
                if _app
                    .try_state::<platform_commands::UpdateState>()
                    .is_some_and(|update| update.installing.load(Ordering::SeqCst))
                {
                    return;
                }
                if _app
                    .state::<ExitState>()
                    .requested
                    .swap(true, Ordering::SeqCst)
                {
                    return;
                }
                _app.state::<ExitState>()
                    .quit_attempts
                    .fetch_add(1, Ordering::SeqCst);
                drop(transition);
                let app = _app.clone();
                std::thread::spawn(move || {
                    let shared = app.state::<Arc<Shared>>().inner().clone();
                    {
                        let _recording = shared.recording.lock().unwrap();
                        shared.quitting.store(true, Ordering::SeqCst);
                    }
                    let flush = flush_frontends(&app, "prepare-exit");
                    if flush.result.is_err() {
                        shared.quitting.store(false, Ordering::SeqCst);
                        app.state::<ExitState>()
                            .requested
                            .store(false, Ordering::SeqCst);
                        let _ = app.emit("exit-cancelled", flush.request_id);
                        show_window(
                            &app,
                            if shared.engine.unsaved_history_count() > 0 {
                                "history"
                            } else {
                                "live"
                            },
                            None,
                        );
                        return;
                    }
                    let (prepared, captured) = mpsc::channel();
                    if shared
                        .dictation
                        .send(dictation::Msg::PrepareShutdown(prepared))
                        .is_ok()
                    {
                        let _ = captured.recv();
                    }
                    while shared.dictation_jobs.load(Ordering::SeqCst) != 0
                        || shared.paste_jobs.load(Ordering::SeqCst) != 0
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if shared.engine.retry_unsaved_history().is_err() {
                        shared.quitting.store(false, Ordering::SeqCst);
                        app.state::<ExitState>()
                            .requested
                            .store(false, Ordering::SeqCst);
                        let _ = app.emit("exit-cancelled", flush.request_id);
                        let _ = app.emit("history-changed", ());
                        show_window(&app, "history", None);
                        return;
                    }
                    match meeting_runtime::stop_and_wait(&shared) {
                        Ok(_) => {
                            while shared.library_jobs.load(Ordering::SeqCst) != 0 {
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            let (reply, done) = mpsc::channel();
                            if shared
                                .dictation
                                .send(dictation::Msg::Shutdown(reply))
                                .is_ok()
                            {
                                let _ = done.recv();
                            }
                            app.state::<cloud::SyncRunner>().stop();
                            app.state::<claude_access::ClaudeAccessRunner>().stop();
                            platform_audio::stop();
                            let listener = shared.hotkey.lock().unwrap().take();
                            drop(listener);
                            app.state::<ExitState>()
                                .approved
                                .store(true, Ordering::SeqCst);
                            app.exit(0);
                        }
                        Err(error) => {
                            let _ = error;
                            LOG.error("Quit deferred after a finalization error");
                            app.state::<ExitState>()
                                .requested
                                .store(false, Ordering::SeqCst);
                            shared.quitting.store(false, Ordering::SeqCst);
                            let _ = app.emit("exit-cancelled", flush.request_id);
                            show_window(&app, "live", None);
                        }
                    }
                });
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => open_default(_app),
            _ => {}
        });
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn queued_hide_cannot_emit_after_quit_or_update_claims_the_global_barrier() {
        for updating_operation in [false, true] {
            let exit = Arc::new(ExitState::default());
            let updating = Arc::new(AtomicBool::new(false));
            assert!(exit.permits_hide(&updating));
            let global_flush = exit.flush_gate.lock().unwrap();
            let queued_exit = exit.clone();
            let queued_update = updating.clone();
            let (queued, waiting) = mpsc::channel();
            let close = std::thread::spawn(move || {
                queued.send(()).unwrap();
                queued_exit.serialize_hide(&queued_update, || "prepare-hide")
            });
            waiting.recv().unwrap();
            if updating_operation {
                updating.store(true, Ordering::SeqCst);
            } else {
                exit.requested.store(true, Ordering::SeqCst);
            }
            assert!(!exit.permits_hide(&updating));
            drop(global_flush);
            assert_eq!(close.join().unwrap(), None);
        }
    }

    #[test]
    fn hide_can_flush_again_after_a_global_transition_is_cancelled() {
        let exit = ExitState::default();
        let updating = AtomicBool::new(true);
        assert_eq!(exit.serialize_hide(&updating, || "prepare-hide"), None);
        updating.store(false, Ordering::SeqCst);
        assert_eq!(
            exit.serialize_hide(&updating, || "prepare-hide"),
            Some("prepare-hide")
        );
    }
}
