@echo off
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust first: https://rustup.rs
    pause
    exit /b 1
)

echo Building release (first build fetches dependencies, may take minutes)...
cargo build --release
if errorlevel 1 (
    echo [ERROR] build failed.
    pause
    exit /b 1
)

echo.
echo Done. Output: target\release\chd-course-grabber.exe
pause
