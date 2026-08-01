@echo off
setlocal EnableExtensions DisableDelayedExpansion
title Install PaperlessLLM

echo PaperlessLLM will be installed for your Windows user only.
echo No administrator permission is required.
echo.
choice /C YN /N /M "Create a Desktop shortcut in addition to the Start menu shortcut? [Y/N] "
if errorlevel 2 (
  set "DESKTOP_OPTION=-NoDesktopShortcut"
) else (
  set "DESKTOP_OPTION=-DesktopShortcut"
)

echo.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0Install-PaperlessLLM.ps1" %DESKTOP_OPTION%
set "RESULT=%ERRORLEVEL%"
echo.
if not "%RESULT%"=="0" (
  echo Installation failed. No PaperlessLLM user data was removed.
) else (
  echo You can now start PaperlessLLM from the Start menu.
)
pause
exit /b %RESULT%
