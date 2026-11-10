//! The floating status pill: transparent, always on top, click-through, never takes focus,
//! bottom-center on the screen that holds the mouse.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use pennel_core::DictationPreviewUpdate;
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "macos"))]
use tauri::PhysicalPosition;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow};

use crate::status::PillState;

pub const LABEL: &str = "pill";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PillModelPresentation {
    pub session_id: String,
    pub generation: u64,
    pub model: pennel_core::ConfiguredDictationModel,
    pub available_models: usize,
    pub enabled: bool,
    pub switched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_shortcut: Option<pennel_core::ShortcutLabel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PillSnapshot {
    #[serde(flatten)]
    pub state: PillState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<DictationPreviewUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_verification: Option<pennel_core::CleanupVerificationAvailability>,
    #[serde(skip)]
    visibility_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_model: Option<PillModelPresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_shortcut: Option<pennel_core::ShortcutLabel>,
}

#[derive(Clone, Serialize)]
struct ProcessingSnapshot<'a> {
    #[serde(flatten)]
    state: &'a PillState,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleanup_verification: &'a Option<pennel_core::CleanupVerificationAvailability>,
}

impl Default for PillSnapshot {
    fn default() -> Self {
        Self {
            state: PillState::Hidden,
            preview: None,
            cleanup_verification: None,
            visibility_generation: 0,
            speech_model: None,
            cancel_shortcut: None,
        }
    }
}

impl PillSnapshot {
    fn may_hide(&self, generation: u64) -> bool {
        self.state == PillState::Hidden && self.visibility_generation == generation
    }

    fn processing_snapshot(&self) -> ProcessingSnapshot<'_> {
        ProcessingSnapshot {
            state: &self.state,
            cleanup_verification: &self.cleanup_verification,
        }
    }

    fn set_state(&mut self, state: PillState) -> bool {
        let changed = self.state != state || (state == PillState::Hidden && self.preview.is_some());
        if (self.state == PillState::Hidden) != (state == PillState::Hidden) {
            self.visibility_generation = self.visibility_generation.wrapping_add(1);
        }
        self.state = state;
        if !matches!(
            self.state,
            PillState::Listening | PillState::HandsFree | PillState::Command | PillState::NoSound
        ) {
            self.cancel_shortcut = None;
        }
        if self.state == PillState::Hidden {
            self.preview = None;
            self.cleanup_verification = None;
            self.speech_model = None;
        }
        changed
    }

    fn begin_preview(&mut self, id: String) -> bool {
        if self.state == PillState::Hidden
            || self
                .preview
                .as_ref()
                .is_some_and(|current| current.session_id == id)
        {
            return false;
        }
        self.preview = Some(DictationPreviewUpdate {
            session_id: id,
            revision: 0,
            text: String::new(),
        });
        true
    }

    fn update_model(&mut self, mut next: PillModelPresentation) -> bool {
        if self.state == PillState::Hidden
            || self
                .preview
                .as_ref()
                .is_none_or(|preview| preview.session_id != next.session_id)
            || self
                .speech_model
                .as_ref()
                .is_some_and(|current| current.generation > next.generation)
        {
            return false;
        }
        // A boot snapshot restores metadata, never an old acknowledgement timer.
        next.switched = false;
        let changed = self.speech_model.as_ref() != Some(&next);
        self.speech_model = Some(next);
        changed
    }

    fn update_preview(&mut self, update: DictationPreviewUpdate) -> bool {
        let Some(current) = self.preview.as_ref() else {
            return false;
        };
        if self.state == PillState::Hidden
            || current.session_id != update.session_id
            || current.revision >= update.revision
        {
            return false;
        }
        self.preview = Some(update);
        true
    }

    fn clear_preview(&mut self, id: Option<&str>) -> bool {
        let Some(current) = self.preview.as_ref() else {
            return false;
        };
        if id.is_some_and(|id| id != current.session_id) {
            return false;
        }
        self.preview = None;
        self.cleanup_verification = None;
        self.speech_model = None;
        true
    }
}

