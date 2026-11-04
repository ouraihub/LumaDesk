@echo off
rem Forward cargo after injecting the portable MSVC environment.
rem Git ships a coreutils link.exe; probe cl.exe before running cargo.
rem Keep this file ASCII-only and without setlocal so setup_x64.bat changes persist.

set "MANIFEST=%~dp0..\Cargo.toml"
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"

where /q cl.exe && goto run
if not exist "%MSVC_ROOT%\setup_x64.bat" (
  echo ERROR: MSVC toolchain not found at %MSVC_ROOT%\setup_x64.bat 1>&2
  echo Set PORTABLE_MSVC_ROOT to the folder containing setup_x64.bat. 1>&2
  exit /b 1
)
call "%MSVC_ROOT%\setup_x64.bat" >nul
where /q cl.exe || (
  echo ERROR: cl.exe not found after injecting MSVC. 1>&2
  exit /b 1
)

:run
set "SUBCOMMAND=%1"
shift
set "ARGS="
set "HAS_MANIFEST="
:collect
if "%1"=="" goto dispatch
if /i "%1"=="--manifest-path" set "HAS_MANIFEST=1"
set "ARGS=%ARGS% %1"
shift
goto collect

:dispatch
if /i "%SUBCOMMAND%"=="--help" goto cargo_global
if /i "%SUBCOMMAND%"=="--version" goto cargo_global
if defined HAS_MANIFEST (
  cargo %SUBCOMMAND%%ARGS%
) else (
  cargo %SUBCOMMAND% --manifest-path "%MANIFEST%"%ARGS%
)
exit /b %ERRORLEVEL%

:cargo_global
cargo %SUBCOMMAND%%ARGS%
exit /b %ERRORLEVEL%
