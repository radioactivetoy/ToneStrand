# Android: file picker and a backups folder that survives uninstall

Builds on `docs/superpowers/specs/2026-11-19-android-app-design.md` (section 6, phases 4 and 6). This
change pulls forward the part of those phases the owner asked for first.

## Intent

On Android every file dialog silently returns nothing today (`src/platform.rs`), so Open, Save as, Make
.hlx, import, IR upload and backup export do nothing. Pedal backups are written, but only to the app's
private storage: an uninstall or "clear data" deletes them, and the owner cannot reach them. The
project's first rule is never to risk the user's presets, so backups must live where the user can reach
them and where they outlive the app. Cloud (Google Drive, OneDrive) is wanted where it is possible.

## Goals

1. Every Open / Save / Import / Export on Android works through the Android system picker, which also
   offers Google Drive and OneDrive for single files when their apps are installed and signed in.
2. The library and the pedal backups live in a shared folder on the tablet, `Documents/ToneStrand`,
   which survives an uninstall and can be reached from the PC over USB or mirrored to the cloud by a sync
   app.
3. The Windows app behaves as today.

## Non-goals (for now)

- Picking a cloud *folder* as the library (Storage Access Framework tree on Drive / OneDrive). Test on
  the owner's tablet on 2026-11-19 was inconclusive: no Google account was signed in and OneDrive is not
  installed. Re-test when both are signed in; if a cloud folder can be picked, that is a later phase.
- ToneStrand's own Drive / OneDrive sign-in and cloud APIs.
- USB, phone layout, NAM, Command Center, self-test (unchanged plan).

## Anti-goals

- A pedal write that goes ahead without a backup the user can find afterwards (the backups page must say
  plainly when backups are only in private storage).
- Overwriting or truncating a file the user picked when the save fails half-way (write the whole content
  first, then copy).
- Any change to how the Windows app's dialogs look or what they return.

## Constraints

- The tablet: Lenovo Tab M10 FHD Plus, Android 10 (API 29). The app: minSdk 26, targetSdk 35.
- Android's picker returns a document link (`content://` URI), not a file path; the app's code works on
  `PathBuf` everywhere (19 call sites of `platform::FileDialog`).
- The picker is a separate activity: its answer arrives later (activity result), while the egui frame
  loop keeps running. Blocking the render thread while it is open is not acceptable (the window's
  surface is destroyed and recreated around the picker).
- Shared storage rules: on Android 10 (API 29) plain file access to `Documents/` needs
  `WRITE_EXTERNAL_STORAGE` (runtime permission) plus `requestLegacyExternalStorage="true"`. On Android
  11+ an app may create and read back its *own* files in `Documents/` by path with no permission, but it
  cannot read files created by another app (a sync app) or by its own earlier install. Android 8–9:
  `WRITE_EXTERNAL_STORAGE` runtime permission, plain paths.
- `android/` stays its own Cargo workspace; nothing Android-specific in the root `Cargo.toml` beyond the
  existing `cfg(target_os = "android")` pattern.

## Approaches considered

1. **Shared folder by plain path + system picker with copy-through-temp (picked by the owner).** The
   library and backups move to `Documents/ToneStrand`; all existing `std::fs` code keeps working. Picked
   documents are copied into a temporary file before the existing code opens them; saves are written to a
   temporary file and then copied to the chosen document.
2. **Any folder the user picks, through Android's document API.** Every library and backup operation
   goes through a storage trait backed by `DocumentsContract` calls. Works with cloud folders if their
   providers allow folder picking (unknown). Much more work; deferred until the cloud-folder test says it
   would pay off.
3. **Keep everything private, with export buttons only.** Rejected: backups would still vanish on
   uninstall unless the user remembers to export.

## Design

### 1. The picker (async on both platforms)

