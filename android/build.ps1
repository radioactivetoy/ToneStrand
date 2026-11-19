# Builds the Android app (cargo-ndk, strip, Gradle), installs it and starts it.
param([string]$Abi = 'x86_64', [string]$Device = 'emulator-5554', [switch]$NoInstall)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$sdk = "$env:LOCALAPPDATA\Android\Sdk"; $jdk = "$env:LOCALAPPDATA\Android\jdk17"
$env:ANDROID_HOME = $sdk; $env:ANDROID_NDK_HOME = "$sdk\ndk\27.2.12479018"; $env:JAVA_HOME = $jdk
$env:PATH = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;$jdk\bin;$env:PATH"
Remove-Item app/jniLibs -Recurse -Force -ErrorAction SilentlyContinue
cargo ndk -t $Abi -P 26 -o app/jniLibs --link-libcxx-shared build --release
if ($LASTEXITCODE) { throw 'cargo ndk failed' }
$strip = Get-ChildItem "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt" -Recurse -Filter llvm-strip.exe | Select-Object -First 1
& $strip.FullName --strip-unneeded "app/jniLibs/$Abi/libtonestrand_android.so"
if ($LASTEXITCODE) { throw "strip failed" }
"sdk.dir=" + ($sdk.Replace('\', '/')) | Set-Content app\local.properties
Push-Location app
& "$env:LOCALAPPDATA\Android\gradle-8.9\bin\gradle.bat" --no-daemon -q assembleDebug
$failed = $LASTEXITCODE; Pop-Location
if ($failed) { throw 'gradle failed' }
$apk = 'app\app\build\outputs\apk\debug\app-debug.apk'
"APK {0:N1} MB: {1}" -f ((Get-Item $apk).Length / 1MB), (Resolve-Path $apk)
if ($NoInstall) { return }
$adb = "$sdk\platform-tools\adb.exe"
& $adb -s $Device uninstall com.tonestrand.spike | Out-Null   # the old test app, if still there
& $adb -s $Device uninstall com.tonestrand.app | Out-Null
& $adb -s $Device install $apk
if ($LASTEXITCODE) { throw 'install failed' }
& $adb -s $Device shell am start -n com.tonestrand.app/com.google.androidgamesdk.GameActivity
