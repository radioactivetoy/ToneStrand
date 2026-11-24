# Builds the Android app (cargo-ndk, strip, Gradle), installs it and starts it.
# -Release builds the release variant (label "ToneStrand"): signed when ANDROID_KEYSTORE_FILE (a path), ANDROID_KEYSTORE_PASSWORD,
# ANDROID_KEY_ALIAS and ANDROID_KEY_PASSWORD are all set, otherwise unsigned. -Version and -VersionCode set versionName and versionCode.
param([string]$Abi = 'x86_64', [string]$Device = 'emulator-5554', [switch]$NoInstall, [switch]$Release, [string]$Version, [int]$VersionCode)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$sdk = "$env:LOCALAPPDATA\Android\Sdk"; $jdk = "$env:LOCALAPPDATA\Android\jdk17"
$env:ANDROID_HOME = $sdk; $env:ANDROID_NDK_HOME = "$sdk\ndk\27.2.12479018"; $env:JAVA_HOME = $jdk
$env:PATH = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;$jdk\bin;$env:PATH"
Remove-Item app/jniLibs -Recurse -Force -ErrorAction SilentlyContinue
cargo ndk -t $Abi -P 26 -o app/jniLibs --link-libcxx-shared build --release
if ($LASTEXITCODE) { throw 'cargo ndk failed' }
$strip = Get-ChildItem "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt" -Recurse -Filter llvm-strip.exe | Select-Object -First 1
if (-not $strip) { throw "llvm-strip.exe not found under $env:ANDROID_NDK_HOME" }
& $strip.FullName --strip-unneeded "app/jniLibs/$Abi/libtonestrand_android.so"
if ($LASTEXITCODE) { throw "strip failed" }
"sdk.dir=" + ($sdk.Replace('\', '/')) | Set-Content app\local.properties
$task = if ($Release) { 'assembleRelease' } else { 'assembleDebug' }
$gradleArgs = @('--no-daemon', '-q', $task)
if ($Version) { $gradleArgs += "-PappVersion=$Version" }
if ($VersionCode) { $gradleArgs += "-PappVersionCode=$VersionCode" }
$signed = $false
if ($Release) {
    $signed = [bool]($env:ANDROID_KEYSTORE_FILE -and $env:ANDROID_KEYSTORE_PASSWORD -and $env:ANDROID_KEY_ALIAS -and $env:ANDROID_KEY_PASSWORD)
    if (-not $signed) { Write-Warning 'Release signing variables are not all set (ANDROID_KEYSTORE_FILE, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD): building an UNSIGNED release APK.' }
}
Push-Location app
& "$env:LOCALAPPDATA\Android\gradle-8.9\bin\gradle.bat" @gradleArgs
$failed = $LASTEXITCODE; Pop-Location
if ($failed) { throw 'gradle failed' }
$apk = if (-not $Release) { 'app\app\build\outputs\apk\debug\app-debug.apk' }
       elseif ($signed) { 'app\app\build\outputs\apk\release\app-release.apk' }
       else { 'app\app\build\outputs\apk\release\app-release-unsigned.apk' }
"APK {0:N1} MB: {1}" -f ((Get-Item $apk).Length / 1MB), (Resolve-Path $apk)
if ($NoInstall) { return }
if ($Release -and -not $signed) { Write-Warning 'An unsigned APK cannot be installed; not installing.'; return }
$adb = "$sdk\platform-tools\adb.exe"
# The old spike app, only if it is there. The new app is updated in place (-r) so its settings, pairing and library stay.
if (& $adb -s $Device shell pm list packages com.tonestrand.spike) { & $adb -s $Device uninstall com.tonestrand.spike | Out-Null }
& $adb -s $Device install -r $apk
if ($LASTEXITCODE) { throw 'install failed' }
& $adb -s $Device shell am start -n com.tonestrand.app/com.tonestrand.app.MainActivity
