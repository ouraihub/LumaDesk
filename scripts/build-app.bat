@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0.."
rem Keep batch files ASCII and CRLF: cmd can misparse UTF-8 comments.
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
if not exist "%MSVC_ROOT%\setup_x64.bat" (
  echo ERROR: MSVC toolchain not found: %MSVC_ROOT%\setup_x64.bat 1>&2
  exit /b 1
)
call "%MSVC_ROOT%\setup_x64.bat"
if errorlevel 1 exit /b 1
echo [1/3] Checking runtime and application...
call "%~dp0check.bat"
if errorlevel 1 exit /b 1
echo [2/3] Building and collecting release artifacts...
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0release-artifacts.ps1"
if errorlevel 1 exit /b 1
echo [3/3] Verifying published artifacts...
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0release-artifacts.ps1" -Verify
if errorlevel 1 exit /b 1
echo Run: %CD%\dist\LumaDesk\LumaDesk.exe
echo Packages: %CD%\dist\installer\ ^(exact path in dist\release.json^)
exit /b 0
