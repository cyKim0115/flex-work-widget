@echo off
chcp 65001 >nul
cd /d "%~dp0"
REM Dev mode (Vite + hot reload). Console stays open for logs.
powershell -NoProfile -ExecutionPolicy Bypass -Command "Set-Location '%~dp0'; if (-not (Test-Path node_modules)) { npm install }; npm run tauri -- dev"
