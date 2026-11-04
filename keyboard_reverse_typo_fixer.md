# Product Requirements Document (PRD) & Prompt
## Project Title: Auto-Reverse Typo Fixer (Windows Keyboard Layout Auto-Corrector)
**Version:** 2.0 — Windows-first, revised after design review

---

## 1. Executive Summary & Objective

**Auto-Reverse Typo Fixer** is a lightweight, privacy-first, open-source background utility for Windows that automatically detects and corrects bidirectional keyboard-layout mismatch typos in real time, then switches the active input language so the user can keep typing without interruption.

| Typed (wrong layout) | Intended | Direction |
|---|---|---|
| `hgsghl` | `السلام` | English keys → Arabic |
| `ugd;l` | `عليكم` | English keys → Arabic |
| `اثممخ` | `hello` | Arabic keys → English |
| `زةي` | `.md` | Arabic keys → English (word starting with a symbol) |

**Objectives**
1. Remove the need to delete and retype text typed in the wrong layout.
2. Keep workflow continuity by auto-switching the OS input language.
3. Be conservative: a wrong correction costs more than a missed one.

### 1.1. Scope Decisions
* **Platform (v1):** Windows 10 (1809+) and Windows 11 only. macOS and Linux are out of scope for v1.
* **Language pair (v1):** Arabic ⇄ English. The architecture must treat a "language pair" as a pluggable unit so other pairs can be added later.
* **Open source:** The full source code is public. A permissive license (MIT or Apache-2.0) is to be chosen; dictionary licenses must be verified for compatibility.
* **Non-goals (v1):** cloud services, LLMs, telemetry, spell-checking or grammar correction, CJK/IME languages, clipboard handling.

### 1.2. Key Changes from v1 of this PRD
* Word boundary is **Space only** (unified for both languages).
* Conversion is based on **physical keys (scan codes + modifier state)**, never on output characters.
* Added **multi-word (run-based) correction**.
* Corrected words are marked with a **non-destructive visual overlay** (not by selecting text).
* Language switching uses `WM_INPUTLANGCHANGEREQUEST` (not a bare `ActivateKeyboardLayout`).
* Dictionaries use an **FST** (finite-state transducer) instead of a raw `HashSet` to respect the memory budget.
* Added password-field detection, elevation strategy, anti-loop rules, metrics, and a test plan.
* Implementation language chosen: **Rust**.

---

## 2. Core Features & Functional Requirements

### 2.1. Bidirectional Auto-Correction
* **English-to-Arabic:** text typed with the English layout active that matches an intended Arabic word is converted.
* **Arabic-to-English:** text typed with the Arabic layout active that matches an intended English word is converted.
* **Fully automatic:** no hotkey or button is required to trigger a correction.
* **Preservation:** surrounding whitespace, capitalization (Shift/CapsLock state), punctuation, digits, and formatting are preserved.

### 2.2. Word Boundary
* The **Space** key is the single, unified delimiter for both languages.
* Characters such as `; , . [ ] '` are **never** treated as delimiters, because in the Arabic layout they produce letters (`ك و ز ج د ط`).
* Optional, **off by default**: Enter and Tab as additional triggers (setting).

