# Android phase 2: the Android app with Box live editing, and the first touch fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A real Android build of ToneStrand (tablet first) that connects to a ToneStrand Box, pairs and live-edits the
pedal, with a working on-screen keyboard, correct colours, the window clear of the system bars, a clean return after
Back, and the first touch fixes (scroll vs drag, knobs heard while turning); the Windows app unchanged except that a
knob is now heard while it turns.

**Architecture:** The app crate becomes a library plus the desktop binary (`src/lib.rs` holds the module list; `main.rs`
is the desktop entry). A separate Cargo workspace `android/` (a `cdylib`) depends on that library and adds the Android
entry (GameActivity via winit's `android-game-activity`), the keyboard glue and the system-bar insets; a small Gradle
project packages it with `androidx.games:games-activity:4.4.0`. Platform-specific calls (file pickers) go through
`src/platform.rs`. Pure logic (keyboard text translation, knob send pacing) lives in the library and is tested on Windows.

**Tech Stack:** Rust (windows-gnu host), eframe/egui 0.31 + glow, winit 0.30 (`android-game-activity`), android-activity
0.6.1, cargo-ndk 4, NDK 27.2.12479018, Android SDK 35, Gradle 8.9 + AGP 8.7.3, JDK 17 (all installed under
`%LOCALAPPDATA%\Android`), the `ToneStrand_Tablet` emulator (2560×1600, API 35, hardware keyboard on).

**Spec:** `docs/superpowers/specs/2026-11-19-android-app-design.md` (sections 2, 3, 5, 7; phases 2 and 3 of section 8).
**Spike it builds on:** branch `spike-android` (`android/` there): what worked and every workaround below come from it.

## Global Constraints

- Branch `android-phase2` from `main`. Never commit to `main`; never push.
- At every commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green on Windows.
- The Windows app looks the same: the six baseline screenshots (`target/baseline/`, commands in `target/baseline/commands.txt`) match byte for byte at the end, except `settings.png` already differs from phase 1 (re-take the baseline from `main` in Task 1 Step 1).
- **One intended desktop behaviour change:** a knob is heard on the pedal while it turns (Task 6). Nothing else on desktop changes behaviour.
- AGENTS.md rules 1-8 apply on Android too: every slot/IR write is preceded by a backup; settings, backups and the library go to app-private storage (`AndroidApp::internal_data_path()`); `files::write_atomic` for files.
- `android/` is its own Cargo workspace (`[workspace]` in `android/Cargo.toml`), never a member of the root workspace, so Android features never reach the Windows build.
- Android builds need, from the spike: the rustup self-contained `dlltool` folder on `PATH` (`%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained`), `android/Cargo.lock` seeded from the root `Cargo.lock`, and `jobserver` pinned to 0.1.32 and `winapi-util` to 0.1.9 (newer ones need an assembler this toolchain lacks).
- GameActivity Java library exactly `androidx.games:games-activity:4.4.0` (what android-activity 0.6.1 is built for), plus `androidx.appcompat:appcompat:1.7.0` and `platform('org.jetbrains.kotlin:kotlin-bom:1.8.22')` (avoids duplicate Kotlin classes). Do not enable the GameActivity prefab.
- Test builds are signed with the Gradle debug key and labelled "(test)"; release signing (owner's key) is not in this phase.
- Plain UI words (AGENTS rule 4); theme colours only (rule 6).

## Review Focus

1. **Back, then open the app again** (the spike showed a black screen): the app must come back with its window. → Task 2 Step 6 (emulator check, scripted).
2. **Two letters typed within one frame, or a held hardware backspace**: every letter arrives once, a held key is not doubled. → tests `two_letters_in_one_frame`, `held_hardware_key_not_doubled` (Task 3).
3. **A knob turned quickly over the Box**: the pedal hears it while it turns but is not flooded (at most one send per 80 ms per knob) and always gets the final value on release. → tests `pacing_*` (Task 6).
4. **Rotating the tablet / resizing the window**: the area clear of the system bars is recomputed every frame. → Task 4 Step 5 (emulator rotate check).
5. **A long press that turns into a drag in a scrolling list** (preset list): a plain drag scrolls; only press-and-hold then move picks the preset up, and a drop still asks before writing (existing confirm). → Task 5 Step 5 (emulator check with `adb shell input swipe`).

---

## File structure

| Path | Responsibility |
|---|---|
| `src/lib.rs` (new) | The app's module list (moved from `main.rs`) and the core re-exports; `pub` what `main.rs` and `android/` use. |
| `src/main.rs` | Desktop entry only: CLI dispatch, window options, `run_native`; uses `tonestrand::…`. |
| `src/platform.rs` (new) | `FileDialog` with rfd's builder calls: desktop delegates to `rfd`, Android returns nothing (pickers come in phase 6). |
| `src/touch_text.rs` (new) | Pure keyboard glue: turn GameActivity's text state and lone key releases into egui events. |
| `src/editors/pacing.rs` (new) | Pure knob send pacing (while turning, every 80 ms; always on release). |
| `src/theme.rs` | `pub fn touch() -> bool` (true on Android). |
| `src/window/connection.rs`, `src/window/library_page.rs`, `src/window/irs.rs` | Lists inside scroll areas: plain drag scrolls on touch; press-and-hold to drag. |
| `src/window/mod.rs` | Getting-started card without the HX Edit step when an effects list is loaded. |
| `Cargo.toml` | `[lib]` + `[[bin]]`; `rfd` only for `cfg(not(target_os = "android"))`. |
| `android/Cargo.toml`, `android/Cargo.lock`, `android/build.rs`, `android/src/lib.rs`, `android/src/keyboard.rs`, `android/src/insets.rs` | The Android library: entry, keyboard (uses `touch_text`), insets, process exit after the window closes. |
| `android/app/` (Gradle: `settings.gradle`, `build.gradle`, `gradle.properties`, `app/build.gradle`, `app/src/main/AndroidManifest.xml`) | Packaging with GameActivity. |
| `android/build.ps1` | Build, strip, package, install and start on a device (`-Abi x86_64|arm64-v8a`, `-Device`). |
| `docs/ANDROID.md` (new) | How to build and run the Android app; what was tried on what. |
| `docs/superpowers/specs/2026-11-19-android-app-design.md` | Updated with the spike's findings (Task 1). |
| `.github/workflows/ci.yml` | An Android build job (informational). |

---

### Task 1: Library + desktop binary, platform pickers, spec update

**Files:** Create `src/lib.rs`, `src/platform.rs`. Modify `src/main.rs`, `Cargo.toml`, the 10 files with `rfd::FileDialog` (`grep -rln "rfd::" src`), `docs/superpowers/specs/2026-11-19-android-app-design.md`.

**Interfaces:**
- Produces: crate `tonestrand` (lib) with `pub mod window`, `pub mod cli`, `pub mod theme`, `pub mod editors`, `pub mod touch_text` (added in Task 3), the other app modules as `pub(crate)` or `pub` as needed, and `pub use tonestrand_core::{…}` exactly as `main.rs` has today. `window::App::new(cc, screenshot: Option<…>)` stays as is and is `pub`.
- Produces: `platform::FileDialog` with `new()`, `add_filter(name, &[ext])`, `set_directory(path)`, `set_title(text)`, `set_file_name(name)`, `pick_file() -> Option<PathBuf>`, `pick_files() -> Option<Vec<PathBuf>>`, `pick_folder() -> Option<PathBuf>`, `save_file() -> Option<PathBuf>` (same shapes as rfd 0.15's calls used today).

- [ ] **Step 1: Fresh baseline from `main`**: build release on `main`, re-run every command in `target/baseline/commands.txt` into `target/baseline/` (replacing), so `settings.png` reflects phase 1. Then create branch `android-phase2`.
- [ ] **Step 2: Move the module list** from `main.rs` into `src/lib.rs`; `main.rs` keeps `fn main` and calls `tonestrand::cli::run`, `tonestrand::window::App::new`, etc. `Cargo.toml`: `[lib] name = "tonestrand", path = "src/lib.rs"` and `[[bin]] name = "tonestrand", path = "src/main.rs"`. The `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` stays in `main.rs`. `build.rs` (photos) is unchanged.
- [ ] **Step 3: `platform::FileDialog`**: desktop (`#[cfg(not(target_os = "android"))]`) wraps `rfd::FileDialog`; Android returns `None`/empty. Replace every `rfd::FileDialog` with `crate::platform::FileDialog`. `Cargo.toml`: `rfd` under `[target.'cfg(not(target_os = "android"))'.dependencies]`.
- [ ] **Step 4: Check**: `cargo fmt; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace` (same test count as before, 123 + any new); `grep -rn "rfd::" src` shows only `src/platform.rs`; `cargo run --release -- --help` unchanged; the six screenshots match `target/baseline/` byte for byte.
- [ ] **Step 5: Spec update** (section 3 "Connections", 5 "Touch and layout", 7 "Build"): GameActivity (not NativeActivity) because NativeActivity never gets the on-screen keyboard's keys; the keyboard glue (winit 0.30 drops GameActivity text; read `text_input_state()`, a one-character placeholder, lone key releases get a press); Gradle + games-activity 4.4.0; colours look washed on the emulator (Task 7); Back then reopen gave a black screen (exit the process when the window closes); the system bars overlap the top bar (content rect). Plain words, a short "Learned from the spike (2026-11-19)" subsection.
- [ ] **Step 6: Commit** — `git commit -m "The app is a library plus the desktop program; file pickers go through platform.rs; spec: what the Android spike taught"`.

---

### Task 2: The Android crate and Gradle packaging; clean exit after Back

**Files:** Create everything under `android/` listed in the file structure (start from `spike-android`'s `android/` — `git show spike-android:android/<file>` — keeping the pins and the Gradle setup, but depending on the library instead of including `src/` by path, and without the rfd stub), `docs/ANDROID.md`.

**Interfaces:**
- Consumes: crate `tonestrand` (Task 1) as `tonestrand = { path = ".." }`, `tonestrand-core = { path = "../core" }`.
- Produces: `android/src/lib.rs` with `#[no_mangle] fn android_main(app: AndroidApp)`; module stubs `keyboard` and `insets` filled by Tasks 3 and 4; a wrapper `struct AndroidShell { inner: tonestrand::window::App, android: AndroidApp, … }` implementing `eframe::App` (delegates `update`, `save`, `on_exit`, and has a `raw_input_hook`).
- Produces: `android/build.ps1 [-Abi x86_64] [-Device emulator-5554] [-NoInstall]` → `android/app/app/build/outputs/apk/debug/app-debug.apk`, installed and started (package `com.tonestrand.app`, label "ToneStrand (test)").

- [ ] **Step 1: Cargo**: `android/Cargo.toml` (`crate-type = ["cdylib"]`, own `[workspace]`, eframe `features = ["default_fonts", "glow", "android-game-activity"]`, winit `0.30` `features = ["android-game-activity"]`, `log`, `android_logger`, the root's `[patch.crates-io]` with `../vendor/...` paths). Seed `android/Cargo.lock` from the root lock, then `cargo update -p jobserver --precise 0.1.32` and `-p winapi-util --precise 0.1.9`.
- [ ] **Step 2: Entry**: `android_main` sets up logging (tag `tonestrand`), points `HOME`, `XDG_CONFIG_HOME`, `XDG_DOCUMENTS_DIR`, `TONESTRAND_LIBRARY` at `internal_data_path()` subfolders, logs panics, runs `eframe::run_native` with `NativeOptions { android_app: Some(app.clone()), .. }` and the wrapper.
- [ ] **Step 3: Exit after the window closes**: after `run_native` returns (Back destroys the activity), call `std::process::exit(0)` so the next launch starts a fresh process (winit cannot make a second event loop in one process).
- [ ] **Step 4: Gradle** under `android/app/` (application id and namespace `com.tonestrand.app`, minSdk 26, targetSdk/compileSdk 35, `jniLibs.srcDirs = ['../jniLibs']`, `useLegacyPackaging true`, the three dependencies from Global Constraints, manifest with `com.google.androidgamesdk.GameActivity`, `android.app.lib_name = tonestrand_android`, `Theme.AppCompat.NoActionBar`, `windowSoftInputMode="adjustResize"`, the `configChanges` list from the spike, permissions INTERNET, ACCESS_NETWORK_STATE, ACCESS_WIFI_STATE, CHANGE_WIFI_MULTICAST_STATE). `.gitignore` for `jniLibs/`, `target/`, Gradle outputs, `local.properties`.
- [ ] **Step 5: `build.ps1`** (from the spike's `package-game.ps1`): env, `cargo ndk -t <abi> -P 26 -o jniLibs --link-libcxx-shared build --release`, `llvm-strip --strip-unneeded`, `local.properties` with forward slashes, `gradle --no-daemon -q assembleDebug`, uninstall old/install new, start `com.tonestrand.app/com.google.androidgamesdk.GameActivity`. Uninstall also `com.tonestrand.spike` once if present.
- [ ] **Step 6: Emulator check** (script it with adb, keep the commands in `docs/ANDROID.md`): start → screenshot shows the editor; `adb shell input keyevent KEYCODE_BACK`, wait 3 s, start again → screenshot shows the editor (not black); `KEYCODE_HOME`, start again → editor. Save the three screenshots to `target/android-check/`.
- [ ] **Step 7: `docs/ANDROID.md`**: tools and where they are, `build.ps1` usage, the emulator, the pins and why, what has been tried on the emulator vs a real tablet (nothing yet).
- [ ] **Step 8: Windows check** (`cargo test --workspace` etc. unchanged) and **commit** — `git commit -m "Android app (GameActivity) built from the same library; exits cleanly after Back so it opens again"`.

---

### Task 3: On-screen keyboard

**Files:** Create `src/touch_text.rs` (in the library, pure), `android/src/keyboard.rs`. Modify `src/lib.rs`, `android/src/lib.rs`.

**Interfaces:**
- Produces in `touch_text`:
  - `pub const PLACEHOLDER: &str = "\u{200B}";`
  - `pub struct KeyPairing { held: Vec<egui::Key> }` with `pub fn new() -> Self` and `pub fn fix(&mut self, events: &mut Vec<egui::Event>) -> bool` — inserts a `pressed: true` before each `Key` release whose press was not seen (the on-screen keyboard sends releases only), tracks keys really held, returns whether any Backspace key event was present.
  - `pub fn events_from_text(text: &str, saw_backspace_key: bool) -> Option<Vec<egui::Event>>` — `None` when `text == PLACEHOLDER` (nothing typed); otherwise: one Backspace (press+release) if the placeholder is gone and no Backspace key came; then the text without the placeholder as `Text` events, each `\n` as Enter (press+release). The caller resets the text state after `Some`.
- Produces in `android/src/keyboard.rs`: `pub struct Keyboard { shown: bool, pairing: KeyPairing }` with `pub fn after_update(&mut self, app: &AndroidApp, wants_keyboard: bool)` (on change: `set_ime_editor_info(TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_VISIBLE_PASSWORD | TYPE_TEXT_FLAG_NO_SUGGESTIONS, Done, IME_FLAG_NO_FULLSCREEN)`, reset the text state to the placeholder with cursor `1..1`, `show_soft_input(true)` / `hide_soft_input(true)`) and `pub fn input_hook(&mut self, app: &AndroidApp, raw: &mut egui::RawInput)` (pairing, then — when shown and `compose_region` is `None` — `events_from_text`, push, reset).

- [ ] **Step 1: Write the failing tests** in `src/touch_text.rs`:

```rust
#[test]
fn letter_typed_before_placeholder() {           // Android puts the cursor before it
    assert_eq!(events_from_text("w\u{200B}", false), Some(vec![Event::Text("w".into())]));
}
#[test]
fn two_letters_in_one_frame() {
    assert_eq!(events_from_text("\u{200B}ab", false), Some(vec![Event::Text("ab".into())]));
}
#[test]
fn nothing_typed() { assert_eq!(events_from_text(PLACEHOLDER, false), None); }
#[test]
fn backspace_seen_once_when_key_and_text_both_report_it() {
    assert_eq!(events_from_text("", true), Some(vec![]));
}
#[test]
fn backspace_from_text_alone() {
    assert_eq!(events_from_text("", false), Some(vec![key(Key::Backspace, true), key(Key::Backspace, false)]));
}
#[test]
fn enter_from_newline() {
    assert_eq!(events_from_text("\u{200B}ok\n", false),
        Some(vec![Event::Text("ok".into()), key(Key::Enter, true), key(Key::Enter, false)]));
}
#[test]
fn lone_release_gets_a_press() {
    let mut p = KeyPairing::new();
    let mut ev = vec![key(Key::Backspace, false)];
    assert!(p.fix(&mut ev));
    assert_eq!(ev, vec![key(Key::Backspace, true), key(Key::Backspace, false)]);
}
#[test]
fn held_hardware_key_not_doubled() {
    let mut p = KeyPairing::new();
    let mut a = vec![key(Key::Backspace, true)];
    p.fix(&mut a);
    let mut b = vec![key(Key::Backspace, false)];         // release in a later frame
    p.fix(&mut b);
    assert_eq!(b, vec![key(Key::Backspace, false)]);
}
```
(`key(k, pressed)` is a test helper building `Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }`.)

- [ ] **Step 2: Run, see them fail** — `cargo test -p tonestrand touch_text` → not found.
- [ ] **Step 3: Implement** `touch_text` (no Android dependency; it compiles and is tested on Windows), then `android/src/keyboard.rs` and hook it into the wrapper (`update` → `after_update(ctx.wants_keyboard_input())`, `raw_input_hook` → `input_hook`).
- [ ] **Step 4: Run tests** — `cargo test --workspace` → pass.
- [ ] **Step 5: Emulator check**: build; New preset; tap the name field → keyboard appears; tap keys w, i, backspace, x, backspace, backspace (coordinates from a screenshot) → name back to "New Preset" (screenshot); type with the PC keyboard → letters arrive once; hold PC backspace → deletes steadily, not double.
- [ ] **Step 6: Commit** — `git commit -m "On-screen keyboard on Android: shown while a text field has focus; letters, backspace and Enter reach the field once"`.

---

### Task 4: Keep clear of the system bars

**Files:** Create `android/src/insets.rs`. Modify `android/src/lib.rs`.

**Interfaces:**
- Produces: `pub fn content_rect_points(app: &AndroidApp, pixels_per_point: f32) -> Option<egui::Rect>` — `app.content_rect()` (pixels) divided by `pixels_per_point`; `None` when empty.
- Consumes: in the wrapper's `raw_input_hook`, set `raw.screen_rect = Some(rect)` (every frame, so rotation and resizes follow).

- [ ] **Step 1: Implement** `insets.rs`. `pixels_per_point` comes from `raw.viewports[&raw.viewport_id].native_pixels_per_point` (fallback `ctx.pixels_per_point()`).
- [ ] **Step 2: If `content_rect()` reports the whole screen** under edge-to-edge (targetSdk 35), get the system-bar insets through JNI instead (`getWindow().getDecorView().getRootWindowInsets().getInsets(WindowInsets.Type.systemBars())`) and report what was needed in the task report. Add `jni = "0.21"` only if this path is needed.
- [ ] **Step 3: Emulator check**: screenshot: the top bar starts below the status bar (clock and battery no longer over "ToneStrand" or "Open"), the status bar at the bottom ends above the gesture bar.
- [ ] **Step 4: Rotate**: `adb shell settings put system accelerometer_rotation 0; adb shell settings put system user_rotation 1` (portrait), screenshot, then `user_rotation 0` back → both clear of the bars, nothing cut off at the top.
- [ ] **Step 5: Commit** — `git commit -m "Android: the window keeps clear of the status and navigation bars, also after rotating"`.

---

### Task 5: Scroll vs drag in lists on touch

**Files:** Modify `src/theme.rs`, `src/window/connection.rs` (pedal preset list, ~line 741-790), `src/window/library_page.rs`, `src/window/irs.rs` (only where a list row inside a `ScrollArea` senses drag).

**Interfaces:**
- Produces: `theme::touch() -> bool` (`cfg!(target_os = "android")`; a later setting can force it) and `theme::list_row_sense() -> egui::Sense` (`Sense::click()` on touch, `Sense::click_and_drag()` on desktop).

- [ ] **Step 1: Desktop unchanged**: every change is behind `theme::touch()`; on desktop the senses and code paths are exactly today's.
- [ ] **Step 2: Touch rows**: rows use `list_row_sense()`, so a plain drag reaches the scroll area and scrolls. Press-and-hold (`Response::long_touched()`) starts carrying that preset: set the same `PresetDrag(i)` payload (`egui::DragAndDrop::set_payload`), keep the scroll area from scrolling while carrying (`ScrollArea::drag_to_scroll(false)` for that frame onwards until release), and highlight the row under the finger; on release over another row, the existing drop path (confirm, backup, write) runs unchanged.
- [ ] **Step 3: Same rule** in the library and IR lists where rows are dragged.
- [ ] **Step 4: Windows check**: tests, clippy, the six screenshots byte for byte.
- [ ] **Step 5: Emulator check** (connected to nothing is fine): `adb shell input swipe 300 1200 300 500 300` on the preset list area → the list scrolls (needs a pedal list: use the Box, or check on the library page with a few files in the app's library folder pushed with `adb push`); a long press then move (`adb shell input swipe x y x y2 1500` holds before moving) → the copy/move confirm appears; cancel it. Screenshots to `target/android-check/`.
- [ ] **Step 6: Commit** — `git commit -m "Touch: a drag scrolls lists; press and hold to pick a preset up"`.

---

### Task 6: Knobs heard while they turn

**Files:** Create `src/editors/pacing.rs`. Modify `src/editors/mod.rs` (knob widget ~line 570-590 and the `param_cell` callers that push `Cmd::SetParam`, ~lines 160-170 and 1160-1175), `src/editors/device.rs` if it has its own send sites (`grep -n "SetParam" src/editors`).

**Interfaces:**
- Produces: `pub const LIVE_STEP: Duration = Duration::from_millis(80);` and `pub fn should_send(last_sent: Option<Instant>, now: Instant, released: bool, changed: bool) -> bool` — true on release (if anything changed since the last send), or while turning when `changed` and `last_sent` is `None` or at least `LIVE_STEP` ago.
- `value_widget` / `param_cell` keep returning `(Option<Value>, commit)`; the send sites add a "send now" decision with `should_send`, keeping the last send time per (block, index) in egui temp memory keyed by the widget id.

- [ ] **Step 1: Write the failing tests** in `pacing.rs`:

```rust
#[test]
fn pacing_first_change_goes_at_once() { let t = Instant::now(); assert!(should_send(None, t, false, true)); }
#[test]
fn pacing_waits_between_sends() {
    let t = Instant::now();
    assert!(!should_send(Some(t), t + Duration::from_millis(40), false, true));
    assert!(should_send(Some(t), t + LIVE_STEP, false, true));
}
#[test]
fn pacing_release_always_sends_the_last_value() {
    let t = Instant::now();
    assert!(should_send(Some(t), t + Duration::from_millis(5), true, true));
}
#[test]
fn pacing_nothing_changed_nothing_sent() { let t = Instant::now(); assert!(!should_send(Some(t), t + LIVE_STEP, false, false)); }
```

- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** `should_send` and use it at the live send sites (live documents only; offline documents unchanged). Release still sends the final value, so the pedal always ends on what the knob shows.
- [ ] **Step 4: Run tests** and the Windows screenshot check.
- [ ] **Step 5: Hand-off note** for the owner's hardware check (pedal through the Box and USB, Windows and Android): turn a knob slowly and quickly; the sound follows while turning; the knob's final value is what the pedal shows after release (read back with `--dump-loaded` if in doubt). Not run by the implementer.
- [ ] **Step 6: Commit** — `git commit -m "A knob is heard while it turns (a send every 80 ms at most, the final value on release)"`.

---

### Task 7: Colours as on Windows

**Files:** Modify `android/src/lib.rs` (and, only if needed, `android/Cargo.toml` / a `[patch]` of eframe for Android only).

**Interfaces:**
- Acceptance: on the emulator, with the dark theme, pixels of `theme::panel()` (#15161a) and `theme::card_bg()` (#1b1d22) in a screenshot are within ±3 per channel of those values (today they are near #404248).

- [ ] **Step 1: Measure first**: a screenshot of a known surface, and the log lines from egui_glow (`RUST_LOG`-equivalent: set android_logger to Debug for `egui_glow`) saying whether sRGB textures / framebuffer are used, and which EGL config glutin picked. Record in the task report.
- [ ] **Step 2: Fix** in this order, stopping at the first that meets the acceptance: (a) ask for a linear (non-sRGB) surface/config — eframe 0.31 builds `ConfigTemplateBuilder`/`SurfaceAttributesBuilder` in `native/glow_integration.rs` without colour-space choices; if a config or surface choice fixes it, apply it through the smallest Android-only patch of eframe (a `[patch.crates-io]` in `android/Cargo.toml` pointing at a vendored copy under `android/vendor/eframe` with only that change, documented in `docs/ANDROID.md`); (b) disable `GL_FRAMEBUFFER_SRGB_EXT` where `EXT_sRGB_write_control` exists, from the wrapper before painting (via `frame.gl()`); (c) eframe's `wgpu` renderer for the Android build only. The Windows build must not change.
- [ ] **Step 3: Emulator check** against the acceptance, light theme too (`Settings > Display`), screenshots to `target/android-check/`. Note in `docs/ANDROID.md` that the real tablet must be checked too (the emulator's GPU path differs).
- [ ] **Step 4: Commit** — `git commit -m "Android: colours as on Windows (no second sRGB step)"`.

---

### Task 8: Box on Android, the getting-started card, CI

**Files:** Modify `android/src/lib.rs` (multicast lock), `src/window/mod.rs` (getting-started card ~line 1220-1270), `.github/workflows/ci.yml`, `docs/ANDROID.md`, `AGENTS.md` (Android pointers), `CHANGELOG.md`.

- [ ] **Step 1: Multicast lock**: at start, through JNI (`jni = "0.21"`; `AndroidApp::vm_as_ptr()` / `activity_as_ptr()`), `WifiManager.createMulticastLock("tonestrand").setReferenceCounted(false).acquire()`, so mDNS finds the Box on a real device. Log success or the error; never fail start-up. (The emulator cannot see mDNS; the owner checks it on the tablet.)
- [ ] **Step 2: Getting-started card**: the "Install HX Edit" step is shown only when no effects list is loaded at all; the remaining steps renumber. Wording otherwise unchanged. (Desktop screenshots hide the card, so they stay identical.)
- [ ] **Step 3: CI**: a job `android` on `ubuntu-latest`, `continue-on-error: true`, that installs the NDK 27.2.12479018 and cargo-ndk, builds `android/` for `aarch64-linux-android` and runs Gradle `assembleDebug` (JDK 17, Gradle 8.9), uploading the debug APK as an artifact. The Windows/macOS jobs are unchanged.
- [ ] **Step 4: Emulator end-to-end** (owner, since pairing needs BOOT on the Box): connect to the Box by address, pair, load a preset, turn a knob, change a snapshot, type a preset name with the on-screen keyboard, Back and reopen. The implementer prepares the build and lists these steps in the hand-off.
- [ ] **Step 5: Docs**: `docs/ANDROID.md` complete (build, run, the emulator, what was tried where); AGENTS.md "Where to change what" gets Android rows; CHANGELOG "Unreleased".
- [ ] **Step 6: Final gate** — Windows: fmt, clippy, tests, the six screenshots byte for byte; Android: `android/build.ps1` builds and installs on the emulator. **Commit** — `git commit -m "Android: multicast lock for finding the Box, getting-started card without the HX Edit step, CI build job, docs"`.
