//! Command-line entry point (console build; tray/overlay/settings UI come later).

use crate::app::App;
use crate::hook::Hooks;
use crate::inject::{self, Mode};
use crate::overlay::Overlay;
use crate::layout_win;
use crate::uia;
use arf_core::config::{self, AppConfig};
use arf_core::dict::{Dictionary, FstDictionary};
use arf_core::layouts::LayoutRegistry;
use arf_core::seed::seed_validator;
use arf_core::validator::Validator;
use arf_core::{Engine, Key, KeyboardLayout, Lang};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

const USAGE: &str = "arf-win [--config <file>] [--dict-dir <dir>] [--selftest] [--inject-test] [--uia-selftest]\n\
  --selftest      print detected layouts and how the keys for \"hgsghl\" translate under each\n\
  --inject-test   after 5 s types test lines into the focused window (use it to diagnose an app)";

fn load_fst(path: &Path) -> Option<Box<dyn Dictionary>> {
    let bytes = std::fs::read(path).ok()?;
    FstDictionary::from_bytes(bytes).ok().map(|d| Box::new(d) as Box<dyn Dictionary>)
}

fn build_validator(dir: &Path) -> Validator {
    match (load_fst(&dir.join("ar.fst")), load_fst(&dir.join("en.fst"))) {
        (Some(ar), Some(en)) => Validator::new(ar, en, true),
        _ => {
            eprintln!(
                "warning: {}\\ar.fst / en.fst not found - using the tiny built-in seed lists (demo only)",
                dir.display()
            );
            seed_validator(true)
        }
    }
}

/// Types the same Arabic word through different injection paths so a problematic application
/// (e.g. Notepad) can be diagnosed. Focus the target window within 5 seconds.
pub fn inject_test() {
    println!("Click into the target app (e.g. Notepad) now. Typing starts in 5 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(5));
    let ar = "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}"; // السلام
    let pause = || std::thread::sleep(std::time::Duration::from_millis(250));
    let lines: [(&str, Mode, bool); 5] = [
        ("A batch   : ", Mode::Batch, false),
        ("B paced   : ", Mode::Paced, false),
        ("C ascii   : hello", Mode::Batch, true),
        ("D replace batch (xxxxxx -> word): ", Mode::Batch, false),
        ("E replace paced (xxxxxx -> word): ", Mode::Paced, false),
    ];
    for (i, (label, mode, ascii_only)) in lines.iter().enumerate() {
        let _ = inject::send_text(label, *mode);
        pause();
        if *ascii_only {
            // label already contains the word
        } else if i < 3 {
            let _ = inject::send_text(ar, *mode);
        } else {
            let _ = inject::send_text("xxxxxx", *mode);
            pause();
            let _ = inject::send_replacement(6, ar, *mode);
        }
        pause();
        let _ = inject::send_enter();
        pause();
    }
    println!("Done. Look at the five lines in the target app and tell me which ones show the Arabic word.");
}

pub fn selftest() {
    let layouts = layout_win::installed_layouts();
    println!("installed layouts: {}", layouts.len());
    // physical keys H G S G H L on a US keyboard
    let scans = [0x23u16, 0x22, 0x1F, 0x22, 0x23, 0x26];
    for hkl in layouts {
        match layout_win::WinLayout::new(hkl) {
            Some(l) => {
                let text: String = scans
                    .iter()
                    .map(|s| l.translate(Key { scan: *s, ..Key::default() }).unwrap_or_else(|| "?".into()))
                    .collect();
                println!("  {hkl:#010x} {:?}: keys H G S G H L -> {text}", l.lang());
            }
            None => println!("  {hkl:#010x} (not Arabic/English, ignored)"),
        }
    }
}

/// Looks for a `data` folder with the dictionaries: next to the executable, in the current
/// directory, or two levels above the executable (running from `target\\release`).
fn find_dict_dir() -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(d) = &exe_dir {
        candidates.push(d.join("data"));
        if let Some(up2) = d.parent().and_then(Path::parent) {
            candidates.push(up2.join("data"));
        }
    }
    candidates.push(PathBuf::from("data"));
    candidates
        .iter()
        .find(|c| c.join("ar.fst").exists() && c.join("en.fst").exists())
        .cloned()
        .unwrap_or_else(|| candidates.into_iter().next().unwrap_or_else(|| PathBuf::from("data")))
}

pub fn run() -> i32 {
    let mut config_path: Option<PathBuf> = std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join("AutoReverseTypoFixer").join("config.toml"));
    let mut dict_dir: PathBuf = find_dict_dir();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--uia-selftest" => {
                return uia::run_diagnostic();
            }
            "--selftest" => {
                selftest();
                return 0;
            }
            "--inject-test" => {
                inject_test();
                return 0;
            }
            "--config" => config_path = args.next().map(PathBuf::from),
            "--dict-dir" => dict_dir = args.next().map(PathBuf::from).unwrap_or(dict_dir),
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }

    let cfg: AppConfig = match config_path.as_ref().filter(|p| p.exists()) {
        Some(p) => match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| config::parse(&t)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("invalid config {}: {e}", p.display());
                return 1;
            }
        },
        None => AppConfig::default(),
    };

    let mut registry = LayoutRegistry::new();
    layout_win::register_installed(&mut registry);
    for lang in [Lang::Arabic, Lang::English] {
        if registry.preferred(lang).is_none() {
            eprintln!("warning: no installed {lang:?} keyboard layout found; corrections to it are impossible");
        }
    }

    let engine = Engine::new(cfg.engine.clone(), build_validator(&dict_dir), registry);
    let (tx, rx) = mpsc::channel();
    let hooks = match Hooks::install(tx) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let overlay = if cfg.overlay { Overlay::spawn() } else { None };
    if cfg.overlay && overlay.is_none() {
        eprintln!("warning: could not create the overlay window; markers disabled");
    }
    std::thread::spawn(move || App::new(engine, cfg, rx, overlay).run());
    println!("Dictionaries: {}", dict_dir.display());
    println!("Auto-Reverse Typo Fixer running. Press Ctrl+C to quit.");
    hooks.run_message_loop();
    0
}
