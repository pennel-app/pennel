use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

pub const WINDOW_MARGIN: f64 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FittedWindow {
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub min_height: f64,
}

/// Gap between the record prompt and the work area's top-right corner, in logical points.
pub const PROMPT_MARGIN: f64 = 12.0;

/// Physical top-left of the record prompt in the work area's top-right corner, kept on screen.
/// macOS places the prompt natively before its first show instead.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn prompt_origin(
    area_position: (i32, i32),
    area_size: (u32, u32),
    scale: f64,
    size: (f64, f64),
) -> (i32, i32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let width = (size.0 * scale).round() as i64;
    let margin = (PROMPT_MARGIN * scale).round() as i64;
    let free = (i64::from(area_size.0) - width).max(0);
    let x = i64::from(area_position.0) + free - margin.min(free);
    let free_height = (i64::from(area_size.1) - (size.1 * scale).round() as i64).max(0);
    let y = i64::from(area_position.1) + margin.min(free_height);
    (x as i32, y as i32)
}

/// Monitor work areas are physical pixels; builder sizes and minima are logical points.
pub fn fit_window_size(
    requested: (f64, f64),
    minimum: (f64, f64),
    physical_area: (u32, u32),
    scale: f64,
) -> FittedWindow {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let available_width = (f64::from(physical_area.0) / scale - WINDOW_MARGIN * 2.0).max(1.0);
    let available_height = (f64::from(physical_area.1) / scale - WINDOW_MARGIN * 2.0).max(1.0);
    let width = requested.0.min(available_width).max(1.0);
    let height = requested.1.min(available_height).max(1.0);
    FittedWindow {
        width,
        height,
        min_width: minimum.0.min(width),
        min_height: minimum.1.min(height),
    }
}

/// Every product screen shares main; onboarding and the nonactivating prompt stay separate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSpec {
    pub label: &'static str,
    pub mode: &'static str,
    pub title: &'static str,
    pub size: (f64, f64),
    pub minimum: (f64, f64),
}

pub fn window_spec(kind: &str) -> WindowSpec {
    let (label, mode, title, size, minimum) = match kind {
        "onboarding" => (
            "onboarding",
            "onboarding",
            "Welcome to Pennel",
            (640.0, 660.0),
            (520.0, 520.0),
        ),
        "prompt" => (
            "record-prompt",
            "prompt",
            "Pennel",
            (420.0, 144.0),
            (420.0, 144.0),
        ),
        _ => (
            "main",
            "workspace",
            "Pennel",
            (980.0, 640.0),
            (760.0, 520.0),
        ),
    };
    WindowSpec {
        label,
        mode,
        title,
        size,
        minimum,
    }
}

