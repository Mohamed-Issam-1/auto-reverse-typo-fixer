# Auto-Reverse Typo Fixer

Offline, open-source Windows utility that detects text typed with the wrong keyboard layout
(Arabic <-> English) and fixes it automatically, then switches the input language.
**Status: working prototype (v0.1.0), not release-ready.** See `docs/PRD.md` for the specification and
`docs/STATUS_REPORT.ar.md` (Arabic) for the detailed status, test results and known problems.

## Layout

| Crate | Role | Status |
|---|---|---|
| `arf-core` | Platform-independent engine: key buffer, converter, validator (dictionary + affixes + n-grams), decision/run logic, undo, config | Implemented, 45 unit tests passing |
| `arf-dictc` | Compiles word lists into FST dictionaries and evaluates false positives / recall | Implemented |
| `arf-win` | Windows layer: low-level hooks, `ToUnicodeEx` layouts, `SendInput`, layout switching, focus/password checks | Runs on Windows (core flows verified manually); several features still unverified, see the status report |

## Build and run (Windows)

    cargo build --release -p arf-win
    .\target\release\arf-win.exe --selftest       # layouts detected + key translation check
    .\target\release\arf-win.exe                  # run (Ctrl+C to quit)
    .\target\release\arf-win.exe --inject-test    # diagnose how an app (e.g. Notepad) handles injected text

Dictionaries live in `data\ar.fst` and `data\en.fst` (found automatically next to the exe, two levels
above it, or in `.\data`). They are compiled from FrequencyWords (CC-BY-SA-4.0, see `data\LICENSE-DATA.md`);
rebuild with `tools\build-dicts.ps1`, which also prints a false-positive / recall evaluation.

Tests (any OS): `cargo test -p arf-core`

## Behaviour notes

* Undo (default `Ctrl+Alt+Z`) restores the original text and layout, protects the undone words for the
  rest of the session, and pauses auto-correction according to `after_undo` (default: until the current
  typing run ends). Config example: `config.example.toml`.
* The marker is a small click-through toast near the caret (or the bottom of the window when the app
  does not expose a caret). It does not select text, so it can never be overwritten by your next key.

## Known gaps (next steps)

1. Notepad: corrected text disappears (see `--inject-test`); cause still being diagnosed.
2. Marker is a toast, not a highlight over the corrected words (needs UI Automation text rectangles).
3. Tier B (n-gram) thresholds are uncalibrated; default mode ignores Tier B.
4. Tray, settings UI, UI Automation password detection (browsers/Electron).
5. Undo hotkey is not swallowed by the hook (Ctrl+Alt+Z also reaches the app).
6. Keys typed during an injection abort the correction (safe) instead of hold-and-replay.
7. Elevation (scheduled task / uiAccess), code signing, installer.

## License

Source code: MIT (`LICENSE`). Dictionary data in `data/`: CC-BY-SA-4.0 (`data/LICENSE-DATA.md`).
