@echo off
setlocal

set PFC_ARCH_VALUE=%PFC_ARCH%
if "%PFC_ARCH_VALUE%"=="" set PFC_ARCH_VALUE=x64

if not "%PFC_SKIP_EXE_BUILD%"=="1" (
  call build_exe.bat
  if errorlevel 1 exit /b %errorlevel%
)

set ISCC_CMD=
where ISCC >nul 2>nul
if not errorlevel 1 set ISCC_CMD=ISCC

if "%ISCC_CMD%"=="" if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set ISCC_CMD=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe
if "%ISCC_CMD%"=="" if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set ISCC_CMD=%ProgramFiles%\Inno Setup 6\ISCC.exe

if "%ISCC_CMD%"=="" (
  echo [ERROR] 未找到 Inno Setup 6 的 ISCC.exe，请先安装 Inno Setup 6。
  exit /b 1
)

"%ISCC_CMD%" /DMyAppArch=%PFC_ARCH_VALUE% /DMyOutputBaseFilename=ProjectFileCompare-Setup-%PFC_ARCH_VALUE% installer\ProjectFileCompare.iss
