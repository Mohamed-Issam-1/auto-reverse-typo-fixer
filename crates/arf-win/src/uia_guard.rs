//! Conservative, background-only UIA focus gate.
//! No key events or user text enter this module. UIA cannot authorize browser
//! correction yet: the separate browser denylist remains mandatory.

use crate::uia::{FocusSample, FocusSafety};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_SAMPLE_AGE: Duration = Duration::from_millis(600);
const REFRESH_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinFocus {
    pub hwnd: isize,
    pub focus: isize,
    pub pid: u32,
}

#[derive(Clone, Copy)]
struct Cached {
    sample: FocusSample,
    win: WinFocus,
    epoch: u64,
    checked_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiaVerdict {
    pub epoch: u64,
    pub safety: FocusSafety,
}

impl UiaVerdict {
    pub fn allows_text(self) -> bool {
        self.safety == FocusSafety::NonPassword
    }
}

pub struct UiaGuard {
    epoch: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Cached>>>,
    stop: Arc<AtomicBool>,
}

impl UiaGuard {
    fn empty() -> Self {
        Self {
            epoch: Arc::new(AtomicU64::new(0)),
            latest: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts a dedicated MTA worker. Startup errors leave the gate Unknown.
    pub fn start() -> Self {
        let guard = Self::empty();
        #[cfg(windows)]
        {
            let epoch = Arc::clone(&guard.epoch);
            let latest = Arc::clone(&guard.latest);
            let stop = Arc::clone(&guard.stop);
            let spawned = std::thread::Builder::new()
                .name("arf-uia-safety".to_owned())
                .spawn(move || {
                    if let Err(error) = run_monitor(epoch, latest, stop) {
                        eprintln!("warning: UIA safety monitor unavailable: {error}; correction remains blocked");
                    }
                });
            if let Err(error) = spawned {
                eprintln!("warning: cannot spawn UIA safety monitor: {error}; correction remains blocked");
            }
        }
        guard
    }

    /// Cheap read on the keyboard worker. Fail closed for missing/stale data,
    /// process/window mismatch, or an event racing with this read.
    pub fn verdict(&self, current: WinFocus) -> UiaVerdict {
        let before = self.epoch.load(Ordering::Acquire);
        let cached = self.latest.lock().ok().and_then(|value| *value);
        let after = self.epoch.load(Ordering::Acquire);
        let safety = match cached {
            Some(entry)
                if current.pid > 0
                    && current.hwnd != 0
                    && before == after
                    && entry.epoch == after
                    && entry.win == current
                    && entry.sample.pid > 0
                    && entry.sample.pid as u32 == current.pid
                    && entry.checked_at.elapsed() <= MAX_SAMPLE_AGE =>
            {
                entry.sample.safety
            }
            _ => FocusSafety::Unknown,
        };
        UiaVerdict { epoch: after, safety }
    }
}

impl Drop for UiaGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(windows)]
fn read_win_focus() -> Option<WinFocus> {
    use crate::sys::{GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 { return None; }
        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(hwnd, &mut pid);
        if tid == 0 || pid == 0 { return None; }
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(tid, &mut info) == 0 || info.hwndFocus == 0 {
            return None; // The active edit control is unknown; fail closed.
        }
        Some(WinFocus { hwnd, focus: info.hwndFocus, pid })
    }
}

#[cfg(windows)]
fn run_monitor(
    epoch: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Cached>>>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    use crate::uia::{query_focused, sample_from_query};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM initialization failed: {e}"))?;
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _com_guard = ComGuard;

    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|e| format!("UI Automation creation failed: {e}"))?;

    let (sender, receiver) = mpsc::sync_channel(1);
    // The COM callback increments epoch BEFORE queuing an event, immediately
    // invalidating any previous NonPassword classification.
    let handler = crate::uia_events::make_guard_handler(sender, Arc::clone(&epoch));
    unsafe { automation.AddFocusChangedEventHandler(None, &handler) }
        .map_err(|e| format!("Cannot register focus handler: {e}"))?;

    let result = loop {
        if stop.load(Ordering::Acquire) {
            break Ok(());
        }
        let before = epoch.load(Ordering::Acquire);
        let before_win = read_win_focus();
        let sample = sample_from_query(query_focused());
        let after_win = read_win_focus();
        let after = epoch.load(Ordering::Acquire);
        // If UIA and Win32 focus disagree, or focus moved during the
        // query, never publish an authorizing result.
        let verified = match (before_win, after_win) {
            (Some(a), Some(b)) if before == after
                && a == b && sample.pid > 0 && sample.pid as u32 == b.pid => Some(b),
            _ => None,
        };
        match latest.lock() {
            Ok(mut state) => {
                *state = verified.map(|win| Cached {
                    sample, win, epoch: after, checked_at: Instant::now(),
                });
            }
            Err(_) => break Err("UIA safety state lock failed".to_owned()),
        }
        match receiver.recv_timeout(REFRESH_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err("UIA focus event channel closed".to_owned());
            }
        }
    };
    let cleanup = unsafe { automation.RemoveFocusChangedEventHandler(&handler) }
        .map_err(|e| format!("Cannot unregister focus handler: {e}"));
    result.and(cleanup)
}

