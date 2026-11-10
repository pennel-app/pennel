//! Owned bindings to the embedded Apple platform library. Native callbacks copy data before returning.
//! Selection, paste and speech recognition wait for asynchronous completions: call them on a worker.
#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, mpsc};

use serde::Deserialize;
use serde_json::Value;

type EventSink = unsafe extern "C" fn(*mut c_void, u32, u32);
type AudioSink = unsafe extern "C" fn(*mut c_void, u32, *const f32, usize, u64);
type LevelSink = unsafe extern "C" fn(*mut c_void, u32, f32);
type CaptureScopeSink = unsafe extern "C" fn(*mut c_void, bool);
type TextSink = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char);

unsafe extern "C" {
    fn pennel_mac_string_free(value: *mut c_char);
    fn pennel_mac_is_main_thread() -> bool;
    fn pennel_mac_permissions() -> *mut c_char;
    fn pennel_mac_preferred_languages() -> *mut c_char;
    fn pennel_mac_request_permission(kind: *const c_char) -> bool;
    fn pennel_mac_foreground() -> *mut c_char;
    fn pennel_mac_pick_style_application(
        owner: *mut c_void,
        title: *const c_char,
        prompt: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    );
    fn pennel_mac_pick_meeting_contact(owner: *mut c_void, sink: TextSink, context: *mut c_void);
    fn pennel_mac_localize_style_application_picker(title: *const c_char, prompt: *const c_char);
    fn pennel_mac_app_identity(id: *const c_char, points: i32) -> *mut c_char;
    fn pennel_mac_external_target_start();
    fn pennel_mac_external_target_capture() -> *mut c_char;
    fn pennel_mac_external_target_capture_current() -> *mut c_char;
    fn pennel_mac_external_target_cancel(token: *const c_char);
    fn pennel_mac_external_target_is_current(token: *const c_char) -> bool;
    fn pennel_mac_external_target_paste(
        text: *const c_char,
        token: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    );
    fn pennel_mac_other_instances() -> *mut c_char;
    fn pennel_mac_migrate_legacy_key(account: *const c_char, error: *mut *mut c_char) -> bool;
    fn pennel_mac_credential_presence(service: *const c_char, account: *const c_char) -> i32;
    fn pennel_mac_read_selection(sink: TextSink, context: *mut c_void);
    fn pennel_mac_copy_text(text: *const c_char, html: *const c_char) -> bool;
    fn pennel_mac_copy_secret(text: *const c_char) -> bool;
    fn pennel_mac_share(
        text: *const c_char,
        title: *const c_char,
        action: *const c_char,
    ) -> *mut c_char;
    fn pennel_mac_hotkey_start_bindings(
        bindings: *const c_char,
        sink: EventSink,
        context: *mut c_void,
    ) -> *mut c_void;
    fn pennel_mac_hotkey_set_bindings(handle: *mut c_void, bindings: *const c_char) -> bool;
    fn pennel_mac_hotkey_set_listening(handle: *mut c_void, listening: bool) -> bool;
    fn pennel_mac_hotkey_capture(
        handle: *mut c_void,
        sink: Option<TextSink>,
        context: *mut c_void,
    ) -> bool;
    fn pennel_mac_hotkey_held(handle: *mut c_void, shift: bool) -> bool;
    fn pennel_mac_hotkey_stop(handle: *mut c_void);
    fn pennel_mac_call_snapshot(apps: *const c_char, browsers: *const c_char) -> *mut c_char;
    fn pennel_mac_calendar_snapshot() -> *mut c_char;
    fn pennel_mac_meeting_speakers_snapshot(app_id: *const c_char) -> *mut c_char;
    fn pennel_mac_calendar_source_available(
        calendar_id: *const c_char,
        source_id: *const c_char,
    ) -> bool;
    fn pennel_mac_capture_start(
        source: u32,
        sink: AudioSink,
        context: *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn pennel_mac_capture_stop(handle: *mut c_void);
    fn pennel_mac_meeting_start_paths_with_scope_and_level(
        directory: *const c_char,
        app: *const c_char,
        microphone_path: *const c_char,
        system_path: *const c_char,
        echo: bool,
        sink: AudioSink,
        context: *mut c_void,
        scope_sink: CaptureScopeSink,
        scope_context: *mut c_void,
        level_sink: LevelSink,
        level_context: *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn pennel_mac_meeting_status(handle: *mut c_void) -> *mut c_char;
    fn pennel_mac_meeting_stop(handle: *mut c_void) -> *mut c_char;
    fn pennel_mac_meeting_stop_status(handle: *mut c_void) -> *mut c_char;
    fn pennel_mac_meeting_quiesce(handle: *mut c_void);
    fn pennel_mac_transcribe(
        samples: *const f32,
        count: usize,
        rate: u32,
        language: *const c_char,
        vocabulary: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    );
    fn pennel_mac_transcribe_timed(
        samples: *const f32,
        count: usize,
        rate: u32,
        language: *const c_char,
        vocabulary: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    );
    fn pennel_mac_speech_preview_start(
        language: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    ) -> *mut c_void;
    fn pennel_mac_speech_preview_start_timed(
        language: *const c_char,
        vocabulary: *const c_char,
        sink: TextSink,
        context: *mut c_void,
    ) -> *mut c_void;
    fn pennel_mac_speech_preview_feed(handle: *mut c_void, samples: *const f32, count: usize);
    fn pennel_mac_speech_preview_feed_at(
        handle: *mut c_void,
        samples: *const f32,
        count: usize,
        start_ms: u64,
    );
    fn pennel_mac_speech_preview_stop(handle: *mut c_void);
    fn pennel_mac_language_model_availability() -> *mut c_char;
    fn pennel_mac_language_model_complete(
        messages: *const c_char,
        timeout_secs: u64,
        sink: TextSink,
        context: *mut c_void,
    );
    fn pennel_mac_decode_audio(
        path: *const c_char,
        count: *mut usize,
        error: *mut *mut c_char,
    ) -> *mut f32;
    fn pennel_mac_float_free(samples: *mut f32, count: usize);
    fn pennel_mac_play_audio(path: *const c_char, offset: u64) -> *mut c_char;
    fn pennel_mac_stop_audio();
    fn pennel_mac_configure_panel(window: *mut c_void, interactive: bool);
    fn pennel_mac_show_panel(window: *mut c_void);
    fn pennel_mac_show_panel_at_bottom(window: *mut c_void, width: f64, height: f64);
    fn pennel_mac_show_panel_at_top_right(
        window: *mut c_void,
        width: f64,
        height: f64,
        margin: f64,
    );
    fn pennel_mac_hide_panel(window: *mut c_void);
    fn pennel_mac_set_accessory();
    fn pennel_mac_product_windows_visible(visible: bool);
}

fn c_string(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "The text contains an invalid null character".into())
}

fn path_string(path: &Path) -> Result<CString, String> {
    c_string(
        path.to_str()
            .ok_or("The file path is not valid Unicode text")?,
    )
}

unsafe fn copy_string(value: *const c_char) -> Option<String> {
    (!value.is_null()).then(|| unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() })
}

