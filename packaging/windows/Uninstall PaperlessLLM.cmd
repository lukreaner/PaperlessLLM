@echo off
setlocal EnableExtensions DisableDelayedExpansion
title Uninstall PaperlessLLM

echo This removes the PaperlessLLM application and its shortcuts.
echo Your database, settings, logs, and Windows Credential Manager entries will be preserved.
echo.
choice /C YN /N /M "Continue with uninstall? [Y/N] "
if errorlevel 2 exit /b 0

echo.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0Uninstall-PaperlessLLM.ps1"
set "RESULT=%ERRORLEVEL%"
if not "%RESULT%"=="0" (
  echo.
  echo Uninstall failed. No PaperlessLLM user data was removed.
  pause
)
exit /b %RESULT%
