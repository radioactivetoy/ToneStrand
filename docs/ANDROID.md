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

On the emulator (host GPU through the Android Emulator OpenGL ES Translator, NVIDIA) the theme colours come out exact, dark and light: `theme::panel()` #15161a reads #15161a, `card_bg()` #1b1d22 reads #1b1d22, and the light palette likewise (checked by screenshot). No eframe patch or framebuffer change was needed. The earlier spike's "too light" colours (#404248) were not reproduced. This is a GPU-path property (the sRGB write-control and EGL surface handling differ between GPU drivers), so **the real tablet must be checked too**: compare a flat panel pixel in a screenshot with #15161a. If it is lighter, the fix to try is disabling `GL_FRAMEBUFFER_SRGB_EXT` (where `EXT_sRGB_write_control` exists) before painting, or a linear EGL surface.
