@echo off
REM One-click build for Windows. Installs Rust if missing, then builds both programs.
where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust not found. Downloading rustup...
  powershell -NoProfile -Command "Invoke-WebRequest https://win.rustup.rs/x86_64 -OutFile rustup-init.exe"
  rustup-init.exe -y --default-toolchain stable
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
)
cargo build --release
if errorlevel 1 (
  echo.
  echo Build failed. If it complains about a linker, install "Build Tools for Visual Studio"
  echo with the "Desktop development with C++" workload, then run this again.
  pause
  exit /b 1
)
echo.
echo Done: target\release\DuneSaveManager-gui.exe
pause
