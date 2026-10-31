@echo off
setlocal EnableExtensions EnableDelayedExpansion
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

echo !LIB! | find /i "Windows Kits" >nul
if errorlevel 1 (
    if defined KITROOT (
        rem The ort (ONNX Runtime) crate lists DXCORE/DirectML libs on the
        rem link line unconditionally; SDKs older than 10.0.19041 don't ship
        rem them. Nothing references their symbols here, so empty stubs
        rem satisfy the linker. Generated under target\, never committed.
        set "UMLIB=!KITROOT!\Lib\!KITVER!\um\x64"
        set "STUBDIR=%~dp0..\src-tauri\target\win-sdk-stubs"
        if not exist "!STUBDIR!" mkdir "!STUBDIR!"
        set "EXTRALIBS=!UMLIB!;!KITROOT!\Lib\!KITVER!\ucrt\x64"
        if not exist "!UMLIB!\DXCORE.lib" (
            if not exist "!STUBDIR!\DXCORE.lib" (
                > "!STUBDIR!\dxcore.def" echo LIBRARY DXCORE
                lib /NOLOGO /MACHINE:X64 /DEF:"!STUBDIR!\dxcore.def" /OUT:"!STUBDIR!\DXCORE.lib" >nul 2>&1
            )
            set "EXTRALIBS=!EXTRALIBS!;!STUBDIR!"
        )
        if not exist "!UMLIB!\DirectML.lib" (
            rem Symbols ARE needed (the prebuilt ONNX Runtime ships with the
            rem DirectML provider compiled in): fetch the real import lib
            rem from Microsoft's DirectML NuGet. The runtime DLL is an OS
            rem component on Windows 10 1903+.
            if not exist "!STUBDIR!\DirectML.lib" (
                curl -fsSL -o "!STUBDIR!\dml.zip" https://www.nuget.org/api/v2/package/microsoft.ai.directml
                %SystemRoot%\System32\tar.exe -xf "!STUBDIR!\dml.zip" -C "!STUBDIR!" bin/x64-win/DirectML.lib
                move /y "!STUBDIR!\bin\x64-win\DirectML.lib" "!STUBDIR!\DirectML.lib" >nul
                rmdir /s /q "!STUBDIR!\bin" >nul 2>&1
                del "!STUBDIR!\dml.zip" >nul 2>&1
            )
            set "EXTRALIBS=!EXTRALIBS!;!STUBDIR!"
        )
        rem Delayed expansion (!VAR!) is required throughout this block:
        rem %VAR% would expand once at parse time and later assignments
        rem would silently drop these paths.
        set "LIB=!LIB!;!EXTRALIBS!"
        set "INCLUDE=!INCLUDE!;!KITROOT!\Include\!KITVER!\ucrt;!KITROOT!\Include\!KITVER!\um;!KITROOT!\Include\!KITVER!\shared;!KITROOT!\Include\!KITVER!\winrt;!KITROOT!\Include\!KITVER!\cppwinrt"
        rem rc.exe (needed by tauri-winres to embed icons/manifests) ships in the SDK bin dir
        set "PATH=!PATH!;!KITROOT!\bin\!KITVER!\x64"
    )
)

%*
