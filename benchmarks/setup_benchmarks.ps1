# SigilYX Benchmark Environment Setup (Windows)
#
# One-script bootstrap for a fresh Windows machine. Installs all prerequisites,
# builds sigilyx-python, generates benchmark data, and builds C++ benchmarks.
#
# Prerequisites (must exist BEFORE running this script):
#   - winget          (ships with Windows 10 1709+ / Windows 11)
#   - PowerShell 5.1+ (ships with Windows 10+)
#
# Usage (from the SigilYX project root):
#   powershell -ExecutionPolicy Bypass -File benchmarks\setup_benchmarks.ps1
#
# What this script does:
#   1. Installs Git, Rust (rustup), uv, pixi via winget (if missing)
#   2. Installs VS Build Tools + Windows SDK (if no MSVC detected)
#   3. Creates a Python 3.12 venv and installs all Python dependencies
#   4. Loads the MSVC environment into the current session
#   5. Builds sigilyx-python via maturin
#   6. Generates benchmark data (15 YXDB files, ~150 MB)
#   7. Installs cmake via pixi
#   8. Clones and builds the Alteryx OpenYXDB C++ benchmark
#
# Re-running is safe — each step checks whether it's already done.

param(
    [switch]$SkipCpp,
    [switch]$SkipDataGen,
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path (Join-Path $ProjectRoot "Cargo.toml"))) {
    # Might be invoked from project root instead of benchmarks/
    if (Test-Path (Join-Path $PSScriptRoot "Cargo.toml")) {
        $ProjectRoot = $PSScriptRoot
    } else {
        Write-Error "Cannot find project root. Run from the SigilYX directory."
        exit 1
    }
}
$BenchmarksDir = Join-Path $ProjectRoot "benchmarks"
$CppDir = Join-Path $BenchmarksDir "cpp"
$VenvDir = Join-Path $ProjectRoot ".venv"
$VenvPython = Join-Path $VenvDir "Scripts\python.exe"
$VenvMaturin = Join-Path $VenvDir "Scripts\maturin.exe"

function Write-Step($msg) { Write-Host "`n===> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "  OK: $msg" -ForegroundColor Green }
function Write-Skip($msg) { Write-Host "  SKIP: $msg" -ForegroundColor Yellow }
function Write-Warn($msg) { Write-Host "  WARN: $msg" -ForegroundColor Yellow }

# Track issues for the summary
$issues = @()

# ============================================================================
# 1. Install system-level tools via winget
# ============================================================================
Write-Step "Checking system tools"

function Ensure-OnPath($name, $extraPaths) {
    # Check current PATH first
    if (Get-Command $name -ErrorAction SilentlyContinue) { return $true }
    # Check known install locations
    foreach ($p in $extraPaths) {
        $full = Join-Path $p $name
        if (Test-Path $full) {
            $dir = Split-Path $full
            $env:Path += ";$dir"
            Write-Warn "Added $dir to PATH for this session"
            return $true
        }
    }
    return $false
}

# -- Git --
if (-not (Ensure-OnPath "git.exe" @("C:\Program Files\Git\bin", "C:\Program Files (x86)\Git\bin"))) {
    Write-Step "Installing Git"
    winget install Git.Git --accept-source-agreements --accept-package-agreements
    $env:Path += ";C:\Program Files\Git\bin"
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        $issues += "Git installed but not on PATH. Restart your terminal after setup."
    }
} else {
    Write-Ok "Git: $(git --version)"
}

# -- Rust --
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Step "Installing Rust via winget"
    winget install Rustlang.Rustup --accept-source-agreements --accept-package-agreements
    # rustup installer adds to PATH but current session won't see it
    $env:Path += ";$env:USERPROFILE\.cargo\bin"
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        $issues += "Rust installed but not on PATH. Restart your terminal after setup."
    }
} else {
    Write-Ok "Rust: $(rustup show active-toolchain 2>$null)"
}

