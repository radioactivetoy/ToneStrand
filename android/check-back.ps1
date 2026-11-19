# Emulator check: start, Back then start again, Home then start again; a screenshot each time.
param([string]$Device = 'emulator-5554', [string]$Out = "$PSScriptRoot\..\target\android-check")
$ErrorActionPreference = 'Stop'
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
function Start-App { & $adb -s $Device shell am start -n com.tonestrand.app/com.google.androidgamesdk.GameActivity | Out-Null; Start-Sleep 6 }
function Shot($name) { cmd /c "`"$adb`" -s $Device exec-out screencap -p > `"$Out\$name.png`"" }
Start-App; Shot '1-start'
& $adb -s $Device shell input keyevent KEYCODE_BACK; Start-Sleep 3
Start-App; Shot '2-after-back'
& $adb -s $Device shell input keyevent KEYCODE_HOME; Start-Sleep 2
Start-App; Shot '3-after-home'
