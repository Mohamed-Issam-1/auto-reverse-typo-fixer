//! Foreground/focus information: layout, process, password-field detection.

use crate::sys::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct FocusCtx {
    pub hwnd: HWND,
    pub focus: HWND,
    pub pid: u32,
    pub hkl: HKL,
    pub is_password: bool,
    /// Lower-case executable name (empty if it could not be queried).
    pub process: String,
}

#[derive(Default)]
pub struct FocusMonitor {
    procs: HashMap<u32, String>,
    password_cache: Option<(HWND, bool)>,
}

impl FocusMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&mut self) -> Option<FocusCtx> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == 0 {
                return None;
            }
            let mut pid = 0u32;
            let tid = GetWindowThreadProcessId(hwnd, &mut pid);
            let mut gti: GUITHREADINFO = std::mem::zeroed();
            gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            let focus = if GetGUIThreadInfo(tid, &mut gti) != 0 && gti.hwndFocus != 0 { gti.hwndFocus } else { hwnd };
            let hkl = GetKeyboardLayout(tid);
            let is_password = self.is_password(focus);
            let process = self.process_name(pid);
            Some(FocusCtx { hwnd, focus, pid, hkl, is_password, process })
        }
    }

    /// Classic Win32 edit controls only. Browsers/Electron/UWP need UI Automation (`IsPassword`),
    /// which is not implemented yet.
    fn is_password(&mut self, focus: HWND) -> bool {
        if let Some((h, v)) = self.password_cache {
            if h == focus {
                return v;
            }
        }
        let v = unsafe {
            let mut buf = [0u16; 32];
            let n = GetClassNameW(focus, buf.as_mut_ptr(), buf.len() as i32);
            let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            class.eq_ignore_ascii_case("Edit") && GetWindowLongW(focus, GWL_STYLE) & ES_PASSWORD != 0
        };
        self.password_cache = Some((focus, v));
        v
    }

    fn process_name(&mut self, pid: u32) -> String {
        if let Some(n) = self.procs.get(&pid) {
            return n.clone();
        }
        let name = unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h == 0 {
                String::new()
            } else {
                let mut buf = [0u16; 520];
                let mut len = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
                CloseHandle(h);
                if ok != 0 {
                    let path = String::from_utf16_lossy(&buf[..len as usize]);
                    path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase()
                } else {
                    String::new()
                }
            }
        };
        if self.procs.len() > 256 {
            self.procs.clear();
        }
        self.procs.insert(pid, name.clone());
        name
    }
}
