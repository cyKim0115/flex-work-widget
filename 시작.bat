@echo off
chcp 65001 >nul
cd /d "%~dp0"

set "INSTALL=%LOCALAPPDATA%\FlexWorkWidget\flex-work-widget.exe"
set "RELEASE=%~dp0src-tauri\target\release\flex-work-widget.exe"

REM Release가 있으면 설치본을 갱신한 뒤 실행 (콘솔 없음).
if exist "%RELEASE%" (
  start "" powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0scripts\launch-user.ps1"
  exit /b 0
)

REM 로컬 release는 없지만 설치본만 있으면 그대로 실행.
if exist "%INSTALL%" (
  start "" "%INSTALL%"
  exit /b 0
)

REM 최초: 릴리스 빌드 후 설치·실행 (진행 로그용 콘솔).
echo First launch: building release (may take a few minutes)...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\launch-user.ps1"
if errorlevel 1 (
  echo.
  echo Failed to launch. See message above.
  pause
)
exit /b %errorlevel%
