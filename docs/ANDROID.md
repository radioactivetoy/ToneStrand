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

## Pins and why

- `android/Cargo.lock` is seeded from the root `Cargo.lock`; `jobserver` is pinned to 0.1.32 and `winapi-util` to 0.1.9, because newer ones need an assembler this toolchain lacks (`cargo update -p jobserver --precise 0.1.32`, same for `winapi-util` with `0.1.9`).
- `androidx.games:games-activity:4.4.0`: the version android-activity 0.6.1 is built for. The prefab is not enabled.
- `androidx.appcompat:appcompat:1.7.0` and `platform('org.jetbrains.kotlin:kotlin-bom:1.8.22')` to avoid duplicate Kotlin classes.
- minSdk 26, compile/targetSdk 35, `useLegacyPackaging true` (libraries stored extractable).
- Gradle's `jniLibs.srcDirs = ['../jniLibs']` is `android/app/jniLibs`, where `cargo ndk -o` writes.

## What has been tried

- Emulator (x86_64, Android API 35 tablet): starts, shows the editor; Back then start again and Home then start again both show the editor (Back only backgrounded the app); after force-stop and after swipe-away from recents a fresh start shows the editor.
- Real tablet: nothing yet.

## Colours

Checked on the emulator only: commit 7a44e96 plus the start-up log line below, release build, 2560x1600, host GPU (NVIDIA, through the emulator's OpenGL ES translator). Screenshots are in `target/android-check/t7-*.png` (not kept in git).

| Surface (pixel in the screenshot) | Dark: expected / read | Light: expected / read |
|---|---|---|
| Left preset panel (100,800) | #15161a / #15161a | |
| Top bar (1500,60) | #15161a / #15161a | #f6f7fa / #f6f7fa |
| Card (50,300) | #1b1d22 / #1b1d22 | #ffffff / #ffffff |
| Background (1280,800) | #101114 / #101114 | #eceef3 / #eceef2 (1 step off) |

So the colours are right here and no patch was needed. The earlier spike saw about #404248 for #15161a. The cause of that washed look is not established: it was either a different graphics path or sample points that were not on flat surfaces.

What the emulator reports (one debug-level line at start-up in `android/src/lib.rs`; raise the logger level to `Debug` to see it): `OpenGL ES 3.1 (4.5.0 NVIDIA 621.24)`, extension `GL_EXT_sRGB_write_control` (lets a program switch the screen's automatic brightness-curve conversion on or off) not offered, `FRAMEBUFFER_SRGB` (0x8DB9, that conversion) off. egui does not turn it on when the extension named `ARB_framebuffer_sRGB` is absent, so nothing is converted twice here.

### Checking the real tablet (the graphics driver differs, so this must be done)

1. Build and install: `powershell androiduild.ps1 -Abi arm64-v8a -Device <serial>` (`adb devices` shows the serial).
2. Screenshot: `adb -s <serial> exec-out screencap -p > tablet-dark.png`.
3. Read the pixel at (100,800) (inside the left preset panel, with the Editor page open and the tablet in landscape at 2560x1600; scale the point for another size). It must be #15161a within 3 per channel; the background at (1280,800) must be #101114 within 3. A value near #404248 means a second conversion is happening.
4. Settings > Display > Light, screenshot again: top bar (1500,60) near #f6f7fa, background (1280,800) near #eceef3.
5. If off, try in this order, stopping at the first that fixes it:
   a. In `android/src/lib.rs`, before painting, turn `FRAMEBUFFER_SRGB` (0x8DB9) off through `frame.gl()` when the extension list has `GL_EXT_sRGB_write_control` (the start-up log line shows whether it does).
   b. A vendored eframe under `android/vendor/eframe`, changed only to ask for a plain (non-sRGB) window surface, wired through `[patch.crates-io]` in `android/Cargo.toml`. Never touch the root `Cargo.toml`.
   c. eframe's `wgpu` renderer, for the Android build only.
