# Build script for Alteryx OpenYXDB benchmark using CMake + MSVC
# Requires Visual Studio 2022 Build Tools

$ErrorActionPreference = "Stop"

$SRCDIR = Join-Path $PSScriptRoot "AlteryxOpenYXDB"
$BUILDDIR = Join-Path $PSScriptRoot "alteryx_build"
$OUTDIR = $PSScriptRoot

# Find Visual Studio Build Tools
$VSRoots = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools",
    "C:\Program Files\Microsoft Visual Studio\2022\BuildTools",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community",
    "C:\Program Files\Microsoft Visual Studio\2022\Community"
)
$VSROOT = $VSRoots | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $VSROOT) {
    Write-Error "Visual Studio 2022 not found"
    exit 1
}
$VCVARS = Join-Path $VSROOT "VC\Auxiliary\Build\vcvars64.bat"
$CMAKE = Join-Path $VSROOT "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if (-not (Test-Path $CMAKE)) {
    # Fall back to system cmake
    $CMAKE = (Get-Command cmake -ErrorAction SilentlyContinue).Source
    if (-not $CMAKE) {
        Write-Error "Cannot find cmake.exe"
        exit 1
    }
}

if (-not (Test-Path (Join-Path $SRCDIR "CMakeLists.txt"))) {
    Write-Error "AlteryxOpenYXDB not found. Clone it first."
    exit 1
}

# Setup MSVC environment
Write-Host "Setting up MSVC environment..."
cmd /c "`"$VCVARS`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match "^([^=]+)=(.*)$") {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
    }
}

# Clean build directory
if (Test-Path $BUILDDIR) {
    Remove-Item -Recurse -Force $BUILDDIR
}

Write-Host "Building Alteryx OpenYXDB library with CMake..."

# CMake configure - add /wd4100 to suppress unreferenced parameter warning
& $CMAKE -S $SRCDIR -B $BUILDDIR -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release -DBUILDING_OPEN_ALTERYX=ON "-DCMAKE_CXX_FLAGS=/wd4100"
if ($LASTEXITCODE -ne 0) {
    Write-Error "CMake configure failed"
    exit 1
}

# CMake build
& $CMAKE --build $BUILDDIR --config Release
if ($LASTEXITCODE -ne 0) {
    Write-Error "CMake build failed"
    exit 1
}

Write-Host ""
Write-Host "Building benchmark executable..."

# Build the benchmark binary
$benchSrc = Join-Path $OUTDIR "alteryx_benchmark.cpp"
$benchExe = Join-Path $OUTDIR "alteryx_openyxdb_benchmark.exe"
$includeDir = Join-Path $SRCDIR "include"

& cl.exe /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /DSRCLIB_REPLACEMENT /DBUILDING_OPEN_ALTERYX /wd4100 `
    "/I$includeDir" `
    "/Fe$benchExe" `
    $benchSrc `
    /link /MACHINE:X64 "/LIBPATH:$BUILDDIR" Open_AlteryxYXDB.lib

if ($LASTEXITCODE -ne 0) {
    Write-Error "Benchmark compilation failed"
    exit 1
}

# Copy DLL next to the exe so it can be found at runtime
$dllSrc = Join-Path $BUILDDIR "Open_AlteryxYXDB.dll"
if (Test-Path $dllSrc) {
    Copy-Item $dllSrc $OUTDIR -Force
    Write-Host "Copied Open_AlteryxYXDB.dll to output directory"
}

Write-Host ""
Write-Host "Build successful: $benchExe"