# -- uv --
if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Step "Installing uv"
    winget install astral-sh.uv --accept-source-agreements --accept-package-agreements
    # Try common install locations
    Ensure-OnPath "uv.exe" @("$env:USERPROFILE\.cargo\bin", "$env:LOCALAPPDATA\uv\bin") | Out-Null
    if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
        $issues += "uv installed but not on PATH. Restart your terminal after setup."
    }
} else {
    Write-Ok "uv: $(uv --version)"
}

# -- pixi --
if (-not (Ensure-OnPath "pixi.exe" @("$env:USERPROFILE\.pixi\bin"))) {
    Write-Step "Installing pixi"
    irm https://pixi.sh/install.ps1 | iex
    $env:Path += ";$env:USERPROFILE\.pixi\bin"
} else {
    Write-Ok "pixi: $(pixi --version)"
}

# ============================================================================
# 2. Ensure MSVC + Windows SDK
# ============================================================================
Write-Step "Checking MSVC toolchain"

function Find-VcVars {
    $candidates = @(
        "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { return $c }
    }
    return $null
}

$vcvars = Find-VcVars
if (-not $vcvars) {
    Write-Step "Installing Visual Studio 2022 Build Tools with C++ workload"
    Write-Host "  This requires administrator privileges and takes several minutes..."
    winget install Microsoft.VisualStudio.2022.BuildTools `
        --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --quiet --wait" `
        --accept-source-agreements --accept-package-agreements
    $vcvars = Find-VcVars
    if (-not $vcvars) {
        $issues += "VS Build Tools install may have failed (needs admin). MSVC is required for Rust and C++ builds."
    }
}