### 2.3. Key-Based Conversion (Core Design Rule)
* The buffer stores **keystrokes** — `(scan code, Shift state, CapsLock state, source layout HKL)` — not the characters that appeared on screen.
* Reason: different physical keys can produce the **same character** in one layout while producing **different characters** in the other (for example, Shift-modified keys that yield the same `<` / `>` symbols in Arabic but different symbols in English). Inverting from output characters is therefore ambiguous; inverting from keys is not.
* The target string is produced by translating the same keys through the **target layout** using the OS keyboard API (`MapVirtualKeyEx` + `ToUnicodeEx` with the "do not change keyboard state" flag), so no layout table is hard-coded. Arabic 101, other Arabic variants, and custom layouts work as long as they are installed.
* **Layout identification:** automatic from the installed layouts (`GetKeyboardLayoutList`). If more than one candidate exists for a language, the user is asked once in settings.
* **Current layout** is read at every word via `GetKeyboardLayout(threadId)` for the focused window's thread — never assumed. Fallback logic is required for apps where this is unreliable (UWP frame host, console windows).
* **CapsLock / Shift:** their state at key time is captured and applied to the target layout, so English output keeps correct capitalization and Arabic output keeps layout-specific shifted characters.
* **Digits** are layout-neutral: they pass through unchanged and are excluded from dictionary lookup.
* **Output length ≠ key count:** some keys produce several characters (e.g. the Arabic `لا` key) and Shift can produce combining marks. The number of Backspaces is computed from the **actual characters inserted**, not from the number of keys.

### 2.4. Local Detection & Validation Engine
Fully offline. A word is corrected only when **all** of the following hold:

1. The word is not excluded by a **pre-filter** (see 2.4.1).
2. The **converted** word is valid in the target language.
3. The word **as typed** is *not* valid in the current layout's language (if both are valid, the as-typed word wins).
4. The word is not in the user dictionary / ignore list.
5. Confidence meets the threshold for the selected sensitivity.

#### 2.4.1. Pre-filters (skip the word)
* Password fields, blacklisted apps, paused states.
* Tokens that look like URLs, e-mails, file paths, commands/flags, camelCase / snake_case identifiers, hex strings.
* Tokens made only of digits/symbols.
* Words shorter than the minimum length (default **3 letters**), unless corrected through run context (2.5).

#### 2.4.2. Symbols Inside Words
Symbols can overlap with Arabic letters, so they are **not** blindly excluded. Instead:
* The whole token is converted by key mapping first.
* Validation runs on the converted token **and** on its "core" (leading/trailing non-letters stripped).
* A built-in pattern list handles common symbol-led tokens, e.g. file extensions (`^\.[a-z0-9]{1,5}$` against a known-extension list), so `زةي` → `.md`, and `(hello` / `hello,` style tokens are handled via their core.
* Tokens containing digits require **Tier A** evidence only (see below).

#### 2.4.3. Evidence Tiers
* **Tier A (strong):** exact dictionary hit or hit after light affix stripping in the target language. Required for standalone correction in the default sensitivity.
* **Tier B (probable):** character n-gram log-likelihood ratio (target-language model vs. as-typed-language model) above a threshold, length ≥ 5. Allowed only in run context (2.5) or in "Aggressive" sensitivity.
* **Sensitivity setting:** Conservative (default) / Balanced / Aggressive.

#### 2.4.4. Dictionaries
* Stored as **FSTs** built offline at compile time (lookup cost grows with word length only — effectively O(1) for bounded word length — and memory stays small compared with a `HashSet<String>`).
* **Arabic:** normalization (strip tashkeel, unify alef forms and similar variants) plus **light affix handling** — common prefixes (و، ف، ب، ك، ل، ال، وال، بال، كال، فال، لل، س) and suffixes (ـه، ـها، ـهم، ـكم، ـني، ـنا، ـون، ـين، ـات، ـة …) — so inflected forms such as "وعليكم" match without being listed.
* **English:** base dictionary + inflection handling (`-s`, `-ed`, `-ing`, …) + a technical-terms list.
* Character n-gram tables for both languages (Tier B).
* Source word lists and frequency data must have licenses verified before bundling.

### 2.5. Multi-Word (Run-Based) Correction
A mistake can span more than one word (e.g. several short words, or words not individually recognized).

