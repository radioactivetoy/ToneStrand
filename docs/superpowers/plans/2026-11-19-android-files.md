# Android file picker and shared backups folder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every file dialog works on Android through the system picker, and the library plus pedal backups live in `Documents/ToneStrand`, which survives an uninstall.

**Architecture:** `src/platform.rs` gets an asynchronous picker (request ids, results polled each frame); the desktop completes requests at once with `rfd`, Android hands them to a backend installed by the `android/` crate, whose Java `MainActivity` runs the system picker and copies documents through the app's cache folder. The Android crate also finds the shared Documents folder, asks for the storage permission and tells the app when the folder becomes usable; the app then switches its library folder and copies missing private files across.

**Tech Stack:** Rust (egui/eframe 0.31, jni 0.21), Java (Android SDK 35, GameActivity 4.4.0), Gradle 8.9 / AGP 8.7.3.

**Spec:** docs/superpowers/specs/2026-11-19-android-files-and-backups-design.md

## Global Constraints

- The Windows app's dialogs look and return what they did; `cargo fmt`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` stay clean.
- No JNI or Java in the root crate: Android specifics live in `android/` (own Cargo workspace) and reach the app only through `platform` hooks; `cfg(target_os = "android")` in the root crate only for choosing behaviour, as today.
- Shared folder: `Environment.getExternalStoragePublicDirectory(DIRECTORY_DOCUMENTS)/ToneStrand`, read through JNI, never hard-coded.
- Manifest: `WRITE_EXTERNAL_STORAGE` with `maxSdkVersion="29"`, `READ_EXTERNAL_STORAGE` with `maxSdkVersion="32"`, `requestLegacyExternalStorage="true"` on `<application>`; activity `com.tonestrand.app.MainActivity` (extends `com.google.androidgamesdk.GameActivity`), keeping today's `configChanges`, `windowSoftInputMode`, launcher intent filter and `android.app.lib_name` meta-data.
- Never overwrite or delete a user file when copying private data across; never truncate a picked document on a failed save (write the whole temp file first).
- Cache paths (the app's cache folder) are never stored in settings (`recent`, last-session tabs).
- Warning text, exact: "Backups are only kept inside the app and are lost if it is uninstalled. Allow storage access to keep them in Documents/ToneStrand." with a button "Allow storage access".
- Android 11+ note in Settings, exact: "On Android 11 and newer, files put in this folder by other apps (a sync app, a PC over USB) or by an earlier install of ToneStrand are not visible here."
- AGENTS.md rule 1: never risk presets; tests never confirm pedal writes.

## Review Focus

- Picker cancelled (back button) for each kind: nothing happens, no error note, no stuck "waiting" state → Task 1 test `cancelled_request_yields_cancelled` and Task 2 handler ignoring `Cancelled`.
- Two requests in flight (tap Open twice quickly): only one picker at a time; a second request while one is pending is refused with no crash → Task 1 test `second_request_while_pending_is_refused`.
- Copy across when the shared folder already has a same-named file (reinstall, clear data): existing file untouched, private one not copied → Task 3 test `copy_missing_never_overwrites`.
- Saving a document that was opened from the picker: "Save" acts as Save as; the cache path never lands in `settings.recent` → Task 2 test `cache_paths_are_not_remembered`.
- Permission granted while the app runs: library switches without restart and the backups page lists files from both folders, de-duplicated → Task 3 test `backups_listed_from_both_folders_once`.

---

### Task 1: Asynchronous picker in `platform`

**Files:**
- Modify: `src/platform.rs`
- Test: unit tests in `src/platform.rs`

**Interfaces:**
- Produces (all in `crate::platform`):
  - `pub enum PickKind { Open { multiple: bool }, Save { suggested: String }, Folder }`
  - `pub struct PickRequest { pub id: u64, pub kind: PickKind, pub title: Option<String>, pub filters: Vec<(String, Vec<String>)>, pub dir: Option<PathBuf> }`
  - `pub enum PickResult { Files(Vec<PathBuf>), SaveTo(PathBuf), Folder(PathBuf), Saved, Cancelled, Failed(String) }`
  - `FileDialog` keeps its builder (`new`, `add_filter`, `set_directory`, `set_title`, `set_file_name`); terminal calls become `open_file(self) -> Option<u64>`, `open_files(self) -> Option<u64>`, `save_file(self) -> Option<u64>`, `pick_folder(self) -> Option<u64>` returning the request id, or `None` when a request is already pending.
  - `pub fn poll() -> Vec<(u64, PickResult)>` — finished requests, oldest first.
  - `pub fn commit_save(id: u64, written: &Path)` — after the app has written the whole file to the `SaveTo` path: desktop does nothing more and queues `Saved`; Android hands it to the backend, which later delivers `Saved` or `Failed`.
  - `pub fn set_backend(backend: Box<dyn Fn(PickRequest) + Send + Sync>)`, `pub fn set_commit_backend(f: Box<dyn Fn(u64, PathBuf) + Send + Sync>)` and `pub fn deliver(id: u64, result: PickResult)` — for the Android crate.
  - Desktop: `Open` → `Files`, `Save` → `SaveTo(path)` (the chosen path itself), `Folder` → `Folder`, cancel → `Cancelled`, all queued immediately by the request call using `rfd`.
  - A request is "pending" from start until its final result is polled (`Files`, `Folder`, `Saved`, `Cancelled`, `Failed`; for saves `SaveTo` is not final).

- [ ] **Step 1: Write failing tests** (`#[cfg(test)]`, using a test backend installed with `set_backend` that records requests, and `deliver` to answer): `cancelled_request_yields_cancelled` (deliver `Cancelled` → `poll()` returns `[(id, Cancelled)]` and a new request is accepted afterwards); `second_request_while_pending_is_refused` (second `open_file()` returns `None` until the first is answered); `save_flow` (`SaveTo` then `commit_save` → backend commit called with the path; `deliver(Saved)` ends it); `filters_and_title_reach_the_backend`. The global state must be resettable for tests (`fn reset_for_tests()` behind `#[cfg(test)]`) and the tests run in one function or behind a mutex, since state is global.
- [ ] **Step 2: Run** `cargo test -p tonestrand platform` — FAIL.
- [ ] **Step 3: Implement** with a global `Mutex<State>` (pending id, next id, result queue, backends). On desktop (`not(target_os = "android")`), when no backend is installed the terminal calls run `rfd` synchronously and queue the result; in tests a backend is installed so `rfd` is not called. On Android with no backend installed, requests fail at once with `Failed("No file picker")`.
- [ ] **Step 4: Run** the tests — PASS. Keep the old synchronous methods temporarily only if Task 2 needs them to compile; Task 2 removes them.
- [ ] **Step 5: Commit** `Platform: asynchronous file picker (request ids, results polled each frame)`.

### Task 2: Every dialog call site on the async picker; Android folder rules

**Files:**
- Modify: `src/window/mod.rs` (add `pending: HashMap<u64, Pick>` to `App`, poll at frame start, `open_file`, `save_doc`, `remember_recent`, HX Edit catalog card), `src/window/bars.rs`, `src/window/compare_window.rs`, `src/window/connection.rs`, `src/window/irs.rs`, `src/window/library_page.rs`, `src/window/restore_all.rs`, `src/window/settings.rs`, `src/window/tests_page.rs`, `src/tonelib.rs`
- Create: `src/window/picks.rs` (the `Pick` enum and the dispatch `fn handle_pick(&mut self, pick: Pick, result: PickResult)`)
- Test: unit tests in `src/window/picks.rs` (pure helpers)

**Interfaces:**
- Consumes: Task 1's API.
- Produces: `pub fn is_cache_path(p: &Path) -> bool` in `crate::platform` (true for paths under the app cache folder; desktop: always false; Android: under the path given by `platform::set_cache_dir(PathBuf)`, set by the Android crate). `Pick` has one variant per call site: `OpenPreset`, `ComparePick(usize)`, `ImportLibrary`, `ImportTones`, `UploadIrs`, `NamCaptures`, `NamOutFolder(Vec<PathBuf>)`, `SaveAs(usize /*doc index*/, Vec<u8>)`, `SavePedalPreset(Vec<u8>)`, `SaveLog`, `BackupAllFolder`, `FullBackupFolder`, `RestoreAllFolder`, `LibraryFolder`, `ToneFolder`, `HxEditFolder`, `TestsFolder`.

- [ ] **Step 1: Write failing tests** in picks.rs for pure helpers: `cache_paths_are_not_remembered` (a helper `fn rememberable(p: &Path) -> bool` returns false for a cache path after `set_cache_dir`), and `android_backup_folder_name` (helper `fn dated_backup_folder(base: &Path, label: &str, secs: u64) -> PathBuf` produces `<base>/Pedal backups/<label> <YYYY-MM-DD HHMM>` — copy the naming the desktop uses for whole-pedal backups if one exists in `core/src/device.rs`/`connection.rs`; if it does, use that function instead and test it).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Convert each call site** to start a request and store `Pick` in `self.pending`; move the work done after the pick into `handle_pick`. `App::update` calls `platform::poll()` once at the start of the frame and dispatches. `Cancelled` does nothing; `Failed(e)` → `self.note(LogLevel::Bad, format!("Could not … : {e}"))`.
  - Saves: `SaveTo(path)` → write the bytes with `files::write_atomic(path, …)`, then `platform::commit_save(id, path)`; keep the `Pick` until `Saved` (then mark the doc saved, set `doc.path` only if `rememberable(path)`; on Android `doc.path` stays `None` so the next Save is a Save as) or `Failed`.
  - `open_file`: `remember_recent` skipped for non-rememberable paths; a document opened from a cache path gets `path: None` and a title from the file stem, so Save acts as Save as. Last-session tabs (`open_file_paths`) skip non-rememberable paths too.
- [ ] **Step 4: Android folder rules** (`cfg!(target_os = "android")`), per the spec's section 2 table: library and tone folder buttons and "Show in Explorer" hidden, path shown read-only; HX Edit folder buttons and the catalog card's "Choose HX Edit folder…" hidden; backup-all and whole-pedal backup go straight to a new dated folder under `<library>/Pedal backups/` (no picker); NAM output goes to `tone_dir().join("From NAM")`; "Restore a whole-pedal backup" shows an in-app list of folders under `<library>/Pedal backups/` (and the private one, Task 3) to pick from; the self-test page stays hidden on Android (check it already is).
- [ ] **Step 5: Run** fmt, `cargo test --workspace`, clippy — clean. Desktop smoke on Windows: `cargo run`, use Open (cancel and pick), Save as, IR import dialog (cancel) — behaviour as before; note what was tried in the report.
- [ ] **Step 6: Commit** `Dialogs: every file dialog through the async picker; fixed folders on Android`.

### Task 3: Shared library folder, copying across, backups from both folders

**Files:**
- Modify: `src/files.rs` (or wherever `unique_path`/`write_atomic` live) for `copy_missing`; `src/window/mod.rs` (library switching, warning card), `src/window/backups.rs`, `src/window/restore_all.rs`, `src/window/settings.rs`, `src/platform.rs` (storage state hook)
- Test: unit tests next to `copy_missing` and in `src/window/backups.rs`

**Interfaces:**
- Consumes: Task 2's Android folder rules (backup folders under `library_dir`).
- Produces:
  - `pub fn copy_missing(from: &Path, to: &Path) -> std::io::Result<usize>` (recursive; copies each file whose relative path does not exist under `to`; creates folders; never overwrites; returns files copied).
  - `crate::platform`: `pub enum Storage { Private, Shared(PathBuf), Refused }`, `pub fn set_storage(s: Storage)` (called by the Android crate, any thread) and `pub fn storage() -> Storage`; `pub fn request_storage_permission()` calling a backend `set_permission_backend(Box<dyn Fn() + Send + Sync>)`. Desktop: `storage()` is always `Shared(<library from settings>)`-equivalent and the app ignores it (no behaviour change).
  - `pub fn set_android_api(v: u32)` / `pub fn android_api() -> u32` (desktop 0).
  - `App` field `private_library: Option<PathBuf>` (Android: the private `Library` path from `TONESTRAND_LIBRARY_PRIVATE` env var set by the Android crate) and `pub(super) fn backup_dirs(&self) -> Vec<PathBuf>` (shared `Pedal backups` first, then the private one if different and present).

- [ ] **Step 1: Write failing tests:** `copy_missing_never_overwrites` (temp dirs: `from/a.txt`="new", `from/sub/b.txt`, `to/a.txt`="old" → returns 1, `to/a.txt` still "old", `to/sub/b.txt` exists); `copy_missing_twice_copies_nothing_more` (second call returns 0); `backups_listed_from_both_folders_once` (a helper `fn scan_backup_dirs(dirs: &[PathBuf]) -> Vec<BackupFile>` merges `scan_backups` of each, de-duplicates by file name keeping the first folder's copy, newest first).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement** `copy_missing`, `scan_backup_dirs`; the backups page and `open_restore` and the restore-all folder list use `backup_dirs()`.
- [ ] **Step 4: Android library switching** in `App::update` (start of frame, only when `cfg!(target_os = "android")`): when `platform::storage()` is `Shared(dir)` and `library_dir != dir` → set `library_dir = dir`, run `copy_missing(private_library, dir)` (note the count in the status line if > 0: "Copied N files into Documents/ToneStrand"), rescan library/tones. When `Private`/`Refused` → warning card on the editor page and on the Pedal backups page with the exact text and button from Global Constraints (button → `platform::request_storage_permission()`). Settings shows the folder path, "Storage access: allowed / not allowed", and the Android 11+ note (exact text) when `platform::android_api() >= 30` (`pub fn android_api() -> u32`, set by the Android crate via `set_android_api(u32)`; desktop 0).
- [ ] **Step 5: Run** fmt/test/clippy — clean. Screenshot the Settings page and the editor with the warning card on desktop by forcing the Android branch is not possible; instead unit-test the decision helper `fn library_for(storage: &Storage, private: &Path) -> (PathBuf, bool /*warn*/)` (`Shared(d)` → `(d, false)`, `Private`/`Refused` → `(private, true)`).
- [ ] **Step 6: Commit** `Library: shared folder on Android, copy missing files across, backups from both folders`.

### Task 4: Android side: MainActivity, picker, permission, shared folder

**Files:**
- Create: `android/app/app/src/main/java/com/tonestrand/app/MainActivity.java`
- Modify: `android/app/app/src/main/AndroidManifest.xml`, `android/app/app/build.gradle` (only if Java needs wiring; the header comment "no Java code of our own" is updated), `android/src/lib.rs`
- Create: `android/src/files.rs` (JNI: picker backend, commit backend, native result callbacks, shared folder, permission, API level, cache dir)
- Modify: `docs/ANDROID.md` (new section "Files and the ToneStrand folder": picker, copy-through-cache, the folder, permission, Android 11+ limitation, how to check on the emulator)

**Interfaces:**
- Consumes: Task 1 (`set_backend`, `set_commit_backend`, `deliver`, `PickRequest`, `PickResult`), Task 2 (`set_cache_dir`), Task 3 (`set_storage`, `Storage`, `set_permission_backend`, `set_android_api`, env `TONESTRAND_LIBRARY_PRIVATE`).
- Java ↔ native contract (names fixed so both sides match):
  - Java instance methods called from Rust via JNI on the activity: `void pickOpen(long id, boolean multiple, String[] mimeTypes)`, `void pickSave(long id, String suggestedName)`, `void commitSave(long id, String tempPath)`, `void requestStorage()`, static-free helpers `String sharedDocumentsDir()`, `String cacheDir()`.
  - Native methods implemented in `android/src/files.rs`: `static native void nativePicked(long id, String[] paths)` (empty array = cancelled), `static native void nativeSaveTarget(long id)` (CREATE_DOCUMENT chosen; Rust replies with `deliver(id, SaveTo(temp))` where temp = `<cache>/save-<id>/<suggested name>`), `static native void nativeSaved(long id, String error /*null = ok*/)`, `static native void nativeStorageChanged(boolean granted)`.
  - Java copies picked documents on a background thread into `<cacheDir>/pick-<id>/<display name>` before calling `nativePicked`; `commitSave` copies the temp file to the remembered URI on a background thread, then `nativeSaved`. Request codes are per-request; the URI for a save is remembered by id. Cache folder emptied in `onCreate`.
  - MIME types: always `*/*` (Android does not know the app's extensions); the app re-checks extensions after picking (existing code already does for open; for imports, files with an unexpected extension are reported, not processed).

- [ ] **Step 1: Java activity + manifest** per Global Constraints; `onRequestPermissionsResult` → `nativeStorageChanged`; at start (`onCreate` after super) request `WRITE_EXTERNAL_STORAGE` only when API ≤ 29 and not granted.
- [ ] **Step 2: `android/src/files.rs`**: at start, before `run_native`: read `sharedDocumentsDir()` + "/ToneStrand", create it and test-write a file; set `TONESTRAND_LIBRARY` to it when usable else to the private `Library`; always set `TONESTRAND_LIBRARY_PRIVATE`; `set_storage`, `set_android_api`, `set_cache_dir`; install the picker, commit and permission backends (they need the JavaVM and activity: attach the current thread with `JavaVM::from_raw(app.vm_as_ptr())`, call on the activity object as `multicast.rs` does; clear pending exceptions). `nativeStorageChanged(true)` → re-test the folder → `set_storage(Shared(dir))`. Use `env.with_local_frame` and the exception-clearing pattern from `android/src/multicast.rs`.
- [ ] **Step 3: Build and check on the emulator** (`android\build.ps1`, emulator-5554): Open a `.hlx` pushed with `adb push <repo test hlx> /sdcard/Download/` → the preset opens; Save as → choose Documents → the file exists (`adb shell ls /sdcard/Documents`); cancel each picker → nothing happens; Settings shows `/storage/emulated/0/Documents/ToneStrand`; logcat has no JNI exceptions. Save screenshots under `target/android-check/files-*.png`.
- [ ] **Step 4: Docs** — `docs/ANDROID.md` section as listed under Files.
- [ ] **Step 5: Commit** `Android: system file picker, shared ToneStrand folder and storage permission`.

The owner then checks on the Lenovo (Android 10): permission prompt, folder created, a slot write's backup lands in `Documents/ToneStrand/Pedal backups`, Open/Save with Drive once signed in.