if ($vcvars) {
    Write-Ok "vcvars64.bat: $vcvars"

    # Check Windows SDK
    $sdkFound = $false
    if (Test-Path "C:\Program Files (x86)\Windows Kits\10\Lib") {
        $sdkVersions = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\Lib" -Directory
        if ($sdkVersions.Count -gt 0) {
            $sdkFound = $true
            Write-Ok "Windows SDK: $($sdkVersions[-1].Name)"
        }
    }
    if (-not $sdkFound) {
        Write-Step "Installing Windows SDK (provides kernel32.lib, ucrt)"
        winget install Microsoft.WindowsSDK.10.0.26100 --accept-source-agreements --accept-package-agreements
        if (-not (Test-Path "C:\Program Files (x86)\Windows Kits\10\Lib")) {
            $issues += "Windows SDK install may have failed. Linking will fail without kernel32.lib."
        }
    }

    # Load MSVC environment
    Write-Step "Loading MSVC environment into current session"
    $envVars = cmd /c "`"$vcvars`" >nul 2>&1 && set"
    foreach ($line in $envVars) {
        if ($line -match "^([^=]+)=(.*)$") {
            [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
    # Verify
    $clPath = (Get-Command cl.exe -ErrorAction SilentlyContinue).Source
    if ($clPath) {
        Write-Ok "cl.exe: $clPath"
    } else {
        $issues += "cl.exe not found after loading vcvars64.bat"
    }
    $linkPath = (Get-Command link.exe -ErrorAction SilentlyContinue).Source
    if ($linkPath) {
        Write-Ok "link.exe: $linkPath"
    } else {
        $issues += "link.exe not found after loading vcvars64.bat"
    }
} else {
    Write-Warn "No MSVC found — Rust builds and C++ benchmarks will fail"
}

# ============================================================================
# 3. Python venv + dependencies
# ============================================================================
Write-Step "Setting up Python environment"

if (-not (Test-Path $VenvPython) -or $Force) {
    uv venv $VenvDir --python 3.12
}
Write-Ok "Python venv: $VenvDir"

Write-Host "  Installing Python packages..."
uv pip install --python $VenvPython maturin polars yxdb pyarrow pandas numpy
Write-Ok "Python packages installed"

# ============================================================================
# 4. Build sigilyx-python
# ============================================================================
Write-Step "Building sigilyx-python (maturin develop --release)"
Write-Host "  This takes 5-10 minutes on first build (LTO + codegen-units=1)..."

# Check if already built (look for the .pyd in site-packages)
$pydExists = Get-ChildItem (Join-Path $VenvDir "Lib\site-packages\sigilyx") -Filter "*.pyd" -ErrorAction SilentlyContinue
if ($pydExists -and -not $Force) {
    Write-Skip "sigilyx already built. Use -Force to rebuild."
} else {
    & $VenvMaturin develop --release
    if ($LASTEXITCODE -ne 0) {
        $issues += "maturin develop --release failed. Check MSVC environment."
    } else {
        Write-Ok "sigilyx-python built successfully"
    }
}

# Quick sanity check
$testResult = & $VenvPython -c "import sigilyx; print(sigilyx.__version__)" 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Ok "sigilyx import: v$testResult"
} else {
    $issues += "sigilyx import failed: $testResult"
}

# ============================================================================
# 5. Generate benchmark data
# ============================================================================
if (-not $SkipDataGen) {
    Write-Step "Generating benchmark data"
    $dataDir = Join-Path $BenchmarksDir "data"
    $yxdbCount = (Get-ChildItem $dataDir -Filter "*.yxdb" -ErrorAction SilentlyContinue).Count
    if ($yxdbCount -ge 15 -and -not $Force) {
        Write-Skip "Found $yxdbCount .yxdb files in benchmarks/data/. Use -Force to regenerate."
    } else {
        & $VenvPython (Join-Path $BenchmarksDir "generate_benchmark_data.py")
        if ($LASTEXITCODE -ne 0) {
            $issues += "Benchmark data generation failed"
        } else {
            Write-Ok "Benchmark data generated"
        }
    }
}

# ============================================================================
# 6. Set up pixi (cmake)
# ============================================================================
Write-Step "Setting up pixi environment (cmake)"
Push-Location $BenchmarksDir
if (-not (Test-Path "pixi.toml")) {
    Write-Warn "No pixi.toml found — creating one"
    @"
[workspace]
name = "sigilyx-benchmarks"
channels = ["conda-forge"]
platforms = ["win-64"]

[dependencies]
cmake = "*"
"@ | Out-File -FilePath "pixi.toml" -Encoding utf8
}
pixi install
# Verify cmake via pixi
$cmakeVersion = pixi run cmake --version 2>&1 | Select-Object -First 1
Write-Ok "pixi cmake: $cmakeVersion"
Pop-Location

# ============================================================================
# 7. Build C++ benchmarks (Alteryx OpenYXDB)
# ============================================================================
if (-not $SkipCpp) {
    Write-Step "Building C++ benchmarks"

    # Clone repos if needed
    $alteryxDir = Join-Path $CppDir "AlteryxOpenYXDB"
    if (-not (Test-Path (Join-Path $alteryxDir "CMakeLists.txt"))) {
        Write-Host "  Cloning alteryx/OpenYXDB..."
        git clone https://github.com/alteryx/OpenYXDB.git $alteryxDir
        if ($LASTEXITCODE -ne 0) {
            $issues += "Failed to clone alteryx/OpenYXDB"
        }
    } else {
        Write-Skip "AlteryxOpenYXDB already cloned"
    }

    $nedDir = Join-Path $CppDir "Open_AlteryxYXDB"
    if (-not (Test-Path (Join-Path $nedDir "Open_AlteryxYXDB.cpp"))) {
        Write-Host "  Cloning AlteryxNed/Open_AlteryxYXDB..."
        git clone https://github.com/AlteryxNed/Open_AlteryxYXDB.git $nedDir
        if ($LASTEXITCODE -ne 0) {
            $issues += "Failed to clone AlteryxNed/Open_AlteryxYXDB"
        }
    } else {
        Write-Skip "Open_AlteryxYXDB already cloned"
    }

    # Build Alteryx OpenYXDB with pixi cmake
    if ((Get-Command cl.exe -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $alteryxDir "CMakeLists.txt"))) {
        $buildDir = Join-Path $CppDir "alteryx_build"
        $benchExe = Join-Path $CppDir "alteryx_openyxdb_benchmark.exe"
        $dllPath = Join-Path $buildDir "Open_AlteryxYXDB.dll"

        if ((Test-Path $benchExe) -and (Test-Path (Join-Path $CppDir "Open_AlteryxYXDB.dll")) -and -not $Force) {
            Write-Skip "alteryx_openyxdb_benchmark.exe already built"
        } else {
            Write-Host "  Configuring Alteryx OpenYXDB with CMake..."
            Push-Location $BenchmarksDir
            pixi run cmake -S "cpp/AlteryxOpenYXDB" -B "cpp/alteryx_build" -G "NMake Makefiles" `
                -DCMAKE_BUILD_TYPE=Release "-DCMAKE_CXX_FLAGS=/wd4100"
            if ($LASTEXITCODE -ne 0) {
                $issues += "CMake configure failed for Alteryx OpenYXDB"
            } else {
                Write-Host "  Building Alteryx OpenYXDB library..."
                pixi run cmake --build "cpp/alteryx_build" --config Release
                if ($LASTEXITCODE -ne 0) {
                    $issues += "CMake build failed for Alteryx OpenYXDB"
                }
            }
            Pop-Location

            if (Test-Path (Join-Path $buildDir "Open_AlteryxYXDB.lib")) {
                Write-Host "  Compiling benchmark executable..."
                Push-Location $CppDir
                cl.exe /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /DSRCLIB_REPLACEMENT /DBUILDING_OPEN_ALTERYX `
                    /wd4100 /wd4267 /wd4244 /wd4458 `
                    "/IAlteryxOpenYXDB\include" `
                    "/Fealteryx_openyxdb_benchmark.exe" `
                    alteryx_benchmark.cpp `
                    /link /MACHINE:X64 "/LIBPATH:alteryx_build" Open_AlteryxYXDB.lib
                Pop-Location

                if ($LASTEXITCODE -ne 0) {
                    $issues += "Alteryx benchmark compilation failed"
                } else {
                    # Copy DLL next to the exe so it can be found at runtime
                    if (Test-Path $dllPath) {
                        Copy-Item $dllPath $CppDir -Force
                        Write-Ok "Built: alteryx_openyxdb_benchmark.exe (+ DLL copied)"
                    }
                }
            }
        }
    } else {
        Write-Warn "Skipping Alteryx C++ build (cl.exe or source not found)"
    }

    # Build NedHarding benchmark
    if ((Get-Command cl.exe -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $nedDir "Open_AlteryxYXDB.cpp"))) {
        $nedExe = Join-Path $CppDir "open_yxdb_benchmark.exe"
        if ((Test-Path $nedExe) -and -not $Force) {
            Write-Skip "open_yxdb_benchmark.exe already built"
        } else {
            Write-Host "  Building NedHarding benchmark..."
            Push-Location $CppDir
            & (Join-Path $CppDir "build.bat")
            Pop-Location
            if (Test-Path $nedExe) {
                Write-Ok "Built: open_yxdb_benchmark.exe"
            } else {
                $issues += "NedHarding benchmark build failed"
            }
        }
    }
}

# ============================================================================
# Summary
# ============================================================================
Write-Host ""
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host "  SETUP COMPLETE" -ForegroundColor Cyan
Write-Host ("=" * 80) -ForegroundColor Cyan
Write-Host ""

if ($issues.Count -gt 0) {
    Write-Host "  Issues encountered:" -ForegroundColor Yellow
    foreach ($issue in $issues) {
        Write-Host "    - $issue" -ForegroundColor Yellow
    }
    Write-Host ""
}

Write-Host "  To run benchmarks:"
Write-Host "    & `"$VenvPython`" benchmarks\benchmark_cross_language.py --runs 50"
Write-Host ""
Write-Host "  To run Alteryx-only comparison:"
Write-Host "    & `"$VenvPython`" benchmarks\benchmark_cross_language.py --runs 50 --files bench_numeric_100000.yxdb"
Write-Host ""
Write-Host "  NOTE: If you just installed tools, restart your terminal to pick up PATH changes,"
Write-Host "        then re-run this script (it will skip already-completed steps)."
Write-Host ""