pub struct Snapshot {
    content: Arc<Mutex<PillSnapshot>>,
    session: Arc<Mutex<Option<Weak<pennel_core::LiveDictationSession>>>>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            content: Arc::new(Mutex::new(PillSnapshot::default())),
            session: Arc::new(Mutex::new(None)),
        }
    }
}

fn live_shortcut(
    app: &AppHandle,
    state: &PillState,
    action: u32,
) -> Option<pennel_core::ShortcutLabel> {
    if !matches!(
        state,
        PillState::Listening | PillState::HandsFree | PillState::Command | PillState::NoSound
    ) {
        return None;
    }
    let shared = app.try_state::<Arc<crate::state::Shared>>()?;
    if !crate::record_shortcut::listening_action_ready(&shared, action) {
        return None;
    }
    let id = match action {
        3 => "cancel",
        4 => "next_model",
        _ => return None,
    };
    let platform = crate::shortcuts::platform();
    pennel_core::effective_shortcut(&shared.settings(), id, platform).map(|binding| {
        pennel_core::format_shortcut(&binding, platform, shared.interface_language())
    })
}

pub fn content_snapshot(app: &AppHandle) -> PillSnapshot {
    app.try_state::<Snapshot>()
        .map(|state| {
            let mut content = state.content.lock().unwrap().clone();
            content.cancel_shortcut = live_shortcut(app, &content.state, 3);
            let session = state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .and_then(Weak::upgrade);
            if content.state != PillState::Hidden
                && let Some(session) = session.filter(|session| {
                    content
                        .preview
                        .as_ref()
                        .is_some_and(|preview| preview.session_id == session.id())
                })
            {
                content.cleanup_verification = Some(session.cleanup_verification_availability());
            }
            content
        })
        .unwrap_or_default()
}

