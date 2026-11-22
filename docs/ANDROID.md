# Android app

The Android app is the desktop library (`tonestrand`, crate at the repo root) started inside a
GameActivity. `android/` is its own Cargo workspace, not a member of the root one, so nothing
Android-specific reaches the Windows build.

## Tools (all under `%LOCALAPPDATA%\Android`)

- `Sdk` (platform-tools, emulator, NDK 27.2.12479018, platform 35), `jdk17`, `gradle-8.9`.
- `cargo-ndk` (cargo install) and the rustup target `x86_64-linux-android` (and `aarch64-linux-android` for a real tablet).
- The rustup self-contained `dlltool` folder on `PATH` (`%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained`); `build.ps1` adds it.

## Build and run

    android\build.ps1 [-Abi x86_64|arm64-v8a] [-Device emulator-5554] [-NoInstall]

It runs `cargo ndk`, strips the library, runs Gradle (`android/app`), and (unless `-NoInstall`)
reinstalls and starts `com.tonestrand.app` (label "ToneStrand (test)"). The APK is
`android/app/app/build/outputs/apk/debug/app-debug.apk`, signed with the Gradle debug key.
It also removes the old spike app `com.tonestrand.spike` if present.

## Emulator

AVD `ToneStrand_Tablet` (2560x1600):
`%LOCALAPPDATA%\Android\Sdk\emulator\emulator.exe -avd ToneStrand_Tablet`. Check with
`adb devices`; wait for `adb shell getprop sys.boot_completed` = 1.

Back / Home check (screenshots go to `target/android-check/`):

    android\check-back.ps1

Start, screenshot; `adb shell input keyevent KEYCODE_BACK`, wait 3 s, start, screenshot;
`KEYCODE_HOME`, start, screenshot. All three must show the editor, not a black screen.

Caveat: on this emulator (API 35) Back on the root activity only moved the app to the background
(`am start` said "intent delivered to running instance"), so this script does NOT exercise the
exit path. A fresh process was exercised separately: `adb shell am force-stop com.tonestrand.app`,
start, screenshot (`4-after-force-stop.png`), and swipe-away from recents
(`KEYCODE_APP_SWITCH`, `input swipe 1280 900 1280 100 200`; the process was gone afterwards,
start gave a new pid and the editor, `6-after-swipe-away.png`). Whether the system killed the
process or the exit ran was not visible (no "window closed; exiting" log line was seen).

After the window closes the Android crate (`android/src/lib.rs`) logs "window closed; exiting" and
calls `std::process::exit(0)`, so the next launch gets a fresh process (winit cannot create a
second event loop in one process). As on desktop, `App::on_exit` saves settings and shuts the
device link down first, which can take up to 8 s.

`build.ps1` installs with `adb install -r`, so settings, Box pairing, backups and the library
survive a rebuild. `allowBackup` is off in the manifest: settings hold the Box pairing token and
Android cloud backup must not copy it.

## Finding the Box (multicast lock)

Android drops Wi-Fi multicast packets, which hides the Box's mDNS announcements. At start `android/src/multicast.rs` takes a `WifiManager.MulticastLock` (tag `tonestrand`, not reference counted) through JNI, using the `CHANGE_WIFI_MULTICAST_STATE` permission, and logs `multicast lock acquired` (or the error; start-up never fails because of it). The emulator can't see mDNS, so only the log line is checked there; finding the Box by name needs a real device on the same Wi-Fi.

## Files and the ToneStrand folder

The activity is `com.tonestrand.app.MainActivity` (extends GameActivity; Java in `android/app/app/src/main/java`). `android/src/files.rs` installs the `tonestrand::platform` backends that call it and implements its native methods (`Java_com_tonestrand_app_MainActivity_*`).

- Picker: Open and Save as use the system picker (`ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT`, type `*/*`; the app re-checks extensions). Only one picker at a time. Folder requests are answered "Not available on Android".
- Copy through the cache: a picked document is copied to `<cache>/pick-<id>/<display name>` on a background thread (duplicate names in one pick become `name (2).ext`), and the app gets that path. Save as writes the whole file to `<cache>/save-<id>/<name>` first, then Java copies it to the chosen document. Cache paths are never remembered (recent files, tabs). The cache is emptied at activity start; `cancel_pending()` is called then so a lost answer cannot block the pickers.
- Folder: `Environment.getExternalStoragePublicDirectory(DIRECTORY_DOCUMENTS)/ToneStrand` (on the emulator `/storage/emulated/0/Documents/ToneStrand`). At start it is created and test-written; if usable it is the library, else the private `Library` is used and the app shows the storage warning.
- Permission: `WRITE_EXTERNAL_STORAGE` (max API 29) is requested at start on Android 10 and older, `READ_EXTERNAL_STORAGE` max API 32, and `requestLegacyExternalStorage="true"`. The answer re-tests the folder and switches the library without a restart (refused gives the warning with the "Allow storage access" button).
- Android 11+ limitation: files put in the folder by other apps or by an earlier install are not visible to the app (Settings says so).

Check on the emulator (API 35): `android\build.ps1`; in the app, New preset, Save as, menu, pick the device, Documents, Save, then `adb shell ls /sdcard/Documents` shows the file; Open, Open file, pick it: the preset opens and `adb shell run-as com.tonestrand.app ls cache` shows `pick-<n>`; Back in either picker does nothing; Settings shows `/storage/emulated/0/Documents/ToneStrand`; `adb logcat -d -s tonestrand:*` has `shared folder ... is usable` and no errors. The permission prompt needs a real Android 10 device (not checked on the emulator).

## Launcher icon

