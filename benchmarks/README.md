# SigilYX Benchmarks

## Overview

This directory contains a cross-language benchmark suite that measures SigilYX
read performance against every available third-party YXDB reader library:

| Target | Language | Library |
|--------|----------|---------|
| sigilyx-rust | Rust | `sigilyx` (workspace crate) |
| sigilyx-py-polars | Python/Rust | `sigilyx` → Polars DataFrame |
| sigilyx-py-arrow | Python/Rust | `sigilyx` → PyArrow Table |
| sigilyx-py-pandas | Python/Rust | `sigilyx` → Pandas DataFrame |
| yxdb-py | Pure Python | `pip install yxdb` |
| yxdb-go | Go | `github.com/tlarsendataguy-yxdb/yxdb-go` |
| yxdb-net | C# (.NET 8) | NuGet `yxdb` v1.1.0 |
| nedharding-openyxdb | C++ | `github.com/AlteryxNed/Open_AlteryxYXDB` |
| alteryx-openyxdb | C++ | `github.com/alteryx/OpenYXDB` |

Each target is optional and auto-detected. The orchestrator skips any
target whose toolchain is not installed.

---

## Quick Start

### One-command setup (Windows — fresh VM)

The bootstrap script installs all prerequisites, builds everything, and
generates benchmark data. Run from the **project root**:

```powershell
powershell -ExecutionPolicy Bypass -File benchmarks\setup_benchmarks.ps1
```

This is idempotent — re-running skips already-completed steps. Use `-Force`
to rebuild everything, `-SkipCpp` to skip C++ benchmarks, or `-SkipDataGen`
to skip data generation.

After setup completes:

```powershell
.venv\Scripts\python.exe benchmarks\benchmark_cross_language.py --runs 50
```

### Manual setup

```bash
# 1. Create a venv and install dependencies (use uv for speed)
uv venv .venv --python 3.12
uv pip install --python .venv/Scripts/python.exe maturin polars yxdb pyarrow pandas numpy

# 2. Build SigilYX Python module (requires MSVC on Windows)
.venv/Scripts/maturin.exe develop --release

# 3. Generate benchmark data
.venv/Scripts/python.exe benchmarks/generate_benchmark_data.py

# 4. Run cross-language benchmarks (skips unavailable targets)
.venv/Scripts/python.exe benchmarks/benchmark_cross_language.py --runs 50
```

---

## Environment Setup

### Prerequisites (all platforms)

- **Python 3.10+** with `pip`
- **Rust toolchain** — `rustup`, `cargo`, `maturin` (`pip install maturin`)
- **Git** — for cloning third-party repos

### Windows

> **Recommended:** Use `setup_benchmarks.ps1` (see Quick Start above), which
> automates all of the steps below. The manual instructions are here for
> reference or if you need to install individual components.

#### System prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| Git | Clone C++ repos | `winget install Git.Git` |
| Rust (rustup) | Rust benchmarks + `maturin` builds | `winget install Rustlang.Rustup` |
| uv | Fast Python env/package manager | `winget install astral-sh.uv` |
| pixi | Provides CMake via conda-forge | `irm https://pixi.sh/install.ps1 \| iex` |
| VS 2022 (Community or BuildTools) | MSVC `cl.exe`, `link.exe` | See below |
| Windows SDK | `kernel32.lib`, ucrt headers | `winget install Microsoft.WindowsSDK.10.0.26100` |

**MSVC + Windows SDK** are required for both Rust (links via `link.exe`) and
C++ benchmarks. Either VS Community or BuildTools works:

```powershell
# Option A: Install VS Build Tools (smaller, CI-friendly)
winget install Microsoft.VisualStudio.2022.BuildTools `
    --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --quiet --wait"

# Option B: If you already have VS Community 2022 with C++ workload, that works too.

# Windows SDK (sometimes not included — required for kernel32.lib)
winget install Microsoft.WindowsSDK.10.0.26100
```

#### Loading MSVC into your shell

The Rust linker and C++ compiler need MSVC environment variables (`LIB`,
`INCLUDE`, `PATH` with `cl.exe`/`link.exe`). Load them for your session:

```powershell
# Find vcvars64.bat (adjust path for BuildTools vs Community)
$vcvars = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