unsafe fn take_string(value: *mut c_char) -> Option<String> {
    let text = unsafe { copy_string(value) };
    if !value.is_null() {
        unsafe { pennel_mac_string_free(value) };
    }
    text
}

unsafe fn take_json(value: *mut c_char) -> Value {
    unsafe { take_string(value) }
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub fn preferred_languages() -> Vec<String> {
    let value = unsafe { take_json(pennel_mac_preferred_languages()) };
    serde_json::from_value(value).unwrap_or_default()
}

pub fn permissions() -> Value {
    unsafe { take_json(pennel_mac_permissions()) }
}

pub fn language_model_availability() -> Value {
    unsafe { take_json(pennel_mac_language_model_availability()) }
}

pub fn request_permission(kind: &str) -> Result<(), String> {
    let kind = c_string(kind)?;
    if unsafe { pennel_mac_request_permission(kind.as_ptr()) } {
        Ok(())
    } else {
        Err("Unknown permission".into())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ForegroundApp {
    pub pid: u32,
    pub bundle_id: String,
}

pub fn foreground() -> Option<ForegroundApp> {
    serde_json::from_value(unsafe { take_json(pennel_mac_foreground()) }).ok()
}

pub fn app_identity(id: &str, points: i32) -> Option<crate::app_identity::AppIdentity> {
    let id = CString::new(id).ok()?;
    serde_json::from_value(unsafe { take_json(pennel_mac_app_identity(id.as_ptr(), points)) }).ok()
}

pub fn pick_style_application(
    window: &tauri::WebviewWindow,
) -> Result<Option<crate::style_application::StyleApplication>, String> {
    let owner = window.ns_window().map_err(|error| error.to_string())?;
    let (title, prompt) = crate::style_application::picker_labels(crate::ui::language());
    let title = c_string(&title)?;
    let prompt = c_string(&prompt)?;
    let selected = wait_text(|sink, context| unsafe {
        pennel_mac_pick_style_application(owner, title.as_ptr(), prompt.as_ptr(), sink, context)
    })?;
    serde_json::from_str(
        selected
            .as_deref()
            .ok_or("The app picker did not respond.")?,
    )
    .map_err(|_| "The selected app has no usable app ID.".into())
}

pub fn pick_meeting_contact(window: &tauri::WebviewWindow) -> Result<Option<Value>, String> {
    let owner = window.ns_window().map_err(|error| error.to_string())?;
    let selected = wait_text(|sink, context| unsafe {
        pennel_mac_pick_meeting_contact(owner, sink, context)
    })?;
    serde_json::from_str(
        selected
            .as_deref()
            .ok_or("The contact picker did not respond.")?,
    )
    .map_err(|_| "The selected contact could not be read.".into())
}

pub fn localize_style_application_picker() -> Result<(), String> {
    let (title, prompt) = crate::style_application::picker_labels(crate::ui::language());
    let title = c_string(&title)?;
    let prompt = c_string(&prompt)?;
    unsafe { pennel_mac_localize_style_application_picker(title.as_ptr(), prompt.as_ptr()) };
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExternalTarget {
    pub token: String,
    pub name: String,
    pub bundle_id: String,
}

pub fn start_external_target_tracking() {
    unsafe { pennel_mac_external_target_start() };
}

pub fn external_target() -> Option<ExternalTarget> {
    serde_json::from_value(unsafe { take_json(pennel_mac_external_target_capture()) }).ok()
}

pub fn current_external_target() -> Option<ExternalTarget> {
    serde_json::from_value(unsafe { take_json(pennel_mac_external_target_capture_current()) }).ok()
}

pub fn cancel_external_target(token: &str) {
    if let Ok(token) = c_string(token) {
        unsafe { pennel_mac_external_target_cancel(token.as_ptr()) };
    }
}

pub fn external_target_is_current(token: &str) -> bool {
    c_string(token)
        .is_ok_and(|token| unsafe { pennel_mac_external_target_is_current(token.as_ptr()) })
}

pub fn paste_external_target(text: &str, token: &str) -> Result<(), String> {
    let text = c_string(text)?;
    let token = c_string(token)?;
    wait_text(|sink, context| unsafe {
        pennel_mac_external_target_paste(text.as_ptr(), token.as_ptr(), sink, context)
    })
    .map(|_| ())
}

/// Running canonical Pennel/Musli apps that predate the shared core's storage locks.
pub fn other_instances() -> Vec<ForegroundApp> {
    serde_json::from_value(unsafe { take_json(pennel_mac_other_instances()) }).unwrap_or_default()
}

/// Checks Keychain metadata without reading the secret or requesting authentication.
pub fn credential_presence(service: &str, account: &str) -> Result<bool, String> {
    let service = CString::new(service).map_err(|_| "Invalid credential service")?;
    let account = CString::new(account).map_err(|_| "Invalid credential account")?;
    match unsafe { pennel_mac_credential_presence(service.as_ptr(), account.as_ptr()) } {
        0 => Ok(false),
        1 => Ok(true),
        status => Err(format!("Credential store is unavailable ({status})")),
    }
}

/// Imports an absent legacy credential with SecItemAdd; current/racing canonical entries win.
pub fn migrate_legacy_key(account: &str) -> Result<bool, String> {
    let account = c_string(account)?;
    let mut error = ptr::null_mut();
    let copied = unsafe { pennel_mac_migrate_legacy_key(account.as_ptr(), &mut error) };
    match unsafe { take_string(error) } {
        Some(error) => Err(error),
        None => Ok(copied),
    }
}

unsafe extern "C" fn text_result(context: *mut c_void, value: *const c_char, error: *const c_char) {
    let sender =
        unsafe { Box::from_raw(context.cast::<mpsc::Sender<Result<Option<String>, String>>>()) };
    let result = match unsafe { copy_string(error) } {
        Some(error) => Err(error),
        None => Ok(unsafe { copy_string(value) }),
    };
    let _ = sender.send(result);
}

fn wait_text(start: impl FnOnce(TextSink, *mut c_void)) -> Result<Option<String>, String> {
    if unsafe { pennel_mac_is_main_thread() } {
        return Err("This operation must run on a background thread".into());
    }
    let (sender, receiver) = mpsc::channel::<Result<Option<String>, String>>();
    let context = Box::into_raw(Box::new(sender)).cast();
    start(text_result, context);
    // The native operation owns the sender until its one terminal callback. No timeout can free it early.
    receiver
        .recv()
        .map_err(|_| "The native operation was interrupted".to_owned())?
}

pub fn read_selection() -> Result<Option<String>, String> {
    wait_text(|sink, context| unsafe { pennel_mac_read_selection(sink, context) })
}

pub fn copy_text(text: &str, html: Option<&str>) -> Result<(), String> {
    let text = c_string(text)?;
    let html = html.map(c_string).transpose()?;
    let copied = unsafe {
        pennel_mac_copy_text(
            text.as_ptr(),
            html.as_ref().map_or(ptr::null(), |h| h.as_ptr()),
        )
    };
    if copied {
        Ok(())
    } else {
        Err("Text could not be copied to the clipboard".into())
    }
}

/// Copies text marked so clipboard managers keep it out of their history.
pub fn copy_secret(text: &str) -> Result<(), String> {
    let text = c_string(text)?;
    if unsafe { pennel_mac_copy_secret(text.as_ptr()) } {
        Ok(())
    } else {
        Err("Copying failed.".into())
    }
}

/// Opens a native share picker or mail composer after an explicit interface action.
pub fn share(text: &str, title: &str, action: &str) -> Result<(), String> {
    let text = c_string(text)?;
    let title = c_string(title)?;
    let action = c_string(action)?;
    match unsafe {
        take_string(pennel_mac_share(
            text.as_ptr(),
            title.as_ptr(),
            action.as_ptr(),
        ))
    } {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum NativeHotkeyEvent {
    Down,
    Up,
    Interrupt,
    Shift(bool),
    Action(u32),
}

type HotkeyCallback = Mutex<Box<dyn FnMut(NativeHotkeyEvent) + Send>>;

unsafe extern "C" fn hotkey_event(context: *mut c_void, kind: u32, value: u32) {
    let event = match kind {
        0 => NativeHotkeyEvent::Down,
        1 => NativeHotkeyEvent::Up,
        2 => NativeHotkeyEvent::Interrupt,
        3 => NativeHotkeyEvent::Shift(value != 0),
        4 => NativeHotkeyEvent::Action(value),
        _ => return,
    };
    let callback = unsafe { &*context.cast::<HotkeyCallback>() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.lock().unwrap_or_else(|p| p.into_inner())(event);
    }));
}

type ShortcutCaptureCallback = Mutex<Option<Box<dyn FnMut(Value) + Send>>>;
unsafe extern "C" fn shortcut_capture_event(
    context: *mut c_void,
    value: *const c_char,
    _error: *const c_char,
) {
    let Some(value) =
        (unsafe { copy_string(value) }).and_then(|text| serde_json::from_str::<Value>(&text).ok())
    else {
        return;
    };
    let callback = unsafe { &*context.cast::<ShortcutCaptureCallback>() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(callback) = callback.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            callback(value);
        }
    }));
}

pub struct Hotkey {
    handle: NonNull<c_void>,
    callback: Box<HotkeyCallback>,
    capture_callback: Box<ShortcutCaptureCallback>,
}

// Swift synchronizes handle access onto the main queue; the owned callback is Send and locked.
unsafe impl Send for Hotkey {}
// Every handle operation is serialized by Swift's main queue; Arc keeps it alive through calls.
unsafe impl Sync for Hotkey {}

impl Hotkey {
    pub fn start(
        bindings: &crate::native_shortcuts::MacShortcutTable,
        callback: impl FnMut(NativeHotkeyEvent) + Send + 'static,
    ) -> Result<Self, String> {
        let mut callback: Box<HotkeyCallback> = Box::new(Mutex::new(Box::new(callback)));
        let bindings =
            c_string(&serde_json::to_string(bindings).map_err(|error| error.to_string())?)?;
        let context = (&mut *callback as *mut HotkeyCallback).cast();
        let handle = NonNull::new(unsafe {
            pennel_mac_hotkey_start_bindings(bindings.as_ptr(), hotkey_event, context)
        })
        .ok_or("Shortcut unavailable. Give Pennel Accessibility access")?;
        Ok(Self {
            handle,
            callback,
            capture_callback: Box::new(Mutex::new(None)),
        })
    }

    pub fn set_bindings(
        &self,
        bindings: &crate::native_shortcuts::MacShortcutTable,
    ) -> Result<(), String> {
        let json = c_string(&serde_json::to_string(bindings).map_err(|error| error.to_string())?)?;
        if unsafe { pennel_mac_hotkey_set_bindings(self.handle.as_ptr(), json.as_ptr()) } {
            Ok(())
        } else {
            Err("Shortcut unavailable — check Accessibility".into())
        }
    }
    pub fn set_listening(&self, listening: bool) -> Result<(), String> {
        if unsafe { pennel_mac_hotkey_set_listening(self.handle.as_ptr(), listening) } {
            Ok(())
        } else {
            Err("Shortcut unavailable — check Accessibility".into())
        }
    }
    pub fn set_capture(
        &self,
        callback: Option<Box<dyn FnMut(Value) + Send>>,
    ) -> Result<(), String> {
        let start = callback.is_some();
        // Context has a stable allocation. Stop on the main queue before clearing the Rust closure.
        if start {
            *self
                .capture_callback
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = callback;
        }
        let context = (&*self.capture_callback as *const ShortcutCaptureCallback)
            .cast_mut()
            .cast();
        let sink = start.then_some(shortcut_capture_event as TextSink);
        let success = unsafe { pennel_mac_hotkey_capture(self.handle.as_ptr(), sink, context) };
        if !start || !success {
            *self
                .capture_callback
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = None;
        }
        if success {
            Ok(())
        } else {
            Err("Shortcut capture unavailable — check Accessibility".into())
        }
    }

    pub fn trigger_held(&self) -> bool {
        unsafe { pennel_mac_hotkey_held(self.handle.as_ptr(), false) }
    }

    pub fn shift_held(&self) -> bool {
        unsafe { pennel_mac_hotkey_held(self.handle.as_ptr(), true) }
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        unsafe { pennel_mac_hotkey_stop(self.handle.as_ptr()) };
        // Native stop disables queued callbacks before this allocation is dropped.
        let _ = &self.callback;
    }
}

#[derive(Debug, Deserialize)]
pub struct CallObservation {
    pub app_id: String,
    pub is_call: bool,
    pub is_playing: bool,
}

pub fn call_snapshot(
    call_apps: &[String],
    browsers: &[String],
) -> Result<Vec<CallObservation>, String> {
    let apps = c_string(&serde_json::to_string(call_apps).map_err(|e| e.to_string())?)?;
    let browsers = c_string(&serde_json::to_string(browsers).map_err(|e| e.to_string())?)?;
    serde_json::from_value(unsafe {
        take_json(pennel_mac_call_snapshot(apps.as_ptr(), browsers.as_ptr()))
    })
    .map_err(|e| e.to_string())
}

#[derive(Debug, Default, Deserialize)]
pub struct MeetingSpeakers {
    pub available: bool,
    #[serde(default)]
    pub call_ended: bool,
    #[serde(default)]
    pub call_active: bool,
    #[serde(default)]
    pub participants: Vec<String>,
    pub speaking: Vec<String>,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub process_started_at_ms: Option<u64>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub duplicate_names: bool,
}

/// Blocks for tens of milliseconds while the meeting app's interface is read; call it off the main thread.
pub fn meeting_speakers_snapshot(app_id: &str) -> MeetingSpeakers {
    let Ok(app_id) = c_string(app_id) else {
        return MeetingSpeakers::default();
    };
    serde_json::from_value(unsafe {
        take_json(pennel_mac_meeting_speakers_snapshot(app_id.as_ptr()))
    })
    .unwrap_or_default()
}

pub fn calendar_snapshot() -> Value {
    unsafe { take_json(pennel_mac_calendar_snapshot()) }
}

pub fn calendar_source_available(calendar_id: &str, source_id: Option<&str>) -> bool {
    let Ok(calendar_id) = CString::new(calendar_id) else {
        return false;
    };
    let source_id = match source_id.map(CString::new).transpose() {
        Ok(value) => value,
        Err(_) => return false,
    };
    unsafe {
        pennel_mac_calendar_source_available(
            calendar_id.as_ptr(),
            source_id
                .as_ref()
                .map_or(ptr::null(), |value| value.as_ptr()),
        )
    }
}

type AudioCallback = Mutex<Box<dyn FnMut(u32, &[f32], u64) + Send>>;

unsafe extern "C" fn audio_frame(
    context: *mut c_void,
    track: u32,
    samples: *const f32,
    count: usize,
    offset: u64,
) {
    if samples.is_null() || count == 0 {
        return;
    }
    let callback = unsafe { &*context.cast::<AudioCallback>() };
    let samples = unsafe { std::slice::from_raw_parts(samples, count) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.lock().unwrap_or_else(|p| p.into_inner())(track, samples, offset);
    }));
}

