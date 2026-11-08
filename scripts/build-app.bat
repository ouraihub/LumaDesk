@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0.."
rem Keep batch files ASCII and CRLF: cmd can misparse UTF-8 comments.
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
if not exist "%MSVC_ROOT%\setup_x64.bat" (
  echo ERROR: MSVC toolchain not found: %MSVC_ROOT%\setup_x64.bat 1>&2
  goto fail
)
@call "%~dp0cargo-msvc.bat" --help >nul
if errorlevel 1 goto fail
set "POWERSHELL="
for /f "delims=" %%P in ('where pwsh.exe 2^>nul') do if not defined POWERSHELL set "POWERSHELL=%%P"
if not defined POWERSHELL set "POWERSHELL=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if not exist "%POWERSHELL%" set "POWERSHELL="
if not defined POWERSHELL (
  echo ERROR: PowerShell was not found. Install pwsh or Windows PowerShell. 1>&2
  goto fail
)
echo Using PowerShell: %POWERSHELL%
echo [1/3] Checking runtime and application...
@call "%~dp0check.bat"
if errorlevel 1 goto fail
echo [2/3] Building and collecting release artifacts...
@"%POWERSHELL%" -NoProfile -ExecutionPolicy Bypass -File "%~dp0release-artifacts.ps1"
if errorlevel 1 goto fail
echo [3/3] Verifying published artifacts...
@"%POWERSHELL%" -NoProfile -ExecutionPolicy Bypass -File "%~dp0release-artifacts.ps1" -Verify
if errorlevel 1 goto fail
echo Run: %CD%\dist\LumaDesk-GPUI\LumaDesk-GPUI.exe
echo Packages: %CD%\dist\gpui\installer\ ^(exact path in dist\gpui\release.json^)
exit /b 0

:fail
echo.
echo ERROR: LumaDesk packaging failed. Review the error above. 1>&2
echo Close any running LumaDesk-GPUI.exe before retrying. 1>&2
if /i not "%CI%"=="true" pause
exit /b 1
