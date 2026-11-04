//! Worker thread: turns raw hook events into engine calls and applies corrections.

use crate::focus::{is_sensitive_host, FocusCtx, FocusMonitor};
use crate::hook::RawEvent;
use crate::inject::{self, Mode};
use crate::overlay::Overlay;
use crate::layout_win;
use crate::sys::HWND;
use arf_core::config::AppConfig;
use arf_core::types::scan;
use arf_core::{Engine, Key, UndoPause};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Mods {
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
    caps: bool,
}

impl Mods {
    /// Returns `true` if `vk` was a modifier key (and updates the state).
    fn update(&mut self, vk: u32, down: bool) -> bool {
        match vk {
            0x10 | 0xA0 | 0xA1 => self.shift = down,
            0x11 | 0xA2 | 0xA3 => self.ctrl = down,
            0x12 | 0xA4 | 0xA5 => self.alt = down,
            0x5B | 0x5C => self.win = down,
            0x14 => {
                if down {
                    self.caps = !self.caps;
                }
            }
            _ => return false,
        }
        true
    }
}

pub struct App {
    engine: Engine,
    cfg: AppConfig,
    rx: Receiver<RawEvent>,
    pending: VecDeque<RawEvent>,
    mods: Mods,
    focus: FocusMonitor,
    last_focus: (HWND, HWND),
    /// Layout we asked the OS to switch to, and when. Verified on a later key press.
    expected: Option<(isize, u64)>,
    overlay: Option<Overlay>,
    start: Instant,
}

impl App {
    pub fn new(engine: Engine, cfg: AppConfig, rx: Receiver<RawEvent>, overlay: Option<Overlay>) -> Self {
        let mods = Mods { caps: inject::caps_lock_on(), ..Mods::default() };
        App {
            engine,
            cfg,
            rx,
            pending: VecDeque::new(),
            mods,
            focus: FocusMonitor::new(),
            last_focus: (0, 0),
            expected: None,
            overlay,
            start: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn run(mut self) {
        loop {
            let ev = match self.pending.pop_front() {
                Some(e) => e,
                None => match self.rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(e) => e,
                    Err(RecvTimeoutError::Timeout) => {
                        let now = self.now();
                        self.engine.tick(now);
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                },
            };
            self.handle(ev);
        }
    }

    fn handle(&mut self, ev: RawEvent) {
        match ev {
            RawEvent::Mouse | RawEvent::Foreign => self.engine.on_break(),
            RawEvent::Key { vk, scan, extended, down } => self.handle_key(vk, scan, extended, down),
        }
    }

    fn undo_hotkey_matches(&self, vk: u32) -> bool {
        let h = &self.cfg.undo_hotkey;
        h.vk == vk && h.ctrl == self.mods.ctrl && h.alt == self.mods.alt && h.shift == self.mods.shift && h.win == self.mods.win
    }

    fn handle_key(&mut self, vk: u32, sc: u16, extended: bool, down: bool) {
        if self.mods.update(vk, down) || !down {
            return;
        }
        let now = self.now();

        // Check context before Undo: never rewrite content in a different window,
        // a password field, or a browser whose focus safety is not yet verified.
        let Some(ctx) = self.focus.current() else {
            self.engine.on_break();
            return;
        };
        if (ctx.hwnd, ctx.focus) != self.last_focus {
            self.last_focus = (ctx.hwnd, ctx.focus);
            self.engine.on_break();
        }
        if !self.cfg.enabled
            || ctx.is_password
            || is_sensitive_host(&ctx.process)
            || self.cfg.blacklist.contains(&ctx.process)
        {
            self.engine.on_break();
            return;
        }

        if self.undo_hotkey_matches(vk) && self.engine.can_undo(now) {
            self.do_undo(now);
            return;
        }
        if self.mods.ctrl || self.mods.alt || self.mods.win {
            self.engine.on_break(); // shortcuts: caret/text may change in ways we cannot track
            return;
        }
        layout_win::register(self.engine.layouts_mut(), ctx.hkl);
        self.verify_switch(&ctx, now);

        if extended {
            self.engine.on_break(); // arrows, Home/End, Delete, numpad Enter, ...
            return;
        }
        match sc {
            scan::SPACE => self.on_space(now, &ctx.process),
            scan::BACKSPACE => self.engine.on_backspace(now),
            s if scan::is_typing_key(s) => {
                let key = Key { scan: s, shift: self.mods.shift, caps: self.mods.caps };
                self.engine.on_key(key, ctx.hkl as u64, now);
            }
            _ => self.engine.on_break(), // Enter, Tab, Esc, F-keys, numpad, ...
        }
    }

    /// A later key press tells us whether the OS honored our layout-switch request.
    fn verify_switch(&mut self, ctx: &FocusCtx, now: u64) {
        if let Some((expected, at)) = self.expected {
            if now.saturating_sub(at) >= 150 {
                self.expected = None;
                if ctx.hkl != expected {
                    self.engine.report_layout_switch_failed();
                }
            }
        }
    }

    fn mode_for(&self, process: &str) -> Mode {
        if self.cfg.paced_injection || self.cfg.paced_apps.iter().any(|p| p == process) {
            Mode::Paced
        } else {
            Mode::Batch
        }
    }

    fn toast(&self, text: &str) {
        if self.cfg.overlay {
            if let Some(o) = &self.overlay {
                o.show(text);
            }
        }
    }

    /// Asks the OS to switch layouts, after a short delay so the injected text is consumed first.
    fn switch_layout(&mut self, hkl: u64, now: u64) {
        if self.cfg.switch_delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.cfg.switch_delay_ms));
        }
        inject::request_layout(hkl as isize);
        self.expected = Some((hkl as isize, now));
    }

    fn on_space(&mut self, now: u64, process: &str) {
        let Some(c) = self.engine.on_space(now) else { return };

        // If anything else was pressed since the Space, the text in the application is no
        // longer what we computed: abort instead of deleting the wrong characters.
        while let Ok(ev) = self.rx.try_recv() {
            self.pending.push_back(ev);
        }
        if self.pending.iter().any(RawEvent::is_press) {
            return;
        }
        let mode = self.mode_for(process);
        if inject::send_replacement(c.backspaces, &c.text, mode).is_err() {
            return;
        }
        self.engine.commit(&c, now);
        self.switch_layout(c.target_layout, now);
        let msg = format!("تم التصحيح  ·  للتراجع: {}", self.cfg.undo_hotkey.display());
        self.toast(&msg);
    }

    fn pause_description(&self) -> String {
        match self.cfg.engine.undo_pause {
            UndoPause::None => "تم التراجع".to_string(),
            UndoPause::Run => "تم التراجع  ·  التصحيح متوقف حتى نهاية الكتابة الحالية".to_string(),
            UndoPause::Words(n) => format!("تم التراجع  ·  التصحيح متوقف للكلمات {n} التالية"),
            UndoPause::Seconds(s) => format!("تم التراجع  ·  التصحيح متوقف {s} ثانية"),
        }
    }

    fn do_undo(&mut self, now: u64) {
        let process = self.focus.current().map(|c| c.process).unwrap_or_default();
        let Some(plan) = self.engine.undo(now) else { return };
        inject::wait_modifiers_released(500);
        if inject::send_replacement(plan.backspaces, &plan.text, self.mode_for(&process)).is_err() {
            return;
        }
        self.switch_layout(plan.restore_layout, now);
        let msg = self.pause_description();
        self.toast(&msg);
    }
}