pub struct Capture {
    handle: Option<NonNull<c_void>>,
    callback: Box<AudioCallback>,
}

// Swift handles capture lifetime on its main queue and joins/drains callbacks during stop.
unsafe impl Send for Capture {}

impl Capture {
    pub fn start(
        source: u32,
        mut callback: impl FnMut(&[f32], u64) + Send + 'static,
    ) -> Result<Self, String> {
        let mut callback: Box<AudioCallback> =
            Box::new(Mutex::new(Box::new(move |_, samples, offset| {
                callback(samples, offset)
            })));
        let mut error = ptr::null_mut();
        let context = (&mut *callback as *mut AudioCallback).cast();
        let handle = NonNull::new(unsafe {
            pennel_mac_capture_start(source, audio_frame, context, &mut error)
        });
        let error = unsafe { take_string(error) };
        let handle = handle
            .ok_or_else(|| error.unwrap_or_else(|| "Audio recording could not start".into()))?;
        Ok(Self {
            handle: Some(handle),
            callback,
        })
    }

    pub fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { pennel_mac_capture_stop(handle.as_ptr()) };
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.finish();
        let _ = &self.callback;
    }
}

unsafe extern "C" fn meeting_level(context: *mut c_void, source: u32, level: f32) {
    let callback = unsafe { &*context.cast::<crate::meeting_level::LevelCallback>() };
    callback(source, level);
}