/// Percent-encodes a route value (a tab or meeting id) for a window URL query.
pub fn query_value(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Legacy URL aliases retain section intent while all product views use main's native window.
pub fn window_url(kind: &str, tab: Option<&str>) -> String {
    let mode = match kind {
        "dictation" | "history" | "settings" => kind,
        _ => window_spec(kind).mode,
    };
    match tab.filter(|_| matches!(mode, "dictation" | "history" | "settings" | "workspace")) {
        Some(tab) => format!("index.html?window={mode}&tab={}", query_value(tab)),
        None => format!("index.html?window={mode}"),
    }
}

pub fn label_spec(label: &str) -> Option<WindowSpec> {
    let kind = match label {
        "main" => "workspace",
        "record-prompt" => "prompt",
        other => other,
    };
    let spec = window_spec(kind);
    (spec.label == label).then_some(spec)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSection {
    Dictation,
    Meetings,
    History,
    Settings,
    Recording,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSourceKind {
    Notes,
    Summary,
    Transcript,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingSourceTarget {
    pub kind: MeetingSourceKind,
    pub start_ms: Option<u64>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utterance_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utterance_fingerprints: Option<Vec<String>>,
}

impl MeetingSourceTarget {
    fn has_valid_identity(&self) -> bool {
        if self
            .meeting_uid
            .as_ref()
            .is_some_and(|uid| uid.trim().is_empty())
        {
            return false;
        }
        match (&self.utterance_ids, &self.utterance_fingerprints) {
            (None, None) => true,
            (Some(ids), Some(fingerprints)) => {
                self.kind == MeetingSourceKind::Transcript
                    && self.meeting_uid.is_some()
                    && !ids.is_empty()
                    && ids.len() == fingerprints.len()
                    && ids
                        .iter()
                        .chain(fingerprints)
                        .all(|value| !value.trim().is_empty())
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRoute {
    pub section: WorkspaceSection,
    pub tab: Option<String>,
    pub meeting_id: Option<String>,
    pub jump_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_target: Option<MeetingSourceTarget>,
}

impl WorkspaceRoute {
    pub fn has_valid_source(&self) -> bool {
        self.source_target.as_ref().is_none_or(|source| {
            self.section == WorkspaceSection::Meetings
                && self.meeting_id.as_deref().is_some_and(valid_meeting_id)
                && !source.text.trim().is_empty()
                && source.has_valid_identity()
        })
    }

    /// The window kind that shows this route.
    pub fn window_kind(&self) -> &'static str {
        match self.section {
            WorkspaceSection::Dictation
            | WorkspaceSection::Meetings
            | WorkspaceSection::History
            | WorkspaceSection::Settings
            | WorkspaceSection::Recording => "workspace",
        }
    }

    fn section(section: WorkspaceSection) -> Self {
        Self {
            section,
            tab: None,
            meeting_id: None,
            jump_ms: None,
            source_target: None,
        }
    }
}

#[derive(Default)]
pub struct Navigation {
    pending: Mutex<HashMap<String, WorkspaceRoute>>,
    onboarding: Mutex<Option<String>>,
    visibility: Mutex<Visibility>,
    launches: Mutex<LaunchQueue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Onboarding,
    Workspace,
    Background,
}

pub fn startup_presentation(menu_bar: bool, status: pennel_core::OnboardingStatus) -> Presentation {
    if status == pennel_core::OnboardingStatus::Pending {
        Presentation::Onboarding
    } else if menu_bar {
        Presentation::Background
    } else {
        Presentation::Workspace
    }
}

pub fn explicit_presentation(
    menu_bar: bool,
    status: pennel_core::OnboardingStatus,
) -> Presentation {
    if menu_bar && status == pennel_core::OnboardingStatus::Pending {
        Presentation::Onboarding
    } else {
        Presentation::Workspace
    }
}

#[derive(Default)]
struct LaunchQueue {
    initialized: bool,
    pending: Vec<Vec<String>>,
}

impl LaunchQueue {
    fn enqueue(&mut self, arguments: Vec<String>) -> Result<(), Vec<String>> {
        if self.initialized {
            Err(arguments)
        } else {
            self.pending.push(arguments);
            Ok(())
        }
    }

    fn initialize(&mut self) -> Vec<Vec<String>> {
        self.initialized = true;
        std::mem::take(&mut self.pending)
    }
}

#[derive(Default)]
struct WindowVisibility {
    revision: u64,
    requested: bool,
    /// One programmatic focus per explicit request: losing focus, a refresh or a repeated
    /// delivery may never pull the window to the front again.
    focus_pending: bool,
    #[cfg(any(target_os = "macos", test))]
    closing: Option<u64>,
}

#[derive(Default)]
struct Visibility(HashMap<String, WindowVisibility>);

impl Visibility {
    fn request(&mut self, label: &str) {
        let window = self.0.entry(label.into()).or_default();
        window.revision = window.revision.wrapping_add(1);
        window.requested = true;
        window.focus_pending = true;
    }

    fn take_focus(&mut self, label: &str) -> bool {
        self.0.get_mut(label).is_some_and(|window| {
            let focus = window.requested && window.focus_pending;
            window.focus_pending = false;
            focus
        })
    }

    fn requested(&self, label: &str) -> bool {
        self.0.get(label).is_some_and(|window| window.requested)
    }

    fn cancel(&mut self, label: &str) {
        let window = self.0.entry(label.into()).or_default();
        window.revision = window.revision.wrapping_add(1);
        window.requested = false;
        window.focus_pending = false;
    }

    #[cfg(any(target_os = "macos", test))]
    fn begin_hide(&mut self, label: &str) -> Option<u64> {
        let window = self.0.entry(label.into()).or_default();
        if window.closing == Some(window.revision) {
            return None;
        }
        window.revision = window.revision.wrapping_add(1);
        window.requested = false;
        window.focus_pending = false;
        window.closing = Some(window.revision);
        Some(window.revision)
    }

    #[cfg(any(target_os = "macos", test))]
    fn hide_current(&self, label: &str, revision: u64) -> bool {
        self.0
            .get(label)
            .is_some_and(|window| window.revision == revision && window.closing == Some(revision))
    }

    #[cfg(any(target_os = "macos", test))]
    fn finish_hide(&mut self, label: &str, revision: u64, success: bool) -> bool {
        let Some(window) = self.0.get_mut(label) else {
            return false;
        };
        if window.closing == Some(revision) {
            window.closing = None;
        }
        if window.revision != revision {
            return false;
        }
        window.requested = !success;
        success
    }
}

pub fn queue_launch(app: &AppHandle, arguments: Vec<String>) -> Result<(), Vec<String>> {
    app.state::<Navigation>()
        .launches
        .lock()
        .unwrap()
        .enqueue(arguments)
}

pub fn initialize_launches(app: &AppHandle) -> Vec<Vec<String>> {
    app.state::<Navigation>()
        .launches
        .lock()
        .unwrap()
        .initialize()
}

pub fn cancel_presentation(app: &AppHandle, label: &str) {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .cancel(label);
}

#[cfg(target_os = "macos")]
pub fn begin_hide(app: &AppHandle, label: &str) -> Option<u64> {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .begin_hide(label)
}

#[cfg(target_os = "macos")]
pub fn finish_hide(app: &AppHandle, label: &str, revision: u64, success: bool) -> bool {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .finish_hide(label, revision, success)
}

#[cfg(target_os = "macos")]
pub fn hide_current(app: &AppHandle, label: &str, revision: u64) -> bool {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .hide_current(label, revision)
}

fn onboarding_tab(tab: Option<&str>) -> Option<&str> {
    tab.filter(|tab| {
        matches!(
            *tab,
            "welcome"
                | "engine"
                | "permissions"
                | "practice"
                | "resume"
                | "dictation-setup"
                | "dictation-permissions"
        )
    })
}

pub fn request_onboarding(app: &AppHandle, tab: Option<&str>) {
    if let Some(tab) = onboarding_tab(tab) {
        *app.state::<Navigation>().onboarding.lock().unwrap() = Some(tab.into());
    }
}

pub fn deliver_onboarding(app: &AppHandle) {
    if let Some(tab) = app.state::<Navigation>().onboarding.lock().unwrap().take() {
        let _ = app.emit_to("onboarding", "navigate", tab);
    }
}

pub fn valid_meeting_id(value: &str) -> bool {
    !value.trim().is_empty()
        && !matches!(value, "." | "..")
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}

fn history_entry_id(tab: &str) -> Option<&str> {
    let id = tab.strip_prefix("entry:")?;
    let valid = !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    valid.then_some(id)
}

pub fn legacy_route(kind: &str, tab: Option<&str>) -> Option<WorkspaceRoute> {
    use WorkspaceSection::*;
    let mut route = WorkspaceRoute::section(match kind {
        "dictation" => Dictation,
        "history" => History,
        "settings" => Settings,
        "live" | "recording" => Recording,
        "library" | "meetings" => Meetings,
        _ => return None,
    });
    match route.section {
        Dictation => {
            route.tab = tab
                .filter(|value| matches!(*value, "pad" | "home"))
                .map(str::to_owned)
        }
        History => {
            route.tab = tab
                .filter(|value| history_entry_id(value).is_some())
                .map(str::to_owned)
        }
        Settings => {
            route.tab = Some(
                match tab.unwrap_or("overview") {
                    "meetings" => "meeting-settings",
                    "local" => "local",
                    known @ ("overview" | "providers" | "models" | "dictation"
                    | "meeting-settings" | "recording" | "dictionary" | "styles"
                    | "appearance" | "account" | "advanced") => known,
                    _ => "overview",
                }
                .into(),
            );
        }
        Meetings => match tab {
            Some("chat" | "__chat__") => route.tab = Some("chat".into()),
            Some("library" | "meetings") | None => {}
            Some(id)
                if valid_meeting_id(id)
                    && !matches!(
                        id,
                        "overview"
                            | "providers"
                            | "models"
                            | "dictation"
                            | "meeting-settings"
                            | "recording"
                            | "dictionary"
                            | "styles"
                            | "appearance"
                            | "account"
                            | "advanced"
                            | "settings"
                            | "history"
                            | "onboarding"
                            | "live"
                            | "pill"
                            | "prompt"
                    ) =>
            {
                route.meeting_id = Some(id.into())
            }
            _ => {}
        },
        _ => {}
    }
    Some(route)
}

/// Generic task requests enter main without replacing their frontend-owned selection.
/// Presenting main itself leaves an explicit pending route untouched.
pub fn window_request_route(kind: &str, tab: Option<&str>) -> Option<WorkspaceRoute> {
    if tab.is_none() {
        let section = match kind {
            "settings" => Some(WorkspaceSection::Settings),
            "dictation" => Some(WorkspaceSection::Dictation),
            "history" => Some(WorkspaceSection::History),
            _ => None,
        };
        if let Some(section) = section {
            return Some(WorkspaceRoute::section(section));
        }
    }
    legacy_route(kind, tab)
}

pub fn request(app: &AppHandle, label: &str, route: Option<WorkspaceRoute>) {
    let navigation = app.state::<Navigation>();
    navigation.visibility.lock().unwrap().request(label);
    if let Some(route) = route {
        navigation
            .pending
            .lock()
            .unwrap()
            .insert(label.into(), route);
    }
}

pub fn deliver(app: &AppHandle, label: &str) {
    let route = app
        .state::<Navigation>()
        .pending
        .lock()
        .unwrap()
        .remove(label);
    if let Some(route) = route {
        let _ = app.emit_to(label, "workspace-navigate", route);
    }
}

/// True once per explicit request, for the show that answers it.
pub fn take_focus(app: &AppHandle, label: &str) -> bool {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .take_focus(label)
}

pub fn requested(app: &AppHandle, label: &str) -> bool {
    app.state::<Navigation>()
        .visibility
        .lock()
        .unwrap()
        .requested(label)
}

#[cfg(test)]
mod tests {

    #[test]
    fn task_specific_dictation_repairs_reach_an_existing_onboarding_window() {
        assert_eq!(
            onboarding_tab(Some("dictation-setup")),
            Some("dictation-setup")
        );
        assert_eq!(
            onboarding_tab(Some("dictation-permissions")),
            Some("dictation-permissions")
        );
        for legacy in ["welcome", "engine", "permissions", "practice", "resume"] {
            assert_eq!(onboarding_tab(Some(legacy)), Some(legacy));
        }
        assert_eq!(onboarding_tab(Some("untrusted-route")), None);
        assert_eq!(onboarding_tab(None), None);
    }

    #[test]
    fn prompt_sits_top_right_inside_the_work_area() {
        assert_eq!(
            prompt_origin((0, 25), (2880, 1750), 2.0, (420.0, 160.0)),
            (2880 - 840 - 24, 25 + 24)
        );
        assert_eq!(
            prompt_origin((-1920, 0), (1920, 1040), 1.0, (420.0, 160.0)),
            (-432, 12)
        );
        assert_eq!(
            prompt_origin((0, 0), (300, 100), 1.0, (420.0, 160.0)),
            (0, 0)
        );
    }
    use super::*;

    #[test]
    fn source_navigation_is_additive_and_only_targets_a_meeting() {
        let mut route = legacy_route("meetings", Some("2026-10-25 Projectoverleg")).unwrap();
        let legacy_json = serde_json::to_string(&route).unwrap();
        assert!(!legacy_json.contains("source_target"));
        assert!(
            serde_json::from_str::<WorkspaceRoute>(&legacy_json)
                .unwrap()
                .has_valid_source()
        );
        route.source_target = Some(MeetingSourceTarget {
            kind: MeetingSourceKind::Notes,
            start_ms: None,
            text: "Besluit over de planning".into(),
            meeting_uid: None,
            utterance_ids: None,
            utterance_fingerprints: None,
        });
        assert!(route.has_valid_source());
        let roundtrip: WorkspaceRoute =
            serde_json::from_str(&serde_json::to_string(&route).unwrap()).unwrap();
        assert_eq!(roundtrip, route);
        route.section = WorkspaceSection::History;
        assert!(!route.has_valid_source());
        route.section = WorkspaceSection::Meetings;
        route.source_target.as_mut().unwrap().text = " ".into();
        assert!(!route.has_valid_source());
    }

    #[test]
    fn durable_source_navigation_roundtrips_identity_without_accepting_partial_anchors() {
        let mut route = legacy_route("meetings", Some("2026-11-03 Room meeting")).unwrap();
        route.source_target = Some(MeetingSourceTarget {
            kind: MeetingSourceKind::Transcript,
            start_ms: Some(1250),
            text: "Ja, zeker.".into(),
            meeting_uid: Some("stable-meeting-uid".into()),
            utterance_ids: Some(vec!["utterance-1".into(), "utterance-2".into()]),
            utterance_fingerprints: Some(vec!["source-1".into(), "source-2".into()]),
        });
        assert!(route.has_valid_source());
        let reopened: WorkspaceRoute =
            serde_json::from_str(&serde_json::to_string(&route).unwrap()).unwrap();
        assert_eq!(reopened, route);
        route
            .source_target
            .as_mut()
            .unwrap()
            .utterance_fingerprints
            .as_mut()
            .unwrap()
            .pop();
        assert!(!route.has_valid_source());
        route
            .source_target
            .as_mut()
            .unwrap()
            .utterance_fingerprints
            .as_mut()
            .unwrap()
            .push("source-2".into());
        route.source_target.as_mut().unwrap().meeting_uid = None;
        assert!(!route.has_valid_source());
    }

    #[test]
    fn mac_startup_only_presents_pending_onboarding() {
        use pennel_core::OnboardingStatus::*;
        assert_eq!(
            startup_presentation(true, Pending),
            Presentation::Onboarding
        );
        assert_eq!(
            startup_presentation(true, Completed),
            Presentation::Background
        );
        assert_eq!(
            startup_presentation(true, Skipped),
            Presentation::Background
        );
        assert_eq!(
            startup_presentation(false, Pending),
            Presentation::Onboarding
        );
        assert_eq!(
            startup_presentation(false, Completed),
            Presentation::Workspace
        );
        assert_eq!(
            startup_presentation(false, Skipped),
            Presentation::Workspace
        );
    }

    #[test]
    fn explicit_mac_open_resumes_the_wizard_or_opens_the_workspace() {
        use pennel_core::OnboardingStatus::*;
        assert_eq!(
            explicit_presentation(true, Pending),
            Presentation::Onboarding
        );
        assert_eq!(
            explicit_presentation(true, Completed),
            Presentation::Workspace
        );
        assert_eq!(
            explicit_presentation(true, Skipped),
            Presentation::Workspace
        );
        assert_eq!(
            explicit_presentation(false, Pending),
            Presentation::Workspace
        );
    }

    #[test]
    fn early_launches_are_drained_once_before_live_dispatch() {
        let mut launches = LaunchQueue::default();
        assert!(launches.enqueue(vec![]).is_ok());
        assert!(launches.enqueue(vec!["--toggle-dictation".into()]).is_ok());
        assert_eq!(
            launches.initialize(),
            vec![vec![], vec!["--toggle-dictation".to_owned()]]
        );
        assert!(launches.initialize().is_empty());
        assert_eq!(
            launches.enqueue(vec!["--toggle-meeting".into()]),
            Err(vec!["--toggle-meeting".into()])
        );
    }

    #[test]
    fn an_explicit_request_focuses_its_window_at_most_once() {
        let mut visibility = Visibility::default();
        assert!(!visibility.take_focus("dictation"));
        visibility.request("dictation");
        assert!(visibility.take_focus("dictation"));
        // Losing focus, refreshing or delivering again never pulls the window back to the front.
        assert!(!visibility.take_focus("dictation"));
        assert!(visibility.requested("dictation"));
        visibility.request("dictation");
        assert!(visibility.take_focus("dictation"));
        assert!(!visibility.take_focus("dictation"));
    }

    #[test]
    fn a_closed_or_cancelled_window_is_never_focused() {
        let mut visibility = Visibility::default();
        visibility.request("settings");
        visibility.cancel("settings");
        assert!(!visibility.take_focus("settings"));
        visibility.request("history");
        let close = visibility.begin_hide("history").unwrap();
        assert!(!visibility.finish_hide("history", close, false));
        assert!(visibility.requested("history"));
        assert!(!visibility.take_focus("history"));
    }

    #[test]
    fn hiding_cancels_delayed_frontend_presentation() {
        let mut visibility = Visibility::default();
        visibility.request("main");
        assert!(visibility.requested("main"));
        let close = visibility.begin_hide("main").unwrap();
        assert!(visibility.hide_current("main", close));
        assert!(!visibility.requested("main"));
        assert!(visibility.begin_hide("main").is_none());
        assert!(visibility.finish_hide("main", close, true));
        assert!(!visibility.requested("main"));
    }

    #[test]
    fn reopening_invalidates_an_older_close_without_losing_a_new_close() {
        let mut visibility = Visibility::default();
        visibility.request("main");
        let old = visibility.begin_hide("main").unwrap();
        visibility.request("main");
        assert!(!visibility.hide_current("main", old));
        assert!(!visibility.finish_hide("main", old, true));
        assert!(visibility.requested("main"));
        let current = visibility.begin_hide("main").unwrap();
        assert!(!visibility.finish_hide("main", old, false));
        assert!(visibility.finish_hide("main", current, true));
    }

    #[test]
    fn failed_or_timed_out_hide_restores_the_requested_window() {
        let mut visibility = Visibility::default();
        visibility.request("main");
        let close = visibility.begin_hide("main").unwrap();
        assert!(!visibility.finish_hide("main", close, false));
        assert!(visibility.requested("main"));
        assert!(visibility.begin_hide("main").is_some());
    }

    #[test]
    fn hiding_one_product_window_keeps_other_windows_requested() {
        let mut visibility = Visibility::default();
        visibility.request("main");
        visibility.request("settings");
        let settings_close = visibility.begin_hide("settings").unwrap();
        assert!(visibility.requested("main"));
        assert!(!visibility.requested("settings"));

        visibility.request("history");
        assert!(visibility.finish_hide("settings", settings_close, true));
        assert!(visibility.requested("main"));
        assert!(visibility.requested("history"));
        assert!(!visibility.requested("settings"));
    }

    #[test]
    fn normal_work_area_preserves_the_approved_wizard_size() {
        assert_eq!(
            fit_window_size((850.0, 690.0), (520.0, 520.0), (1920, 1040), 1.0),
            FittedWindow {
                width: 850.0,
                height: 690.0,
                min_width: 520.0,
                min_height: 520.0
            }
        );
    }

    #[test]
    fn high_dpi_short_work_area_clamps_requested_height_and_minimum() {
        assert_eq!(
            fit_window_size((850.0, 690.0), (520.0, 520.0), (1920, 1080), 2.0),
            FittedWindow {
                width: 850.0,
                height: 492.0,
                min_width: 520.0,
                min_height: 492.0
            }
        );
    }

    #[test]
    fn tiny_work_area_never_retains_an_unreachable_minimum() {
        assert_eq!(
            fit_window_size((850.0, 690.0), (520.0, 520.0), (800, 600), 2.0),
            FittedWindow {
                width: 352.0,
                height: 252.0,
                min_width: 352.0,
                min_height: 252.0
            }
        );
    }

    #[test]
    fn fractional_scale_and_main_window_minimum_share_the_same_fitting_rules() {
        assert_eq!(
            fit_window_size((1040.0, 740.0), (760.0, 520.0), (1536, 864), 1.5),
            FittedWindow {
                width: 976.0,
                height: 528.0,
                min_width: 760.0,
                min_height: 520.0
            }
        );
        assert_eq!(
            fit_window_size((850.0, 690.0), (520.0, 520.0), (1200, 900), 0.0),
            fit_window_size((850.0, 690.0), (520.0, 520.0), (1200, 900), 1.0)
        );
    }

    #[test]
    fn all_product_screens_share_the_main_window() {
        for (kind, label, title) in [
            ("settings", "main", "Pennel"),
            ("history", "main", "Pennel"),
            ("dictation", "main", "Pennel"),
            ("onboarding", "onboarding", "Welcome to Pennel"),
            ("library", "main", "Pennel"),
            ("live", "main", "Pennel"),
            ("workspace", "main", "Pennel"),
        ] {
            let spec = window_spec(kind);
            assert_eq!((spec.label, spec.title), (label, title));
            assert_eq!(label_spec(label), Some(spec));
        }
        assert_eq!(label_spec("settings"), None);
        assert_eq!(label_spec("dictation"), None);
        assert_eq!(label_spec("history"), None);
        assert_eq!(label_spec("record-prompt"), Some(window_spec("prompt")));
        assert_eq!(label_spec("pill"), None);
        assert_eq!(
            query_value("2026-10-25 Overleg/&x"),
            "2026-10-25%20Overleg%2F%26x"
        );
        assert_eq!(
            legacy_route("settings", Some("styles"))
                .unwrap()
                .window_kind(),
            "workspace"
        );
        assert_eq!(
            legacy_route("live", None).unwrap().window_kind(),
            "workspace"
        );
        assert_eq!(
            legacy_route("dictation", Some("pad"))
                .unwrap()
                .tab
                .as_deref(),
            Some("pad")
        );
    }

    #[test]
    fn opening_settings_without_a_category_restores_the_frontend_category() {
        let route = window_request_route("settings", None).unwrap();
        assert_eq!(route.section, WorkspaceSection::Settings);
        assert_eq!(route.tab, None);
        assert_eq!(route.window_kind(), "workspace");
        assert_eq!(window_spec("settings"), window_spec("workspace"));
        assert_eq!(
            window_request_route("settings", Some("models"))
                .unwrap()
                .tab
                .as_deref(),
            Some("models")
        );
    }

    #[test]
    fn presenting_main_keeps_explicit_pending_product_routes() {
        let route = legacy_route("settings", Some("appearance")).unwrap();
        let kind = route.window_kind();
        assert_eq!(window_spec(kind).label, "main");
        assert_eq!(window_request_route(kind, None), None);
        for task in ["settings", "history", "dictation", "live"] {
            let explicit = legacy_route(task, Some("models")).unwrap();
            assert_eq!(explicit.window_kind(), "workspace");
            assert_eq!(window_request_route(explicit.window_kind(), None), None);
        }
        let history = window_request_route("history", None).unwrap();
        assert_eq!(history.section, WorkspaceSection::History);
        assert_eq!(history.tab, None);
        let dictation = window_request_route("dictation", None).unwrap();
        assert_eq!(dictation.section, WorkspaceSection::Dictation);
        assert_eq!(dictation.tab, None);
        assert_eq!(
            window_request_route("dictation", Some("home"))
                .unwrap()
                .tab
                .as_deref(),
            Some("home")
        );
        assert_eq!(
            window_request_route("dictation", Some("pad"))
                .unwrap()
                .tab
                .as_deref(),
            Some("pad")
        );
    }

    #[test]
    fn first_product_window_urls_keep_legacy_section_intent_unambiguous() {
        assert_eq!(window_url("dictation", None), "index.html?window=dictation");
        assert_eq!(
            window_url("dictation", Some("pad")),
            "index.html?window=dictation&tab=pad"
        );
        assert_eq!(window_url("history", None), "index.html?window=history");
        assert_eq!(
            window_url("history", Some("entry:qa-retained")),
            "index.html?window=history&tab=entry%3Aqa-retained"
        );
        assert_eq!(
            window_url("settings", Some("dictation")),
            "index.html?window=settings&tab=dictation"
        );
        assert_eq!(window_url("settings", None), "index.html?window=settings");
        assert_eq!(window_url("workspace", None), "index.html?window=workspace");
        assert_eq!(
            window_url("onboarding", Some("practice")),
            "index.html?window=onboarding"
        );
        assert_eq!(window_url("prompt", None), "index.html?window=prompt");
    }

    #[test]
    fn targeted_history_keeps_the_result_id_as_data() {
        let id = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
        let tab = format!("entry:{id}");
        assert_eq!(history_entry_id(&tab), Some(id));
        assert_eq!(
            legacy_route("history", Some(&tab)).unwrap().tab.as_deref(),
            Some(tab.as_str())
        );
        for invalid in [
            "entry:",
            "entry:../history",
            "entry:<script>",
            "entry:%3Cscript%3E",
            "entry:result\n",
            "other",
        ] {
            assert!(history_entry_id(invalid).is_none());
            assert!(
                legacy_route("history", Some(invalid))
                    .unwrap()
                    .tab
                    .is_none()
            );
        }
        assert!(history_entry_id(&format!("entry:{}", "x".repeat(129))).is_none());
    }

    #[test]
    fn settings_without_a_category_opens_task_readiness() {
        assert_eq!(
            legacy_route("settings", Some("advanced"))
                .unwrap()
                .tab
                .as_deref(),
            Some("advanced")
        );
        assert!(
            legacy_route("library", Some("advanced"))
                .unwrap()
                .meeting_id
                .is_none()
        );
        assert_eq!(
            legacy_route("settings", None).unwrap().tab.as_deref(),
            Some("overview")
        );
        assert_eq!(
            legacy_route("settings", Some("unknown"))
                .unwrap()
                .tab
                .as_deref(),
            Some("overview")
        );
        assert_eq!(
            legacy_route("settings", Some("models"))
                .unwrap()
                .tab
                .as_deref(),
            Some("models")
        );
        assert!(
            legacy_route("library", Some("overview"))
                .unwrap()
                .meeting_id
                .is_none()
        );
    }

    #[test]
    fn old_window_routes_land_in_one_workspace() {
        assert_eq!(
            legacy_route("history", None).unwrap().section,
            WorkspaceSection::History
        );
        assert_eq!(
            legacy_route("live", None).unwrap().section,
            WorkspaceSection::Recording
        );
        assert_eq!(
            legacy_route("settings", Some("meetings"))
                .unwrap()
                .tab
                .as_deref(),
            Some("meeting-settings")
        );
        assert_eq!(
            legacy_route("library", Some("chat"))
                .unwrap()
                .tab
                .as_deref(),
            Some("chat")
        );
        assert_eq!(
            legacy_route("library", Some("2026-10-25 Productoverleg"))
                .unwrap()
                .meeting_id
                .as_deref(),
            Some("2026-10-25 Productoverleg")
        );
    }

    #[test]
    fn meeting_routes_never_accept_paths_or_controls() {
        for bad in ["../meeting", ".", "..", "a/b", "a\\b", "a:b", "a\n"] {
            assert!(!valid_meeting_id(bad));
            assert!(
                legacy_route("library", Some(bad))
                    .unwrap()
                    .meeting_id
                    .is_none()
            );
        }
    }
}