* A **run** is a sequence of consecutive words typed under the same layout without cursor movement.
* **Run breaks (buffer reset):** mouse click, arrow/Home/End/PageUp/PageDown, Enter, Tab, Escape, Ctrl/Alt/Win shortcuts, focus change, idle timeout (default 10 s).
* When a word qualifies for correction, the tool **walks backwards** through earlier words of the same run and corrects them too, using a relaxed threshold (the run context is extra evidence), until it reaches a word that is **confidently valid as typed** — that word marks the boundary of the run.
* Caps: max 10 words / 120 characters per retroactive rewrite (configurable).
* After the correction, the layout is switched; subsequent words are typed in the right layout. If the layout switch did not take effect (verified via `GetKeyboardLayout`), the tool stays in a short "sticky mismatch" mode that applies the relaxed threshold to the next few words.

### 2.6. Anti-Loop Rules (One Conversion per Word)
* Each word token has a state: `Original → Converted` or `Original → Rejected`. A converted or rejected word is **never re-evaluated**.
* Events generated by the tool itself are ignored: `LLKHF_INJECTED` flag plus a private signature in `dwExtraInfo`.
* If both interpretations are meaningless or both valid, nothing happens.
* After a user **Undo**, the word is marked *Rejected* and is not auto-corrected again; repeated undos of the same word offer adding it to the user dictionary.

### 2.7. System Input Language Auto-Switching
* After a correction, the tool switches the **focused application's** input language to the target layout.
* Primary method: post `WM_INPUTLANGCHANGEREQUEST` with the target `HKL` to the focused window. (`ActivateKeyboardLayout` alone affects only the *calling thread*, i.e. the tool itself, so it cannot be the primary mechanism.)
* Fallback: Text Services Framework (`ITfInputProcessorProfileMgr::ActivateProfile`).
* After switching, verify with `GetKeyboardLayout`; log a local diagnostic counter (no text) if the switch did not apply.

### 2.8. Visual Marker for Corrected Words (Non-Destructive)
* **Requirement:** the user must be able to see which words were auto-corrected.
* **Design decision:** the corrected text is **not selected** in the target application. A selection would be replaced by the very next character the user types (the user is still typing), destroying the correction.
* **Implementation:** a click-through, non-activating overlay window (`WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`) draws a highlight/underline over the corrected words for about 1.2 s, then fades out.
* **Positioning:** caret rectangle via `GetGUIThreadInfo`; for apps that do not expose it (Chromium/Electron, UWP), via UI Automation `TextPattern`; final fallback is a small toast near the caret/mouse.
* **Settings:** style (underline / highlight / toast), duration, optional sound, per-app disable.

### 2.9. Smart Undo
* **User-customizable hotkey** (any chord or single key). Proposed default: `Ctrl + Alt + Z`.
* Valid until the next regular keystroke or a timeout (default 5 s).
* **Action:** delete the corrected text, restore the exact original keystrokes' output, and switch the input language back to the previous layout. For run-based corrections, the **whole** last action is reverted.
* A single-entry buffer holds the undo data (original characters, corrected characters, previous HKL).

### 2.10. Password & Sensitive Fields
* The tool is **disabled** in password fields and its buffer is cleared on entering one.
* Detection:
  * Win32: `ES_PASSWORD` style on the focused control.
  * Browsers / Electron / UWP: UI Automation `IsPassword` property.
  * The result is **cached on focus-change events** (UIA focus-changed handler), never queried per keystroke, to protect the 50 ms budget.
* If the field type is unknown, the tool falls back to normal behavior, which is limited by the pre-filters and blacklist.

### 2.11. Customization & Configuration
* **App blacklist / whitelist** by process name. Default blacklist: terminals (cmd, PowerShell, Windows Terminal), IDEs (VS Code, Visual Studio, JetBrains), password managers, remote-desktop/VM clients.
* **Games:** auto-pause during exclusive full-screen (`SHQueryUserNotificationState`).
* **Layout mapping:** auto-detected; manual override per language.
* **Custom user dictionary** and ignore list (plain text).
* **Hotkeys:** Undo, global enable/disable toggle.
* **Per-language** enable/disable, sensitivity level, minimum word length.
* **Startup** with Windows, system tray menu (enable/disable, current app exclusion, open settings, quit).
* Config is **data-only** (TOML), strictly validated, no executable paths.