pub struct MeetingCapture {
    level_callback: Box<crate::meeting_level::LevelCallback>,
    handle: Option<NonNull<c_void>>,
    callback: Box<AudioCallback>,
    source_integrity: Arc<crate::meeting_speaker_evidence::AppAudioSourceIntegrity>,
}

/// Native retains this context only until quiescence/stop has invalidated pending rebuilds and
/// drained callbacks. MeetingCapture owns its Arc through that shutdown; sampler clones may outlive it.
unsafe extern "C" fn meeting_capture_scope(context: *mut c_void, app_scoped: bool) {
    if let Some(source) = unsafe {
        context
            .cast::<crate::meeting_speaker_evidence::AppAudioSourceIntegrity>()
            .as_ref()
    } {
        source.observe(app_scoped);
    }
}

unsafe impl Send for MeetingCapture {}

/// Borrowed only by the monitor that owns the retained native handle. It is used before
/// signalling the monitor to finish, so diagnostics I/O cannot delay hardware shutdown.
pub struct MeetingCaptureQuiescence(NonNull<c_void>);

unsafe impl Send for MeetingCaptureQuiescence {}

impl MeetingCaptureQuiescence {
    pub fn quiesce(&self) {
        unsafe { pennel_mac_meeting_quiesce(self.0.as_ptr()) }
    }
}

