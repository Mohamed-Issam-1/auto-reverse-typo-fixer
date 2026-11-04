//! Low-level keyboard/mouse hooks. Callbacks only enqueue and return immediately:
//! Windows silently drops hooks whose callbacks take too long (LowLevelHooksTimeout).

use crate::sys::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

/// Marks events injected by this program ("ARFT").
pub const SIGNATURE: usize = 0x4152_4654;

/// Monotonic generation for physical key presses, mouse clicks, and foreign input.
/// It is updated in the low-level hook BEFORE the event is queued for the worker.
#[derive(Default)]
struct ActivityTracker {
    generation: AtomicU64,
}

impl ActivityTracker {
    fn note_input(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }

    fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn matches(&self, event_generation: u64) -> bool {
        event_generation != 0 && self.current() == event_generation
    }
}

static INPUT_ACTIVITY: ActivityTracker = ActivityTracker { generation: AtomicU64::new(0) };

/// Used by the worker to reject a rewrite if any newer real input arrived.
pub fn activity_is_current(event_generation: u64) -> bool {
    INPUT_ACTIVITY.matches(event_generation)
}

#[derive(Clone, Copy, Debug)]
pub enum RawEvent {
    Key { vk: u32, scan: u16, extended: bool, down: bool, activity: u64 },
    /// Input injected by another program: positions in the target app are no longer trustworthy.
    Foreign,
    Mouse,
}

impl RawEvent {
    /// A key press or click (something that changes the text or the caret).
    pub fn is_press(&self) -> bool {
        match self {
            RawEvent::Key { down, .. } => *down,
            _ => true,
        }
    }
}

static TX: OnceLock<Sender<RawEvent>> = OnceLock::new();

fn emit(ev: RawEvent) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(ev);
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION && lparam != 0 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if k.dwExtraInfo == SIGNATURE {
            // our own injection: ignore (prevents feedback loops)
        } else if k.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) != 0 {
            if down {
                INPUT_ACTIVITY.note_input();
                emit(RawEvent::Foreign);
            }
        } else {
            let activity = if down { INPUT_ACTIVITY.note_input() } else { INPUT_ACTIVITY.current() };
            emit(RawEvent::Key {
                vk: k.vkCode,
                scan: k.scanCode as u16,
                extended: k.flags & LLKHF_EXTENDED != 0,
                down,
                activity,
            });
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION
        && matches!(wparam as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN)
    {
        INPUT_ACTIVITY.note_input();
        emit(RawEvent::Mouse);
    }
    CallNextHookEx(0, code, wparam, lparam)
}

pub struct Hooks {
    kb: HHOOK,
    mouse: HHOOK,
}

impl Hooks {
    pub fn install(tx: Sender<RawEvent>) -> Result<Hooks, String> {
        TX.set(tx).map_err(|_| "hooks already installed".to_string())?;
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            if kb == 0 {
                return Err("SetWindowsHookExW(WH_KEYBOARD_LL) failed".into());
            }
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            if mouse == 0 {
                UnhookWindowsHookEx(kb);
                return Err("SetWindowsHookExW(WH_MOUSE_LL) failed".into());
            }
            Ok(Hooks { kb, mouse })
        }
    }

    /// Runs the message loop that low-level hooks require. Returns when `WM_QUIT` arrives.
    pub fn run_message_loop(&self) {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.kb);
            UnhookWindowsHookEx(self.mouse);
        }
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn newer_physical_input_invalidates_replacement_token() {
        let clock = ActivityTracker::default();
        assert!(!clock.matches(0));
        let space = clock.note_input();
        assert!(clock.matches(space));
        clock.note_input(); // another key or click while a rewrite is planned
        assert!(!clock.matches(space));
    }

    #[test]
    fn key_up_does_not_invalidate_press_token() {
        let clock = ActivityTracker::default();
        let press = clock.note_input();
        // A key-up event reads current(), without advancing the generation.
        assert_eq!(clock.current(), press);
        assert!(clock.matches(press));
    }
}