pub fn refresh_verification(app: &AppHandle) {
    let snapshot = content_snapshot(app);
    let _ = app.emit("dictation-state", snapshot.processing_snapshot());
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct PillMetrics {
    window_width: f64,
    window_height: f64,
}

fn metrics() -> &'static PillMetrics {
    #[derive(Deserialize)]
    struct Tokens {
        pill: PillMetrics,
    }
    static METRICS: OnceLock<PillMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        serde_json::from_str::<Tokens>(include_str!("../../../../assets/brand/tokens.json"))
            .expect("checked-in pill metrics must be valid")
            .pill
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn fitted_geometry(
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
) -> Option<(i32, i32, u32, u32)> {
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let width = (metrics().window_width * scale)
        .round()
        .max(1.0)
        .min(f64::from(size.0)) as u32;
    let height = (metrics().window_height * scale)
        .round()
        .max(1.0)
        .min(f64::from(size.1)) as u32;
    Some((
        origin.0 + ((size.0 - width) / 2) as i32,
        origin.1 + (size.1 - height) as i32,
        width,
        height,
    ))
}

const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
/// The length of the pill's leave fade in `pill.css`.
const LEAVE: Duration = Duration::from_millis(140);

/// Orders the window out once the pill has faded, unless it was shown again meanwhile.
fn hide_after_leaving(app: AppHandle, content: Arc<Mutex<PillSnapshot>>, generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(LEAVE);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            // Keep validation and the native action ordered against the next state change.
            let content = content.lock().unwrap();
            if content.may_hide(generation)
                && let Some(window) = handle.get_webview_window(LABEL)
            {
                platform::hide(&window);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(id: &str, revision: u64, text: &str) -> DictationPreviewUpdate {
        DictationPreviewUpdate {
            session_id: id.into(),
            revision,
            text: text.into(),
        }
    }

    fn listening() -> PillSnapshot {
        let mut content = PillSnapshot::default();
        content.set_state(PillState::Listening);
        content.begin_preview("first".into());
        content
    }

    #[test]
    fn preview_boot_snapshot_retains_status_and_latest_text() {
        let mut content = listening();
        assert!(content.update_preview(preview("first", 2, "Een voorlopige zin")));
        content.set_state(PillState::Working);
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["state"], "working");
        assert_eq!(json["preview"]["session_id"], "first");
        assert_eq!(json["preview"]["revision"], 2);
        assert_eq!(json["preview"]["text"], "Een voorlopige zin");
    }

    fn model(generation: u64) -> PillModelPresentation {
        PillModelPresentation {
            session_id: "first".into(),
            generation,
            model: pennel_core::ConfiguredDictationModel {
                choice: pennel_core::ModelChoice {
                    provider_id: "apple".into(),
                    model: "dictation".into(),
                },
                label: "Apple dictation".into(),
            },
            available_models: 2,
            enabled: true,
            switched: true,
            next_shortcut: None,
        }
    }

    #[test]
    fn speech_model_metadata_stays_with_capture_and_boot_does_not_replay_confirmation() {
        let mut content = listening();
        assert!(content.update_model(model(2)));
        assert!(!content.update_model(model(1)));
        let mut old = model(3);
        old.session_id = "previous".into();
        assert!(!content.update_model(old));
        let snapshot = serde_json::to_value(&content).unwrap();
        assert_eq!(snapshot["speech_model"]["generation"], 2);
        assert_eq!(snapshot["speech_model"]["switched"], false);
        assert!(snapshot["speech_model"].get("text").is_none());
        content.set_state(PillState::Hidden);
        assert!(content.speech_model.is_none());
        assert!(!content.update_model(model(4)));
    }

    #[test]
    fn cancel_hint_is_owned_by_listening_presentation_and_clears_before_processing() {
        let mut content = listening();
        content.cancel_shortcut = Some(pennel_core::ShortcutLabel {
            keycaps: vec!["Ctrl".into(), "X".into()],
            text: "Ctrl+X".into(),
        });
        assert_eq!(
            serde_json::to_value(&content).unwrap()["cancel_shortcut"]["text"],
            "Ctrl+X"
        );
        content.set_state(PillState::Working);
        assert!(content.cancel_shortcut.is_none());
        content.set_state(PillState::Hidden);
        assert!(
            serde_json::to_value(&content)
                .unwrap()
                .get("cancel_shortcut")
                .is_none()
        );
    }

    #[test]
    fn structured_notices_reach_the_webview_as_named_fields() {
        let mut content = listening();
        content.set_state(PillState::Problem {
            title: "No connection".into(),
            detail: Some("Your words are kept in History".into()),
        });
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["state"], "problem");
        assert_eq!(json["title"], "No connection");
        assert_eq!(json["detail"], "Your words are kept in History");
        assert_eq!(json["preview"]["session_id"], "first");
        let metadata = serde_json::to_value(content.processing_snapshot()).unwrap();
        assert_eq!(metadata["state"], "problem");
        assert!(metadata.get("preview").is_none());
    }

    #[test]
    fn frozen_verification_metadata_survives_processing_and_clears_with_its_preview() {
        let mut content = listening();
        content.cleanup_verification = Some(pennel_core::CleanupVerificationAvailability {
            enabled: true,
            available: false,
            reason_code: "unqualified".into(),
            provider_id: Some("frozen-provider".into()),
            model: Some("frozen-model".into()),
            endpoint: Some("https://frozen.example/v1/decisions".into()),
        });
        content.set_state(PillState::Working);
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(
            json["cleanup_verification"]["endpoint"],
            "https://frozen.example/v1/decisions"
        );
        assert_eq!(json["cleanup_verification"]["reason_code"], "unqualified");
        let metadata = serde_json::to_value(content.processing_snapshot()).unwrap();
        assert!(metadata.get("preview").is_none());
        assert!(metadata.get("text").is_none());
        assert_eq!(metadata["state"], "working");
        assert!(!content.clear_preview(Some("different-session")));
        assert!(content.cleanup_verification.is_some());
        assert!(content.clear_preview(Some("first")));
        assert!(content.cleanup_verification.is_none());
    }

    #[test]
    fn late_session_and_nonincreasing_revisions_never_replace_current_text() {
        let mut content = listening();
        assert!(content.update_preview(preview("first", 3, "De nieuwste tekst")));
        assert!(!content.update_preview(preview("old", 99, "Een oud resultaat")));
        assert!(!content.update_preview(preview("first", 2, "Een trager resultaat")));
        assert!(!content.update_preview(preview("first", 3, "Een dubbel resultaat")));
        assert_eq!(content.preview.as_ref().unwrap().text, "De nieuwste tekst");
    }

    #[test]
    fn old_completion_cannot_clear_a_new_capture() {
        let mut content = listening();
        content.update_preview(preview("first", 1, "Eerder"));
        assert!(content.begin_preview("second".into()));
        assert!(!content.clear_preview(Some("first")));
        assert!(!content.update_preview(preview("first", 100, "Te laat")));
        let current = content.preview.as_ref().unwrap();
        assert_eq!(current.session_id, "second");
        assert_eq!(current.revision, 0);
        assert!(current.text.is_empty());
    }

    #[test]
    fn repeated_begin_does_not_rewind_an_existing_preview() {
        let mut content = listening();
        content.update_preview(preview("first", 4, "Al herkend"));
        assert!(!content.begin_preview("first".into()));
        assert_eq!(content.preview.as_ref().unwrap().revision, 4);
        assert_eq!(content.preview.as_ref().unwrap().text, "Al herkend");
    }

    #[test]
    fn hiding_clears_transient_text_and_refuses_late_updates() {
        let mut content = listening();
        content.update_preview(preview("first", 1, "Tijdelijke tekst"));
        assert!(content.set_state(PillState::Hidden));
        assert!(content.preview.is_none());
        assert!(!content.update_preview(preview("first", 2, "Later")));
        assert!(!content.begin_preview("second".into()));
        assert!(
            serde_json::to_value(&content)
                .unwrap()
                .get("preview")
                .is_none()
        );
    }

    #[test]
    fn an_earlier_leave_cannot_hide_a_later_captures_leave() {
        let mut content = listening();
        content.set_state(PillState::Hidden);
        let first_leave = content.visibility_generation;
        assert!(content.may_hide(first_leave));
        content.set_state(PillState::Listening);
        assert!(!content.may_hide(first_leave));
        content.set_state(PillState::Hidden);
        assert!(content.may_hide(content.visibility_generation));
        assert!(!content.may_hide(first_leave));
    }

    #[test]
    fn repeated_hidden_emissions_do_not_restart_the_current_leave() {
        let mut content = listening();
        content.set_state(PillState::Hidden);
        let leave = content.visibility_generation;
        assert!(!content.set_state(PillState::Hidden));
        assert!(content.may_hide(leave));
        assert!(
            serde_json::to_value(&content)
                .unwrap()
                .get("visibility_generation")
                .is_none()
        );
    }

    #[test]
    fn core_caption_is_preserved_for_identical_grapheme_layout_in_both_renderers() {
        let mut content = listening();
        let text = format!("{}e\u{301}👩‍👩‍👧", "é".repeat(180));
        content.update_preview(preview("first", 1, &text));
        let visible = &content.preview.as_ref().unwrap().text;
        assert_eq!(visible, &text);
    }

    #[test]
    fn pill_geometry_stays_inside_small_scaled_and_negative_work_areas() {
        let (x, y, width, height) = fitted_geometry((-1400, 20), (350, 100), 1.25).unwrap();
        assert_eq!((x, y, width, height), (-1400, 20, 350, 100));
        let (x, y, width, height) = fitted_geometry((-1920, 24), (1920, 1056), 1.0).unwrap();
        assert_eq!(width, metrics().window_width as u32);
        assert_eq!(height, metrics().window_height as u32);
        assert!(x >= -1920 && x + width as i32 <= 0);
        assert!(y >= 24 && y + height as i32 <= 1080);
        assert!(fitted_geometry((0, 0), (0, 100), 1.0).is_none());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn show_without_focus(window: &WebviewWindow) {
    platform::show(window);
}
pub fn hide_without_focus(window: &WebviewWindow) {
    platform::hide(window);
}

pub fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let window = tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pill.html".into()))
        .title("Pennel")
        .inner_size(metrics().window_width, metrics().window_height)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    #[cfg(target_os = "macos")]
    if let Ok(pointer) = window.ns_window() {
        unsafe {
            crate::macos::configure_panel(pointer, false);
        }
    }
    Ok(window)
}

pub struct Pill {
    app: AppHandle,
    content: Arc<Mutex<PillSnapshot>>,
    session: Arc<Mutex<Option<Weak<pennel_core::LiveDictationSession>>>>,
    last_level: Mutex<Instant>,
}

impl Pill {
    pub fn new(app: AppHandle) -> Self {
        let content = app.state::<Snapshot>().content.clone();
        let session = app.state::<Snapshot>().session.clone();
        Self {
            app,
            content,
            session,
            last_level: Mutex::new(Instant::now()),
        }
    }

    pub fn show(&self, state: PillState) {
        let (was_hidden, generation) = {
            let mut content = self.content.lock().unwrap();
            let was_hidden = content.state == PillState::Hidden;
            if !content.set_state(state.clone()) {
                return;
            }
            content.cancel_shortcut = live_shortcut(&self.app, &content.state, 3);
            if state != PillState::Hidden {
                let session = self
                    .session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(Weak::upgrade);
                if let Some(session) = session.filter(|session| {
                    content
                        .preview
                        .as_ref()
                        .is_some_and(|preview| preview.session_id == session.id())
                }) {
                    content.cleanup_verification =
                        Some(session.cleanup_verification_availability());
                }
            }
            let _ = self
                .app
                .emit("dictation-state", content.processing_snapshot());
            let _ = self.app.emit_to(LABEL, "pill-state", &*content);
            if state == PillState::Hidden {
                self.emit_preview(&content);
            }
            (was_hidden, content.visibility_generation)
        };
        self.sync_visibility(was_hidden, generation);
    }

    pub fn begin_preview(
        &self,
        id: impl Into<String>,
        session: &Arc<pennel_core::LiveDictationSession>,
    ) {
        let mut content = self.content.lock().unwrap();
        if content.begin_preview(id.into()) {
            content.cleanup_verification = Some(session.cleanup_verification_availability());
            *self.session.lock().unwrap() = Some(Arc::downgrade(session));
            self.emit_preview(&content);
            let _ = self
                .app
                .emit("dictation-state", content.processing_snapshot());
        }
        drop(content);
        self.refresh_model(session, false);
    }

    pub fn refresh_model(&self, session: &Arc<pennel_core::LiveDictationSession>, switched: bool) {
        let shared = self.app.state::<Arc<crate::state::Shared>>();
        let settings = shared.settings();
        let models = shared.configured_dictation_models();
        let model = models
            .iter()
            .find(|model| model.choice == session.speech_model())
            .cloned();
        let mut content = self.content.lock().unwrap();
        let cancel = live_shortcut(&self.app, &content.state, 3);
        if content.cancel_shortcut != cancel {
            content.cancel_shortcut = cancel;
            let _ = self.app.emit_to(LABEL, "pill-state", &*content);
        }
        if content
            .preview
            .as_ref()
            .is_none_or(|preview| preview.session_id != session.id())
        {
            return;
        }
        let Some(model) = model else {
            content.speech_model = None;
            let _ = self
                .app
                .emit_to(LABEL, "pill-model", Option::<PillModelPresentation>::None);
            return;
        };
        let next = PillModelPresentation {
            session_id: session.id(),
            generation: session.preview_generation(),
            model,
            available_models: models.len(),
            enabled: settings.pill_model_switch_enabled,
            switched,
            next_shortcut: live_shortcut(&self.app, &content.state, 4),
        };
        if content.update_model(next.clone()) || switched {
            let _ = self.app.emit_to(LABEL, "pill-model", next);
        }
    }

    pub fn update_preview(&self, update: DictationPreviewUpdate) {
        let mut content = self.content.lock().unwrap();
        if content.update_preview(update) {
            self.emit_preview(&content);
        }
    }

    pub fn clear_preview(&self, id: Option<&str>) {
        let mut content = self.content.lock().unwrap();
        if content.clear_preview(id) {
            self.emit_preview(&content);
        }
    }

    fn emit_preview(&self, content: &PillSnapshot) {
        let _ = self.app.emit_to(LABEL, "pill-preview", &content.preview);
    }

    fn sync_visibility(&self, position: bool, generation: u64) {
        let app = self.app.clone();
        let handle = app.clone();
        let content = self.content.clone();
        let _ = app.run_on_main_thread(move || {
            let latest = content.lock().unwrap();
            if latest.visibility_generation != generation {
                return;
            }
            let existing = handle.get_webview_window(LABEL);
            let is_new = existing.is_none();
            let window = existing.or_else(|| {
                (latest.state != PillState::Hidden)
                    .then(|| create(&handle).ok())
                    .flatten()
            });
            let Some(window) = window else {
                return;
            };
            let _ = window.emit_to(LABEL, "pill-state", &*latest);
            if latest.state == PillState::Hidden {
                hide_after_leaving(handle, content.clone(), generation);
            } else if position || is_new {
                platform::show_at_bottom(&handle, &window);
            } else {
                platform::show(&window);
            }
        });
    }

    /// Throttled: the microphone callback fires far more often than the pill can redraw.
    pub fn level(&self, level: f32) {
        let mut last = self.last_level.lock().unwrap();
        if last.elapsed() < LEVEL_INTERVAL {
            return;
        }
        *last = Instant::now();
        let _ = self.app.emit_to(LABEL, "pill-level", level);
    }

    #[cfg(not(target_os = "macos"))]
    fn position(app: &AppHandle, window: &WebviewWindow) {
        let cursor = app.cursor_position().ok();
        let monitors = app.available_monitors().unwrap_or_default();
        let monitor = cursor
            .and_then(|c| {
                monitors.iter().find(|m| {
                    let (p, s) = (m.position(), m.size());
                    c.x >= f64::from(p.x)
                        && c.x < f64::from(p.x) + f64::from(s.width)
                        && c.y >= f64::from(p.y)
                        && c.y < f64::from(p.y) + f64::from(s.height)
                })
            })
            .cloned()
            .or_else(|| app.primary_monitor().ok().flatten());
        let Some(monitor) = monitor else {
            return;
        };
        let area = monitor.work_area();
        let Some((x, y, width, height)) = fitted_geometry(
            (area.position.x, area.position.y),
            (area.size.width, area.size.height),
            monitor.scale_factor(),
        ) else {
            return;
        };
        let _ = window.set_size(tauri::PhysicalSize::new(width, height));
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(windows)]
mod platform {
    use tauri::WebviewWindow;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SetWindowPos, ShowWindow,
    };

    fn hwnd(window: &WebviewWindow) -> Option<HWND> {
        window.hwnd().ok().map(|h| HWND(h.0))
    }

    /// Tauri's show() may activate the window; SW_SHOWNOACTIVATE keeps focus in the user's app.
    pub fn show(window: &WebviewWindow) {
        if let Some(hwnd) = hwnd(window) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    pub fn show_at_bottom(app: &tauri::AppHandle, window: &WebviewWindow) {
        super::Pill::position(app, window);
        show(window);
    }

    pub fn hide(window: &WebviewWindow) {
        if let Some(hwnd) = hwnd(window) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use tauri::WebviewWindow;
    pub fn show(window: &WebviewWindow) {
        if let Ok(pointer) = window.ns_window() {
            unsafe {
                crate::macos::show_panel(pointer);
            }
        }
    }
    /// Tauri applies position and size asynchronously on macOS, after the panel would already be
    /// ordered in at the default centred frame; the panel is placed synchronously instead.
    pub fn show_at_bottom(_app: &tauri::AppHandle, window: &WebviewWindow) {
        if let Ok(pointer) = window.ns_window() {
            let metrics = super::metrics();
            unsafe {
                crate::macos::show_panel_at_bottom(
                    pointer,
                    metrics.window_width,
                    metrics.window_height,
                );
            }
        }
    }

    pub fn hide(window: &WebviewWindow) {
        if let Ok(pointer) = window.ns_window() {
            unsafe {
                crate::macos::hide_panel(pointer);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use tauri::WebviewWindow;

    pub fn show(window: &WebviewWindow) {
        let _ = window.show();
    }

    pub fn show_at_bottom(app: &tauri::AppHandle, window: &WebviewWindow) {
        super::Pill::position(app, window);
        show(window);
    }

    pub fn hide(window: &WebviewWindow) {
        let _ = window.hide();
    }
}
