@echo off
REM Build script for Alteryx OpenYXDB benchmark using CMake + MSVC
REM Requires Visual Studio 2022 Build Tools

setlocal

set "SRCDIR=%~dp0AlteryxOpenYXDB"
set "BUILDDIR=%~dp0alteryx_build"
set "OUTDIR=%~dp0"

REM Search for vcvars64.bat in BuildTools and Community editions
set "VCVARS="
for %%V in (
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
    "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
    "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
) do (
    if exist %%V set "VCVARS=%%~V"
)
if "%VCVARS%"=="" (
    echo ERROR: Cannot find vcvars64.bat. Install Visual Studio 2022 BuildTools or Community with C++ workload.
    exit /b 1
)

REM Search for cmake: VS bundled, then system PATH
set "CMAKE="
for %%C in (
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
) do (
    if exist %%C set "CMAKE=%%~C"
)
if "%CMAKE%"=="" (
    where cmake >nul 2>&1
    if %errorlevel% equ 0 (
        for /f "delims=" %%P in ('where cmake') do set "CMAKE=%%P"
    ) else (
        echo ERROR: Cannot find cmake.exe. Install it via VS, pixi, or add to PATH.
        exit /b 1
    )
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
echo Building benchmark executable...
cl.exe /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /DSRCLIB_REPLACEMENT /DBUILDING_OPEN_ALTERYX /wd4100 /wd4267 /wd4244 /wd4458 ^
    /I"%SRCDIR%\include" ^
    /Fe"%OUTDIR%alteryx_openyxdb_benchmark.exe" ^
    "%OUTDIR%alteryx_benchmark.cpp" ^
    /link /MACHINE:X64 /LIBPATH:"%BUILDDIR%" Open_AlteryxYXDB.lib 2>&1

if errorlevel 1 (
    echo ERROR: Benchmark compilation failed
    exit /b 1
)

REM Copy DLL next to the exe so it can be found at runtime
if exist "%BUILDDIR%\Open_AlteryxYXDB.dll" (
    copy /Y "%BUILDDIR%\Open_AlteryxYXDB.dll" "%OUTDIR%" >nul
    echo Copied Open_AlteryxYXDB.dll to output directory
)

echo.
echo Build successful: %OUTDIR%alteryx_openyxdb_benchmark.exe