# Import environment variables into PowerShell
cmd /c "`"$vcvars`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match "^([^=]+)=(.*)$") {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
    }
}

# Verify
cl.exe 2>&1 | Select-Object -First 1   # Should print MSVC version
```

#### Python + Rust (required)

```powershell
# Create venv with uv (much faster than pip)
uv venv .venv --python 3.12
uv pip install --python .venv\Scripts\python.exe maturin polars yxdb pyarrow pandas numpy

# Build SigilYX Python module (5-10 min first time due to LTO)
.venv\Scripts\maturin.exe develop --release

# Verify
.venv\Scripts\python.exe -c "import sigilyx; print(sigilyx.__version__)"
```

#### C++ — NedHarding & Alteryx (optional)

```powershell
# Install pixi cmake
cd benchmarks
pixi install          # reads pixi.toml, installs cmake from conda-forge

# Clone the C++ YXDB libraries
cd cpp
git clone https://github.com/AlteryxNed/Open_AlteryxYXDB.git
git clone https://github.com/alteryx/OpenYXDB.git AlteryxOpenYXDB

# Build (uses vcvars64.bat internally to find cl.exe)
.\build.bat               # NedHarding benchmark
.\build_alteryx.bat        # Alteryx OpenYXDB benchmark (CMake + DLL auto-copied)
```

> **Note:** `build_alteryx.bat` automatically copies `Open_AlteryxYXDB.dll`
> next to the exe. If you build manually, ensure the DLL is in the same
> directory as `alteryx_openyxdb_benchmark.exe` or it will fail at runtime
> with exit code 0xC0000135 (STATUS_DLL_NOT_FOUND).

#### Go (optional)

```powershell
winget install GoLang.Go

cd benchmarks/go
go mod tidy
```

#### .NET 8 (optional)

```powershell
winget install Microsoft.DotNet.SDK.8

cd benchmarks/dotnet
dotnet restore
```

### Linux (Ubuntu / Debian)

#### Python + Rust (required)

```bash
# System packages
sudo apt update
sudo apt install -y python3 python3-pip python3-venv git curl

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Python environment
python3 -m venv .venv
source .venv/bin/activate
pip install maturin polars yxdb pyarrow pandas

# Build SigilYX
maturin develop --release
```

#### C++ — NedHarding & Alteryx (optional)

On Linux, use GCC or Clang instead of MSVC. The C++ libraries are
cross-platform C++ with no Windows-specific dependencies:

```bash
# Install build tools
sudo apt install -y build-essential cmake

# Clone repos
cd benchmarks/cpp
git clone https://github.com/AlteryxNed/Open_AlteryxYXDB.git
git clone https://github.com/alteryx/OpenYXDB.git AlteryxOpenYXDB

# Build NedHarding benchmark
g++ -O2 -std=c++17 -DUNICODE \
    -I Open_AlteryxYXDB -I Open_AlteryxYXDB/RecordLib -I Open_AlteryxYXDB/liblzf-3.6 \
    -o open_yxdb_benchmark \
    benchmark.cpp \
    Open_AlteryxYXDB/Open_AlteryxYXDB.cpp \
    Open_AlteryxYXDB/RecordLib/Record.cpp \
    Open_AlteryxYXDB/RecordLib/FieldBase.cpp \
    Open_AlteryxYXDB/liblzf-3.6/lzf_c.c \
    Open_AlteryxYXDB/liblzf-3.6/lzf_d.c

# Build NedHarding dump tool (for cross-impl tests)
g++ -O2 -std=c++17 -DUNICODE \
    -I Open_AlteryxYXDB -I Open_AlteryxYXDB/RecordLib -I Open_AlteryxYXDB/liblzf-3.6 \
    -o open_yxdb_dump \
    dump.cpp \
    Open_AlteryxYXDB/Open_AlteryxYXDB.cpp \
    Open_AlteryxYXDB/RecordLib/Record.cpp \
    Open_AlteryxYXDB/RecordLib/FieldBase.cpp \
    Open_AlteryxYXDB/liblzf-3.6/lzf_c.c \
    Open_AlteryxYXDB/liblzf-3.6/lzf_d.c

# Build Alteryx OpenYXDB benchmark (uses CMake)
mkdir -p alteryx_build && cd alteryx_build
cmake ../AlteryxOpenYXDB -DCMAKE_BUILD_TYPE=Release -DBUILDING_OPEN_ALTERYX=ON
cmake --build . --config Release
cd ..
g++ -O2 -std=c++17 -DUNICODE -D_UNICODE -DSRCLIB_REPLACEMENT -DBUILDING_OPEN_ALTERYX \
    -I AlteryxOpenYXDB/include \
    -o alteryx_openyxdb_benchmark \
    alteryx_benchmark.cpp \
    -L alteryx_build -lOpen_AlteryxYXDB
```

