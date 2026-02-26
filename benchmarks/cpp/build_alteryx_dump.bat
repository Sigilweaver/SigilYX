@echo off
REM Build script for Alteryx OpenYXDB dump tool using CMake + MSVC
REM Requires Visual Studio 2022 Build Tools

setlocal

REM Try BuildTools first, then Community, then Professional, then Enterprise
set "VCVARS="
for %%E in (BuildTools Community Professional Enterprise) do (
    if not defined VCVARS (
        if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat" (
            set "VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat"
        )
        if exist "C:\Program Files\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat" (
            set "VCVARS=C:\Program Files\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat"
        )
    )
)

REM Try to find cmake from VS or PATH
set "CMAKE="
where cmake >nul 2>&1 && set "CMAKE=cmake"
if not defined CMAKE (
    for %%E in (BuildTools Community Professional Enterprise) do (
        if not defined CMAKE (
            if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\%%E\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" (
                set "CMAKE=C:\Program Files (x86)\Microsoft Visual Studio\2022\%%E\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
            )
            if exist "C:\Program Files\Microsoft Visual Studio\2022\%%E\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" (
                set "CMAKE=C:\Program Files\Microsoft Visual Studio\2022\%%E\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
            )
        )
    )
)

set "SRCDIR=%~dp0AlteryxOpenYXDB"
set "BUILDDIR=%~dp0alteryx_build"
set "OUTDIR=%~dp0"

if not defined VCVARS (
    echo ERROR: Cannot find vcvars64.bat — install Visual Studio 2022 Build Tools, Community, Professional, or Enterprise
    exit /b 1
)
if not defined CMAKE (
    echo WARNING: Cannot find cmake.exe — library rebuild will not be possible
)
if not exist "%SRCDIR%\CMakeLists.txt" (
    echo ERROR: AlteryxOpenYXDB not found. Clone it first:
    echo   git clone https://github.com/alteryx/OpenYXDB.git benchmarks/cpp/AlteryxOpenYXDB
    exit /b 1
)

echo Setting up MSVC environment...
call "%VCVARS%" >nul 2>&1

REM Only rebuild library if not already built
if not exist "%BUILDDIR%\Open_AlteryxYXDB.lib" (
    echo Building Alteryx OpenYXDB library with CMake...
    if not exist "%BUILDDIR%" mkdir "%BUILDDIR%"

    "%CMAKE%" -S "%SRCDIR%" -B "%BUILDDIR%" -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release -DBUILDING_OPEN_ALTERYX=ON -DCMAKE_CXX_FLAGS="/wd4100" 2>&1
    if errorlevel 1 (
        echo ERROR: CMake configure failed
        exit /b 1
    )

    "%CMAKE%" --build "%BUILDDIR%" --config Release 2>&1
    if errorlevel 1 (
        echo ERROR: CMake build failed
        exit /b 1
    )
) else (
    echo Alteryx OpenYXDB library already built, skipping...
)

echo.
echo Building dump executable...
cl.exe /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /DSRCLIB_REPLACEMENT /DBUILDING_OPEN_ALTERYX /D_CRT_SECURE_NO_WARNINGS /wd4100 /wd4267 /wd4244 /wd4458 ^
    /I"%SRCDIR%\include" ^
    /Fe"%OUTDIR%alteryx_openyxdb_dump.exe" ^
    "%OUTDIR%alteryx_dump.cpp" ^
    /link /MACHINE:X64 /LIBPATH:"%BUILDDIR%" Open_AlteryxYXDB.lib 2>&1

if errorlevel 1 (
    echo ERROR: Dump tool compilation failed
    exit /b 1
)

echo.
echo Build successful: %OUTDIR%alteryx_openyxdb_dump.exe

REM Clean up .obj files
del /q "%~dp0*.obj" 2>nul

endlocal