/// Diagnostic only. No hooks, rewriting or input-content access.
#[cfg(windows)]
pub fn run_guard_watch() -> i32 {
    let guard = UiaGuard::start();
    println!("UIA safety gate watch: 20 seconds. Switch between ordinary/password fields.");
    println!("Gate=NonPassword is NOT permission to correct text inside browsers.");
    let until = Instant::now() + Duration::from_secs(20);
    let mut previous: Option<(u32, UiaVerdict)> = None;
    while Instant::now() < until {
        let current = read_win_focus();
        let state = guard.verdict(current.unwrap_or(WinFocus { hwnd: 0, focus: 0, pid: 0 }));
        let pid = current.map_or(0, |c| c.pid);
        if previous != Some((pid, state)) {
            println!("gate: pid={pid} epoch={} classification={:?}", state.epoch, state.safety);
            previous = Some((pid, state));
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    println!("UIA gate watch complete. No input content was accessed.");
    0
}

/// Read-only microbenchmark for the cached gate. Does not start UIA, hooks or injection.
/// This measures lookup cost, NOT full end-to-end typing latency.
#[cfg(windows)]
pub fn run_guard_bench() -> i32 {
    let guard = UiaGuard::empty();
    let focus = WinFocus { hwnd: 100, focus: 101, pid: 42 };
    *guard.latest.lock().unwrap() = Some(Cached {
        sample: FocusSample { pid: 42, safety: FocusSafety::NonPassword },
        win: focus,
        epoch: 0,
        checked_at: Instant::now(),
    });
    const N: u32 = 10_000;
    let started = Instant::now();
    let mut allowed = 0u32;
    for _ in 0..N {
        if guard.verdict(focus).allows_text() { allowed += 1; }
    }
    let elapsed = started.elapsed();
    guard.epoch.fetch_add(1, Ordering::AcqRel);
    let denied = (0..N).filter(|_| !guard.verdict(focus).allows_text()).count();
    println!("Cached gate: {N} checks in {:?}; allowed={allowed}", elapsed);
    println!("After invalidation: {denied}/{N} checks denied");
    println!("No input content, Win32 hooks, UIA services, or injection were accessed.");
    if denied != N as usize { 1 } else { 0 }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> WinFocus {
        WinFocus { hwnd: 100, focus: 101, pid: 42 }
    }

    fn record(guard: &UiaGuard, safety: FocusSafety, age: Duration) {
        *guard.latest.lock().unwrap() = Some(Cached {
            sample: FocusSample { pid: 42, safety },
            win: context(),
            epoch: guard.epoch.load(Ordering::Acquire),
            checked_at: Instant::now() - age,
        });
    }

    #[test]
    fn new_epoch_requires_new_matching_sample() {
        let guard = UiaGuard::empty();
        record(&guard, FocusSafety::NonPassword, Duration::ZERO);
        assert!(guard.verdict(context()).allows_text());
        guard.epoch.fetch_add(1, Ordering::AcqRel);
        assert!(!guard.verdict(context()).allows_text());
        record(&guard, FocusSafety::NonPassword, Duration::ZERO);
        assert!(guard.verdict(context()).allows_text());
    }

    #[test]
    fn unknown_process_or_missing_focus_never_authorizes() {
        let guard = UiaGuard::empty();
        record(&guard, FocusSafety::NonPassword, Duration::ZERO);
        assert!(!guard.verdict(WinFocus { pid: 0, ..context() }).allows_text());
        assert!(!guard.verdict(WinFocus { focus: 0, ..context() }).allows_text());
        assert!(!guard.verdict(WinFocus { hwnd: 0, ..context() }).allows_text());
    }
    #[test]
    fn fails_closed_until_matching_fresh_non_password_sample() {
        let guard = UiaGuard::empty();
        assert!(!guard.verdict(context()).allows_text());
        record(&guard, FocusSafety::NonPassword, Duration::ZERO);
        assert!(guard.verdict(context()).allows_text());
        assert!(!guard.verdict(WinFocus { pid: 41, ..context() }).allows_text());
        assert!(!guard.verdict(WinFocus { focus: 102, ..context() }).allows_text());
        assert!(!guard.verdict(WinFocus { hwnd: 102, ..context() }).allows_text());
        record(&guard, FocusSafety::Password, Duration::ZERO);
        assert!(!guard.verdict(context()).allows_text());
        record(&guard, FocusSafety::Unknown, Duration::ZERO);
        assert!(!guard.verdict(context()).allows_text());
    }

    #[test]
    fn focus_event_invalidates_old_authorization() {
        let guard = UiaGuard::empty();
        record(&guard, FocusSafety::NonPassword, Duration::ZERO);
        assert!(guard.verdict(context()).allows_text());
        guard.epoch.fetch_add(1, Ordering::AcqRel);
        assert_eq!(guard.verdict(context()).safety, FocusSafety::Unknown);
    }

    #[test]
    fn stale_samples_cannot_authorize_input() {
        let guard = UiaGuard::empty();
        record(&guard, FocusSafety::NonPassword, Duration::from_secs(2));
        assert_eq!(guard.verdict(context()).safety, FocusSafety::Unknown);
    }
}
