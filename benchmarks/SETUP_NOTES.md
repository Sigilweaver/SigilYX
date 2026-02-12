# Benchmark Setup Notes

Setup log for the SigilYX cross-language benchmark against Alteryx OpenYXDB on
Windows, using **uv**, **pixi**, and **rustup**.

## Tools Used

| Tool | Version | Purpose |
|------|---------|---------|
| uv | 0.10.4 | Python venv creation + package installation |
| pixi | 0.63.2 | CMake provisioning via conda-forge |
| rustup | stable (cargo 1.93.1) | Rust toolchain management |
| maturin | 1.12.4 | Build sigilyx-python Rust extension |
| VS Community 2022 | MSVC 14.44.35207 | C/C++ compiler & linker |

## Issues Encountered

### 1. PowerShell Execution Policy blocks venv activation

**Error:**
```
.venv\Scripts\Activate.ps1 : File ... cannot be loaded because running
scripts is disabled on this system.
```

**Impact:** `Activate.ps1` can't run, but `uv pip install` still installs
packages into `.venv` without activation. Commands need to use the full path
to the venv Python: `& "path\.venv\Scripts\python.exe"`.

**Workaround:** Invoke Python and tools via absolute paths instead of
activating the venv.

---

### 2. MSVC `link.exe` not found (missing on PATH)

**Error:**
```
error: linker `link.exe` not found
note: the msvc targets depend on the msvc linker but `link.exe` was not found
```

**Cause:** `cargo`/`maturin` can't find the MSVC linker unless `vcvars64.bat`
has been sourced in the current session. The system had VS Community 2022
installed but the VC tools weren't on PATH.

**Fix:** Load the MSVC environment into PowerShell before building:
```powershell
$vars = cmd /c '"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set'
foreach ($line in $vars) {
    if ($line -match '^([^=]+)=(.*)$') {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
```

---

### 3. Windows SDK not installed — `kernel32.lib` not found

**Error:**
```
LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'
```

**Cause:** VS Community 2022 was installed with the C++ workload's compiler
(`cl.exe`, `link.exe`) but the **Windows 10/11 SDK** was not installed. The
SDK provides `kernel32.lib`, `ucrt` libraries, and other system `.lib` files
needed for linking.

**Fix:**
```powershell
winget install Microsoft.WindowsSDK.10.0.26100
```

After installing, re-source `vcvars64.bat` so the `LIB` environment variable
includes the SDK paths:
- `C:\Program Files (x86)\Windows Kits\10\lib\10.0.26100.0\ucrt\x64`
- `C:\Program Files (x86)\Windows Kits\10\lib\10.0.26100.0\um\x64`

---

### 4. Zombie processes from interrupted builds lock build artifacts

**Error:**
```
LINK : fatal error LNK1104: cannot open file '...\target\release\deps\rustversion-*.dll'
error: failed to remove file '...\target\release\deps\bytemuck_derive-*.dll'
  Caused by: Access is denied. (os error 5)
```

**Cause:** Interrupting a `maturin develop --release` or `cargo build` mid-compile
leaves orphan `cargo`, `rustc`, and `link` processes that hold file locks on
build artifacts in `target/`.

**Fix:**
```powershell
Get-Process -Name "cargo","rustc","link","cl","maturin" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item -Recurse -Force target
```

---

### 5. `git` not on PATH

**Error:**
```
git : The term 'git' is not recognized ...
```

**Cause:** Git for Windows was installed at `C:\Program Files\Git\bin\` but
not on the session's PATH.

**Fix:**
```powershell
$env:Path += ";C:\Program Files\Git\bin"
```

---

### 6. `pixi` not on PATH after install

**Symptom:** `pixi` command not found despite successful installation.

**Cause:** The pixi installer adds `%USERPROFILE%\.pixi\bin` to the user PATH,
but the current PowerShell session doesn't pick up PATH changes until restarted.

**Fix:**
```powershell
$env:Path += ";$env:USERPROFILE\.pixi\bin"
```

---

### 7. pixi.toml `[project]` deprecated in favor of `[workspace]`

**Warning:**
```
WARN The `project` field is deprecated. Use `workspace` instead.
```

**Impact:** Cosmetic warning only. pixi still works.

**Fix:** Use `[workspace]` instead of `[project]` in `pixi.toml`.

---

### 8. `build_alteryx.bat` hardcodes BuildTools paths

**Issue:** The build script hardcodes paths to VS 2022 **BuildTools** edition:
```bat
set "VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\..."
set "CMAKE=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\..."
```

Systems with VS **Community** edition fail because those paths don't exist.

**Fix:** Updated the script to search multiple VS edition paths (BuildTools,
Community) and fall back to system cmake if the VS-bundled one isn't found.

---

### 9. Alteryx OpenYXDB builds as a DLL — runtime DLL not found

**Error:** Exit code `0xC0000135` (`STATUS_DLL_NOT_FOUND`) when running the
benchmark executable.

**Cause:** The Alteryx OpenYXDB CMake project builds a **shared library**
(`Open_AlteryxYXDB.dll`) by default, not a static `.lib`. The benchmark
executable links against the import library but can't find the DLL at runtime.

**Fix:** Copy the DLL next to the benchmark executable:
```powershell
Copy-Item benchmarks\cpp\alteryx_build\Open_AlteryxYXDB.dll benchmarks\cpp\
```

---

### 10. CMake `BUILDING_OPEN_ALTERYX` variable not used by project

**Warning:**
```
CMake Warning:
  Manually-specified variables were not used by the project:
    BUILDING_OPEN_ALTERYX
