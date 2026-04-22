@echo off
setlocal

set PY_CMD=python
where python >nul 2>nul
if errorlevel 1 set PY_CMD=py

%PY_CMD% -m PyInstaller --noconfirm --clean ProjectFileCompare.spec
