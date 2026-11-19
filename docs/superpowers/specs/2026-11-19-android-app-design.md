# ToneStrand for Android: design

Date: 2026-11-19. Branch: `android`. Status: draft for the owner's approval.

## Intent

ToneStrand is a desktop editor (Rust, egui/eframe) for Helix-family pedals, over USB or through a ToneStrand Box.
The owner wants it on **Android tablets** first, with phones evaluated, and an **iOS** version possible later
as long as keeping that door open does not make the Android work much harder.

The main use is **live editing of the pedal through the ToneStrand Box** (Wi-Fi), with USB as a second
connection where the device allows it. Over time the app should carry every desktop feature.

Two things block a straight port today:
1. ToneStrand reads the effect catalog (names, parameters, ranges) from an installed HX Edit, which does not
   exist on Android or iOS.
2. The layout and interactions are made for a mouse and keyboard (right-click menus, hover, small targets,
   F-keys). A 1280x800 landscape render already fits; a phone portrait render does not work at all.

## Goals

1. An Android app (tablet landscape first) that connects to a ToneStrand Box and live-edits the loaded preset:
   chain (add, change, move, remove, on/off, split/merge), knobs, snapshots, controllers, "Save on pedal".
2. USB connection (USB OTG on Android) using the same protocol code, where the hardware allows it.
3. Pedal presets: list, load, copy/move/swap slots, send a file to a slot, with **backups before every write**.
4. IRs: list, upload, download, rename, remove (with IR backups).
5. Compare and restore (a slot backup, a whole-pedal backup).
6. A preset library that can live in a cloud folder (OneDrive, Google Drive) chosen through Android's system
   folder picker, ideally the same folder the desktop app uses.
7. Effect catalogs shipped with the app, converted from HX Edit and stored in the repository per HX Edit version.
8. A phone layout (portrait) after the tablet one, on the same code.
9. The Windows app keeps building, passing its tests and behaving as before at every commit on this branch.

## Non-goals (for now)

- NAM A2 Lite support and the Command Center editor on Android (later phases; their code stays shared and
  compiled, just not surfaced in the Android UI until checked).
- The hidden self-test page, `--box-proxy` and the command-line tools on Android.
- Signing in to OneDrive or Google Drive with their own APIs. Cloud access goes through Android's
  Storage Access Framework (SAF) only.
- iOS itself. Only keep it possible: no Android-only assumptions in shared code; platform pieces behind traits.
- Publishing to the Play Store (later decision; sideloaded APK and GitHub release first).

## Anti-goals (a failure even if it "works")

- **Any path that can lose or corrupt a user's preset or IR.** All of AGENTS.md rule 1 applies unchanged on
  Android: backup before every slot/IR write, `encode_checked`, read-back, 750 ms settle after flash writes,
  stop on a half-done transfer, `Wire::slow` through the Box. A write must never depend on a cloud folder being
  reachable: backups go to app-private storage first.
- Breaking or degrading the Windows app (layout, shortcuts, behaviour) to make the tablet work.
- A fork of the UI code: the Android app and the desktop app are the same crate and the same UI code.
- A slow, laggy knob: live edits must feel like the desktop over the Box.

## Constraints

- Same crate, same `src/` tree; Android is a build target (`cdylib` + `android_main`), not a copy.
- `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings` stay green on Windows at every
  commit. A CI job is added for the Android build (it may start as allowed-to-fail until the toolchain is pinned).
- Ground rules in `AGENTS.md` apply. Rule 3 (no Line 6 files) is **changed by the owner's decision** for the
  catalog (see "Catalog" below); it must be rewritten in `AGENTS.md` and the README's third-party section in the
  same change that adds the first catalog.
