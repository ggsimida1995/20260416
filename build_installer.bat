@echo off
setlocal

call build_exe.bat
if errorlevel 1 exit /b %errorlevel%

set ISCC_CMD=
where ISCC >nul 2>nul
if not errorlevel 1 set ISCC_CMD=ISCC

if "%ISCC_CMD%"=="" if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set ISCC_CMD=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe
if "%ISCC_CMD%"=="" if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set ISCC_CMD=%ProgramFiles%\Inno Setup 6\ISCC.exe

if "%ISCC_CMD%"=="" (
  echo [ERROR] 未找到 Inno Setup 6 的 ISCC.exe，请先安装 Inno Setup 6。
  exit /b 1
)

"%ISCC_CMD%" installer\ProjectFileCompare.iss