---

## 3. Technical Architecture & Non-Functional Requirements

### 3.1. Technology Choice: Rust
* No garbage-collector or runtime → small, predictable memory (typically a few MB) and near-zero idle CPU.
* Native Windows API access via the `windows` crate (hooks, `SendInput`, UIA, TSF).
* Memory safety matters for a tool that sits on every keystroke.
* Good FST support (`fst` crate), static single binary, easy reproducible builds.
* **Process layout:** a tiny headless **daemon** (hook, engine, overlay, tray) plus an **on-demand settings app** (separate process, closed when not in use) so the background footprint stays small.
* C# with Native AOT is an acceptable fallback if Rust development speed becomes a blocker, but it is not the baseline.

### 3.2. Module Layout
`hook` → `keybuffer` → `layout` (HKL, ToUnicodeEx) → `converter` → `validator` (FST, affixes, n-grams, patterns) → `decision` (tiers, run logic, anti-loop) → `injector` (SendInput) → `layout_switcher` → `overlay` → `undo` → `config` → `tray` → `focus_monitor` (UIA, password detection, app detection)

### 3.3. Threading Model
* **Hook thread:** `WH_KEYBOARD_LL` callback does the minimum — copy the event into a lock-free queue and **return immediately**. Windows silently removes low-level hooks whose callbacks exceed the `LowLevelHooksTimeout`.
* **Worker thread:** buffering, conversion, validation, decision.
* **Injector:** one atomic `SendInput` batch: `N` Backspaces + the corrected text (+ trailing Space) sent as `KEYEVENTF_UNICODE` events (independent of layout-switch timing).
* **UI threads:** overlay, tray, UIA focus monitor.
* **Race handling:** real keystrokes arriving while an injection is in flight are held and replayed in order afterwards. The window is expected to be under 10 ms.

### 3.4. Replacement Strategy
* The trigger Space is **not** swallowed (zero added latency for normal typing). After the decision, the tool removes the word plus the Space with Backspaces and types the corrected word plus Space.
* Per-app strategy override (setting) for apps where Backspace behaves unusually (autocomplete popups, some terminals): alternative = select-previous-word-and-replace.
* Character counts for Backspaces are computed per 2.3; per-app calibration tests are part of the test plan.

### 3.5. Performance & Efficiency
| Stage | Budget |
|---|---|
| Hook callback return | ≤ 1 ms |
| Convert + validate + decide | ≤ 5 ms |
| Injection (`SendInput` batch complete) | ≤ 10 ms |
| **End-to-end: Space key-down → injection submitted** | **< 50 ms (p99)** |
| Resident RAM (daemon) | < 30 MB (target < 15 MB) |
| Idle CPU | ≈ 0% |

* The 50 ms target is measured to the completion of the injection call. Rendering inside the target application is outside the tool's control but must be measured on the app matrix (Section 7).

### 3.6. Privacy & Security
* **No keylogging:** keystrokes live only in an in-memory buffer holding the current run (max 120 chars), zeroized on reset/drop, never written to disk, logs, or crash dumps.
* **No networking:** the code contains no network functionality; dependencies are audited (`cargo-deny`) to ensure none pull it in.
* **Diagnostics** are counters only (corrections, undos, switch failures) — never text.
* **Open source** with reproducible builds and minimal dependencies; no plugin system.

### 3.7. Elevation Strategy
* **Requirement:** corrections must also work inside elevated (administrator) windows. A hook and `SendInput` from a standard-integrity process cannot reach them.
* **v1 approach:** run elevated via a scheduled task at logon with highest privileges (no UAC prompt at each login).
* **Preferred long-term approach:** standard-user daemon with manifest `uiAccess="true"`, which requires an **Authenticode-signed** binary installed in a secure location (`Program Files`). This reaches elevated windows without running fully as administrator.
* **Risk note:** an always-elevated global keyboard hook is a high-value target. Mitigations: open source, signed releases, no network code, minimal dependencies, strictly validated data-only config, and a config/dictionary location that a non-admin process cannot tamper with.
* The secure desktop (UAC prompts, lock screen) is unreachable by design and is not supported.

