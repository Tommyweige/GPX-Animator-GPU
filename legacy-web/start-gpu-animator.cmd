@echo off
setlocal
cd /d "%~dp0"

where node >nul 2>nul
if errorlevel 1 (
  echo [GPX Animator GPU] Node.js 20 or newer is required.
  echo Download: https://nodejs.org/
  pause
  exit /b 1
)

echo Starting GPX Animator GPU...
node server.mjs --open
if errorlevel 1 pause