impl MeetingCapture {
    pub fn app_audio_source(
        &self,
    ) -> Arc<crate::meeting_speaker_evidence::AppAudioSourceIntegrity> {
        self.source_integrity.clone()
    }

    pub fn quiescence(&self) -> MeetingCaptureQuiescence {
        MeetingCaptureQuiescence(self.handle.expect("capture has not been stopped"))
    }
    pub fn start(
        directory: &Path,
        microphone_path: &Path,
        system_path: &Path,
        app_id: Option<&str>,
        echo: bool,
        callback: impl FnMut(u32, &[f32], u64) + Send + 'static,
        on_level: crate::meeting_level::LevelCallback,
    ) -> Result<Self, String> {
        let directory = path_string(directory)?;
        let microphone_path = path_string(microphone_path)?;
        let system_path = path_string(system_path)?;
        let app = app_id.map(c_string).transpose()?;
        let mut callback: Box<AudioCallback> = Box::new(Mutex::new(Box::new(callback)));
        let context = (&mut *callback as *mut AudioCallback).cast();
        let mut error = ptr::null_mut();
        let source_integrity =
            Arc::new(crate::meeting_speaker_evidence::AppAudioSourceIntegrity::new(app_id));
        let scope_context = Arc::as_ptr(&source_integrity).cast_mut().cast();
        let mut level_callback = Box::new(on_level);
        let level_context =
            (&mut *level_callback as *mut crate::meeting_level::LevelCallback).cast();
        let handle = NonNull::new(unsafe {
            pennel_mac_meeting_start_paths_with_scope_and_level(
                directory.as_ptr(),
                app.as_ref().map_or(ptr::null(), |a| a.as_ptr()),
                microphone_path.as_ptr(),
                system_path.as_ptr(),
                echo,
                audio_frame,
                context,
                meeting_capture_scope,
                scope_context,
                meeting_level,
                level_context,
                &mut error,
            )
        });
        let error = unsafe { take_string(error) };
        let handle = handle
            .ok_or_else(|| error.unwrap_or_else(|| "Meeting recording could not start".into()))?;
        Ok(Self {
            handle: Some(handle),
            callback,
            level_callback,
            source_integrity,
        })
    }