### 3.8. Code Signing & Distribution
* Authenticode signing of the binary and installer (options: OV/EV certificate, or a free open-source signing program such as SignPath Foundation — check eligibility).
* Distribution: GitHub Releases (signed installer + portable build), optional MSIX/winget later.
* Submit builds to antivirus vendors / Microsoft for false-positive review; publish checksums.

---

## 4. Operational Workflow

1. **Listen:** the hook thread captures low-level key events (ignoring injected ones) and queues them.
2. **Context check:** on focus change, `focus_monitor` updates app identity and password-field status (cached).
3. **Buffer:** the worker accumulates keystrokes `(scan code, Shift, CapsLock, HKL)` for the current word; digits are kept as neutral tokens.
4. **Trigger:** the **Space** key finalizes the word.
5. **Evaluate:**
   * Apply pre-filters.
   * Convert the keys through the opposite layout.
   * Validate (Tier A; Tier B in run context/Aggressive), verify the as-typed word is not valid in the current language.
   * If it qualifies, walk back through the run (2.5).
6. **Execute (valid mismatch):**
   * Inject Backspaces, then the corrected text plus Space (one batch).
   * Post the layout-change request and verify it.
   * Mark words as `Converted`, store the undo entry.
   * Show the overlay marker.
7. **Ignore (no mismatch):** keep the word in the run history (as `Original`) and continue.
8. **Undo (hotkey):** revert text, restore previous layout, mark words `Rejected`.

---

## 5. Edge Cases

| Case | Required behavior |
|---|---|
| Word valid in both languages | Keep as typed |
| Name, brand, username, code identifier | Not corrected unless Tier A; user dictionary available |
| Key producing 2 characters (Arabic `لا`) | Backspace count from actual inserted characters |
| Shifted symbols identical across keys | Resolved through scan codes (2.3) |
| CapsLock on | State captured and applied to the target layout |
| Digits in a token | Pass through; Tier A required |
| AltGr / dead keys | Handled via the OS API; dead-key state not disturbed |
| Pasted text or IME composition | Not buffered; run reset |
| Layout switched manually mid-word | Buffer reset |
| Layout switch failure | Fall back to per-word mode with sticky relaxed threshold |
| Elevated windows | Supported through the elevation strategy (3.7) |

---

## 6. Success Metrics & Acceptance Criteria

* **False positives (wrong corrections of correctly typed text):** ≤ 0.5% of words (stretch goal ≤ 0.1%) on the benchmark corpus in Conservative mode.
* **Recall:** ≥ 85% of wrong-layout dictionary words of length ≥ 3 are corrected.
* **Latency:** p99 < 50 ms from Space key-down to injection submitted.
* **Memory:** < 30 MB resident; idle CPU ≈ 0%.
* **Privacy:** zero network connections; zero persisted keystrokes (verified by test).
* **Undo:** restores the exact original text and layout in 100% of test cases.

---

## 7. Test Plan

* **Unit tests:** scan-code → character mapping for Arabic and English layouts (including Shift/CapsLock variants), round-trip conversion, the `<` / `>` ambiguity cases, ligature key (`لا`), symbol-led tokens (`زةي` → `.md`), affix handling.
* **Corpus tests:** feed large Arabic and English texts, typed in their **correct** layout, through the engine → measure false positives. Feed the same texts typed in the **wrong** layout → measure recall.
* **Adversarial inputs:** URLs, e-mails, paths, code, commands, usernames, hashes, mixed Arabic/English sentences, names, abbreviations.
* **Run logic:** multi-word mistakes, short words, run-boundary detection, cursor-movement resets.
* **Concurrency:** fast typing (≥ 120 WPM), keys arriving during injection, injected-event filtering, no loops.
* **Application matrix:** Notepad, Word, Chrome, Edge, Firefox, WhatsApp/Telegram/Discord, Electron apps, Windows Search, UWP apps, elevated apps, password fields (Win32 and web), RDP/VM windows.
* **Performance:** latency histogram per app, memory over 24 h, idle CPU.
* **Security:** verify no keystroke data on disk/logs/dumps; dependency audit; config tampering tests.

