@echo off
setlocal
set "SCRIPT=%~dp0diagnose-environment.ps1"
if not exist "%SCRIPT%" (
  echo Missing diagnose-environment.ps1. Keep both files in the same folder.
  pause
  exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT%" %*
if errorlevel 1 (
  echo Collection failed. Please send the error message to the developer.
) else (
  echo Report saved as flair-environment-*.txt on your Desktop.
)
pause
