@echo off
setlocal
cd /d "%~dp0.."
if not exist "dist\LumaDesk\LumaDesk.exe" (
  echo Packaged target not found. Run scripts\build-app.bat first.
  pause
  exit /b 1
)
start "LumaDesk" "%CD%\dist\LumaDesk\LumaDesk.exe"