#### Go (optional)

```bash
sudo apt install -y golang-go
# Or install latest: https://go.dev/dl/

cd benchmarks/go
go mod tidy
```

#### .NET 8 (optional)

```bash
# Microsoft package repository
sudo apt install -y dotnet-sdk-8.0
# Or: https://learn.microsoft.com/en-us/dotnet/core/install/linux-ubuntu

cd benchmarks/dotnet
dotnet restore
```

### Using pixi for C/C++ Toolchains (cross-platform)

[pixi](https://pixi.sh) can manage C/C++ compilers and CMake via conda-forge
packages, avoiding manual VS Build Tools or `apt install` steps. This is
especially useful on Linux where you want to avoid system-wide installs.

```bash
# Install pixi
curl -fsSL https://pixi.sh/install.sh | bash   # Linux/macOS
# Or: winget install prefix-dev.pixi             # Windows
```

Create a `pixi.toml` in the `benchmarks/` directory (or project root):

```toml
[workspace]
name = "sigilyx-benchmarks"
channels = ["conda-forge"]
platforms = ["linux-64", "win-64", "osx-64", "osx-arm64"]

[dependencies]
cxx-compiler = "*"  # GCC on Linux, Clang on macOS
cmake = "*"
```

Then activate and build:

```bash
# Linux / macOS
cd benchmarks
pixi install
pixi shell

# Now g++ / cmake are on PATH — build as usual:
cd cpp
git clone https://github.com/AlteryxNed/Open_AlteryxYXDB.git
g++ -O2 -std=c++17 -DUNICODE \
    -I Open_AlteryxYXDB -I Open_AlteryxYXDB/RecordLib -I Open_AlteryxYXDB/liblzf-3.6 \
    -o open_yxdb_benchmark \
    benchmark.cpp \
    Open_AlteryxYXDB/Open_AlteryxYXDB.cpp \
    Open_AlteryxYXDB/RecordLib/Record.cpp \
    Open_AlteryxYXDB/RecordLib/FieldBase.cpp \
    Open_AlteryxYXDB/liblzf-3.6/lzf_c.c \
    Open_AlteryxYXDB/liblzf-3.6/lzf_d.c
```

On **Windows**, pixi provides the `vs_buildtools` activation but MSVC itself
still requires Visual Studio Build Tools installed. pixi is most useful on
Windows for managing CMake and other build dependencies. For MSVC, use the
native build scripts (`build.bat`, `build_dump.bat`, `build_alteryx.bat`).

> **Note:** The benchmark orchestrator (`benchmark_cross_language.py`)
> auto-detects toolchains at runtime. It will skip any target whose
> compiler, runtime, or source repo is not found. You only need to
> install the toolchains you want to compare against.

---

## Generating Benchmark Data

```bash
python benchmarks/generate_benchmark_data.py
```

This generates 15 deterministic YXDB files (5 profiles × 3 row counts)
in `benchmarks/data/` using `seed=42`:

| Profile | Columns | Types | Purpose |
|---------|--------:|-------|---------|
| narrow | 2 | Int64, Float64 | Minimal overhead, max row throughput |
| numeric | 5 | Int32, Int64, Float32, Float64, Int16 | Pure numeric decode |
| mixed | 8 | Int64, Float64, Utf8, Bool, Date, DateTime, Int16, Utf8 | Real-world schema |
| string_heavy | 5 | V_WString (short, medium, long, nullable, mixed) | UTF-16 transcode stress |
| wide | 50 | 15×Int64, 15×Float64, 10×V_WString, 5×Bool, 5×Date | Column-count stress |

Row counts: 1,000 / 10,000 / 100,000

Options:
```bash
python benchmarks/generate_benchmark_data.py --rows 1000 10000
python benchmarks/generate_benchmark_data.py --profiles numeric mixed
```

---

## Running Benchmarks

### Cross-Language (all targets)

```bash
# Default: 50 runs per target per file, 100K-row files
python benchmarks/benchmark_cross_language.py

# More runs for tighter confidence intervals
python benchmarks/benchmark_cross_language.py --runs 200

# Single file
python benchmarks/benchmark_cross_language.py --files bench_numeric_100000.yxdb

# Skip specific targets
python benchmarks/benchmark_cross_language.py --skip-go --skip-dotnet
```

### Python-only (4 targets)

```bash
python benchmarks/benchmark_python_formats.py --runs 100
```

### Single file

```bash
python benchmarks/benchmark.py path/to/file.yxdb --iterations 10
```

### Rust-only read suite (uses test files)

```bash
python benchmarks/benchmark_suite.py
```

### Writer benchmark

```bash
python benchmarks/benchmark_writer.py
```

---

## Cross-Implementation Tests

Validate that SigilYX produces identical values to the NedHarding C++
reference implementation:

```bash
# Requires: NedHarding dump tool built (see C++ setup above)
python benchmarks/test_cross_impl.py
```

This runs 25 tests across 9 test files:
- **Read comparison** — SigilYX vs C++ on the same files, field-by-field
- **Round-trip** — Write with SigilYX → read back with C++ → compare values
- **Type fidelity** — Verify field type preservation across write/read

---

## Output

- **Console**: Summary tables with median time, throughput, and vs-fastest ratios
- **JSON**: `benchmarks/results_cross_language.json` — full statistical output
  (mean, median, stdev, p5/p25/p75/p95, CV, throughput per target per file)

---

## File Layout

```
benchmarks/
├── README.md                       # This file
├── METHODOLOGY.md                  # Detailed statistical methodology
├── SETUP_NOTES.md                  # Known setup issues and workarounds
├── setup_benchmarks.ps1            # One-command Windows bootstrap script
├── pixi.toml                       # pixi config (provides cmake via conda-forge)
├── benchmark_cross_language.py     # Cross-language orchestrator (main entry point)
├── benchmark_python_formats.py     # Python-only multi-format benchmark
├── benchmark_suite.py              # Quick test-file benchmark
├── benchmark.py                    # Single-file benchmark
├── benchmark_writer.py             # Write performance benchmark
├── generate_benchmark_data.py      # Data generator (deterministic, seed=42)
├── test_cross_impl.py              # Cross-implementation correctness tests
├── test_roundtrip.py               # SigilYX round-trip tests
├── data/                           # Generated .yxdb files (gitignored)
│   └── .gitignore
├── rust/                           # Rust benchmark binary
│   ├── Cargo.toml
│   └── src/main.rs
├── go/                             # Go benchmark binary
│   ├── go.mod
│   └── main.go
├── dotnet/                         # .NET benchmark binary
│   ├── YxdbBenchmark.csproj
│   └── Program.cs
└── cpp/                            # C++ benchmarks + dump tool
    ├── benchmark.cpp               # NedHarding benchmark harness
    ├── alteryx_benchmark.cpp       # Alteryx OpenYXDB benchmark harness
    ├── dump.cpp                    # NedHarding dump tool (cross-impl tests)
    ├── build.bat                   # Windows: build NedHarding benchmark
    ├── build_alteryx.bat           # Windows: build Alteryx benchmark (CMake)
    ├── build_alteryx.ps1           # Windows: build Alteryx benchmark (PowerShell)
    ├── build_dump.bat              # Windows: build NedHarding dump tool (cmd)
    ├── build_dump.ps1              # Windows: build NedHarding dump tool (PowerShell)
    ├── Open_AlteryxYXDB/           # Cloned repo (gitignored)
    └── AlteryxOpenYXDB/            # Cloned repo (gitignored)
```

---

*See [METHODOLOGY.md](METHODOLOGY.md) for detailed statistical methodology
and [PERFORMANCE.md](../PERFORMANCE.md) for results.*