    pub fn status(&self) -> Value {
        self.handle.map_or(Value::Null, |handle| unsafe {
            take_json(pennel_mac_meeting_status(handle.as_ptr()))
        })
    }

    pub fn stop(mut self) -> Result<Value, String> {
        let Some(handle) = self.handle.take() else {
            return Err("The audio recording is unavailable.".into());
        };
        let status = unsafe { take_json(pennel_mac_meeting_stop_status(handle.as_ptr())) };
        if !status.is_object() {
            return Err("The recording status could not be checked.".into());
        }
        Ok(status)
    }

    fn finish(&mut self) -> Result<(), String> {
        if let Some(handle) = self.handle.take()
            && let Some(error) = unsafe { take_string(pennel_mac_meeting_stop(handle.as_ptr())) }
        {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for MeetingCapture {
    fn drop(&mut self) {
        let _ = self.finish();
        let _ = (&self.callback, &self.level_callback);
    }
}

type PreviewCallback = Mutex<Box<dyn FnMut(String) + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppleSpeechPreviewStatus {
    Warming,
    Ready,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppleSpeechPreviewEvent {
    Status {
        status: AppleSpeechPreviewStatus,
        reason: Option<String>,
    },
    Fragment {
        start_ms: u64,
        end_ms: u64,
        text: String,
        is_final: bool,
    },
}

unsafe extern "C" fn preview_text(
    context: *mut c_void,
    value: *const c_char,
    error: *const c_char,
) {
    if context.is_null() || !error.is_null() {
        return;
    }
    let Some(text) = (unsafe { copy_string(value) }) else {
        return;
    };
    let callback = unsafe { &*context.cast::<PreviewCallback>() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.lock().unwrap_or_else(|p| p.into_inner())(text);
    }));
}

pub struct AppleSpeechPreview {
    handle: Option<NonNull<c_void>>,
    callback: Box<PreviewCallback>,
}

// Swift serializes input and drains repeated callbacks before the handle is released.
unsafe impl Send for AppleSpeechPreview {}
unsafe impl Sync for AppleSpeechPreview {}

impl AppleSpeechPreview {
    pub fn start(
        language: Option<&str>,
        callback: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_with(language, None, callback)
    }

    pub fn start_timed(
        language: Option<&str>,
        vocabulary: &[String],
        mut callback: impl FnMut(AppleSpeechPreviewEvent) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_with(language, Some(vocabulary), move |json| {
            if let Ok(event) = serde_json::from_str(&json) {
                callback(event);
            }
        })
    }

    fn start_with(
        language: Option<&str>,
        vocabulary: Option<&[String]>,
        callback: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        let language = language.map(c_string).transpose()?;
        let vocabulary = vocabulary
            .map(|words| {
                serde_json::to_string(&words.iter().take(100).collect::<Vec<_>>())
                    .map_err(|_| "Invalid speech vocabulary".to_owned())
                    .and_then(|json| c_string(&json))
            })
            .transpose()?;
        let mut callback: Box<PreviewCallback> = Box::new(Mutex::new(Box::new(callback)));
        let context = (&mut *callback as *mut PreviewCallback).cast();
        let language = language
            .as_ref()
            .map_or(ptr::null(), |value| value.as_ptr());
        let handle = NonNull::new(unsafe {
            if let Some(vocabulary) = vocabulary.as_ref() {
                pennel_mac_speech_preview_start_timed(
                    language,
                    vocabulary.as_ptr(),
                    preview_text,
                    context,
                )
            } else {
                pennel_mac_speech_preview_start(language, preview_text, context)
            }
        })
        .ok_or_else(|| "Live Apple preview is unavailable".to_owned())?;
        Ok(Self {
            handle: Some(handle),
            callback,
        })
    }

    pub fn feed(&self, samples: &[f32]) {
        if let Some(handle) = self.handle {
            unsafe {
                pennel_mac_speech_preview_feed(handle.as_ptr(), samples.as_ptr(), samples.len())
            };
        }
    }

    pub fn feed_at(&self, samples: &[f32], start_ms: u64) {
        if let Some(handle) = self.handle {
            unsafe {
                pennel_mac_speech_preview_feed_at(
                    handle.as_ptr(),
                    samples.as_ptr(),
                    samples.len(),
                    start_ms,
                )
            };
        }
    }

    /// Callback publication stops synchronously; native analyzer teardown retains its own lifetime.
    pub fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { pennel_mac_speech_preview_stop(handle.as_ptr()) };
        }
    }
}

