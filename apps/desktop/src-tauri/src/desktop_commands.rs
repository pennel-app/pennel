use std::sync::Mutex;

#[cfg(any(target_os = "macos", test))]
use pennel_core::InterfaceLanguage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCommand {
    Settings,
    NewRecording,
    Find,
    ToggleSidebar,
    Dictation,
    Meetings,
    History,
}

impl DesktopCommand {
    #[cfg(target_os = "macos")]
    fn id(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::NewRecording => "new_recording",
            Self::Find => "find",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::Dictation => "dictation",
            Self::Meetings => "meetings",
            Self::History => "history",
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn label(self, language: InterfaceLanguage, state: &CommandState) -> &'static str {
        let dutch = language == InterfaceLanguage::Dutch;
        match (self, dutch) {
            (Self::Settings, false) => "Settings…",
            (Self::Settings, true) => "Instellingen…",
            (Self::NewRecording, false) if state.recording_active => "Return to recording",
            (Self::NewRecording, true) if state.recording_active => "Terug naar opname",
            (Self::NewRecording, false) => "New recording…",
            (Self::NewRecording, true) => "Nieuwe opname…",
            (Self::Find, false) => "Find…",
            (Self::Find, true) => "Zoeken…",
            (Self::ToggleSidebar, false) if state.sidebar_visible => "Hide sidebar",
            (Self::ToggleSidebar, true) if state.sidebar_visible => "Verberg navigatiekolom",
            (Self::ToggleSidebar, false) => "Show sidebar",
            (Self::ToggleSidebar, true) => "Toon navigatiekolom",
            (Self::Dictation, false) => "Dictation",
            (Self::Dictation, true) => "Dicteren",
            (Self::Meetings, false) => "Meetings",
            (Self::Meetings, true) => "Vergaderingen",
            (Self::History, false) => "History",
            (Self::History, true) => "Geschiedenis",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CommandState {
    pub ready: bool,
    pub settings: bool,
    pub new_recording: bool,
    pub find: bool,
    pub toggle_sidebar: bool,
    pub dictation: bool,
    pub meetings: bool,
    pub history: bool,
    pub sidebar_visible: bool,
    pub recording_active: bool,
}

impl Default for CommandState {
    fn default() -> Self {
        Self {
            ready: false,
            settings: false,
            new_recording: false,
            find: false,
            toggle_sidebar: false,
            dictation: false,
            meetings: false,
            history: false,
            sidebar_visible: true,
            recording_active: false,
        }
    }
}

impl CommandState {
    fn enabled(&self, command: DesktopCommand) -> bool {
        self.ready
            && match command {
                DesktopCommand::Settings => self.settings,
                DesktopCommand::NewRecording => self.new_recording,
                DesktopCommand::Find => self.find,
                DesktopCommand::ToggleSidebar => self.toggle_sidebar,
                DesktopCommand::Dictation => self.dictation,
                DesktopCommand::Meetings => self.meetings,
                DesktopCommand::History => self.history,
            }
    }
}

#[derive(Default)]
struct CommandMenus {
    state: Mutex<CommandState>,
    #[cfg(target_os = "macos")]
    items: Mutex<Vec<(DesktopCommand, tauri::menu::MenuItem<tauri::Wry>)>>,
}

#[cfg(target_os = "macos")]
pub fn install(app: &AppHandle, menu: &tauri::menu::Menu<tauri::Wry>) -> tauri::Result<()> {
    use tauri::menu::{MenuItem, MenuItemKind, PredefinedMenuItem};
    if app.try_state::<CommandMenus>().is_some() {
        return Ok(());
    }
    app.manage(CommandMenus::default());
    let menus = menu.items()?;
    let submenus: Vec<_> = menus
        .iter()
        .filter_map(|item| match item {
            MenuItemKind::Submenu(submenu) => Some(submenu.clone()),
            _ => None,
        })
        .collect();
    let native = app.state::<CommandMenus>();
    let state = native.state.lock().unwrap().clone();
    let mut items = native.items.lock().unwrap();
    for (command, title) in [
        (DesktopCommand::Settings, "App"),
        (DesktopCommand::NewRecording, "File"),
        (DesktopCommand::Find, "Edit"),
        (DesktopCommand::ToggleSidebar, "View"),
        (DesktopCommand::Dictation, "View"),
        (DesktopCommand::Meetings, "View"),
        (DesktopCommand::History, "View"),
    ] {
        let target = if title == "App" {
            submenus.first().cloned()
        } else {
            submenus
                .iter()
                .find(|submenu| submenu.text().is_ok_and(|name| name == title))
                .cloned()
        };
        let Some(target) = target else { continue };
        let shortcut = accelerator(app, command);
        let item = MenuItem::with_id(
            app,
            format!("desktop-{}", command.id()),
            command.label(crate::ui::language(), &state),
            false,
            shortcut.as_deref(),
        )?;
        match title {
            "App" => {
                target.insert(&item, 2)?;
                target.insert(&PredefinedMenuItem::separator(app)?, 3)?;
            }
            "File" => {
                target.insert(&item, 0)?;
                target.insert(&PredefinedMenuItem::separator(app)?, 1)?;
            }
            "Edit" => {
                target.append(&PredefinedMenuItem::separator(app)?)?;
                target.append(&item)?;
            }
            _ => target.append(&item)?,
        }
        items.push((command, item));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn accelerator(app: &AppHandle, command: DesktopCommand) -> Option<String> {
    if crate::shortcut_capture::capturing() {
        return None;
    }
    let shared = app.try_state::<std::sync::Arc<crate::state::Shared>>()?;
    pennel_core::effective_shortcut(
        &shared.settings(),
        command.id(),
        crate::shortcuts::platform(),
    )
    .as_ref()
    .and_then(pennel_core::menu_accelerator)
}

pub fn refresh(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(native) = app.try_state::<CommandMenus>() {
        let state = native.state.lock().unwrap().clone();
        // Native setters can synchronously dispatch to the UI thread; never hold a shared lock.
        let items = native.items.lock().unwrap().clone();
        for (command, item) in items {
            item.set_text(command.label(crate::ui::language(), &state))?;
            item.set_accelerator(accelerator(app, command))?;
            item.set_enabled(state.enabled(command))?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

#[tauri::command]
pub fn set_desktop_command_state(window: WebviewWindow, state: CommandState) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the workspace can update its application commands".into());
    }
    let app = window.app_handle();
    if app.try_state::<CommandMenus>().is_none() {
        app.manage(CommandMenus::default());
    }
    *app.state::<CommandMenus>().state.lock().unwrap() = state;
    refresh(app).map_err(|error| error.to_string())
}

pub fn dispatch(app: &AppHandle, id: &str) {
    if crate::shortcut_capture::capturing() {
        return;
    }
    let Some(id) = id.strip_prefix("desktop-") else {
        return;
    };
    let Ok(command) =
        serde_json::from_value::<DesktopCommand>(serde_json::Value::String(id.into()))
    else {
        return;
    };
    let Some(native) = app.try_state::<CommandMenus>() else {
        return;
    };
    if !native.state.lock().unwrap().enabled(command) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        crate::macos::product_windows_visible(true);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        crate::notify_visibility(app, "main", true);
        let _ = app.emit_to(
            "main",
            "desktop-command",
            serde_json::json!({"command":command}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_save_refreshes_accelerators_independently_of_language_and_command_state() {
        let source = include_str!("commands.rs");
        let apply = source
            .split("fn apply_settings(")
            .nth(1)
            .unwrap()
            .split("fn refresh_cleanup_verification")
            .next()
            .unwrap();
        assert!(apply.contains("crate::desktop_commands::refresh(app).map_err(err)?;"));
        let state = CommandState {
            ready: true,
            find: true,
            ..Default::default()
        };
        let mut settings = pennel_core::default_settings();
        let get = |settings: &pennel_core::Settings| {
            pennel_core::effective_shortcut(settings, "find", pennel_core::ShortcutPlatform::Macos)
                .as_ref()
                .and_then(pennel_core::menu_accelerator)
        };
        assert_eq!(get(&settings).as_deref(), Some("Cmd+F"));
        settings.shortcuts.insert(
            "find".into(),
            Some(pennel_core::ShortcutBinding {
                mods: vec!["meta".into(), "alt".into()],
                code: Some("KeyG".into()),
                key: Some("g".into()),
            }),
        );
        assert_eq!(get(&settings).as_deref(), Some("Alt+Cmd+G"));
        settings.shortcuts.insert("find".into(), None);
        assert_eq!(get(&settings), None);
        settings.shortcuts.remove("find");
        assert_eq!(get(&settings).as_deref(), Some("Cmd+F"));
        assert!(state.enabled(DesktopCommand::Find));
    }

    #[test]
    fn readiness_and_per_action_state_both_gate_commands() {
        let mut state = CommandState {
            settings: true,
            ..Default::default()
        };
        assert!(!state.enabled(DesktopCommand::Settings));
        state.ready = true;
        assert!(state.enabled(DesktopCommand::Settings));
        assert!(!state.enabled(DesktopCommand::NewRecording));
        state.settings = false;
        assert!(!state.enabled(DesktopCommand::Settings));
    }

    #[test]
    fn dynamic_labels_preserve_command_identity() {
        let mut state = CommandState::default();
        assert_eq!(
            DesktopCommand::ToggleSidebar.label(InterfaceLanguage::English, &state),
            "Hide sidebar"
        );
        state.sidebar_visible = false;
        assert_eq!(
            DesktopCommand::ToggleSidebar.label(InterfaceLanguage::Dutch, &state),
            "Toon navigatiekolom"
        );
        state.recording_active = true;
        assert_eq!(
            DesktopCommand::NewRecording.label(InterfaceLanguage::English, &state),
            "Return to recording"
        );
        assert_eq!(
            serde_json::to_value(DesktopCommand::NewRecording).unwrap(),
            "new_recording"
        );
        assert!(serde_json::from_str::<DesktopCommand>("\"arbitrary\"").is_err());
    }
}
