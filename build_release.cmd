@echo off
setlocal
cd /d "%~dp0"
set "VERSION="
cargo build --release
if errorlevel 1 exit /b 1
for /f "usebackq tokens=3 delims= " %%V in (`findstr /b /c:"version = " Cargo.toml`) do set "VERSION=%%~V"
if not defined VERSION (
    echo Version introuvable dans Cargo.toml 1>&2
    exit /b 1
)
copy /Y "target\release\audiok.exe" "target\release\audiok_%VERSION%_win_x64.exe" >nul
echo target\release\audiok_%VERSION%_win_x64.exe