impl Drop for AppleSpeechPreview {
    fn drop(&mut self) {
        self.finish();
        let _ = &self.callback;
    }
}

pub fn transcribe(
    samples: &[f32],
    rate: u32,
    language: Option<&str>,
    vocabulary: &[String],
) -> Result<String, String> {
    let language = language.map(c_string).transpose()?;
    let vocabulary = c_string(
        &serde_json::to_string(vocabulary).map_err(|_| "Invalid speech vocabulary".to_owned())?,
    )?;
    wait_text(|sink, context| unsafe {
        pennel_mac_transcribe(
            samples.as_ptr(),
            samples.len(),
            rate,
            language.as_ref().map_or(ptr::null(), |l| l.as_ptr()),
            vocabulary.as_ptr(),
            sink,
            context,
        )
    })
    .map(|text| text.unwrap_or_default())
}

pub fn transcribe_timed(
    samples: &[f32],
    rate: u32,
    language: Option<&str>,
    vocabulary: &[String],
) -> Result<pennel_core::SpeechTranscript, String> {
    if rate == 0 {
        return Err("No usable audio".into());
    }
    let language = language.map(c_string).transpose()?;
    let vocabulary =
        c_string(&serde_json::to_string(vocabulary).map_err(|_| "Invalid speech vocabulary")?)?;
    let json = wait_text(|sink, context| unsafe {
        pennel_mac_transcribe_timed(
            samples.as_ptr(),
            samples.len(),
            rate,
            language
                .as_ref()
                .map_or(ptr::null(), |value| value.as_ptr()),
            vocabulary.as_ptr(),
            sink,
            context,
        )
    })?
    .ok_or("Apple Dictation returned no result")?;
    decode_timed_transcript(
        &json,
        (samples.len() as u64)
            .saturating_mul(1_000)
            .div_ceil(u64::from(rate)),
    )
}

fn decode_timed_transcript(
    json: &str,
    audio_duration_ms: u64,
) -> Result<pennel_core::SpeechTranscript, String> {
    let value: Value = serde_json::from_str(json).map_err(|_| "Invalid timed speech result")?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or("Invalid timed speech result")?
        .to_owned();
    let result = serde_json::from_value::<pennel_core::SpeechTranscript>(value)
        .unwrap_or_else(|_| pennel_core::SpeechTranscript::plain(text));
    Ok(pennel_core::SpeechTranscript::validated(
        result.text,
        result.spans,
        result.timing_kind,
        audio_duration_ms,
    )
    .trimmed())
}

pub fn language_model_complete(
    messages: Vec<pennel_core::LanguageModelMessage>,
    timeout_secs: u64,
) -> Result<String, String> {
    let messages =
        c_string(&serde_json::to_string(&messages).map_err(|_| "Invalid language model request")?)?;
    wait_text(|sink, context| unsafe {
        pennel_mac_language_model_complete(messages.as_ptr(), timeout_secs, sink, context)
    })
    .and_then(|text| {
        text.filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Apple Intelligence returned no text.".into())
    })
}

pub fn decode_audio(path: &Path) -> Result<Vec<f32>, String> {
    let path = path_string(path)?;
    let mut count = 0;
    let mut error = ptr::null_mut();
    let samples = unsafe { pennel_mac_decode_audio(path.as_ptr(), &mut count, &mut error) };
    if let Some(error) = unsafe { take_string(error) } {
        return Err(error);
    }
    if samples.is_null() {
        return Ok(Vec::new());
    }
    let result = unsafe { std::slice::from_raw_parts(samples, count) }.to_vec();
    unsafe { pennel_mac_float_free(samples, count) };
    Ok(result)
}

