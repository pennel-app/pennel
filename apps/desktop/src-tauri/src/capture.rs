//! Window captures for visual review of preview builds (scripts/capture-desktop-windows.sh).
//! Only active in the isolated capture preview started with `PENNEL_WINDOW_CAPTURE=1`.
#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_void};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const VARIABLE: &str = "PENNEL_WINDOW_CAPTURE";
const DATA_VARIABLE: &str = "PENNEL_CAPTURE_DATA_DIR";
const IDENTIFIER: &str = "dev.pennel.Pennel.capture.preview";
const LIVE_CALLS_VARIABLE: &str = "PENNEL_CAPTURE_LIVE_CALLS";

#[link(name = "objc")]
unsafe extern "C" {
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

fn active() -> &'static bool {
    static ACTIVE: OnceLock<bool> = OnceLock::new();
    ACTIVE.get_or_init(|| std::env::var(VARIABLE).is_ok_and(|value| value == "1"))
}

pub fn enabled(app: &AppHandle) -> bool {
    *active() && crate::is_preview(app) && app.config().identifier == IDENTIFIER
}

fn fixture_directory(
    identifier: &str,
    capture: bool,
    value: Option<&std::ffi::OsStr>,
) -> Result<Option<PathBuf>, String> {
    if identifier != IDENTIFIER && value.is_none() {
        return Ok(None);
    }
    if identifier != IDENTIFIER || !capture {
        return Err(
            "An isolated capture directory requires the capture app and capture mode".into(),
        );
    }
    let path = value
        .map(PathBuf::from)
        .ok_or("The capture app requires an isolated capture directory")?;
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err(
            "The capture directory must be an absolute, non-root path without traversal".into(),
        );
    }
    Ok(Some(path))
}

/// A capture build never falls back to either installed application's data directory.
pub fn data_directory(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    fixture_directory(
        &app.config().identifier,
        enabled(app),
        std::env::var_os(DATA_VARIABLE).as_deref(),
    )
}

/// Capture previews normally skip observation; live acceptance explicitly opts into the same
/// production call detector while retaining capture mode's other isolation guarantees.
pub fn call_detection_enabled(app: &AppHandle) -> Result<bool, String> {
    call_detection_for_configuration(
        &app.config().identifier,
        enabled(app),
        std::env::var_os(LIVE_CALLS_VARIABLE).as_deref(),
        std::env::var_os(DATA_VARIABLE).as_deref(),
    )
}

#[derive(Serialize)]
pub struct CaptureRuntimeStatus {
    global_hotkey_active: bool,
    dictation_active: bool,
    dictation_jobs: usize,
    recording_busy: bool,
    auto_detect_enabled: bool,
    call_detector_configured: bool,
    audio_source_status: Option<serde_json::Value>,
    speaker_evidence_status: Option<serde_json::Value>,
}

/// Read actual isolation state without enabling hooks or requesting permission.
#[tauri::command]
pub fn capture_runtime_status(
    app: AppHandle,
    shared: tauri::State<'_, std::sync::Arc<crate::state::Shared>>,
) -> Result<CaptureRuntimeStatus, String> {
    if !enabled(&app) {
        return Err("Capture runtime inspection requires the isolated capture app".into());
    }
    let directory = data_directory(&app)?.ok_or("Capture runtime inspection is unavailable")?;
    if directory != shared.data_dir {
        return Err("Capture runtime inspection requires the owned capture directory".into());
    }
    let global_hotkey_active = shared.hotkey.lock().unwrap().is_some();
    let recording_busy = shared.recording.lock().unwrap().is_busy();
    let settings = shared.settings();
    let audio_source_status = shared
        .meeting
        .lock()
        .unwrap()
        .as_ref()
        .map(crate::meeting::MeetingRecorder::audio_source_status);
    let speaker_evidence_status = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|context| context.speaker_evidence.as_ref())
        .map(crate::meeting_speaker_evidence::EvidenceSampler::live_status);
    Ok(CaptureRuntimeStatus {
        global_hotkey_active,
        dictation_active: shared
            .dictation_active
            .load(std::sync::atomic::Ordering::SeqCst),
        dictation_jobs: shared
            .dictation_jobs
            .load(std::sync::atomic::Ordering::SeqCst),
        recording_busy,
        auto_detect_enabled: settings.auto_detect,
        call_detector_configured: call_detection_enabled(&app)?,
        audio_source_status,
        speaker_evidence_status,
    })
}

