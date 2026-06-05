@echo off
setlocal
chcp 65001 >nul
set PYTHONUTF8=1
set PYTHONIOENCODING=utf-8

pushd "%~dp0" || (
    echo [TinyVim] Failed to enter release folder:
    echo %~dp0
    pause
    exit /b 1
)

set "TINYVIM_EXE=%CD%\tinyvim-x86_64-windows.exe"
if not exist "%TINYVIM_EXE%" (
    echo [TinyVim] Cannot find tinyvim-x86_64-windows.exe.
    echo Keep tinyvim-windows.cmd and tinyvim-x86_64-windows.exe in the same folder.
    echo Current folder: %CD%
    popd
    pause
    exit /b 1
)

set "TINYVIM_ATTEMPT=1"
:run_tinyvim
"%TINYVIM_EXE%" %*
set "TINYVIM_EXIT=%ERRORLEVEL%"

if "%TINYVIM_EXIT%"=="5" if %TINYVIM_ATTEMPT% LSS 8 (
    echo.
    echo [TinyVim] Windows denied access while starting TinyVim. Retrying...
    echo [TinyVim] This is often Windows Security/antivirus scanning a new exe.
    timeout /t 1 /nobreak >nul
    set /a TINYVIM_ATTEMPT+=1
    goto run_tinyvim
)

popd

echo.
if "%TINYVIM_EXIT%"=="5" (
    echo [TinyVim] Windows still says: Access is denied.
    echo [TinyVim] Try moving this folder out of Downloads/OneDrive, or allow it in Windows Security.
    echo [TinyVim] Folder: %~dp0
) else if not "%TINYVIM_EXIT%"=="0" (
    echo [TinyVim] Exited with code %TINYVIM_EXIT%.
) else (
    echo [TinyVim] Exited.
)

if /i not "%TINYVIM_NO_PAUSE%"=="1" pause
exit /b %TINYVIM_EXIT%
