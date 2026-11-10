use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::Duration;

use pennel_core::{InterfaceLanguage, RecordingPhase};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Listener, Manager};

use crate::state::Shared;

static CHANGES: OnceLock<SyncSender<()>> = OnceLock::new();

pub fn changed() {
    if let Some(sender) = CHANGES.get() {
        let _ = sender.try_send(());
    }
}

pub struct UpdateOnDrop;

impl Drop for UpdateOnDrop {
    fn drop(&mut self) {
        changed();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeetingAction {
    Start,
    Stop,
    Open,
}

fn meeting_action(phase: RecordingPhase) -> MeetingAction {
    match phase {
        RecordingPhase::Idle | RecordingPhase::Error => MeetingAction::Start,
        RecordingPhase::Recording => MeetingAction::Stop,
        RecordingPhase::Starting | RecordingPhase::Stopping | RecordingPhase::Finalizing => {
            MeetingAction::Open
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Presentation {
    status: String,
    action: String,
    action_enabled: bool,
    recording_icon: bool,
}

fn presentation(
    phase: RecordingPhase,
    title: Option<&str>,
    elapsed_secs: u64,
    language: InterfaceLanguage,
) -> Presentation {
    let (status, action, action_enabled) = match phase {
        RecordingPhase::Recording => (
            format!(
                "● {} · {:02}:{:02}",
                title.unwrap_or("Meeting"),
                elapsed_secs / 60,
                elapsed_secs % 60
            ),
            "Stop recording",
            true,
        ),
        RecordingPhase::Starting => ("Starting recording…".into(), "Starting recording…", false),
        RecordingPhase::Stopping | RecordingPhase::Finalizing => (
            "Finalizing recording…".into(),
            "Finalizing recording…",
            true,
        ),
        RecordingPhase::Idle | RecordingPhase::Error => ("Pennel".into(), "Start recording", true),
    };
    Presentation {
        status: pennel_core::localize_ui_text(language, &status),
        action: pennel_core::localize_ui_text(language, action),
        action_enabled,
        recording_icon: matches!(phase, RecordingPhase::Recording | RecordingPhase::Stopping),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    meeting: Presentation,
    hint: String,
    dark: bool,
    recording: bool,
    accelerators: [Option<String>; 3],
}

fn snapshot(app: &AppHandle, shared: &Shared) -> Snapshot {
    let (phase, title, elapsed) = {
        let state = shared.recording.lock().unwrap().snapshot();
        let context = shared.meeting_context.lock().unwrap();
        let context = context
            .as_ref()
            .filter(|context| state.session_id.as_deref() == Some(&context.token));
        (
            state.phase,
            context.map(|context| context.title.clone()),
            context.map_or(0, |context| {
                context
                    .duration_secs
                    .unwrap_or_else(|| context.started.elapsed().as_secs())
            }),
        )
    };
    let permitted = crate::macos::permissions()
        .get("accessibility")
        .and_then(|value| value.as_str())
        == Some("granted");
    let listening = shared.hotkey.lock().unwrap().is_some()
        && shared.hotkey_error.lock().unwrap().is_none()
        && permitted;
    let hint = if listening {
        crate::hotkey_hint(shared)
    } else if !permitted {
        "Dictation is waiting for Accessibility access".into()
    } else {
        "Shortcut unavailable (see log)".into()
    };
    let dark = match shared.settings().appearance {
        pennel_core::AppearancePreference::Dark => true,
        pennel_core::AppearancePreference::Light => false,
        pennel_core::AppearancePreference::System => {
            app.get_webview_window("main")
                .and_then(|window| window.theme().ok())
                == Some(tauri::Theme::Dark)
        }
    };
    Snapshot {
        meeting: presentation(
            phase,
            title.as_deref(),
            elapsed,
            shared.interface_language(),
        ),
        hint: pennel_core::localize_ui_text(shared.interface_language(), &hint),
        dark,
        recording: phase == RecordingPhase::Recording,
        accelerators: ["record", "open_pennel", "settings"].map(|id| {
            if crate::shortcut_capture::capturing() {
                return None;
            }
            pennel_core::effective_shortcut(&shared.settings(), id, crate::shortcuts::platform())
                .as_ref()
                .and_then(pennel_core::menu_accelerator)
        }),
    }
}

struct Items {
    status: MenuItem<tauri::Wry>,
    meeting: MenuItem<tauri::Wry>,
    hint: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
}

fn apply(app: &AppHandle, items: &Items, current: &Snapshot, previous: Option<&Snapshot>) {
    if previous.is_none_or(|previous| previous.accelerators != current.accelerators) {
        for (item, accelerator) in [&items.meeting, &items.open, &items.settings]
            .into_iter()
            .zip(&current.accelerators)
        {
            let _ = item.set_accelerator(accelerator.as_deref());
        }
    }
    if previous.is_none_or(|previous| previous.meeting != current.meeting) {
        let _ = items.status.set_text(&current.meeting.status);
        let _ = items.status.set_enabled(current.recording);
        let _ = items.meeting.set_text(&current.meeting.action);
        let _ = items.meeting.set_enabled(current.meeting.action_enabled);
    }
    if previous.is_none_or(|previous| previous.hint != current.hint) {
        let _ = items.hint.set_text(&current.hint);
    }
    if let Some(tray) = app.tray_by_id("pennel") {
        if previous.is_none_or(|previous| {
            previous.meeting.recording_icon != current.meeting.recording_icon
                || previous.dark != current.dark
        }) {
            let bytes: &[u8] = if current.meeting.recording_icon {
                if current.dark {
                    include_bytes!("../icons/macos-recording-dark.png")
                } else {
                    include_bytes!("../icons/macos-recording-light.png")
                }
            } else {
                include_bytes!("../icons/macos-menubar.png")
            };
            if let Ok(icon) = tauri::image::Image::from_bytes(bytes) {
                let _ = tray.set_icon_with_as_template(Some(icon), !current.meeting.recording_icon);
            }
        }
        if previous.is_none_or(|previous| previous.meeting.status != current.meeting.status) {
            let _ = tray.set_tooltip(Some(&current.meeting.status));
        }
    }
}

pub fn build(app: &AppHandle, shared: &Shared) -> tauri::Result<()> {
    let initial = snapshot(app, shared);
    let status = MenuItem::with_id(
        app,
        "meeting-status",
        &initial.meeting.status,
        false,
        None::<&str>,
    )?;
    let meeting = MenuItem::with_id(
        app,
        "meeting",
        &initial.meeting.action,
        initial.meeting.action_enabled,
        initial.accelerators[0].as_deref(),
    )?;
    let hint = MenuItem::with_id(app, "hint", &initial.hint, false, None::<&str>)?;
    let open = MenuItem::with_id(
        app,
        "open",
        crate::ui::message("Open Pennel"),
        true,
        initial.accelerators[1].as_deref(),
    )?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        crate::ui::message("Settings…"),
        true,
        initial.accelerators[2].as_deref(),
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &meeting,
            &PredefinedMenuItem::separator(app)?,
            &hint,
            &MenuItem::with_id(
                app,
                "dictation",
                crate::ui::message("Start/stop dictation"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &MenuItem::with_id(
                app,
                "history",
                crate::ui::message("History…"),
                true,
                None::<&str>,
            )?,
            &settings,
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
                Some("CmdOrCtrl+Q"),
            )?,
        ],
    )?;
    crate::ui::register_tray(app, &menu)?;
    TrayIconBuilder::with_id("pennel")
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/macos-menubar.png"
        ))?)
        .icon_as_template(true)
        .tooltip("Pennel")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "dictation" => {
                if let Some(shared) = app.try_state::<Arc<Shared>>() {
                    let _ = shared.dictation.send(crate::dictation::Msg::ManualToggle {
                        copy_only: crate::clipboard_mode(),
                    });
                }
            }
            "meeting" => {
                if let Some(shared) = app.try_state::<Arc<Shared>>() {
                    let phase = shared.recording.lock().unwrap().snapshot().phase;
                    match meeting_action(phase) {
                        MeetingAction::Open => crate::show_main(app, Some("meeting")),
                        MeetingAction::Start | MeetingAction::Stop => {
                            let app = app.clone();
                            let shared = shared.inner().clone();
                            std::thread::spawn(move || {
                                crate::call_detection::toggle_recording(app, shared)
                            });
                        }
                    }
                }
            }
            "meeting-status" => crate::show_main(app, Some("meeting")),
            "open" => crate::open_default(app),
            "settings" => crate::show_window(app, "settings", None),
            "history" => crate::show_window(app, "history", None),
            "log" => {
                let _ = crate::commands::open_log_file(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    let items = Items {
        status,
        meeting,
        hint,
        open,
        settings,
    };
    apply(app, &items, &initial, None);
    let (sender, receiver) = mpsc::sync_channel(1);
    let _ = CHANGES.set(sender);
    for event in ["settings-changed", "tauri://theme-changed"] {
        app.listen_any(event, |_| changed());
    }
    let app = app.clone();
    let shared = app.state::<Arc<Shared>>().inner().clone();
    std::thread::Builder::new()
        .name("pennel-menubar".into())
        .spawn(move || {
            let mut previous = initial;
            loop {
                let wake = if previous.recording {
                    match receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => true,
                        Err(RecvTimeoutError::Disconnected) => false,
                    }
                } else {
                    receiver.recv().is_ok()
                };
                if !wake {
                    break;
                }
                let current = snapshot(&app, &shared);
                apply(&app, &items, &current, Some(&previous));
                previous = current;
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_menu_follows_actual_lifecycle_and_elapsed_time() {
        let idle = presentation(RecordingPhase::Idle, None, 0, InterfaceLanguage::English);
        assert_eq!(idle.action, "Start recording");
        assert!(!idle.recording_icon);
        let recording = presentation(
            RecordingPhase::Recording,
            Some("Werkoverleg"),
            125,
            InterfaceLanguage::English,
        );
        assert_eq!(recording.status, "● Werkoverleg · 02:05");
        assert_eq!(recording.action, "Stop recording");
        assert!(recording.recording_icon);
        let stopping = presentation(
            RecordingPhase::Stopping,
            None,
            0,
            InterfaceLanguage::English,
        );
        assert_eq!(stopping.action, "Finalizing recording…");
        assert!(stopping.recording_icon);
        assert!(
            !presentation(
                RecordingPhase::Finalizing,
                None,
                0,
                InterfaceLanguage::English
            )
            .recording_icon
        );
        assert_eq!(
            presentation(RecordingPhase::Error, None, 0, InterfaceLanguage::English),
            idle
        );
    }

    #[test]
    fn dutch_menu_copy_keeps_the_recorded_title_untouched() {
        let idle = presentation(RecordingPhase::Idle, None, 0, InterfaceLanguage::Dutch);
        assert_eq!(idle.action, "Start opname");
        let title = "Geef Pennel toegang tot de microfoon.";
        let active = presentation(
            RecordingPhase::Recording,
            Some(title),
            3,
            InterfaceLanguage::English,
        );
        assert_eq!(active.status, format!("● {title} · 00:03"));
        assert_eq!(active.action, "Stop recording");
    }

    #[test]
    fn starting_cannot_start_twice_and_finishing_only_opens_the_recording() {
        assert!(
            !presentation(
                RecordingPhase::Starting,
                None,
                0,
                InterfaceLanguage::English
            )
            .action_enabled
        );
        assert_eq!(meeting_action(RecordingPhase::Idle), MeetingAction::Start);
        assert_eq!(meeting_action(RecordingPhase::Error), MeetingAction::Start);
        assert_eq!(
            meeting_action(RecordingPhase::Recording),
            MeetingAction::Stop
        );
        for phase in [
            RecordingPhase::Starting,
            RecordingPhase::Stopping,
            RecordingPhase::Finalizing,
        ] {
            assert_eq!(meeting_action(phase), MeetingAction::Open);
        }
    }

    #[test]
    fn recording_artwork_uses_the_shared_accent_in_both_appearances() {
        let tokens: serde_json::Value =
            serde_json::from_str(include_str!("../../../../assets/brand/tokens.json")).unwrap();
        for (appearance, bytes) in [
            (
                "light",
                include_bytes!("../icons/macos-recording-light.png").as_slice(),
            ),
            (
                "dark",
                include_bytes!("../icons/macos-recording-dark.png").as_slice(),
            ),
        ] {
            let image = tauri::image::Image::from_bytes(bytes).unwrap();
            assert_eq!((image.width(), image.height()), (36, 36));
            let accent = tokens["colors"]["accent"][appearance].as_str().unwrap();
            let expected: Vec<u8> = (1..7)
                .step_by(2)
                .map(|index| u8::from_str_radix(&accent[index..index + 2], 16).unwrap())
                .chain([255])
                .collect();
            let center = (18 * image.width() as usize + 18) * 4;
            assert_eq!(&image.rgba()[center..center + 4], expected.as_slice());
            assert_eq!(image.rgba()[3], 0);
        }
    }
}