- `platform` gets an asynchronous picker API. A call site starts a request with a purpose and gets the
  result in a later frame:
  - `FileDialog` keeps its builder (`add_filter`, `set_directory`, `set_title`, `set_file_name`).
  - New terminal calls `open_file(purpose)`, `open_files(purpose)`, `save_file(purpose, write: impl
    FnOnce(&Path) -> io::Result<()>)` (or equivalent: the app writes to a temp path it is given, the
    platform delivers it to the chosen place), and `pick_folder(purpose)`.
  - `App` polls finished requests at the start of each frame and dispatches on the purpose (an enum, one
    variant per current call site: OpenPreset, ComparePick(side), ImportLibrary, ImportTones,
    UploadIrs, NamPick, SaveAs, SavePedalPreset, SaveLog, …).
  - Desktop: the same API, completed immediately with `rfd`'s blocking dialogs, so both platforms take
    the same path through the app. The visible behaviour on Windows is unchanged (the result is handled
    in the same or the next frame).
- Android side:
  - A small Java activity, `com.tonestrand.app.MainActivity extends GameActivity`, replaces
    `com.google.androidgamesdk.GameActivity` in the manifest. It starts `ACTION_OPEN_DOCUMENT`
    (single/multiple, MIME types from the filters, falling back to `*/*` because Android does not know
    `.hlx` / `.hxpreset` / `.hxb` / `.hls` / `.nam`) and `ACTION_CREATE_DOCUMENT` (suggested name from
    `set_file_name`).
  - On a result, Java copies each picked document into the app's cache folder under a per-request
    subfolder, keeping its display name (`OpenableColumns.DISPLAY_NAME`), on a background thread, then
    reports the paths to native code (a JNI native method or a queue the Rust side polls).
  - For a save, Rust first writes the whole content to a temp file; after `ACTION_CREATE_DOCUMENT`
    returns, Java copies it to the URI and reports success or the error. Cancel or failure leaves the
    temp file and reports "not saved".
  - Temp files are deleted when the request is handled (opens) or after the copy (saves); the cache
    folder is cleared at start-up.
- File names: the existing code uses the picked file's name and extension (e.g. to tell `.hlx` from
  `.hxpreset`), which the display-name rule preserves.
- Opened files are copies: "Save" on a document opened from the picker cannot write back to the original
  document. On Android such a document behaves like a new one: Save asks where to save (Save as). Builder
  checks how `open_file` records the document's origin and makes "Save" fall back to Save as for temp
  copies. Paths in the app's cache folder are never stored in settings (recent files, last folders),
  since the cache is cleared at start-up.