```

**Impact:** The Alteryx OpenYXDB CMakeLists.txt doesn't use this variable.
The build script passes it but it's ignored. The define is still needed for
the benchmark's `cl.exe` compilation step (`/DBUILDING_OPEN_ALTERYX`).

---

### 11. Slow Rust release builds (LTO)

**Observation:** The workspace `Cargo.toml` uses:
```toml
[profile.release]
lto = "fat"
codegen-units = 1
```

This makes release builds (both `maturin develop --release` and the Rust
benchmark) take 5–10+ minutes as the linker performs whole-program
optimization across all crates including polars.

**Recommendation:** For iterative development, consider `maturin develop`
(debug mode) or a custom profile with `lto = "thin"`.

---

### 12. VS Build Tools installer requires elevation (exit code 1602)

**Error:**
```
winget install Microsoft.VisualStudio.2022.BuildTools ... 
Installer failed with exit code: 1602
```

**Cause:** The VS Build Tools installer requires administrator privileges or
interactive confirmation. Since VS Community 2022 with C++ was already
installed, this was unnecessary — we used the Community edition instead.

---

## Final Setup Commands (Summary)

```powershell
# 1. Create venv and install Python deps
uv venv .venv --python 3.12
uv pip install maturin polars yxdb pyarrow pandas numpy

# 2. Load MSVC environment
$vars = cmd /c '"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set'
foreach ($line in $vars) {
    if ($line -match '^([^=]+)=(.*)$') {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}

# 3. Build sigilyx Python extension
.venv\Scripts\maturin.exe develop --release

# 4. Generate benchmark data
& .venv\Scripts\python.exe benchmarks\generate_benchmark_data.py

# 5. Install pixi and set up cmake
$env:Path += ";$env:USERPROFILE\.pixi\bin"
cd benchmarks; pixi install; cd ..

# 6. Clone and build Alteryx OpenYXDB
cd benchmarks\cpp
git clone https://github.com/alteryx/OpenYXDB.git AlteryxOpenYXDB
cd ..\..

# Build library with pixi cmake
cd benchmarks
pixi run cmake -S cpp/AlteryxOpenYXDB -B cpp/alteryx_build -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release
pixi run cmake --build cpp/alteryx_build --config Release
cd ..

# Compile benchmark and copy DLL
cd benchmarks\cpp
cl.exe /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /DSRCLIB_REPLACEMENT /DBUILDING_OPEN_ALTERYX /wd4100 /wd4267 /wd4244 /wd4458 /I"AlteryxOpenYXDB\include" /Fe"alteryx_openyxdb_benchmark.exe" alteryx_benchmark.cpp /link /MACHINE:X64 /LIBPATH:"alteryx_build" Open_AlteryxYXDB.lib
Copy-Item alteryx_build\Open_AlteryxYXDB.dll .
cd ..\..

# 7. Run benchmark
& .venv\Scripts\python.exe benchmarks\benchmark_cross_language.py --runs 10 --files bench_numeric_100000.yxdb
```

## Benchmark Results (10 runs, bench_numeric_100000.yxdb)

| Target | Language | Median | Throughput | vs fastest |
|--------|----------|--------|------------|------------|
| sigilyx-rust | Rust (native) | 4.06 ms | 24.6M rows/s | fastest |
| sigilyx-py-polars | Python (Rust) | 4.88 ms | 20.5M rows/s | 1.2x |
| sigilyx-py-arrow | Python (Rust) | 5.04 ms | 19.8M rows/s | 1.2x |
| **alteryx-openyxdb** | **C++** | **5.88 ms** | **17.0M rows/s** | **1.4x** |
| sigilyx-py-pandas | Python (Rust) | 6.14 ms | 16.3M rows/s | 1.5x |
| sigilyx-rust-row | Rust (native) | 8.67 ms | 11.5M rows/s | 2.1x |
| sigilyx-py-rows | Python (Rust) | 26.55 ms | 3.8M rows/s | 6.5x |
| yxdb-py | Pure Python | 395.78 ms | 253K rows/s | 97.4x |