---

## 8. Roadmap

1. **Phase 0 – Validation (optional):** offline prototype to measure dictionary/n-gram false-positive rates on corpora (no hook needed).
2. **Phase 1 – Core MVP:** hook, buffer, layout/converter, validator (Tier A), injector, layout switcher, anti-loop.
3. **Phase 2 – UX:** overlay marker, undo, tray, settings app, password detection, blacklist.
4. **Phase 3 – Intelligence:** affix handling, n-grams (Tier B), run-based correction, sensitivity levels.
5. **Phase 4 – Hardening:** elevation/uiAccess, signing, installer, benchmarks, AV submissions.
6. **Post-v1:** additional language pairs, macOS/Linux investigation.

---

## 9. Open Decisions

* License: MIT vs Apache-2.0.
* Default Undo shortcut (proposed `Ctrl + Alt + Z`).
* Enter/Tab as optional triggers (default off).
* Final dictionary sources and their licenses.
* Elevation: scheduled-task (v1) vs `uiAccess` once signing is ready.

---

## 10. Development Prompts for AI Code Generation

Use the **master context** first in every session, then send the **stage prompts** one at a time and review each stage before moving on.

### 10.1. Master Context Prompt

```text
You are helping build "Auto-Reverse Typo Fixer": an open-source, offline, Windows-only (10 1809+/11) background utility written in Rust (windows crate) that automatically fixes text typed with the wrong keyboard layout, in both directions between Arabic and English (e.g. "hgsghl" -> "السلام", "اثممخ" -> "hello", "زةي" -> ".md"), then switches the focused app's input language.

Hard requirements:
- 100% offline. No networking code, no telemetry, no LLM. Never log or persist keystrokes; keystroke buffers are in-memory only and zeroized.
- Word delimiter is the Space key only, for both languages. Characters like ; , . [ ] ' are NOT delimiters.
- The buffer stores keystrokes (scan code, Shift, CapsLock, source HKL), never output characters. Convert by translating the same keys through the target layout using MapVirtualKeyEx + ToUnicodeEx with the "do not change keyboard state" flag. No hard-coded layout tables.
- Current layout comes from GetKeyboardLayout for the focused window's thread; never assumed.
- Correct a word only if: not pre-filtered; the converted word is valid in the target language; the as-typed word is NOT valid in the current language; not in the user dictionary; confidence threshold met. Default requires Tier A (dictionary / affix-aware FST hit). Tier B (n-gram ratio) only in run context or Aggressive mode.
- Digits are layout-neutral. Symbols may be part of words; validate the full converted token and its stripped core; support symbol-led tokens such as file extensions (".md").
- Multi-word runs: when a word qualifies, walk back through earlier words of the same run (no cursor movement) with a relaxed threshold until a confidently valid as-typed word is found. Max 10 words / 120 chars.
- One conversion per word. Ignore injected events (LLKHF_INJECTED + dwExtraInfo signature). After Undo the word is Rejected permanently.
- Low-level hook callback must only enqueue and return immediately. Processing happens on a worker thread. End-to-end (Space key-down to SendInput submitted) must be < 50 ms p99; RAM < 30 MB.
- Replacement: do not swallow the trigger Space; inject N Backspaces (N computed from actual inserted characters, including multi-char keys like Arabic "لا") then the corrected text + Space using KEYEVENTF_UNICODE in one SendInput batch. Hold and replay real keys that arrive during injection.
- Language switch: post WM_INPUTLANGCHANGEREQUEST with the target HKL to the focused window; fallback to TSF ITfInputProcessorProfileMgr::ActivateProfile; verify with GetKeyboardLayout.
- Disabled in password fields (Win32 ES_PASSWORD, UI Automation IsPassword cached on focus-change events), blacklisted apps, and exclusive full-screen games.
- Visual feedback: a click-through, non-activating overlay that highlights the corrected words for ~1.2 s. Do NOT select the corrected text.
- Undo: user-customizable global hotkey (default Ctrl+Alt+Z), valid until the next regular key or 5 s; restores original text and previous layout; reverts the whole last action.
- Dictionaries: offline-built FSTs (fst crate) for Arabic (with normalization and prefix/suffix stripping) and English (with inflection handling), plus char n-gram tables.
- Config: TOML, data-only, strictly validated. System tray with enable/disable, current-app exclusion, settings, quit. Settings UI runs as a separate on-demand process.

Code quality: modular crates/modules (hook, keybuffer, layout, converter, validator, decision, injector, layout_switcher, overlay, undo, config, tray, focus_monitor), unit tests for every pure module, no unsafe without a safety comment.
```