- *(Designer's call)* **Call sites:** turning each `if let Some(x) = FileDialog…pick()` into a request
  plus a later handler is a mechanical refactor, one purpose variant per site; work done right after the
  pick moves into the handler. Two-step flows (NAM convert: pick captures, then an output folder) chain
  a second request on desktop; on Android the second step is the fixed folder from section 2.
- Java code: `android/` has no Java source today. The plan adds `android/app/app/src/main/java/…`
  (`MainActivity`, the picker and copy code, the storage-permission request) and any Gradle wiring it
  needs; the manifest's activity name and `android.app.lib_name` meta-data move to `MainActivity`.

### 2. Folder dialogs on Android

Android's picker can return a folder only as a document tree, not as a path, so the `pick_folder` call
sites change on Android:

| Call site | Android behaviour |
|---|---|
| Library folder (settings) | Fixed to the ToneStrand folder; the setting shows the path, read-only. |
| Tone (IR/NAM) folder (settings) | Fixed to the existing default `<library>/IRs and NAM` (`App::tone_dir`), i.e. `<ToneStrand>/IRs and NAM`; read-only. |
| HX Edit folder (settings, catalog card) | Hidden (the built-in effects list is used). |
| Back up all presets / whole-pedal backup destination | A new dated folder under `<ToneStrand>/Pedal backups/`, named as the desktop names them; the folder name is shown in the status line. |
| Restore a whole-pedal backup | An in-app list of the backup folders found under `<ToneStrand>/Pedal backups/` (and the private backups folder, if it holds any). |
| NAM output folder | `<ToneStrand>/IRs and NAM/From NAM` (designer's call). |
| Self-test folder | The self-test page stays hidden on Android. |

### 3. The ToneStrand folder

- Path: `Environment.getExternalStoragePublicDirectory(DIRECTORY_DOCUMENTS)/ToneStrand`, read through
  JNI at start-up (not hard-coded).
- Android 10 and lower: request `WRITE_EXTERNAL_STORAGE` at start-up (and `READ_EXTERNAL_STORAGE`);
  manifest gets `requestLegacyExternalStorage="true"` and
  `WRITE_EXTERNAL_STORAGE` with `maxSdkVersion="29"`.
- If the folder is usable (created, a test file written and deleted), `TONESTRAND_LIBRARY` points at it
  instead of the private `Library`. If not (permission refused, storage missing), the app keeps the
  private `Library` and shows a warning card on the editor page and in Settings: "Backups are only kept
  inside the app and are lost if it is uninstalled. Allow storage access to keep them in
  Documents/ToneStrand." with a button that asks again (or opens the app's settings page when Android
  will not ask again).
- How the library folder is chosen: Android keeps setting `TONESTRAND_LIBRARY` (which wins over
  `settings.library` in `App::new`, src/window/mod.rs:273); a `settings.library` saved earlier is ignored
  on Android.
- *(Designer's call)* **Permission and start-up order:** start-up never waits for the permission answer.
  `App::new` starts with the shared folder when it is already usable, else with the private `Library`
  and the warning card. The permission request is made by `MainActivity` at start-up; when the answer
  arrives (granted later, or through the card's button), the app switches `library_dir` to the shared
  folder at the start of the next frame and runs the copy below. Switching is safe between operations:
  the device thread receives the backup folder with each command. Every start re-checks, so a refusal
  is not permanent.
- *(Designer's call)* **Copying existing data is idempotent by name, not tracked by a marker:**
  whenever the shared folder is usable and the private `Library` holds files, each private file whose
  relative path does not exist in the shared folder is copied across; existing files are never
  overwritten or deleted. This survives "clear data" and reinstalls (settings are private and would lose
  a marker). The private copy stays; pedal backups are kept twice.
- *(Designer's call)* The Pedal backups page and the whole-pedal restore list read both the shared
  `Pedal backups` folder and the private one, de-duplicated by file name, so a backup made while
  permission was refused is still listed.
- Android 11+ limitation, stated in Settings next to the path: files put into the folder by another app
  (a sync app, a PC over USB) or by an earlier install are not visible to ToneStrand on Android 11 and
  newer. Lifting it needs either "All files access" (`MANAGE_EXTERNAL_STORAGE`; fine for an APK the
  owner installs, restricted on Google Play) or approach 2. Decision deferred to the owner (see "Left to
  decide"). Not an issue on the owner's Android 10 tablet.

### 4. Backups

- Pedal slot backups and whole-pedal backups keep their names and folder layout
  (`<library>/Pedal backups/…`), so they land in the ToneStrand folder when it is usable.
- The rule "back up before writing" is unchanged; if the backup cannot be written the write is refused
  as today.
- The Pedal backups page shows where backups are kept and the warning above when that is private
  storage. It gets an "Export…" action on a backup file (uses the save picker, so Drive / OneDrive work
  for single files).

### 5. Settings page on Android

Shows the ToneStrand folder path, whether storage access is granted, the Android 11+ note when it
applies, and hides the folder-choice buttons and "Show in Explorer".

## Verification

- Windows: `cargo fmt`, `cargo test --workspace`, `cargo clippy -- -D warnings`; every desktop dialog
  still opens and returns what it did (owner checks Open, Save as, import, IR upload, backup folder).
- Android emulator (API 35) and the owner's tablet (API 29): open a `.hlx` from Downloads, Save as to
  Documents, import a ToneLib file, export a backup, cancel each picker (nothing happens, no error),
  permission refused path (warning shows, backups still made privately), first-run move of the private
  Library.
- With Drive signed in on the tablet: open and save a single preset on Drive. If OneDrive is installed:
  same.
- Pedal writes on the tablet: a slot write makes its backup in `Documents/ToneStrand/Pedal backups`
  (self-test on a spare slot is desktop-only; the owner writes to a spare slot by hand and cancels
  nothing else).

## Left to the builder

- The exact async API shape and the purpose enum; whether the Java side copies or hands the `content://`
  URI to Rust to copy through JNI (Java copy preferred: simpler stream handling).
- How native code learns of results (native method callback into a `Mutex<VecDeque>` or polling a
  Java-side queue through JNI each frame).
- MIME types per filter.

## Left to decide (owner)

- Android 11+ visibility of files from other apps: "All files access" for the APK, approach 2, or accept
  the limitation for now. Only matters on a tablet newer than the owner's.