pub fn play_audio(path: &Path, offset_ms: u64) -> Result<(), String> {
    let path = path_string(path)?;
    match unsafe { take_string(pennel_mac_play_audio(path.as_ptr(), offset_ms)) } {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub fn stop_audio() {
    unsafe { pennel_mac_stop_audio() };
}

/// `window` must be a live NSWindow obtained from this process's Tauri window.
pub unsafe fn configure_panel(window: *mut c_void, interactive: bool) {
    unsafe { pennel_mac_configure_panel(window, interactive) };
}

/// `window` must be a live NSWindow obtained from this process's Tauri window.
pub unsafe fn show_panel(window: *mut c_void) {
    unsafe { pennel_mac_show_panel(window) };
}

/// Shows the panel bottom-centre on the screen under the pointer, sized in points.
/// `window` must be a live NSWindow obtained from this process's Tauri window.
pub unsafe fn show_panel_at_bottom(window: *mut c_void, width: f64, height: f64) {
    unsafe { pennel_mac_show_panel_at_bottom(window, width, height) };
}

/// Shows the panel top-right on the screen under the pointer, sized and inset in points.
/// `window` must be a live NSWindow obtained from this process's Tauri window.
pub unsafe fn show_panel_at_top_right(window: *mut c_void, width: f64, height: f64, margin: f64) {
    unsafe { pennel_mac_show_panel_at_top_right(window, width, height, margin) };
}

/// `window` must be a live NSWindow obtained from this process's Tauri window.
pub unsafe fn hide_panel(window: *mut c_void) {
    unsafe { pennel_mac_hide_panel(window) };
}

pub fn set_accessory() {
    unsafe { pennel_mac_set_accessory() };
}

pub fn product_windows_visible(visible: bool) {
    unsafe { pennel_mac_product_windows_visible(visible) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_reject_embedded_nuls() {
        assert!(c_string("hello\0world").is_err());
    }

    #[test]
    fn timed_preview_contract_keeps_status_and_ranges_separate_from_text() {
        assert_eq!(
            serde_json::from_value::<AppleSpeechPreviewEvent>(serde_json::json!({
                "kind": "status", "status": "unavailable", "reason": "assets_missing"
            }))
            .unwrap(),
            AppleSpeechPreviewEvent::Status {
                status: AppleSpeechPreviewStatus::Unavailable,
                reason: Some("assets_missing".into()),
            }
        );
        assert_eq!(
            serde_json::from_value::<AppleSpeechPreviewEvent>(serde_json::json!({
                "kind": "fragment", "start_ms": 1250, "end_ms": 2750,
                "text": "café 👩🏽‍💻", "is_final": true
            }))
            .unwrap(),
            AppleSpeechPreviewEvent::Fragment {
                start_ms: 1250,
                end_ms: 2750,
                text: "café 👩🏽‍💻".into(),
                is_final: true,
            }
        );
        assert!(
            serde_json::from_value::<AppleSpeechPreviewEvent>(serde_json::json!({
                "kind": "status", "status": "invented"
            }))
            .is_err()
        );
    }

    #[test]
    fn authoritative_timing_does_not_change_text_or_add_a_chunk_offset() {
        let result = decode_timed_transcript(
            r#"{"text":" café ja. ","timing_kind":"word","spans":[{"start_byte":0,"end_byte":6,"start_ms":100,"end_ms":400},{"start_byte":6,"end_byte":11,"start_ms":450,"end_ms":900}]}"#,
            1000,
        )
        .unwrap();
        assert_eq!(result.text, "café ja.");
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].start_ms, 100);
        assert_eq!(result.spans[1].end_ms, 900);
    }

    #[test]
    fn malformed_timing_retains_complete_authoritative_text() {
        for spans in [
            r#"[{"start_byte":0,"end_byte":4,"start_ms":0,"end_ms":500}]"#,
            r#"[{"start_byte":-1,"end_byte":5,"start_ms":0,"end_ms":500}]"#,
        ] {
            let json = format!(r#"{{"text":"café","timing_kind":"word","spans":{spans}}}"#);
            assert_eq!(
                decode_timed_transcript(&json, 1000).unwrap(),
                pennel_core::SpeechTranscript::plain("café")
            );
        }
    }

    #[test]
    fn native_permission_snapshot_has_expected_fields_without_prompting() {
        let snapshot = permissions();
        for field in [
            "microphone",
            "accessibility",
            "calendar",
            "speech",
            "system_audio",
        ] {
            assert!(snapshot[field].is_string(), "missing permission {field}");
        }
        assert!(snapshot["apple_speech"].is_boolean());
    }

    #[test]
    fn native_scope_callback_updates_the_retained_sampler_guard_without_a_native_handle() {
        let source = Arc::new(
            crate::meeting_speaker_evidence::AppAudioSourceIntegrity::new(Some(
                crate::meeting_speaker_evidence::TEAMS,
            )),
        );
        let check = source.check(crate::meeting_speaker_evidence::TEAMS);
        let context = Arc::as_ptr(&source).cast_mut().cast();
        assert!(!check());
        unsafe { meeting_capture_scope(context, true) };
        assert!(check());
        // The sampler's Arc keeps the context alive through finalization after capture stops.
        drop(source);
        unsafe { meeting_capture_scope(context, false) };
        assert!(!check());
        unsafe { meeting_capture_scope(context, true) };
        assert!(!check());
    }
}
