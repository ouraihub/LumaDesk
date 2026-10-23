@echo off
setlocal
cd /d "%~dp0.."
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
if not exist "%MSVC_ROOT%\setup_x64.bat" exit /b 1
call "%MSVC_ROOT%\setup_x64.bat"
if errorlevel 1 exit /b 1
call cargo +1.99.0 test --release --locked --manifest-path ..\LumaKit\Cargo.toml --workspace
if errorlevel 1 exit /b %errorlevel%
call cargo +1.99.0 test --release --locked
exit /b %errorlevel%
