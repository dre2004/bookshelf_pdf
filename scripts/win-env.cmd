@echo off
rem Sets up the MSVC build environment (compiler, linker, Windows SDK) and runs
rem the given command, e.g. `scripts\win-env.cmd cargo check`.
rem
rem vcvars64 only knows about SDKs registered at their install location; when
rem the Windows SDK lives on another drive (or is not registered), it is found
rem here by scanning the usual roots and appended to LIB/INCLUDE manually.

call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 exit /b 1

set "KITROOT="
set "KITVER="
for %%D in (C D E F) do (
    if exist "%%D:\Program Files (x86)\Windows Kits\10\Lib" (
        set "KITROOT=%%D:\Program Files (x86)\Windows Kits\10"
        for /f "delims=" %%V in ('dir /b /ad "%%D:\Program Files (x86)\Windows Kits\10\Lib" ^| sort') do set "KITVER=%%V"
    )
)

echo %LIB% | find /i "Windows Kits" >nul
if errorlevel 1 (
    if defined KITROOT (
        set "LIB=%LIB%;%KITROOT%\Lib\%KITVER%\um\x64;%KITROOT%\Lib\%KITVER%\ucrt\x64"
        set "INCLUDE=%INCLUDE%;%KITROOT%\Include\%KITVER%\ucrt;%KITROOT%\Include\%KITVER%\um;%KITROOT%\Include\%KITVER%\shared;%KITROOT%\Include\%KITVER%\winrt;%KITROOT%\Include\%KITVER%\cppwinrt"
        rem rc.exe (needed by tauri-winres to embed icons/manifests) ships in the SDK bin dir
        set "PATH=%PATH%;%KITROOT%\bin\%KITVER%\x64"
    )
)

%*