fn call_detection_for_configuration(
    identifier: &str,
    capture: bool,
    opt_in: Option<&std::ffi::OsStr>,
    directory: Option<&std::ffi::OsStr>,
) -> Result<bool, String> {
    let Some(opt_in) = opt_in else {
        return Ok(identifier != IDENTIFIER);
    };
    if opt_in != std::ffi::OsStr::new("1") {
        return Err("PENNEL_CAPTURE_LIVE_CALLS must be unset or 1".into());
    }
    fixture_directory(identifier, capture, directory)?
        .ok_or("Live capture call detection requires an isolated capture directory")?;
    Ok(true)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.pending");
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[derive(Deserialize, Serialize)]
pub struct CaptureEnvironment {
    material: String,
    appearance: String,
    reduce_transparency: bool,
    reduce_motion: bool,
    increase_contrast: bool,
    titlebar_height: f64,
    #[serde(default)]
    titlebar_controls_width: f64,
    focused: bool,
}

#[derive(Deserialize, Serialize)]
pub struct CaptureSnapshot {
    request_id: String,
    environment: CaptureEnvironment,
    #[serde(default)]
    frontend_environment: Option<CaptureEnvironment>,
    document_language: String,
    appearance: String,
    ready: bool,
    error: String,
    selected_tab: Option<String>,
    viewport: [f64; 2],
    scroll_width: f64,
}

#[tauri::command]
pub fn capture_snapshot(
    app: AppHandle,
    window: tauri::WebviewWindow,
    snapshot: CaptureSnapshot,
) -> Result<(), String> {
    let directory = data_directory(&app)?.ok_or("Capture is unavailable")?;
    if snapshot.request_id.len() > 128
        || snapshot.document_language.len() > 32
        || snapshot.appearance.len() > 32
        || snapshot.error.len() > 2048
    {
        return Err("Invalid capture metadata".into());
    }
    write_json(
        &directory.join(format!("capture-state-{}.json", window.label())),
        &snapshot,
    )
}

/// WebKit stops painting a web view whose window is occluded (for example while the screen is locked),
/// which leaves captures blank. Capture mode keeps the views painting.
pub fn prepare(app: &AppHandle, window: &tauri::WebviewWindow) {
    if !enabled(app) {
        return;
    }
    let directory = data_directory(app).ok().flatten();
    let label = window.label().to_owned();
    let _ = window.with_webview(move |webview| unsafe {
        let view = webview.inner();
        if view.is_null() {
            return;
        }
        let responds: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> bool =
            std::mem::transmute(objc_msgSend as *const ());
        let set: unsafe extern "C" fn(*mut c_void, *mut c_void, bool) =
            std::mem::transmute(objc_msgSend as *const ());
        let selector = sel_registerName(c"_setWindowOcclusionDetectionEnabled:".as_ptr());
        if responds(
            view,
            sel_registerName(c"respondsToSelector:".as_ptr()),
            selector,
        ) {
            set(view, selector, false);
        }
        if let Some(directory) = directory {
            let get: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const ());
            let native_window = get(view, sel_registerName(c"window".as_ptr()));
            if !native_window.is_null() {
                let number: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i64 =
                    std::mem::transmute(objc_msgSend as *const ());
                let window_id = number(native_window, sel_registerName(c"windowNumber".as_ptr()));
                let _ = write_json(
                    &directory.join(format!("capture-window-{label}.json")),
                    &serde_json::json!({ "label": label, "window_id": window_id, "pid": std::process::id() }),
                );
            }
        }
    });
}