- egui's default fonts lack some symbols (rule 5) — same on Android.
- The owner will provide an Android tablet for testing; no device is available today. Phones: whatever the
  owner has. Until the tablet arrives, use the Android emulator for build/run checks (no USB, Box over the
  emulator's network).

## Approaches considered

**A. The existing egui app built for Android (chosen).** eframe runs on Android through winit's
`android-activity`. The protocol, documents, chain and editors are reused; small JNI glue covers USB
permission and the device file descriptor, the SAF folder picker, the Wi-Fi multicast lock for mDNS, and
replacements for `rfd` file dialogs. Touch adaptations go into the shared UI. *Trade-off:* not a native look;
egui's on-screen keyboard (IME) support on Android is weak, which affects renaming, search and tags. iOS later
via the same route (winit supports iOS) with the same rough edges.

**B. Rust core + native UI (Jetpack Compose, later SwiftUI).** Most native feel, but a rewrite of ~12-15k lines
of UI (chain, editors, controllers, pages) and two to three UIs to keep in step. Rejected as the first step;
**kept possible** by the safeguard below.

**C. One cross-platform UI (Flutter / Compose Multiplatform) over a Rust core.** B's rewrite plus a third
technology; the only gain over B is iOS, which is secondary. Dropped.

**Safeguard (owner's choice, part of A):** the pedal and document code is kept free of UI types, so approach B
remains open without redoing it if egui touch or the keyboard proves unacceptable on a real tablet.

## Design

### 1. Code layout and the safeguard

Keep one crate, but draw a hard line between *core* and *UI*:

- **Core** (no `eframe`/`egui` imports): `proto`, `msgpack`, `wire`, `session`, `device`, `devpreset`,
  `devpreset_to_hlx`, `hlx`, `units`, `snapshots` (data parts), `catalog` (data parts), `backup`, `irs`, `nam`,
  `files`, `library`/`folders`/`tonelib` (data parts), `boxes` (data parts), `compare` (data parts).
- Today `catalog.rs` uses `egui::Color32` for category colours and `library.rs` has egui code. Replace
  `Color32` in core with a plain `Rgb(u8,u8,u8)` (converted in the UI) and move UI parts of those files into
  `window/` or `editors/`.
- Enforce it with a test or CI check (e.g. a `core` module tree that a grep/compile check proves egui-free,
  or moving core into a `tonestrand-core` workspace crate). **Builder's call** which; a workspace crate is the
  stronger guarantee and is what UniFFI would bind to if approach B is ever taken.

### 2. Platform layer

A `platform` module with one trait per platform service, a desktop implementation (today's behaviour) and an
Android implementation (JNI via the `jni` and `ndk-context` crates, or a small Kotlin helper class called
through JNI):

| Service | Desktop | Android |
|---|---|---|
| Pick file(s) / folder / save location | `rfd` (as today, 10 call sites) | SAF intents (`ACTION_OPEN_DOCUMENT`, `ACTION_OPEN_DOCUMENT_TREE`, `ACTION_CREATE_DOCUMENT`); async, result delivered on a later frame |
| Read/write files in a picked location | `std::fs` + `files::write_atomic` | SAF document URIs through `ContentResolver` (no atomic rename: write to a new document then delete/rename the old; see Library) |
| App folders (settings, backups, logs) | `dirs::config_dir()` etc. | app-private storage (`getFilesDir()`) |
| USB pedals | `nusb` list/open | `UsbManager` list + permission request; open, get the fd, hand it to Rust |
| mDNS for the Box | `mdns-sd` | `mdns-sd` while holding a `WifiManager.MulticastLock`; manual address and the `.local` lookup as fallback |
| Open a web link | `webbrowser` | an `ACTION_VIEW` intent |
| Keep the screen on while editing | n/a | `FLAG_KEEP_SCREEN_ON` (setting) |

Pickers become **asynchronous** everywhere (desktop included, via a small wrapper that still uses `rfd`
synchronously) so call sites have one shape: request → pending → result on a later frame.

### 3. Connections

- **Box (first):** `BoxWire` (tungstenite over TCP) works as is. Pairing (press BOOT) and the token are stored
  in the Android settings file in app-private storage. Direct mode (192.168.4.1) is offered when no box is found.
  Android may route traffic to mobile data when the Box's own network has no internet: bind the socket to the
  Wi-Fi network (`ConnectivityManager.bindProcessToNetwork` or a per-socket bind) when in direct mode.
- **USB (second):** a `UsbWire` that is built from an Android file descriptor. First verify whether `nusb` 0.2
  can wrap a Linux/Android fd (the crate is on 0.1.14 today; upgrading also touches the desktop `UsbWire` and must
  be checked on Windows). If not, an Android-only `UsbWire` that does bulk transfers through JNI
  (`UsbDeviceConnection.bulkTransfer`) on the device thread. The `Wire` trait already hides the transport.
- Android lifecycle: when the app goes to the background the device thread keeps the link for a short time,
  then disconnects cleanly between commands (never mid-transfer). Writes in progress finish before suspend is
  allowed; on return, reconnect and re-read the live preset.

- **Learned from the spike (2026-11-19):** Box pairing and live editing worked from the emulator. Knobs and
  block drag worked on touch. A knob sends to the pedal only when released.

### 4. Catalog (the HX Edit bypass)

- **Converter:** a desktop command-line tool `tonestrand --export-catalog <hx edit res folder> <out dir>` that
  runs the existing `Catalog::load` parsing and writes **only what ToneStrand uses** (symbols in order, model
  names, categories, colours, parameter names/types/ranges/display specs, DSP loads, picker groups) as one
  compact file (`catalog.json.gz` or MessagePack; builder's call) plus a `meta.json` (HX Edit version, date,
  source file hashes).
- **Storage:** `catalogs/<hx-edit-version>/` in the repository, committed. The newest is embedded in the Android
  build (`include_bytes!`); older ones stay for reference and diffing.
- *(Designer's call)* **First catalog:** one, from the HX Edit installed on the owner's machine. Older versions are
  added only if the owner supplies installs; the format allows it.
- **Choice at run time:** desktop: local HX Edit if found (as today, unless the embedded catalog is from a newer HX
  Edit version and the local version can be read), else the embedded newest catalog (so the
  desktop app also works without HX Edit — a side benefit). Android/iOS: the embedded newest catalog. The pedal's
  firmware version is not read today (nothing in `PROTOCOL.md`), but `Helix.sym` only appends models, so the
  newest catalog covers presets from older firmware; a model the pedal refuses (error -302) is reported as
  "your pedal does not know this model" as today. Reading the firmware version is a later improvement.
- **A test** that loading the converted catalog gives the same `Catalog` (models, parameters, formatting) as
  loading the HX Edit folder it came from, on the machine where HX Edit is installed (skipped when absent).
- **Docs:** rewrite AGENTS.md rule 3 and the README's "Third-party material" section to list the converted
  catalogs as Line 6-derived data kept by the owner's decision, under the same removal-on-request notice.

### 5. Touch and layout

Shared UI changes, active when the input is touch (`egui` reports touch events; a setting can force it):

- **Targets:** minimum 44-48 dp hit areas for knobs, chain blocks, switches and menu items; a "touch" spacing
  scale.
- **Knobs:** vertical drag to turn (as today with the mouse), with a fine mode (second finger or a long-press
  then drag); tap opens a typed-value field; double-tap resets to default if the desktop has that.
- **Context menus:** long-press opens what right-click opens today (chain blocks, slots, presets, IRs).
- **Drag and drop in the chain and preset list:** long-press to pick up, drag, drop; plain drags scroll.
- **Hover-only things:** tooltips and `theme::help` "?" become tap-to-show.
- **Keyboard shortcuts** stay on desktop; on Android they work with a hardware keyboard, nothing depends on them.
- **Text entry:** use egui's text fields and check how the on-screen keyboard behaves on the tablet (show/hide,
  covering the field). If it is unusable, an Android-native text dialog (JNI `AlertDialog` with an `EditText`)
  for rename/search/tag is the fallback — small, isolated, and the same idea works on iOS.
- **Tablet landscape:** today's layout (left preset list, chain on top, block panel below) with touch spacing;
  the left panel collapsible. **Tablet portrait:** the left panel becomes a slide-over drawer.
- **Phone portrait (later phase):** one column. Top bar with the preset name and connection; the chain scrolls
  sideways; the selected block's panel below it; snapshots as a row of chips; the preset list, library, IRs and
  settings as full-screen pages behind a bottom bar. Chosen by available width, not by device type.
- **Desktop unchanged:** every touch change is gated on touch input or narrow width, so the Windows layout and
  mouse behaviour stay as they are. Desktop screenshots before/after (`--screenshot`) are compared at each step.

**Learned from the spike (2026-11-19):**

- **GameActivity, not NativeActivity.** With NativeActivity, Android says the keyboard is shown but Gboard
  draws no keys: there is no input connection. GameActivity has one, so the app uses it.
- **Keyboard glue.** winit 0.30 drops GameActivity's text events, so the app reads `text_input_state()` itself,
  with a one-character placeholder in the field, and gives a lone key release the key press it is missing.
  One backspace tap arrives twice (a key release and a text change); count it once.
- **Colours look washed on the emulator** (the panel colour #15161a shows as about #404248). Check on a real
  tablet before changing the palette (Task 7).
- **Back, then reopen, gave a black screen.** Exit the process when the window closes.
- **The system bars overlap the top bar.** Lay the app out inside the content rectangle the system gives.
- **The preset list cannot scroll by touch:** a drag becomes drag-and-drop. Plain drags must scroll; long-press
  picks up.

### 6. Library and backups

- **Backups** (slot and IR backups, whole-pedal backups): always written to app-private storage first, with the
  same naming and `unique_path` rules. A setting can also copy them to the library folder (cloud) afterwards; a
  failed copy never blocks or undoes a write. A "Pedal backups" page lists them and can export them.
- *(Designer's call)* **Desktop backups do not move**: they stay in `<library>/Pedal backups` as today. Android
  uses the same folder layout and file names, so an Android copy that lands in the shared library folder shows up
  on the desktop's backups page, and the Android backups page lists both its private backups and any it finds in
  the library folder.
- *(Designer's call)* **Backups must survive an uninstall.** App-private storage is deleted on uninstall, so the
  copy to a user-visible folder is **on by default**: at first run the app asks for a backups folder (the library
  folder by default, or any SAF folder). Until a copy has succeeded, the backups page shows a plain warning. The
  private copy is still written first, so a write never waits on that folder.
- **Signing (owner's decision):** every APK that reaches a user, the first one included, is signed with one release
  key, held by the owner (an offline copy) and stored as a GitHub Actions secret, so updates never need an uninstall. Debug-signed builds are labelled as test builds and
  never published.
- **Library folder:** chosen through SAF (`ACTION_OPEN_DOCUMENT_TREE`). The library code works on a small
  storage trait (list, read, write-new, replace, rename, delete) with a `std::fs` implementation (desktop) and a
  SAF implementation (Android). Tags (`.tonestrand-tags.json`) and folders work the same, so a OneDrive folder
  synced with the PC is shared with the desktop app.
- *(Designer's call)* **Replace without atomic rename:** when the provider supports delete and rename, write the
  new content to a new document, check it reads back, then delete the old one and rename the new one; on any
  failure the old one is kept. When the provider cannot delete or rename, **never truncate and overwrite in
  place**: save under a new name (`unique_path` rules) and leave the old file. Tags file: same rule.
- **Risk to check first:** Google Drive's document provider has historically not supported picking a whole
  folder. If that holds, Drive gets "open a preset" and "save a copy" only, and full library folders work with
  OneDrive and local storage. **Owner's decision:** if no cloud provider allows picking a folder on the tablet,
  the library lives inside the app with import and export (single files through the picker), and sharing with the
  desktop is revisited later (sync-app folder or the cloud APIs). Phase 6 starts by testing folder picking with
  OneDrive and Google Drive on the owner's tablet.

### 7. Build, CI and the Windows app

- `Cargo.toml`: `crate-type = ["rlib", "cdylib"]` under a target-specific setup; the binary target stays as is.
  Desktop-only dependencies (`rfd`, `nusb` if replaced on Android, `webbrowser`) move under
  `[target.'cfg(not(target_os = "android"))'.dependencies]`; Android-only ones (`jni`, `ndk-context`,
  `android-activity` via eframe's feature) under `cfg(target_os = "android")`.
- **Decided by the spike:** Gradle with `androidx.games:games-activity:4.4.0` (plus appcompat and kotlin-bom
  1.8.22), with the Rust library built by `cargo-ndk`. The activity is GameActivity (see section 5).
- Build with `cargo-ndk` (or `xbuild`) into a minimal Gradle project under `android/` (manifest with the USB
  device filter for vendor 0x0e41, internet, Wi-Fi multicast; the small Kotlin helper for SAF/USB permission).
  Targets: `arm64-v8a` (tablets and phones), `x86_64` (emulator). Builder's call between `cargo-apk`,
  `xbuild` and Gradle + `cargo-ndk`; a Gradle project is preferred because SAF and USB permission need Java/Kotlin
  callbacks.
- CI: a new job builds the APK on Ubuntu; the existing Windows/macOS jobs unchanged.
- **Every commit on the branch:** `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`
  on Windows, the release Windows build starts and the `--screenshot` renders of the editor match the `main`
  renders (same pixels, or differences explained).

### 8. Phases

*(Designer's calls)*
- **Hardware:** the owner has an HX Stomp and a ToneStrand Box. Every check that talks to a pedal is a hand-off to
  the owner on that pedal (self-test on a spare slot), and the plan has a stop-and-wait point there. The `nusb`
  upgrade is **held until phase 5**, so the Windows USB path does not change without a hardware check.
- **Phase 1 may start before the spike:** the core/UI split and the catalog export are needed by approaches A and B
  alike.
- **Spike go/no-go (switch to approach B, or add the native text dialog, if any fails on the tablet):** a knob drag
  follows the finger without visible lag; a chain block can be picked, dragged and dropped on the first try most of
  the time; a preset can be renamed with the on-screen keyboard without the field being hidden or input lost. The
  last one alone failing means the native text dialog, not approach B.
- **What Android shows before phase 8:** everything except NAM, Command Center, the self-test and the command-line
  tools. Offline `.hlx` and `.hxpreset` editing, `.hxb`/`.hls` import, ToneLib import and whole-pedal restore come
  with phases 4 and 6; pages not yet touched by the touch pass stay hidden on Android until they are.
- Files beyond those listed in section 1 also use egui (`controllers.rs`, `modelinfo.rs`, `units.rs`, `folders.rs`,
  `tonelib.rs`); phase 1 sorts each into core or UI. The 15 MB of `assets/models` is embedded as today; whether to
  load it lazily on Android is the builder's call.

0. **Spike (throwaway):** the current app built for Android unchanged, installed on the tablet: open a file,
   turn knobs, drag blocks, rename a preset, connect to the Box by address. Learn about touch and the keyboard.
   Report back before phase 1.
1. **Core/UI split + catalog export** (desktop only, no Android yet): egui-free core, `--export-catalog`, the
   first `catalogs/<version>/`, embedded fallback on desktop, rule 3 rewritten. Windows verified.
2. **Android shell:** the Gradle project, platform layer (settings, app folders, pickers as stubs), Box
   connection with mDNS + manual address, live editing works on the tablet over the Box.
3. **Touch pass:** targets, knob gestures, long-press menus, drag, tap tooltips, text entry check; tablet
   landscape and portrait.
4. **Pedal presets and backups:** slot list, copy/move/swap, send to slot, backups page, compare and restore.
5. **USB:** permission, fd, `UsbWire` on Android.
6. **IRs and the library:** SAF library folder, tags, search; IR upload/download.
7. **Phone layout.**
8. **Later:** NAM, Command Center, firmware version detection, iOS evaluation.

## Left to the builder

- Workspace crate vs. module boundary for the core (a workspace crate is preferred).
- Catalog file format (gzip JSON vs. MessagePack) and exact fields, as long as the equality test holds.
- Android build tooling details, minimum Android version (suggest API 26+; SAF tree and USB host are older).
- Exact touch sizes and gestures, within the rules above, checked on the tablet.
- How async pickers are threaded through the existing call sites.
