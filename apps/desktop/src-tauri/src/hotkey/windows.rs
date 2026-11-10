//! Public low-level hooks decide withholding; RegisterHotKey reserves one-press global bindings.
use super::{
    CaptureSlot, HotkeyEvent, Key, TriggerBinding, TriggerTracker, capture_callback, publish,
    route_shared,
};
use crate::gesture::KeyEdge;
use pennel_core::{Settings, ShortcutPlatform};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, ToUnicodeEx, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, MSG, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
};

struct Shared {
    tracker: Mutex<TriggerTracker>,
    sender: Mutex<Option<Sender<HotkeyEvent>>>,
    capture: CaptureSlot,
    physical: Mutex<crate::portable_shortcuts::PhysicalKeys>,
}
fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        tracker: Mutex::new(TriggerTracker::default()),
        sender: Mutex::new(None),
        capture: Mutex::new(None),
        physical: Mutex::new(crate::portable_shortcuts::PhysicalKeys::default()),
    })
}
pub struct Hook;
const VK_MASK: u16 = 0xE8;
fn key_for(vk: u32) -> Key {
    match vk {
        0xA2 => Key::LeftCtrl,
        0xA3 => Key::RightCtrl,
        0xA4 => Key::LeftAlt,
        0xA5 => Key::RightAlt,
        0x5B => Key::LeftSuper,
        0x5C => Key::RightSuper,
        0xA0 => Key::LeftShift,
        0xA1 => Key::RightShift,
        0x10 | 0x11 | 0x12 | 0x14 | 0x90 | 0x91 | 0xE8 => Key::OtherModifier,
        _ => Key::Other(vk),
    }
}
fn send_mask_key() {
    let input = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_MASK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(
            &[input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)],
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}
fn base_key(info: &KBDLLHOOKSTRUCT, code: &str) -> String {
    if matches!(
        code,
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
    ) || code.starts_with('F')
    {
        return code.into();
    }
    let mut text = [0u16; 8];
    let state = [0u8; 256];
    let count = unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        // Bit 2 avoids changing the real keyboard's dead-key/IME buffer.
        ToUnicodeEx(
            info.vkCode,
            info.scanCode,
            &state,
            &mut text,
            4,
            Some(GetKeyboardLayout(thread)),
        )
    };
    if count > 0 && count as usize <= text.len() {
        let text = String::from_utf16_lossy(&text[..count as usize]);
        if text.chars().count() == 1 && !text.chars().any(char::is_control) {
            return text;
        }
    }
    "Unidentified".into()
}
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.flags.0 & LLKHF_INJECTED.0 == 0 {
            let down = info.flags.0 & LLKHF_UP.0 == 0;
            let state = shared();
            let identity = ((info.scanCode as u64) << 32)
                | (((info.flags.0 & LLKHF_EXTENDED.0 != 0) as u64) << 31)
                | if info.scanCode == 0 {
                    info.vkCode as u64
                } else {
                    0
                };
            let (virtual_key, repeated) = state
                .physical
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .key(identity, info.vkCode, down);
            let key = key_for(virtual_key);
            let routed = route_shared(&state.tracker, key, down);
            let (capturing, mods, mask) = {
                let tracker = state.tracker.lock().unwrap_or_else(|p| p.into_inner());
                (
                    tracker.capture_active(),
                    tracker.capture_modifiers(),
                    tracker.menu_mask_needed(key, down),
                )
            };
            if capturing && !repeated {
                let modifier = !matches!(key, Key::Other(_));
                let code = (!modifier).then(|| {
                    crate::portable_shortcuts::virtual_code(virtual_key)
                        .unwrap_or_else(|| "Unidentified".into())
                });
                let logical = code.as_deref().map(|code| base_key(info, code));
                capture_callback(
                    &state.capture,
                    serde_json::json!({"mods":mods,"code":code,"key":logical,"down":down,"modifier":modifier}),
                );
            }
            if mask {
                std::thread::spawn(send_mask_key);
            }
            if let Some(sender) = state.sender.lock().ok().and_then(|sender| sender.clone()) {
                for event in &routed.events {
                    // The first down reaches RegisterHotKey, which owns one-press dispatch and OS reservation.
                    if !matches!(event, HotkeyEvent::Action(1 | 2)) {
                        let _ = sender.send(*event);
                    }
                }
            }
            let registered_down = routed
                .events
                .iter()
                .any(|event| matches!(event, HotkeyEvent::Action(1 | 2)));
            if routed.withhold && !registered_down {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        shared()
            .tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .mouse_down();
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
impl Hook {
    pub fn start(settings: &Settings, sender: Sender<HotkeyEvent>) -> Result<Self, String> {
        *shared().sender.lock().unwrap() = Some(sender);
        let hook = Hook;
        hook.set_bindings(settings, false);
        let (ready, receive) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("pennel-hotkey".into())
            .spawn(move || {
                let module = unsafe { GetModuleHandleW(None) };
                let keyboard = module.and_then(|module| unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0)
                });
                let keyboard = match keyboard {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = ready.send(Err(format!("keyboard hook failed: {error}")));
                        return;
                    }
                };
                let mouse = unsafe { GetModuleHandleW(None) }.and_then(|module| unsafe {
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(module.into()), 0)
                });
                if let Err(error) = mouse {
                    unsafe {
                        let _ = UnhookWindowsHookEx(keyboard);
                    }
                    let _ = ready.send(Err(format!("mouse hook failed: {error}")));
                    return;
                }
                let _ = ready.send(Ok(()));
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        receive.recv().map_err(|error| error.to_string())??;
        Ok(hook)
    }
    pub fn set_bindings(&self, settings: &Settings, next_model: bool) {
        let plan = crate::portable_shortcuts::plan(
            settings,
            ShortcutPlatform::Windows,
            false,
            false,
            next_model,
        );
        let sender = shared()
            .sender
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut tracker = shared().tracker.lock().unwrap_or_else(|p| p.into_inner());
        let reset = tracker.is_active() && tracker.binding != plan.hold;
        tracker.set_binding(plan.hold);
        tracker.set_listening_bindings(plan.listening);
        publish(&tracker);
        drop(tracker);
        if reset && let Some(sender) = sender {
            let _ = sender.send(HotkeyEvent::Interrupt);
            let _ = sender.send(HotkeyEvent::Edge(KeyEdge::Up, super::now()));
        }
    }
    pub fn set_registered_bindings(&self, bindings: Vec<(u32, TriggerBinding)>) {
        shared()
            .tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_always_bindings(bindings);
    }
    pub fn set_listening(&self, listening: bool) -> Result<(), String> {
        shared()
            .tracker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_listening(listening);
        Ok(())
    }
    pub fn set_capture(
        &self,
        callback: Option<Box<dyn FnMut(serde_json::Value) + Send>>,
    ) -> Result<(), String> {
        let start = callback.is_some();
        if start {
            *shared().capture.lock().unwrap_or_else(|p| p.into_inner()) =
                callback.map(|callback| Arc::new(Mutex::new(callback)));
        }
        let mut tracker = shared().tracker.lock().unwrap_or_else(|p| p.into_inner());
        tracker.set_capture(start);
        publish(&tracker);
        drop(tracker);
        if !start {
            *shared().capture.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
        Ok(())
    }
}
