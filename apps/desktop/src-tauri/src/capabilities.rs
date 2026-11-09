//! Runtime capabilities describe the current OS and permission state without requesting access.

use serde::{Deserialize, Serialize};
#[cfg(any(windows, target_os = "linux"))]
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
#[cfg(any(windows, target_os = "linux"))]
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Available,
    PermissionRequired,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub status: CapabilityStatus,
    pub reason: Option<String>,
}

impl Capability {
    fn localize(&mut self) {
        self.reason = self.reason.take().map(crate::ui::message);
    }
    pub fn available() -> Self {
        Self {
            status: CapabilityStatus::Available,
            reason: None,
        }
    }

    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            status: CapabilityStatus::Unsupported,
            reason: Some(reason.into()),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: CapabilityStatus::Unavailable,
            reason: Some(reason.into()),
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn permission(permission: &PermissionInfo, description: &str) -> Self {
        match permission.status {
            PermissionStatus::Granted | PermissionStatus::NotRequired => Self::available(),
            PermissionStatus::Unsupported => Self::unsupported(description),
            PermissionStatus::Unknown => Self::unavailable(description),
            _ => Self {
                status: CapabilityStatus::PermissionRequired,
                reason: Some(description.into()),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Microphone,
    Accessibility,
    Calendar,
    Speech,
    SystemAudio,
}

impl PermissionKind {
    #[cfg(any(target_os = "macos", test))]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Accessibility => "accessibility",
            Self::Calendar => "calendar",
            Self::Speech => "speech",
            Self::SystemAudio => "system_audio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    Granted,
    NotDetermined,
    Denied,
    Restricted,
    NotRequired,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionInfo {
    pub kind: PermissionKind,
    pub status: PermissionStatus,
}

#[cfg(any(target_os = "macos", test))]
fn permission_status(value: Option<&str>) -> PermissionStatus {
    match value {
        Some("authorized" | "granted" | "true") => PermissionStatus::Granted,
        Some("not_determined" | "notDetermined") => PermissionStatus::NotDetermined,
        Some("denied" | "false") => PermissionStatus::Denied,
        Some("restricted" | "write_only") => PermissionStatus::Restricted,
        Some("not_required") => PermissionStatus::NotRequired,
        Some("unsupported") => PermissionStatus::Unsupported,
        _ => PermissionStatus::Unknown,
    }
}

pub fn permissions() -> Vec<PermissionInfo> {
    #[cfg(target_os = "macos")]
    return permissions_from(&crate::macos::permissions());
    #[cfg(not(target_os = "macos"))]
    permissions_from(&serde_json::Value::Null)
}

/// Each macOS status lookup is a round trip to the privacy service (about 150 ms for all of
/// them), so one native snapshot serves every capability.
fn permissions_from(
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] native: &serde_json::Value,
) -> Vec<PermissionInfo> {
    [
        PermissionKind::Microphone,
        PermissionKind::Accessibility,
        PermissionKind::Calendar,
        PermissionKind::Speech,
        PermissionKind::SystemAudio,
    ]
    .into_iter()
    .map(|kind| {
        #[cfg(target_os = "macos")]
        let status = permission_status(native.get(kind.as_str()).and_then(|value| value.as_str()));
        #[cfg(not(target_os = "macos"))]
        let status = match kind {
            PermissionKind::Calendar | PermissionKind::Speech => PermissionStatus::Unsupported,
            PermissionKind::Microphone => PermissionStatus::Unknown,
            _ => PermissionStatus::NotRequired,
        };
        PermissionInfo { kind, status }
    })
    .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformCapabilities {
    pub global_hotkey: Capability,
    pub record_shortcut: Capability,
    pub microphone: Capability,
    pub system_audio: Capability,
    pub echo_cancellation: Capability,
    pub call_detection: Capability,
    pub calendar: Capability,
    pub apple_speech: Capability,
    pub apple_intelligence: Capability,
    pub selection: Capability,
    pub pad_external_paste: Capability,
    pub rich_clipboard: Capability,
    pub audio_playback: Capability,
    pub diarization: Capability,
    pub meeting_app_speakers: Capability,
    pub local_models: Capability,
    pub local_acceleration: &'static str,
}

impl PlatformCapabilities {
    pub fn localized(mut self) -> Self {
        for capability in [
            &mut self.global_hotkey,
            &mut self.record_shortcut,
            &mut self.microphone,
            &mut self.system_audio,
            &mut self.echo_cancellation,
            &mut self.call_detection,
            &mut self.calendar,
            &mut self.apple_speech,
            &mut self.apple_intelligence,
            &mut self.selection,
            &mut self.pad_external_paste,
            &mut self.rich_clipboard,
            &mut self.audio_playback,
            &mut self.diarization,
            &mut self.meeting_app_speakers,
            &mut self.local_models,
        ] {
            capability.localize();
        }
        self
    }
}

fn local_models_capability(unavailable_reason: Option<String>) -> Capability {
    unavailable_reason.map_or_else(Capability::available, Capability::unsupported)
}

#[cfg(any(target_os = "macos", test))]
fn apple_intelligence_capability(native: &serde_json::Value) -> Capability {
    let reason = native
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("Apple Intelligence availability could not be checked. Try again.");
    match native.get("status").and_then(|value| value.as_str()) {
        Some("available") => Capability::available(),
        Some("unsupported") => Capability::unsupported(reason),
        _ => Capability::unavailable(reason),
    }
}

#[cfg(any(target_os = "macos", test))]
fn apple_speech_capability(permission: &PermissionInfo) -> Capability {
    match permission.status {
        PermissionStatus::Restricted => Capability::unavailable(
            "Speech recognition is restricted by this Mac’s settings or management policy.",
        ),
        PermissionStatus::Denied => Capability::permission(
            permission,
            "Enable Pennel in System Settings → Privacy & Security → Speech Recognition.",
        ),
        _ => Capability::permission(
            permission,
            "Give Pennel Speech Recognition access to use Apple dictation.",
        ),
    }
}

#[cfg(any(target_os = "linux", test))]
fn call_capability(detection: Result<(), String>, browser: Result<(), String>) -> Capability {
    match detection {
        Ok(()) => Capability {
            status: CapabilityStatus::Available,
            reason: browser.err(),
        },
        Err(reason) => Capability::unavailable(reason),
    }
}

#[cfg(any(windows, test))]
fn echo_capability(result: Result<bool, String>) -> Capability {
    match result {
        Ok(true) => Capability::available(),
        Ok(false) => Capability::unsupported(
            "This microphone provides no active echo cancellation. Use headphones.",
        ),
        Err(reason) => Capability::unavailable(reason),
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Clone)]
struct ProbeState {
    #[cfg(target_os = "linux")]
    archive: Capability,
    #[cfg(target_os = "linux")]
    detection: Capability,
    #[cfg(windows)]
    echo: Capability,
}

#[cfg(any(windows, target_os = "linux"))]
impl ProbeState {
    fn pending(reason: &str) -> Self {
        Self {
            #[cfg(target_os = "linux")]
            archive: Capability::unavailable(reason),
            #[cfg(target_os = "linux")]
            detection: Capability::unavailable(reason),
            #[cfg(windows)]
            echo: Capability::unavailable(reason),
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn probe_cache() -> &'static Mutex<ProbeState> {
    static STATE: OnceLock<Mutex<ProbeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ProbeState::pending("Checking availability…")))
}

/// Device/CLI checks never run on the IPC or UI thread. Windows only initializes a probe stream;
/// it does not start microphone recording. Linux only queries codec and audio-server metadata.
pub fn spawn_probe(app: &tauri::AppHandle) {
    #[cfg(any(windows, target_os = "linux"))]
    {
        static STARTED: AtomicBool = AtomicBool::new(false);
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let events = app.clone();
        if std::thread::Builder::new().name("pennel-capabilities".into()).spawn(move || {
            #[cfg(windows)]
            let state = ProbeState { echo: echo_capability(crate::windows_meetings::echo_cancellation_available()) };
            #[cfg(target_os = "linux")]
            let state = {
                let archive = match crate::linux_meetings::archive_support() {
                    Ok(()) => Capability::available(),
                    Err(reason) => Capability::unavailable(reason),
                };
                let detection = crate::linux_meetings::detection_support();
                let browser = if crate::linux_input::is_wayland() {
                    Err("Detecting browser calls requires an X11 session. On Wayland you can record browser meetings manually; native call apps can still be detected through the audio server.".into())
                } else { crate::linux_meetings::browser_detection_support() };
                ProbeState { archive, detection: call_capability(detection, browser) }
            };
            *probe_cache().lock().unwrap() = state;
            let _ = events.emit("capabilities-changed", ());
        }).is_err() {
            *probe_cache().lock().unwrap() = ProbeState::pending("Availability could not be checked. Restart Pennel.");
            let _ = app.emit("capabilities-changed", ());
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = app;
}

pub fn snapshot(hotkey_error: Option<&str>) -> PlatformCapabilities {
    let mut result = PlatformCapabilities {
        global_hotkey: hotkey_error.map_or_else(Capability::available, Capability::unavailable),
        record_shortcut: Capability::unsupported(
            "The global recording shortcut is not available on this platform yet. Start recording from Pennel.",
        ),
        microphone: Capability::available(),
        system_audio: Capability::available(),
        echo_cancellation: Capability::unsupported(
            "Echo cancellation is unavailable on this device. Use headphones.",
        ),
        call_detection: Capability::unsupported(
            "Automatic meeting detection is unavailable on this platform.",
        ),
        calendar: Capability::unsupported(
            "Calendar information is currently only available through the macOS calendar.",
        ),
        apple_speech: Capability::unsupported("Apple Dictation requires macOS 26 or later."),
        apple_intelligence: Capability::unsupported(
            "Apple Intelligence is only available on a supported Mac running macOS 26 or later.",
        ),
        selection: Capability::available(),
        pad_external_paste: Capability::available(),
        rich_clipboard: Capability::available(),
        audio_playback: Capability::unsupported(
            "Audio playback is not available on this platform yet.",
        ),
        diarization: Capability::unsupported(
            "Speaker recognition requires a saved audio track and a local speaker model.",
        ),
        meeting_app_speakers: Capability::unsupported(
            "Meeting participant names are currently only read from Microsoft Teams on macOS.",
        ),
        local_models: local_models_capability(pennel_core::local_models_unavailable_reason()),
        local_acceleration: if cfg!(target_os = "macos") {
            "metal"
        } else if cfg!(feature = "vulkan") {
            "vulkan"
        } else {
            "cpu"
        },
    };
    #[cfg(target_os = "macos")]
    {
        let native = crate::macos::permissions();
        let permissions = permissions_from(&native);
        let find = |kind| {
            permissions
                .iter()
                .find(|entry| entry.kind == kind)
                .expect("all permission kinds are present")
        };
        result.microphone = Capability::permission(
            find(PermissionKind::Microphone),
            "Give Pennel microphone access in System Settings.",
        );
        result.system_audio.reason =
            Some("System audio access is checked when you start recording.".into());
        result.selection = Capability::permission(
            find(PermissionKind::Accessibility),
            "Give Pennel Accessibility access to read selections and paste text.",
        );
        result.record_shortcut = Capability::permission(
            find(PermissionKind::Accessibility),
            "Give Pennel Accessibility access to use the recording shortcut.",
        );
        result.calendar = Capability::permission(
            find(PermissionKind::Calendar),
            "Give Pennel calendar access to use meeting information.",
        );
        result.meeting_app_speakers = Capability::permission(
            find(PermissionKind::Accessibility),
            "Give Pennel Accessibility access to read meeting participants.",
        );
        result.call_detection = Capability::available();
        result.echo_cancellation = Capability::available();
        result.audio_playback = Capability::available();
        result.diarization = Capability::available();
        result.apple_intelligence =
            apple_intelligence_capability(&crate::macos::language_model_availability());
        if native
            .get("apple_speech")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            result.apple_speech = apple_speech_capability(find(PermissionKind::Speech));
        }
    }
    #[cfg(windows)]
    {
        result.call_detection = Capability::available();
        result.audio_playback = Capability::available();
        result.diarization = Capability::available();
        result.echo_cancellation = probe_cache().lock().unwrap().echo.clone();
        result.record_shortcut = Capability::available();
    }
    #[cfg(target_os = "linux")]
    {
        let probe = probe_cache().lock().unwrap().clone();
        result.call_detection = probe.detection;
        result.audio_playback = probe.archive.clone();
        result.diarization = probe.archive;
        if crate::linux_input::is_wayland() {
            result.global_hotkey = Capability::unavailable(crate::linux_input::SHORTCUT_HELP);
            result.selection = Capability::unavailable(
                "Reading selections and automatic pasting are not supported on Wayland. Pennel copies the text; paste it with Ctrl+V yourself.",
            );
            result.rich_clipboard = Capability::unsupported(
                "On Wayland, Pennel copies plain text only. Paste it yourself with Ctrl+V.",
            );
            result.record_shortcut = Capability::unavailable(
                "Wayland: configure a recording shortcut for pennel --toggle-meeting in your desktop settings.",
            );
        } else if crate::linux_input::has_x11_display() {
            result.record_shortcut = Capability::available();
            result.rich_clipboard = Capability { status: CapabilityStatus::Available, reason: Some("Rich copying is supported. After dictation, Pennel on Linux restores only text or one image from the previous clipboard.".into()) };
        } else {
            let reason =
                "No X11 session is available for global shortcuts, selections or the clipboard.";
            result.global_hotkey = Capability::unavailable(reason);
            result.selection = Capability::unavailable(reason);
            result.rich_clipboard = Capability::unavailable(reason);
            result.record_shortcut = Capability::unavailable(reason);
        }
    }
    if result.record_shortcut.status == CapabilityStatus::Available
        && let Some(reason) = crate::record_shortcut::error()
    {
        result.record_shortcut = Capability::unavailable(reason);
    }
    #[cfg(target_os = "macos")]
    if result.record_shortcut.status == CapabilityStatus::Available
        && let Some(reason) = hotkey_error
    {
        result.record_shortcut = Capability::unavailable(reason);
    }
    result.pad_external_paste = result.selection.clone();
    #[cfg(target_os = "linux")]
    if crate::linux_input::is_wayland() {
        result.pad_external_paste = Capability::unsupported(
            "Wayland does not allow automatic external pasting. Copy the text and paste it with Ctrl+V yourself.",
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_processor_makes_local_models_unsupported_with_the_core_reason() {
        assert_eq!(local_models_capability(None), Capability::available());
        let old = local_models_capability(Some(pennel_core::UNSUPPORTED_PROCESSOR.into()));
        assert_eq!(old.status, CapabilityStatus::Unsupported);
        assert_eq!(
            old.reason.as_deref(),
            Some("This processor is too old for local models. Choose a cloud provider.")
        );
    }

    #[test]
    fn apple_model_availability_is_native_and_fail_closed() {
        let disabled = apple_intelligence_capability(&serde_json::json!({
            "status": "unavailable", "reason": "Schakel Apple Intelligence in."
        }));
        assert_eq!(disabled.status, CapabilityStatus::Unavailable);
        assert_eq!(
            disabled.reason.as_deref(),
            Some("Schakel Apple Intelligence in.")
        );
        assert_eq!(
            apple_intelligence_capability(
                &serde_json::json!({"status": "available", "reason": null})
            )
            .status,
            CapabilityStatus::Available
        );
        assert_eq!(
            apple_intelligence_capability(
                &serde_json::json!({"status": "unsupported", "reason": "Mac vereist."})
            )
            .status,
            CapabilityStatus::Unsupported
        );
        for unknown in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"status": "ready"}),
        ] {
            assert_eq!(
                apple_intelligence_capability(&unknown).status,
                CapabilityStatus::Unavailable
            );
        }
    }

    #[test]
    fn denied_and_restricted_speech_explain_their_distinct_recovery() {
        let mut speech = PermissionInfo {
            kind: PermissionKind::Speech,
            status: PermissionStatus::Denied,
        };
        let denied = apple_speech_capability(&speech);
        assert_eq!(denied.status, CapabilityStatus::PermissionRequired);
        assert!(denied.reason.unwrap().contains("System Settings"));
        speech.status = PermissionStatus::Restricted;
        assert_eq!(
            apple_speech_capability(&speech).status,
            CapabilityStatus::Unavailable
        );
        speech.status = PermissionStatus::Granted;
        assert_eq!(
            apple_speech_capability(&speech).status,
            CapabilityStatus::Available
        );
    }

    #[test]
    fn browser_limitations_do_not_disable_native_call_detection() {
        let capability = call_capability(Ok(()), Err("Browsercalls vereisen X11.".into()));
        assert_eq!(capability.status, CapabilityStatus::Available);
        assert_eq!(
            capability.reason.as_deref(),
            Some("Browsercalls vereisen X11.")
        );
        let capability = call_capability(Err("Audioserver niet bereikbaar.".into()), Ok(()));
        assert_eq!(capability.status, CapabilityStatus::Unavailable);
        assert_eq!(
            capability.reason.as_deref(),
            Some("Audioserver niet bereikbaar.")
        );
    }

    #[test]
    fn echo_support_requires_an_active_endpoint_effect_not_just_a_supported_os() {
        assert_eq!(
            echo_capability(Ok(true)).status,
            CapabilityStatus::Available
        );
        assert_eq!(
            echo_capability(Ok(false)).status,
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            echo_capability(Err("Geen microfoon.".into())).status,
            CapabilityStatus::Unavailable
        );
    }

    #[test]
    fn unknown_native_permission_never_becomes_granted() {
        assert_eq!(permission_status(None), PermissionStatus::Unknown);
        assert_eq!(
            permission_status(Some("unrecognized")),
            PermissionStatus::Unknown
        );
        assert_eq!(
            permission_status(Some("authorized")),
            PermissionStatus::Granted
        );
        assert_eq!(permission_status(Some("denied")), PermissionStatus::Denied);
        assert_eq!(
            permission_status(Some("write_only")),
            PermissionStatus::Restricted
        );
    }

    #[test]
    fn denied_permission_remains_a_recoverable_capability() {
        let permission = PermissionInfo {
            kind: PermissionKind::Microphone,
            status: PermissionStatus::Denied,
        };
        let capability = Capability::permission(&permission, "Microfoon vereist toestemming.");
        assert_eq!(capability.status, CapabilityStatus::PermissionRequired);
        assert_eq!(
            serde_json::to_value(&capability).unwrap(),
            serde_json::json!({"status":"permission_required","reason":"Microfoon vereist toestemming."})
        );
    }

    #[test]
    fn permission_names_are_the_stable_native_contract() {
        for kind in [
            PermissionKind::Microphone,
            PermissionKind::Accessibility,
            PermissionKind::Calendar,
            PermissionKind::Speech,
            PermissionKind::SystemAudio,
        ] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
    }
}
