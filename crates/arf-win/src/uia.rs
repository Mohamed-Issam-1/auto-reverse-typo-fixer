//! UI Automation focus diagnostics (no keyboard hook, no text collection).
//! This does NOT unlock browser correction; browsers remain fail-closed in focus.rs.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusSafety {
    Password,
    NonPassword,
    Unknown,
}

/// Fail closed if either focus or the IsPassword property cannot be confirmed.
pub fn classify(is_password: Option<bool>, has_focus: Option<bool>) -> FocusSafety {
    match (is_password, has_focus) {
        (Some(true), Some(true)) => FocusSafety::Password,
        (Some(false), Some(true)) => FocusSafety::NonPassword,
        _ => FocusSafety::Unknown,
    }
}

#[cfg(windows)]
fn query_focused() -> Result<(i32, FocusSafety), String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    // This function is called on its own MTA thread, never the hook or UI thread.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM initialization failed: {e}"))?;

    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            // Balanced with the successful CoInitializeEx above, on this same thread.
            unsafe { CoUninitialize() };
        }
    }
    let _com_guard = ComGuard;

    // Both COM interface objects are scoped to this MTA thread.
    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|e| format!("UI Automation initialization failed: {e}"))?;

    let element = unsafe { automation.GetFocusedElement() }
        .map_err(|e| format!("GetFocusedElement failed: {e}"))?;

    // Do NOT query Value, TextPattern, Name, or other user content.
    let pid = unsafe { element.CurrentProcessId() }.unwrap_or_default();
    let password = unsafe { element.CurrentIsPassword() }.ok().map(|v| v.as_bool());
    let focused = unsafe { element.CurrentHasKeyboardFocus() }.ok().map(|v| v.as_bool());

    Ok((pid, classify(password, focused)))
}

/// One-time, read-only probe. No key interception or text injection is enabled.
pub fn run_diagnostic() -> i32 {
    println!("UIA diagnostic only: focus the test field within 8 seconds.");
    let worker = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(8));
        #[cfg(windows)]
        {
            query_focused()
        }
        #[cfg(not(windows))]
        {
            Err::<(i32, FocusSafety), String>("Windows is required".into())
        }
    });
    match worker.join() {
        Ok(Ok((pid, classification))) => {
            println!("Focused process ID: {pid}");
            println!("Focus classification: {classification:?}");
            println!("No text was read or stored.");
            0
        }
        Ok(Err(err)) => {
            eprintln!("UIA diagnostic: {err}");
            1
        }
        Err(_) => {
            eprintln!("UIA diagnostic thread failed");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classified_focused_fields() {
        assert_eq!(classify(Some(true), Some(true)), FocusSafety::Password);
        assert_eq!(classify(Some(false), Some(true)), FocusSafety::NonPassword);
    }

    #[test]
    fn uncertain_state_blocks_correction() {
        assert_eq!(classify(Some(false), Some(false)), FocusSafety::Unknown);
        assert_eq!(classify(Some(true), None), FocusSafety::Unknown);
        assert_eq!(classify(None, Some(true)), FocusSafety::Unknown);
        assert_eq!(classify(None, None), FocusSafety::Unknown);
    }
}