`android\make-icons.ps1` (PowerShell and System.Drawing, no other tools) writes square and round PNGs for mdpi to xxxhdpi (48 to 192 px) into `android/app/app/src/main/res/mipmap-*` from `assets/logos/AppIcon.jpg`; the manifest sets `icon` and `roundIcon`. Re-run it after changing the picture.

## CI

Job `android` in `.github/workflows/ci.yml` (ubuntu-latest, `continue-on-error`): JDK 17, NDK 27.2.12479018, `cargo install cargo-ndk --locked`, `cargo ndk -t arm64-v8a -P 26 -o app/jniLibs --link-libcxx-shared build --release` in `android/`, Gradle 8.9 `assembleDebug` in `android/app`, and the debug APK is uploaded as the artifact `tonestrand-android-debug`. It has not been seen running yet (written without a way to run Actions).

## Pins and why

- `android/Cargo.lock` is seeded from the root `Cargo.lock`; `jobserver` is pinned to 0.1.32 and `winapi-util` to 0.1.9, because newer ones need an assembler this toolchain lacks (`cargo update -p jobserver --precise 0.1.32`, same for `winapi-util` with `0.1.9`).
- `androidx.games:games-activity:4.4.0`: the version android-activity 0.6.1 is built for. The prefab is not enabled.
- `androidx.appcompat:appcompat:1.7.0` and `platform('org.jetbrains.kotlin:kotlin-bom:1.8.22')` to avoid duplicate Kotlin classes.
- minSdk 26, compile/targetSdk 35, `useLegacyPackaging true` (libraries stored extractable).
- Gradle's `jniLibs.srcDirs = ['../jniLibs']` is `android/app/jniLibs`, where `cargo ndk -o` writes.

## What has been tried

- Emulator (x86_64, Android API 35 tablet): starts, shows the editor; Back then start again and Home then start again both show the editor (Back only backgrounded the app); after force-stop and after swipe-away from recents a fresh start shows the editor.
- Emulator: multicast lock acquired (log line); the launcher icon shows in the app drawer (`target/android-check/t8-drawer.png`).
- Real tablet: nothing yet. To check there: pairing and connecting to the Box by name (mDNS) and by address, loading a preset, turning a knob, changing a snapshot, typing a preset name on the on-screen keyboard, Back and reopen.
  - Insets on the tablet: top bar clear of the status bar, no blank band above the on-screen keyboard; note the tablet's Android version.
  - IR/NAM (tonelib) list: press, hold and move an item onto a folder.
  - Carry a preset onto another slot on the real pedal: the confirm appears, a backup is taken first (check the Pedal backups list), then the write.

## Colours

Checked on the emulator only: commit 7a44e96 plus the start-up log line below, release build, 2560x1600, host GPU (NVIDIA, through the emulator's layer that runs Android graphics on the PC's graphics card). Screenshots are in `target/android-check/t7-*.png` (not kept in git).

| Surface (pixel in the screenshot) | Dark: expected / read | Light: expected / read |
|---|---|---|
| Left preset panel (100,800) | #15161a / #15161a | (not read in Light) |
| Top bar (1500,60) | #15161a / #15161a | #f6f7fa / #f6f7fa |
| Card (50,300) dark; (1000,300) light | #1b1d22 / #1b1d22 | #ffffff / #ffffff |
| Background (1280,800) dark; (1900,600) light | #101114 / #101114 | #eceef3 / #eceef2 (1 step off) |

So the colours are right here and no patch was needed. The earlier spike saw about #404248 for #15161a. The cause of that washed look is not established: it was either a different graphics path or sample points that were not on flat surfaces.

What the emulator reports (one debug-level line at start-up in `android/src/lib.rs`; raise the logger level to `Debug` to see it): `OpenGL ES 3.1 (4.5.0 NVIDIA 621.24)`, extension `GL_EXT_sRGB_write_control` (lets a program switch the screen's automatic brightness-curve conversion on or off) not offered, `FRAMEBUFFER_SRGB` (0x8DB9, that conversion) off. From reading egui's code (not measured), egui only switches it on when the driver lists an extension ending in `ARB_framebuffer_sRGB`, which GLES drivers normally do not, so nothing should be converted twice here.

### Checking the real tablet (the graphics driver differs, so this must be done)

1. Build and install: `android\build.ps1 -Abi arm64-v8a -Device <serial>` from PowerShell (`adb devices` shows the serial).
2. Screenshot: `adb -s <serial> exec-out screencap -p > tablet-dark.png`.
3. Read a pixel by opening the PNG in Paint and using the colour picker, or in PowerShell: `Add-Type -AssemblyName System.Drawing; $c=[System.Drawing.Bitmap]::FromFile("C:\path\to\tablet-dark.png").GetPixel(100,800); '{0:x2}{1:x2}{2:x2}' -f $c.R,$c.G,$c.B`. Read the pixel at (100,800) (inside the left preset panel, with the Editor page open and the tablet in landscape at 2560x1600; scale the point for another size). It must be #15161a within 3 per channel; the background at (1280,800) must be #101114 within 3. A value near #404248 may mean a second conversion is happening.
4. Settings > Display > Light, screenshot again: top bar (1500,60) within 3 of #f6f7fa, background (1900,600) within 3 of #eceef3.
5. If off, try in this order, stopping at the first that fixes it:
   a. In `android/src/lib.rs`, before painting, turn `FRAMEBUFFER_SRGB` (0x8DB9) off through `frame.gl()` when the extension list has `GL_EXT_sRGB_write_control` (the start-up log line shows whether it does).
   b. A vendored eframe under `android/vendor/eframe`, changed only to ask for a plain (non-sRGB) window surface, wired through `[patch.crates-io]` in `android/Cargo.toml`. Never touch the root `Cargo.toml`.
   c. eframe's `wgpu` renderer, for the Android build only.
