//! Low-level keyboard hook (WH_KEYBOARD_LL): sees every key system-wide without needing focus
//! and without admin rights. Elevated windows do not deliver keys to a non-elevated hook.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use pennel_core::HotkeyTrigger;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLKHF_UP, MSG, SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL,
};

use super::{HotkeyEvent, Key, TriggerTracker, dispatch};
use crate::gesture::KeyEdge;

struct Shared {
    tracker: Mutex<TriggerTracker>,
    sender: Mutex<Option<Sender<HotkeyEvent>>>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        tracker: Mutex::new(TriggerTracker::default()),
        sender: Mutex::new(None),
    })
}

pub struct Hook;

/// An unassigned virtual key: tapping it while Win is down stops the Start menu from opening on release.
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
        // Generic Ctrl/Alt/Shift, Caps/Num/Scroll Lock and the mask key itself.
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
    let inputs = [input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.flags.0 & LLKHF_INJECTED.0 == 0 {
            let down = info.flags.0 & LLKHF_UP.0 == 0;
            let key = key_for(info.vkCode);
            let shared = shared();
            if let Some(sender) = shared.sender.lock().ok().and_then(|s| s.clone()) {
                let events = dispatch(&shared.tracker, &sender, key, down);
                let chord = shared
                    .tracker
                    .lock()
                    .is_ok_and(|t| t.trigger == Some(HotkeyTrigger::ControlSuper));
                let engaged = events
                    .iter()
                    .any(|e| matches!(e, HotkeyEvent::Edge(KeyEdge::Down, _)));
                if chord && engaged {
                    std::thread::spawn(send_mask_key);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

impl Hook {
    pub fn start(trigger: HotkeyTrigger, sender: Sender<HotkeyEvent>) -> Result<Self, String> {
        let shared = shared();
        *shared.tracker.lock().unwrap() = TriggerTracker::new(trigger);
        *shared.sender.lock().unwrap() = Some(sender);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("pennel-hotkey".into())
            .spawn(move || {
                let module = unsafe { GetModuleHandleW(None) };
                let hook = module.and_then(|module| unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0)
                });
                if let Err(e) = hook {
                    let _ = ready_tx.send(Err(format!("keyboard hook failed: {e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())?.map(|()| Hook)
    }

    pub fn set_trigger(&self, trigger: HotkeyTrigger) {
        if let Ok(mut tracker) = shared().tracker.lock() {
            tracker.set_trigger(trigger);
        }
    }
}
