@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0.."
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
if not exist "%MSVC_ROOT%\setup_x64.bat" (
  echo ERROR: MSVC toolchain not found: %MSVC_ROOT%\setup_x64.bat 1>&2
  exit /b 1
)
call "%MSVC_ROOT%\setup_x64.bat" >nul
where /q cl.exe
if errorlevel 1 exit /b 1
call "%~dp0cargo-msvc.bat" build --release
if errorlevel 1 exit /b 1
echo Built: %CD%\target\release\luma-desk.exe
echo Package with scripts\build-app.bat; output goes to dist\LumaDesk-GPUI\ and dist\gpui\installer\.
