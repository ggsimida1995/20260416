@echo off
setlocal

set PY_CMD=py
where py >nul 2>nul
if errorlevel 1 set PY_CMD=python

%PY_CMD% -m PyInstaller --noconfirm --clean ProjectFileCompare.spec
