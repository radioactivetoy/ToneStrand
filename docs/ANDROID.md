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
After the window closes the library calls `std::process::exit(0)` so the next launch gets a
fresh process (winit cannot create a second event loop in one process).

## Pins and why

- `android/Cargo.lock` is seeded from the root `Cargo.lock`; `jobserver` is pinned to 0.1.32 and `winapi-util` to 0.1.9, because newer ones need an assembler this toolchain lacks (`cargo update -p jobserver --precise 0.1.32`, same for `winapi-util` with `0.1.9`).
- `androidx.games:games-activity:4.4.0`: the version android-activity 0.6.1 is built for. The prefab is not enabled.
- `androidx.appcompat:appcompat:1.7.0` and `platform('org.jetbrains.kotlin:kotlin-bom:1.8.22')` to avoid duplicate Kotlin classes.
- minSdk 26, compile/targetSdk 35, `useLegacyPackaging true` (libraries stored extractable).
- Gradle's `jniLibs.srcDirs = ['../jniLibs']` is `android/app/jniLibs`, where `cargo ndk -o` writes.

## What has been tried

- Emulator (x86_64, Android API 35 tablet): starts, shows the editor; Back then start again and Home then start again both show the editor.
- Real tablet: nothing yet.
