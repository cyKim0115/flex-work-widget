@echo off
chcp 65001 >nul
cd /d "%~dp0"

set "INSTALL=%LOCALAPPDATA%\FlexWorkWidget\flex-work-widget.exe"
set "RELEASE=%~dp0src-tauri\target\release\flex-work-widget.exe"

REM Already installed — start GUI with no console.
if exist "%INSTALL%" (
  start "" "%INSTALL%"
  exit /b 0
)

REM Release binary exists — copy/install then start (hidden helper).
if exist "%RELEASE%" (
  start "" powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0scripts\launch-user.ps1"
  exit /b 0
)

REM First launch: build once (console shows progress), then app starts and this window closes.
echo First launch: building release (may take a few minutes)...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\launch-user.ps1"
if errorlevel 1 (
  echo.
  echo Failed to launch. See message above.
  pause
)
exit /b %errorlevel%
