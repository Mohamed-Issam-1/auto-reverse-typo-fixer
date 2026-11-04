//! Text injection and input-language switching.

use crate::hook::SIGNATURE;
use crate::sys::*;

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        type_: INPUT_KEYBOARD,
        u: INPUT_UNION { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: SIGNATURE } },
    }
}

const SCAN_BACKSPACE: u16 = 0x0E;
const SCAN_ENTER: u16 = 0x1C;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Everything in ONE `SendInput` call (fastest, atomic).
    Batch,
    /// Backspaces first, a short pause, then one character at a time with tiny pauses.
    /// For applications that mishandle large bursts of synthetic input.
    Paced,
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!("SendInput accepted {sent} of {} events (blocked by UIPI or another hook?)", inputs.len()))
    }
}

fn backspaces_inputs(n: usize) -> Vec<INPUT> {
    let mut v = Vec::with_capacity(n * 2);
    for _ in 0..n {
        v.push(key_input(VK_BACK, SCAN_BACKSPACE, 0));
        v.push(key_input(VK_BACK, SCAN_BACKSPACE, KEYEVENTF_KEYUP));
    }
    v
}

fn char_inputs(unit: u16) -> [INPUT; 2] {
    [key_input(0, unit, KEYEVENTF_UNICODE), key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]
}

/// Deletes `backspaces` characters and types `text`. Output uses `KEYEVENTF_UNICODE`, so it does
/// not depend on the layout that is active at that moment.
pub fn send_replacement(backspaces: usize, text: &str, mode: Mode) -> Result<(), String> {
    send_replacement_checked(backspaces, text, mode, || true)
}

/// Additional caller-supplied focus gate for each SendInput call.
/// This is best-effort; focus may still change after a check, and paced
/// injection can be partially applied if authorization disappears.
pub fn send_replacement_checked(
    backspaces: usize,
    text: &str,
    mode: Mode,
    mut target_valid: impl FnMut() -> bool,
) -> Result<(), String> {
    match mode {
        Mode::Batch => {
            let mut inputs = backspaces_inputs(backspaces);
            for unit in text.encode_utf16() {
                inputs.extend(char_inputs(unit));
            }
            if !target_valid() { return Err("Focus safety changed during injection".to_owned()); }
            send(&inputs)
        }
        Mode::Paced => {
            if !target_valid() { return Err("Focus safety changed during injection".to_owned()); }
            send(&backspaces_inputs(backspaces))?;
            std::thread::sleep(std::time::Duration::from_millis(8));
            for unit in text.encode_utf16() {
                if !target_valid() { return Err("Focus safety changed during injection".to_owned()); }
                send(&char_inputs(unit))?;
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
            Ok(())
        }
    }
}

/// Types text only (no deletion). Used by `--inject-test`.
pub fn send_text(text: &str, mode: Mode) -> Result<(), String> {
    send_replacement(0, text, mode)
}

pub fn send_enter() -> Result<(), String> {
    send(&[key_input(VK_RETURN, SCAN_ENTER, 0), key_input(VK_RETURN, SCAN_ENTER, KEYEVENTF_KEYUP)])
}

/// Asks the foreground application to switch its input language.
/// (`ActivateKeyboardLayout` would only affect this process's own thread.)
/// Do not post a layout change to another foreground window after a correction delay.
/// This narrows, but cannot eliminate, the focus-change race at the Win32 boundary.
pub fn request_layout_if_foreground(expected_hwnd: HWND, hkl: HKL) -> bool {
    unsafe {
        expected_hwnd != 0
            && GetForegroundWindow() == expected_hwnd
            && PostMessageW(expected_hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl) != 0
    }
}
pub fn request_layout(hkl: HKL) {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd != 0 {
            PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl);
        }
    }
}

/// Waits (bounded) until Shift/Ctrl/Alt/Win are physically released, so injected text is
/// not interpreted as a shortcut (e.g. Ctrl+Alt+Backspace).
pub fn wait_modifiers_released(timeout_ms: u64) {
    let start = std::time::Instant::now();
    let down = |vk: i32| unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 };
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if !(down(VK_SHIFT) || down(VK_CONTROL) || down(VK_MENU) || down(VK_LWIN) || down(VK_RWIN)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn unsafe_target_blocks_both_injection_modes_before_any_send() {
        for mode in [Mode::Batch, Mode::Paced] {
            assert!(send_replacement_checked(3, "abc", mode, || false).is_err());
        }
    }
}
pub fn caps_lock_on() -> bool {
    unsafe { GetKeyState(VK_CAPITAL) & 1 != 0 }
}