### 10.2. Stage Prompts

```text
STAGE 1 - Hook + Buffer: Implement the WH_KEYBOARD_LL hook thread (enqueue-only, lock-free queue, immediate return, injected-event filtering) and the keybuffer that records (scan code, Shift, CapsLock, HKL) per word, finalizes on Space, and resets on run-break events (mouse click, navigation keys, Enter/Tab/Esc, Ctrl/Alt/Win combos, focus change, idle timeout). Include unit tests with synthetic events.

STAGE 2 - Layout + Converter: Implement layout discovery (GetKeyboardLayoutList, GetKeyboardLayout for the focused thread) and the converter that maps a keystroke sequence through a target HKL using MapVirtualKeyEx + ToUnicodeEx (non-destructive flag). Return both the converted string and the exact character count the original keys produced. Add tests: hgsghl->السلام, ugd;l->عليكم, اثممخ->hello, زةي->.md, the Arabic "لا" key, Shift/CapsLock cases, and keys that yield identical symbols in one layout but different ones in the other.

STAGE 3 - Validator: Build the dictionary/FST loader and builder tool, Arabic normalization + affix stripping, English inflection handling, pattern pre-filters (URL, e-mail, path, camelCase, snake_case, digits-only), symbol-core extraction and extension patterns, and char n-gram scoring. Expose Tier A / Tier B results. Add a corpus harness that measures false-positive and recall rates.

STAGE 4 - Decision + Run logic + Anti-loop: Implement the decision engine (rules, sensitivity levels, user dictionary, run-based backward walk, sticky mismatch mode, one-conversion-per-word state machine, Rejected state after Undo).

STAGE 5 - Injector + Layout switcher: Implement the atomic SendInput batch (Backspaces from actual character counts, KEYEVENTF_UNICODE output, trailing Space), hold-and-replay of keys typed during injection, per-app replacement strategy, WM_INPUTLANGCHANGEREQUEST switching with TSF fallback and verification. Add a latency benchmark.

STAGE 6 - Focus monitor + Safety: Implement app/process detection, blacklist/whitelist, UI Automation focus-changed handler with cached IsPassword, Win32 ES_PASSWORD detection, full-screen game pause.

STAGE 7 - UX: Implement the overlay marker (caret rect via GetGUIThreadInfo, UIA TextPattern fallback, toast fallback), Undo manager with customizable hotkey, tray menu, TOML config, and the on-demand settings app.

STAGE 8 - Hardening: Elevated run via scheduled task, uiAccess manifest option, Authenticode signing pipeline, installer, memory/latency/idle-CPU soak tests, dependency audit (no network crates), and the full test matrix from the PRD.
```