/// `--capture-show <window> [tab]` opens a window on a route; `--capture-eval <label> <script>` drives
/// a fixture state that has no route (a meeting tab, a dialog).
pub fn dispatch(app: &AppHandle, arguments: &[String]) -> bool {
    if !enabled(app) {
        return false;
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--capture-inspect")
    {
        if let (Some(label), Some(request)) = (arguments.get(index + 1), arguments.get(index + 2))
            && let Some(window) = app.get_webview_window(label)
        {
            prepare(app, &window);
            if let Ok(request) = serde_json::to_string(request) {
                let _ = window.eval(format!(
                    r#"(async()=>{{await document.fonts.ready;const i=window.__TAURI_INTERNALS__;let environment;try{{environment=await i.invoke('desktop_environment')}}catch{{environment={{material:'unavailable',appearance:matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light',reduce_transparency:false,reduce_motion:matchMedia('(prefers-reduced-motion:reduce)').matches,increase_contrast:false,titlebar_height:0,titlebar_controls_width:0,focused:document.hasFocus()}}}};await i.invoke('capture_snapshot',{{snapshot:{{request_id:{request},environment,frontend_environment:{{material:document.documentElement.dataset.nativeMaterial||'unavailable',appearance:document.documentElement.dataset.nativeAppearance||environment.appearance,reduce_transparency:document.documentElement.dataset.reduceTransparency==='true',reduce_motion:document.documentElement.dataset.reduceMotion==='true',increase_contrast:document.documentElement.dataset.increaseContrast==='true',titlebar_height:parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--native-titlebar-height'))||0,titlebar_controls_width:parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--native-titlebar-controls-width'))||0,focused:document.documentElement.dataset.windowFocused==='true'}},document_language:document.documentElement.lang,appearance:document.documentElement.dataset.appearance||'system',ready:!!document.querySelector('#app > *')&&!document.querySelector('.startup-loading,.startup-error,[aria-busy="true"]'),error:(document.querySelector('.startup-error')?.textContent||'').slice(0,2048),selected_tab:document.querySelector('[data-tab][aria-selected="true"]')?.dataset.tab||null,viewport:[innerWidth,innerHeight],scroll_width:document.documentElement.scrollWidth}}}})}})()"#
                ));
            }
        }
        return true;
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--capture-size")
    {
        if let (Some(label), Some(width), Some(height)) = (
            arguments.get(index + 1),
            arguments
                .get(index + 2)
                .and_then(|value| value.parse::<u32>().ok()),
            arguments
                .get(index + 3)
                .and_then(|value| value.parse::<u32>().ok()),
        ) && (200..=4096).contains(&width)
            && (200..=4096).contains(&height)
            && let Some(window) = app.get_webview_window(label)
        {
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
        }
        return true;
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--capture-show")
    {
        let kind = arguments
            .get(index + 1)
            .map(String::as_str)
            .unwrap_or("workspace");
        let tab = arguments.get(index + 2).map(String::as_str);
        crate::show_window(app, kind, tab);
        return true;
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--capture-eval")
    {
        if let (Some(label), Some(script)) = (arguments.get(index + 1), arguments.get(index + 2))
            && let Some(window) = app.get_webview_window(label)
        {
            let _ = window.eval(script);
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn live_call_detection_requires_an_explicit_isolated_capture_opt_in() {
        let directory = Some(OsStr::new("/tmp/pennel-live-capture-owned"));
        assert!(!call_detection_for_configuration(IDENTIFIER, true, None, directory).unwrap());
        assert!(
            call_detection_for_configuration(IDENTIFIER, true, Some(OsStr::new("1")), directory)
                .unwrap()
        );
        assert!(call_detection_for_configuration("dev.pennel.Pennel", false, None, None).unwrap());
    }

    #[test]
    fn live_call_detection_opt_in_rejects_wrong_identity_capture_mode_or_directory() {
        let opt_in = Some(OsStr::new("1"));
        let directory = Some(OsStr::new("/tmp/pennel-live-capture-owned"));
        for identifier in ["dev.pennel.Pennel", "dev.pennel.Pennel.preview"] {
            assert!(call_detection_for_configuration(identifier, true, opt_in, directory).is_err());
        }
        assert!(call_detection_for_configuration(IDENTIFIER, false, opt_in, directory).is_err());
        assert!(call_detection_for_configuration(IDENTIFIER, true, opt_in, None).is_err());
        for path in ["relative", "/", "/tmp/../other-profile"] {
            assert!(
                call_detection_for_configuration(IDENTIFIER, true, opt_in, Some(OsStr::new(path)))
                    .is_err()
            );
        }
    }

    #[test]
    fn live_call_detection_rejects_malformed_opt_in_values() {
        for value in ["", "0", "true", "yes", " 1"] {
            assert!(
                call_detection_for_configuration(
                    IDENTIFIER,
                    true,
                    Some(OsStr::new(value)),
                    Some(OsStr::new("/tmp/pennel-live-capture-owned"))
                )
                .is_err()
            );
        }
    }

    #[test]
    fn installed_apps_do_not_accept_capture_directory_overrides() {
        for identifier in ["dev.pennel.Pennel", "dev.pennel.Pennel.preview"] {
            assert!(fixture_directory(identifier, true, Some(OsStr::new("/tmp/fixture"))).is_err());
            assert!(
                fixture_directory(identifier, false, None)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn capture_identity_requires_an_explicit_isolated_directory_and_mode() {
        assert!(fixture_directory(IDENTIFIER, false, Some(OsStr::new("/tmp/fixture"))).is_err());
        assert!(fixture_directory(IDENTIFIER, true, None).is_err());
        for invalid in ["relative", "/", "/tmp/../fixture"] {
            assert!(fixture_directory(IDENTIFIER, true, Some(OsStr::new(invalid))).is_err());
        }
        assert_eq!(
            fixture_directory(IDENTIFIER, true, Some(OsStr::new("/tmp/pennel-fixture"))).unwrap(),
            Some(PathBuf::from("/tmp/pennel-fixture")),
        );
    }
}
