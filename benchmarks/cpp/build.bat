@echo off
REM Build script for Open_AlteryxYXDB benchmark
REM Requires Visual Studio Build Tools 2022

setlocal

REM Find Visual Studio Build Tools
set "VSDIR=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
if not exist "%VSDIR%" (
    set "VSDIR=C:\Program Files\Microsoft Visual Studio\2022\BuildTools"
)
if not exist "%VSDIR%" (
    set "VSDIR=C:\Program Files (x86)\Microsoft Visual Studio\2022\Community"
)
if not exist "%VSDIR%" (
    set "VSDIR=C:\Program Files\Microsoft Visual Studio\2022\Community"
)
if not exist "%VSDIR%" (
    echo ERROR: Visual Studio 2022 Build Tools not found
    exit /b 1
)

REM Set up MSVC environment
call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

set SRCDIR=%~dp0Open_AlteryxYXDB
set OUTDIR=%~dp0

echo Building Open_AlteryxYXDB benchmark...

cl.exe /nologo /EHsc /O2 /DUNICODE /D_CRT_SECURE_NO_WARNINGS /D_SCL_SECURE_NO_DEPRECATE ^
    /I"%SRCDIR%" /I"%SRCDIR%\RecordLib" /I"%SRCDIR%\liblzf-3.6" ^
    /Fe"%OUTDIR%open_yxdb_benchmark.exe" ^
    "%~dp0benchmark.cpp" ^
    "%SRCDIR%\Open_AlteryxYXDB.cpp" ^
    "%SRCDIR%\RecordLib\Record.cpp" ^
    "%SRCDIR%\RecordLib\FieldBase.cpp" ^
    "%SRCDIR%\liblzf-3.6\lzf_c.c" ^
    "%SRCDIR%\liblzf-3.6\lzf_d.c" ^
    /link /MACHINE:X64

if %ERRORLEVEL% neq 0 (
    echo ERROR: Build failed
    exit /b 1
)

echo Build successful: %OUTDIR%open_yxdb_benchmark.exe

REM Clean up .obj files
del /q "%~dp0*.obj" 2>nul

endlocal
