#!/usr/bin/env python3
"""
SigilYX Cross-Language Benchmark Orchestrator
==============================================

Runs read benchmarks across all available YXDB libraries and produces
a unified comparison report.

Targets (11 total):
  - sigilyx-rust:           Rust native (sigilyx::read_yxdb -> Polars DataFrame)
  - sigilyx-rust-row:       Rust native (YxdbRowReader row-by-row)
  - sigilyx-py-polars:      Python/Rust (sigilyx.read_yxdb -> Polars DataFrame)
  - sigilyx-py-arrow:       Python/Rust (sigilyx.read_yxdb_arrow -> PyArrow Table)
  - sigilyx-py-pandas:      Python/Rust (sigilyx.read_yxdb_pandas -> Pandas DataFrame)
  - sigilyx-py-rows:        Python/Rust (YxdbRowReader row-by-row)
  - yxdb-py:                Pure Python (yxdb row-by-row)
  - yxdb-go:                Go (yxdb-go row-by-row)
  - yxdb-net:               C# .NET 8 (yxdb-net row-by-row)
  - alteryx-openyxdb:       C++ (github.com/alteryx/OpenYXDB row-by-row)
  - nedharding-openyxdb:    C++ (github.com/AlteryxNed/Open_AlteryxYXDB row-by-row)

Usage:
    python benchmarks/benchmark_cross_language.py
    python benchmarks/benchmark_cross_language.py --runs 200
    python benchmarks/benchmark_cross_language.py --files bench_numeric_100000.yxdb
    python benchmarks/benchmark_cross_language.py --skip-go --skip-dotnet
    python benchmarks/benchmark_cross_language.py --no-cache
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
BENCHMARKS_DIR = PROJECT_ROOT / "benchmarks"
DATA_DIR = BENCHMARKS_DIR / "data"
TEST_FILES_DIR = PROJECT_ROOT / "sigilyx" / "test_files"
CACHE_DIR = BENCHMARKS_DIR / ".bench_cache"


# ============================================================================
# Result caching
# ============================================================================

def _cache_key(target: str, filename: str, runs: int) -> str:
    """Deterministic cache filename for a (target, file, runs) triple."""
    slug = f"{target}|{filename}|{runs}"
    h = hashlib.sha256(slug.encode()).hexdigest()[:12]
    safe_name = filename.replace(".yxdb", "")
    return f"{target}_{safe_name}_{runs}_{h}.json"


def load_cached_result(target: str, filename: str, runs: int) -> dict | None:
    path = CACHE_DIR / _cache_key(target, filename, runs)
    if path.exists():
        try:
            with open(path) as f:
                return json.load(f)
        except (json.JSONDecodeError, OSError):
            return None
    return None


def save_cached_result(target: str, filename: str, runs: int, result: dict) -> None:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    path = CACHE_DIR / _cache_key(target, filename, runs)
    with open(path, "w") as f:
        json.dump(result, f, indent=2)


# ============================================================================
# Toolchain detection
# ============================================================================

def _find_go() -> str | None:
    go = shutil.which("go")
    if go:
        return go
    for candidate in [
        r"C:\Program Files\Go\bin\go.exe",
        r"C:\Program Files (x86)\Go\bin\go.exe",
    ]:
        if os.path.isfile(candidate):
            return candidate
    return None


def _find_msvc_vcvars() -> str | None:
    candidates = [
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    ]
    for c in candidates:
        if os.path.isfile(c):
            return c
    return None


def detect_toolchains() -> dict[str, bool]:
    tc = {}
    tc["python"] = True
    tc["rust"] = shutil.which("cargo") is not None
    tc["go"] = _find_go() is not None
    tc["dotnet"] = shutil.which("dotnet") is not None

    # NedHarding C++
    ned_dir = BENCHMARKS_DIR / "cpp" / "Open_AlteryxYXDB"
    tc["nedharding-cpp"] = _find_msvc_vcvars() is not None and ned_dir.exists()

    # Alteryx OpenYXDB C++
    alteryx_dir = BENCHMARKS_DIR / "cpp" / "AlteryxOpenYXDB"
    tc["alteryx-cpp"] = _find_msvc_vcvars() is not None and alteryx_dir.exists()

    return tc


# ============================================================================
# Build helpers
# ============================================================================

def build_rust_benchmark() -> str | None:
    rust_dir = BENCHMARKS_DIR / "rust"
    if not (rust_dir / "Cargo.toml").exists():
        return None

    print("  Building Rust benchmark...")
    try:
        result = subprocess.run(
            ["cargo", "build", "--release"],
            cwd=str(rust_dir),
            capture_output=True,
            text=True,
            timeout=300,
        )
        # Binary is in the workspace target directory
        binary = PROJECT_ROOT / "target" / "release" / "sigilyx_rust_benchmark.exe"
        if not binary.exists():
            binary = PROJECT_ROOT / "target" / "release" / "sigilyx_rust_benchmark"
        if result.returncode == 0 and binary.exists():
            print(f"  Built: {binary}")
            return str(binary)
        else:
            print(f"  WARNING: Rust build failed (exit code {result.returncode})")
            if result.stderr:
                for line in result.stderr.strip().splitlines()[-5:]:
                    print(f"    {line}")
            return None
    except Exception as e:
        print(f"  WARNING: Rust build failed: {e}")
        return None


def build_go_benchmark() -> str | None:
    go_dir = BENCHMARKS_DIR / "go"
    if not (go_dir / "main.go").exists():
        return None
    go_exe = _find_go()
    if not go_exe:
        return None

    print("  Building Go benchmark...")
    try:
        subprocess.run([go_exe, "mod", "tidy"], cwd=str(go_dir),
                       capture_output=True, check=True, timeout=120)
        binary_name = "yxdb_go_benchmark.exe" if sys.platform == "win32" else "yxdb_go_benchmark"
        binary_path = str(go_dir / binary_name)
        subprocess.run([go_exe, "build", "-o", binary_path, "."],
                       cwd=str(go_dir), capture_output=True, check=True, timeout=120)
        print(f"  Built: {binary_path}")
        return binary_path
    except Exception as e:
        print(f"  WARNING: Go build failed: {e}")
        return None


def build_dotnet_benchmark() -> bool:
    dotnet_dir = BENCHMARKS_DIR / "dotnet"
    if not (dotnet_dir / "YxdbBenchmark.csproj").exists():
        return False
    print("  Building .NET benchmark...")
    try:
        subprocess.run(["dotnet", "build", "-c", "Release", "--nologo", "-v", "quiet"],
                       cwd=str(dotnet_dir), capture_output=True, text=True, check=True, timeout=120)
        print("  Built successfully")
        return True
    except Exception as e:
        print(f"  WARNING: .NET build failed: {e}")
        return False


def build_nedharding_benchmark() -> str | None:
    cpp_dir = BENCHMARKS_DIR / "cpp"
    build_bat = cpp_dir / "build.bat"
    if not build_bat.exists():
        return None
    print("  Building NedHarding C++ benchmark...")
    try:
        result = subprocess.run([str(build_bat)], cwd=str(cpp_dir),
                                capture_output=True, text=True, timeout=120)
        binary_path = str(cpp_dir / "open_yxdb_benchmark.exe")
        if result.returncode == 0 and os.path.isfile(binary_path):
            print(f"  Built: {binary_path}")
            return binary_path
        else:
            print(f"  WARNING: NedHarding build failed (exit code {result.returncode})")
            return None
    except Exception as e:
        print(f"  WARNING: NedHarding build failed: {e}")
        return None


def build_alteryx_benchmark() -> str | None:
    cpp_dir = BENCHMARKS_DIR / "cpp"
    build_bat = cpp_dir / "build_alteryx.bat"
    if not build_bat.exists():
        return None
    binary_path = str(cpp_dir / "alteryx_openyxdb_benchmark.exe")
    dll_path = cpp_dir / "Open_AlteryxYXDB.dll"
    dll_src = cpp_dir / "alteryx_build" / "Open_AlteryxYXDB.dll"
    if os.path.isfile(binary_path):
        # Ensure the DLL is next to the exe (common cause of silent runtime failures)
        if not dll_path.exists() and dll_src.exists():
            import shutil
            shutil.copy2(str(dll_src), str(dll_path))
            print(f"  Copied missing Open_AlteryxYXDB.dll to {cpp_dir}")
        print(f"  Alteryx benchmark already built: {binary_path}")
        return binary_path
    print("  Building Alteryx OpenYXDB C++ benchmark...")
    try:
        result = subprocess.run([str(build_bat)], cwd=str(cpp_dir),
                                capture_output=True, text=True, timeout=300)
        if result.returncode == 0 and os.path.isfile(binary_path):
            # Build scripts should copy the DLL, but ensure it here as a fallback
            if not dll_path.exists() and dll_src.exists():
                import shutil
                shutil.copy2(str(dll_src), str(dll_path))
                print(f"  Copied Open_AlteryxYXDB.dll to {cpp_dir}")
            print(f"  Built: {binary_path}")
            return binary_path
        else:
            print(f"  WARNING: Alteryx build failed (exit code {result.returncode})")
            if result.stderr:
                print(f"  stderr: {result.stderr[:500]}")
            return None
    except Exception as e:
        print(f"  WARNING: Alteryx build failed: {e}")
        return None


# ============================================================================
# Benchmark runners
# ============================================================================

def run_python_formats_benchmark(test_files: list[str], runs: int, use_cache: bool = True) -> list[dict]:
    """Run the Python multi-format benchmark (sigilyx-polars/arrow/pandas + yxdb-py)."""
    output_path = str(BENCHMARKS_DIR / "results_python_formats.json")
    cmd = [
        sys.executable,
        str(BENCHMARKS_DIR / "benchmark_python_formats.py"),
        "--runs", str(runs),
        "--output", output_path,
        "--files",
    ] + [os.path.basename(f) for f in test_files]

    if not use_cache:
        cmd.append("--no-cache")

    # Set data-dir if using benchmark data
    if test_files and DATA_DIR.as_posix() in test_files[0].replace("\\", "/"):
        cmd.extend(["--data-dir", str(DATA_DIR)])

    print(f"\n  Running Python multi-format benchmark ({runs} runs)...")
    result = subprocess.run(cmd, capture_output=False, timeout=7200)

    if result.returncode != 0:
        print("  WARNING: Python benchmark exited with errors")
        return []

    try:
        with open(output_path) as f:
            data = json.load(f)
        # Annotate results with target names
        results = data.get("results", [])
        for r in results:
            if r.get("output_type") == "Polars DataFrame":
                r["target"] = "sigilyx-py-polars"
            elif r.get("output_type") == "PyArrow Table":
                r["target"] = "sigilyx-py-arrow"
            elif r.get("output_type") == "Pandas DataFrame":
                r["target"] = "sigilyx-py-pandas"
            elif r.get("output_type") == "tuple (row-by-row)":
                r["target"] = "sigilyx-py-rows"
            elif "yxdb" in r.get("library", ""):
                r["target"] = "yxdb-py"
        return results
    except (FileNotFoundError, json.JSONDecodeError):
        return []


def run_external_benchmark(binary: str, test_files: list[str], runs: int,
                           target_name: str, use_flags: bool = True,
                           extra_flags: list[str] | None = None,
                           use_cache: bool = True) -> list[dict]:
    """Run an external benchmark binary for each test file.

    Args:
        use_flags: If True, use -file/-runs flags. If False, use positional args.
        extra_flags: Additional command-line flags to pass after the standard flags.
        use_cache: If True, return cached results when available.
    """
    results = []
    for file_path in test_files:
        fname = os.path.basename(file_path)
        print(f"\n  Running {target_name} on {fname} ({runs} runs)...", end="", flush=True)

        # Check cache first
        if use_cache:
            cached = load_cached_result(target_name, fname, runs)
            if cached is not None:
                median = cached.get("median_s", 0)
                tp = cached.get("throughput_rows_per_s", 0)
                print(f" {median:.6f}s, {tp/1e6:.1f}M rows/s  [cached]")
                results.append(cached)
                continue

        try:
            if use_flags:
                cmd = [binary, "-file", file_path, "-runs", str(runs)]
            else:
                cmd = [binary, file_path, str(runs)]

            if extra_flags:
                cmd.extend(extra_flags)

            result = subprocess.run(cmd, capture_output=True, text=True, timeout=3600)
            if result.returncode == 0 and result.stdout.strip():
                data = json.loads(result.stdout)
                data["target"] = target_name
                results.append(data)
                median = data.get("median_s", 0)
                tp = data.get("throughput_rows_per_s", 0)
                print(f" {median:.6f}s, {tp/1e6:.1f}M rows/s")
                save_cached_result(target_name, fname, runs, data)
            else:
                # 0xC0000135 = STATUS_DLL_NOT_FOUND (shows as -1073741515 signed)
                if result.returncode in (-1073741515, 0xC0000135):
                    print(f" FAILED (DLL not found)")
                    print(f"    Ensure Open_AlteryxYXDB.dll is next to the benchmark exe")
                else:
                    print(f" FAILED (exit {result.returncode})")
                if result.stderr:
                    for line in result.stderr.strip().splitlines()[:3]:
                        print(f"    {line}")
        except subprocess.TimeoutExpired:
            print(f" TIMEOUT")
        except Exception as e:
            print(f" ERROR: {e}")
    return results


def run_dotnet_benchmark(test_files: list[str], runs: int, use_cache: bool = True) -> list[dict]:
    dotnet_dir = str(BENCHMARKS_DIR / "dotnet")
    results = []
    for file_path in test_files:
        fname = os.path.basename(file_path)
        print(f"\n  Running yxdb-net on {fname} ({runs} runs)...", end="", flush=True)

        # Check cache first
        if use_cache:
            cached = load_cached_result("yxdb-net", fname, runs)
            if cached is not None:
                median = cached.get("median_s", 0)
                tp = cached.get("throughput_rows_per_s", 0)
                print(f" {median:.6f}s, {tp/1e6:.1f}M rows/s  [cached]")
                results.append(cached)
                continue

        try:
            result = subprocess.run(
                ["dotnet", "run", "-c", "Release", "--no-build", "--",
                 "-file", file_path, "-runs", str(runs)],
                capture_output=True, text=True, cwd=dotnet_dir, timeout=3600,
            )
            if result.returncode == 0 and result.stdout.strip():
                data = json.loads(result.stdout)
                data["target"] = "yxdb-net"
                results.append(data)
                median = data.get("median_s", 0)
                tp = data.get("throughput_rows_per_s", 0)
                print(f" {median:.6f}s, {tp/1e6:.1f}M rows/s")
                save_cached_result("yxdb-net", fname, runs, data)
            else:
                print(f" FAILED (exit {result.returncode})")
                if result.stderr:
                    for line in result.stderr.strip().splitlines()[:3]:
                        print(f"    {line}")
        except Exception as e:
            print(f" ERROR: {e}")
    return results


# ============================================================================
# Report
# ============================================================================

def format_time(seconds: float) -> str:
    if seconds < 0.001:
        return f"{seconds * 1_000_000:.1f}us"
    elif seconds < 1.0:
        return f"{seconds * 1_000:.2f}ms"
    else:
        return f"{seconds:.3f}s"


def format_throughput(tp: float) -> str:
    if tp >= 1_000_000:
        return f"{tp / 1_000_000:.1f}M"
    elif tp >= 1_000:
        return f"{tp / 1_000:.0f}K"
    else:
        return f"{tp:.0f}"


# Target display order
TARGET_ORDER = [
    "sigilyx-rust", "sigilyx-rust-row",
    "sigilyx-py-polars", "sigilyx-py-arrow", "sigilyx-py-pandas", "sigilyx-py-rows",
    "yxdb-py", "yxdb-go", "yxdb-net", "alteryx-openyxdb", "nedharding-openyxdb",
]


def print_unified_report(all_results: list[dict]):
    print()
    print("=" * 120)
    print("CROSS-LANGUAGE COMPARISON REPORT")
    print("=" * 120)
    print()

    # Group results by file
    files: dict[str, dict[str, dict]] = {}
    for r in all_results:
        fname = r.get("file", "unknown")
        target = r.get("target", r.get("library", "unknown"))
        if fname not in files:
            files[fname] = {}
        files[fname][target] = r

    # Collect all targets that have results, in order
    seen_targets = []
    for target in TARGET_ORDER:
        for libs in files.values():
            if target in libs and target not in seen_targets:
                seen_targets.append(target)
                break

    # Per-file detailed tables
    for fname, libs in files.items():
        sample = next(iter(libs.values()))
        rows = sample.get("rows", 0)
        cols = sample.get("cols", 0)
        fsize = sample.get("file_size_bytes", 0)

        print(f"  {fname}  ({fsize / 1024:.1f} KB, {rows:,} rows x {cols} cols)")
        print(f"  {'-' * 115}")
        print(f"  {'Target':<22} {'Language':<18} {'Median':>10} {'Mean':>10} "
              f"{'CV':>6} {'Throughput':>14} {'vs fastest':>12}")
        print(f"  {'-' * 115}")

        # Find the fastest median for this file
        medians = [(t, r.get("median_s", float("inf"))) for t, r in libs.items()]
        fastest_median = min(m for _, m in medians) if medians else 0

        for target in seen_targets:
            if target not in libs:
                continue
            r = libs[target]
            median = r.get("median_s", 0)
            mean = r.get("mean_s", 0)
            cv = r.get("cv", 0)
            lang = r.get("language", "?")
            tp = r.get("throughput_rows_per_s", 0)

            if fastest_median > 0 and median > 0:
                ratio = median / fastest_median
                if ratio <= 1.01:
                    ratio_str = "fastest"
                else:
                    ratio_str = f"{ratio:.1f}x"
            else:
                ratio_str = "N/A"

            print(f"  {target:<22} {lang:<18} {format_time(median):>10} "
                  f"{format_time(mean):>10} {cv:>5.3f} {format_throughput(tp) + ' rows/s':>14} {ratio_str:>12}")

        print()

    # Throughput summary matrix
    print("  THROUGHPUT SUMMARY (median, rows/s)")
    print(f"  {'-' * 115}")

    # Shorten target names for column headers
    short_names = {
        "sigilyx-rust": "rust",
        "sigilyx-rust-row": "rust-row",
        "sigilyx-py-polars": "py-polars",
        "sigilyx-py-arrow": "py-arrow",
        "sigilyx-py-pandas": "py-pandas",
        "sigilyx-py-rows": "py-rows",
        "yxdb-py": "yxdb-py",
        "yxdb-go": "yxdb-go",
        "yxdb-net": "yxdb-net",
        "alteryx-openyxdb": "alteryx",
        "nedharding-openyxdb": "nedharding",
    }

    header = f"  {'File':<30}"
    for t in seen_targets:
        header += f" {short_names.get(t, t):>12}"
    print(header)
    print(f"  {'-' * (30 + 13 * len(seen_targets))}")

    for fname, libs in files.items():
        row = f"  {fname:<30}"
        for t in seen_targets:
            if t in libs:
                tp = libs[t].get("throughput_rows_per_s", 0)
                row += f" {format_throughput(tp):>12}"
            else:
                row += f" {'---':>12}"
        print(row)

    print()


# ============================================================================
# Main
# ============================================================================

def main():
    parser = argparse.ArgumentParser(
        description="Cross-language YXDB read benchmark orchestrator (11 targets)",
    )
    parser.add_argument("--runs", type=int, default=100,
                        help="Number of timed runs per target per file (default: 100)")
    parser.add_argument("--files", nargs="+", default=None,
                        help="Specific file names to benchmark")
    parser.add_argument("--data-dir", type=str, default=None,
                        help="Directory with benchmark data (default: benchmarks/data)")
    parser.add_argument("--output", type=str, default=None,
                        help="Write combined JSON results to this path")
    parser.add_argument("--skip-rust", action="store_true")
    parser.add_argument("--skip-go", action="store_true")
    parser.add_argument("--skip-dotnet", action="store_true")
    parser.add_argument("--skip-nedharding", action="store_true")
    parser.add_argument("--skip-alteryx-cpp", action="store_true")
    parser.add_argument("--no-cache", action="store_true",
                        help="Ignore cached results and re-run all benchmarks")
    args = parser.parse_args()

    use_cache = not args.no_cache

    print("=" * 120)
    print("SigilYX Cross-Language Read Benchmark (11 targets)")
    print("=" * 120)
    print()

    # Resolve data directory
    data_dir = Path(args.data_dir) if args.data_dir else DATA_DIR

    # Resolve test files
    if args.files:
        file_names = args.files
    else:
        # Auto-discover .yxdb files in data dir
        if data_dir.exists():
            file_names = sorted(f.name for f in data_dir.glob("*.yxdb"))
        else:
            file_names = ["ManyRecords.yxdb"]
            data_dir = TEST_FILES_DIR

    test_files = []
    for f in file_names:
        p = data_dir / f
        if p.exists():
            test_files.append(str(p))
        elif (TEST_FILES_DIR / f).exists():
            test_files.append(str(TEST_FILES_DIR / f))
        else:
            print(f"  WARNING: File not found: {f}")

    if not test_files:
        print("  ERROR: No test files found. Run generate_benchmark_data.py first.")
        sys.exit(1)

    print(f"  Data dir:    {data_dir}")
    print(f"  Test files:  {len(test_files)}")
    print(f"  Runs/target: {args.runs}")
    print(f"  Cache:       {'enabled (use --no-cache to force re-run)' if use_cache else 'disabled'}")
    print()

    # Detect toolchains
    print("Detecting toolchains...")
    toolchains = detect_toolchains()
    for name, available in toolchains.items():
        status = "available" if available else "not found"
        print(f"  {name:>16}: {status}")
    print()

    all_results: list[dict] = []
    step = 0
    total_steps = 7

    # -- Step 1: Python formats ------------------------------------------------
    step += 1
    print(f"[{step}/{total_steps}] Python benchmarks (sigilyx-py-polars/arrow/pandas/rows + yxdb-py)")
    print("-" * 80)
    python_results = run_python_formats_benchmark(test_files, args.runs, use_cache)
    all_results.extend(python_results)

    # -- Step 2: Rust (columnar) -----------------------------------------------
    rust_binary = None
    step += 1
    if toolchains["rust"] and not args.skip_rust:
        print(f"\n[{step}/{total_steps}] Rust benchmark (sigilyx-rust, columnar)")
        print("-" * 80)
        rust_binary = build_rust_benchmark()
        if rust_binary:
            rust_results = run_external_benchmark(rust_binary, test_files, args.runs,
                                                   "sigilyx-rust", use_flags=True,
                                                   use_cache=use_cache)
            all_results.extend(rust_results)
    else:
        reason = "skipped" if args.skip_rust else "Rust toolchain not found"
        print(f"\n[{step}/{total_steps}] Rust benchmark -- {reason}")

    # -- Step 3: Rust (row-by-row) ---------------------------------------------
    step += 1
    if rust_binary and not args.skip_rust:
        print(f"\n[{step}/{total_steps}] Rust benchmark (sigilyx-rust-row, row-by-row)")
        print("-" * 80)
        rust_row_results = run_external_benchmark(rust_binary, test_files, args.runs,
                                                    "sigilyx-rust-row", use_flags=True,
                                                    extra_flags=["-mode", "row"],
                                                    use_cache=use_cache)
        all_results.extend(rust_row_results)
    else:
        reason = "skipped" if args.skip_rust else "Rust binary not available"
        print(f"\n[{step}/{total_steps}] Rust row benchmark -- {reason}")

    # -- Step 4: Go ------------------------------------------------------------
    step += 1
    if toolchains["go"] and not args.skip_go:
        print(f"\n[{step}/{total_steps}] Go benchmark (yxdb-go)")
        print("-" * 80)
        binary = build_go_benchmark()
        if binary:
            go_results = run_external_benchmark(binary, test_files, args.runs,
                                                 "yxdb-go", use_flags=True,
                                                 use_cache=use_cache)
            all_results.extend(go_results)
    else:
        reason = "skipped" if args.skip_go else "Go toolchain not found"
        print(f"\n[{step}/{total_steps}] Go benchmark -- {reason}")

    # -- Step 5: .NET ----------------------------------------------------------
    step += 1
    if toolchains["dotnet"] and not args.skip_dotnet:
        print(f"\n[{step}/{total_steps}] .NET benchmark (yxdb-net)")
        print("-" * 80)
        if build_dotnet_benchmark():
            dotnet_results = run_dotnet_benchmark(test_files, args.runs, use_cache)
            all_results.extend(dotnet_results)
    else:
        reason = "skipped" if args.skip_dotnet else ".NET toolchain not found"
        print(f"\n[{step}/{total_steps}] .NET benchmark -- {reason}")

    # -- Step 6: NedHarding C++ ------------------------------------------------
    step += 1
    if toolchains["nedharding-cpp"] and not args.skip_nedharding:
        print(f"\n[{step}/{total_steps}] NedHarding C++ benchmark (Open_AlteryxYXDB)")
        print("-" * 80)
        binary = build_nedharding_benchmark()
        if binary:
            ned_results = run_external_benchmark(binary, test_files, args.runs,
                                                  "nedharding-openyxdb", use_flags=False,
                                                  use_cache=use_cache)
            all_results.extend(ned_results)
    else:
        reason = "skipped" if args.skip_nedharding else "NedHarding C++ not found"
        print(f"\n[{step}/{total_steps}] NedHarding C++ -- {reason}")

    # -- Step 7: Alteryx OpenYXDB C++ ------------------------------------------
    step += 1
    if toolchains["alteryx-cpp"] and not args.skip_alteryx_cpp:
        print(f"\n[{step}/{total_steps}] Alteryx OpenYXDB C++ benchmark")
        print("-" * 80)
        binary = build_alteryx_benchmark()
        if binary:
            alteryx_results = run_external_benchmark(binary, test_files, args.runs,
                                                      "alteryx-openyxdb", use_flags=False,
                                                      use_cache=use_cache)
            all_results.extend(alteryx_results)
    else:
        reason = "skipped" if args.skip_alteryx_cpp else "Alteryx C++ not found"
        print(f"\n[{step}/{total_steps}] Alteryx OpenYXDB C++ -- {reason}")

    # -- Unified report --------------------------------------------------------
    if all_results:
        print_unified_report(all_results)

    # -- Write combined JSON ---------------------------------------------------
    output_path = args.output or str(BENCHMARKS_DIR / "results_cross_language.json")
    with open(output_path, "w") as f:
        json.dump({
            "benchmark": "sigilyx-cross-language-read",
            "platform": sys.platform,
            "python_version": sys.version,
            "timed_runs": args.runs,
            "toolchains": {k: v for k, v in toolchains.items()},
            "results": all_results,
        }, f, indent=2)

    print(f"  Combined results written to: {output_path}")
    print()
    print("=" * 120)
    print("Benchmark complete.")
    print("=" * 120)


if __name__ == "__main__":
    main()
