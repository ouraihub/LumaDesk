@echo off
setlocal
cd /d "%~dp0.."
call "%~dp0cargo-msvc.bat" test --release --locked --manifest-path ..\LumaKit\Cargo.toml --workspace
if errorlevel 1 exit /b %errorlevel%
call "%~dp0cargo-msvc.bat" test --release --locked
exit /b %errorlevel%
