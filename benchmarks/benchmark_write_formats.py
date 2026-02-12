#!/usr/bin/env python3
"""
SigilYX Multi-Format Write Benchmark
======================================

Benchmarks write performance across SigilYX Python input formats and the
official Alteryx OpenYXDB C++ library.

Targets:
  - sigilyx-write-polars:       sigilyx.write_yxdb(path, df)           <- polars.DataFrame
  - sigilyx-write-arrow:        sigilyx.write_yxdb_arrow(path, table)   <- pyarrow.Table
  - sigilyx-write-pandas:       sigilyx.write_yxdb_pandas(path, df)     <- pandas.DataFrame
  - alteryx-openyxdb-write:     Alteryx OpenYXDB C++ library (external exe)

Methodology:
  - Source data: pre-loaded from benchmark .yxdb files into memory
  - 10 warmup runs per target per file (writes discarded)
  - GC disabled during timed runs
  - time.perf_counter() (monotonic, ~100 ns on Windows)
  - Each run writes to a fresh temp file in a TemporaryDirectory
  - Timer wraps only the write call (no read, no conversion, no cleanup)
  - C++ target runs as an external process with its own timing

Result caching:
  - Results cached to .bench_cache/ keyed by (target, file, runs)
  - Use --no-cache to force a fresh run

Usage:
    python benchmarks/benchmark_write_formats.py
    python benchmarks/benchmark_write_formats.py --runs 100
    python benchmarks/benchmark_write_formats.py --files bench_mixed_100000.yxdb
    python benchmarks/benchmark_write_formats.py --no-cache
"""

from __future__ import annotations

import argparse
import gc
import hashlib
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "python"))

import polars as pl
import sigilyx as yx

WARMUP_RUNS = 10
CACHE_DIR = Path(__file__).resolve().parent / ".bench_cache"


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


# Check for optional dependencies
try:
    import pyarrow
    HAS_PYARROW = True
except ImportError:
    HAS_PYARROW = False

try:
    import pandas
    HAS_PANDAS = True
except ImportError:
    HAS_PANDAS = False


def compute_stats(times: list[float]) -> dict:
    n = len(times)
    if n == 0:
        return {}

    sorted_t = sorted(times)
    mean = sum(sorted_t) / n
    median = (sorted_t[n // 2 - 1] + sorted_t[n // 2]) / 2.0 if n % 2 == 0 else sorted_t[n // 2]

    variance = sum((t - mean) ** 2 for t in sorted_t) / max(n - 1, 1)
    stdev = math.sqrt(variance)

    def percentile(p):
        idx = int(n * p)
        return sorted_t[min(max(idx, 0), n - 1)]

    p5, p25, p75, p95 = percentile(0.05), percentile(0.25), percentile(0.75), percentile(0.95)
    cv = stdev / mean if mean > 0 else 0

    return {
        "count": n,
        "mean_s": mean,
        "median_s": median,
        "stdev_s": stdev,
        "min_s": sorted_t[0],
        "max_s": sorted_t[-1],
        "p5_s": p5,
        "p25_s": p25,
        "p75_s": p75,
        "p95_s": p95,
        "iqr_s": p75 - p25,
        "cv": cv,
    }


def format_time(seconds: float) -> str:
    if seconds < 0.001:
        return f"{seconds * 1_000_000:.1f} us"
    elif seconds < 1.0:
        return f"{seconds * 1_000:.3f} ms"
    else:
        return f"{seconds:.3f} s"


def format_throughput(rows: int, median_s: float) -> str:
    if median_s <= 0:
        return "N/A"
    tp = rows / median_s
    if tp >= 1_000_000:
        return f"{tp / 1_000_000:.2f}M rows/s"
    elif tp >= 1_000:
        return f"{tp / 1_000:.1f}K rows/s"
    else:
        return f"{tp:.0f} rows/s"


def format_throughput_mb(mb_per_s: float) -> str:
    if mb_per_s >= 1000:
        return f"{mb_per_s / 1000:.2f} GB/s"
    elif mb_per_s >= 1:
        return f"{mb_per_s:.1f} MB/s"
    else:
        return f"{mb_per_s * 1024:.1f} KB/s"


# ---------------------------------------------------------------------------
# Write functions — each wraps ONLY the write call
# ---------------------------------------------------------------------------

def write_sigilyx_polars(df: pl.DataFrame, tmp_path: str) -> tuple[int, int]:
    """Write a Polars DataFrame to YXDB. Returns (rows, cols)."""
    yx.write_yxdb(tmp_path, df)
    return df.height, df.width


def write_sigilyx_arrow(table, tmp_path: str) -> tuple[int, int]:
    """Write a PyArrow Table to YXDB. Returns (rows, cols)."""
    yx.write_yxdb_arrow(tmp_path, table)
    return table.num_rows, table.num_columns


def write_sigilyx_pandas(pdf, tmp_path: str) -> tuple[int, int]:
    """Write a Pandas DataFrame to YXDB. Returns (rows, cols)."""
    yx.write_yxdb_pandas(tmp_path, pdf)
    return len(pdf), len(pdf.columns)


# ---------------------------------------------------------------------------
# Data pre-loading
# ---------------------------------------------------------------------------

def preload_data(file_path: str) -> dict:
    """Read a .yxdb file into all target formats. Returns dict of format->data."""
    data = {}

    polars_df = yx.read_yxdb(file_path)
    data["polars"] = polars_df

    if HAS_PYARROW:
        data["arrow"] = yx.read_yxdb_arrow(file_path)

    if HAS_PANDAS:
        data["pandas"] = yx.read_yxdb_pandas(file_path)

    return data


# ---------------------------------------------------------------------------
# External C++ benchmark
# ---------------------------------------------------------------------------

BENCHMARKS_DIR = Path(__file__).resolve().parent
ALTERYX_WRITE_EXE = BENCHMARKS_DIR / "cpp" / "alteryx_openyxdb_write_benchmark.exe"
ALTERYX_BUILD_SCRIPT = BENCHMARKS_DIR / "cpp" / "build_alteryx_write.bat"


def find_or_build_alteryx_write_benchmark() -> Path | None:
    """Find or build the Alteryx OpenYXDB C++ write benchmark executable."""
    if ALTERYX_WRITE_EXE.exists():
        return ALTERYX_WRITE_EXE

    if not ALTERYX_BUILD_SCRIPT.exists():
        return None

    print("    Building Alteryx C++ write benchmark...", end=" ", flush=True)
    try:
        result = subprocess.run(
            ["cmd", "/c", str(ALTERYX_BUILD_SCRIPT)],
            capture_output=True, text=True, timeout=300,
            cwd=str(BENCHMARKS_DIR / "cpp"),
        )
        if result.returncode == 0 and ALTERYX_WRITE_EXE.exists():
            print("OK")
            return ALTERYX_WRITE_EXE
        else:
            print("FAILED")
            if result.stderr:
                print(f"      {result.stderr[:200]}")
            return None
    except (subprocess.TimeoutExpired, OSError) as e:
        print(f"FAILED ({e})")
        return None


def run_external_write_benchmark(
    exe_path: Path, source_file: str, runs: int
) -> dict | None:
    """Run the C++ write benchmark and parse its JSON output."""
    try:
        result = subprocess.run(
            [str(exe_path), source_file, str(runs)],
            capture_output=True, text=True, timeout=3600,
        )
        if result.returncode != 0:
            return None
        if not result.stdout.strip():
            return None
        return json.loads(result.stdout)
    except (subprocess.TimeoutExpired, json.JSONDecodeError, OSError):
        return None


# ---------------------------------------------------------------------------
# Target registry
# ---------------------------------------------------------------------------

TARGETS: dict[str, dict] = {}


def register_target(name: str, write_fn, data_key: str, input_type: str,
                    language: str, library: str, version: str, available: bool = True,
                    external: bool = False):
    TARGETS[name] = {
        "write_fn": write_fn,
        "data_key": data_key,
        "input_type": input_type,
        "language": language,
        "library": library,
        "version": version,
        "available": available,
        "external": external,
    }


# ---------------------------------------------------------------------------
# Benchmark runner
# ---------------------------------------------------------------------------

def benchmark_target(target_name: str, file_path: str, preloaded: dict, runs: int) -> dict | None:
    """Run warmup + timed write benchmark for one target on one file."""
    target = TARGETS[target_name]
    if not target["available"]:
        return None

    write_fn = target["write_fn"]
    data_key = target["data_key"]

    if data_key not in preloaded:
        return None

    data_obj = preloaded[data_key]

    with tempfile.TemporaryDirectory() as tmpdir:
        # --- Warmup ---
        rows, cols = 0, 0
        for i in range(WARMUP_RUNS):
            tmp_path = os.path.join(tmpdir, f"warmup_{i}.yxdb")
            result = write_fn(data_obj, tmp_path)
            if result is None:
                return None
            rows, cols = result

        # Clean up warmup files to free disk space
        for i in range(WARMUP_RUNS):
            tmp_path = os.path.join(tmpdir, f"warmup_{i}.yxdb")
            try:
                os.unlink(tmp_path)
            except OSError:
                pass

        # --- Timed runs ---
        gc.collect()
        gc.disable()
        times = []
        for i in range(runs):
            tmp_path = os.path.join(tmpdir, f"run_{i}.yxdb")

            start = time.perf_counter()
            write_fn(data_obj, tmp_path)
            elapsed = time.perf_counter() - start

            times.append(elapsed)
        gc.enable()

        # Capture output file size from last run
        last_path = os.path.join(tmpdir, f"run_{runs - 1}.yxdb")
        file_size = os.path.getsize(last_path)

    stats = compute_stats(times)
    fname = os.path.basename(file_path)
    source_file_size = os.path.getsize(file_path)

    throughput_rows = rows / stats["median_s"] if stats["median_s"] > 0 else 0
    throughput_mb = (file_size / 1_048_576) / stats["median_s"] if stats["median_s"] > 0 else 0

    return {
        "library": target["library"],
        "version": target["version"],
        "language": target["language"],
        "file": fname,
        "rows": rows,
        "cols": cols,
        "source_file_size_bytes": source_file_size,
        "output_file_size_bytes": file_size,
        "input_type": target["input_type"],
        "throughput_rows_per_s": throughput_rows,
        "throughput_mb_per_s": throughput_mb,
        **stats,
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="SigilYX Python multi-format write benchmark",
    )
    parser.add_argument("--runs", type=int, default=50,
                        help="Number of timed runs per target per file (default: 50)")
    parser.add_argument("--files", nargs="+", default=None,
                        help="Specific file names to benchmark")
    parser.add_argument("--data-dir", type=str, default=None,
                        help="Directory containing benchmark YXDB files")
    parser.add_argument("--output", type=str, default=None,
                        help="Write JSON results to this path")
    parser.add_argument("--no-cache", action="store_true",
                        help="Ignore cached results and re-run all benchmarks")
    args = parser.parse_args()

    use_cache = not args.no_cache

    # Get sigilyx version
    try:
        sigilyx_version = yx.__version__
    except AttributeError:
        sigilyx_version = "0.2.0"

    # Register targets
    register_target("sigilyx-write-polars", write_sigilyx_polars,
                     "polars", "Polars DataFrame",
                     "Python (Rust backend)", "sigilyx", sigilyx_version)
    register_target("sigilyx-write-arrow", write_sigilyx_arrow,
                     "arrow", "PyArrow Table",
                     "Python (Rust backend)", "sigilyx", sigilyx_version,
                     available=HAS_PYARROW)
    register_target("sigilyx-write-pandas", write_sigilyx_pandas,
                     "pandas", "Pandas DataFrame",
                     "Python (Rust backend)", "sigilyx", sigilyx_version,
                     available=HAS_PANDAS)

    # Detect/build external C++ write benchmark
    alteryx_write_exe = find_or_build_alteryx_write_benchmark()
    register_target("alteryx-openyxdb-write", None,
                     "external", "C++ RecordData (in-memory)",
                     "C++", "alteryx-openyxdb", "main (2024)",
                     available=alteryx_write_exe is not None,
                     external=True)

    # Resolve data directory
    if args.data_dir:
        data_dir = Path(args.data_dir)
    else:
        data_dir = Path(__file__).resolve().parent / "data"

    # Resolve test files
    if args.files:
        file_names = args.files
    else:
        if data_dir.exists():
            file_names = sorted(f.name for f in data_dir.glob("*.yxdb"))
        else:
            test_dir = PROJECT_ROOT / "sigilyx" / "test_files"
            file_names = ["ManyRecords.yxdb"]
            data_dir = test_dir

    test_files = []
    for f in file_names:
        p = data_dir / f
        if p.exists():
            test_files.append(str(p))
        else:
            alt = PROJECT_ROOT / "sigilyx" / "test_files" / f
            if alt.exists():
                test_files.append(str(alt))
            else:
                print(f"  WARNING: File not found: {f}")

    if not test_files:
        print("  ERROR: No test files found. Run generate_benchmark_data.py first.")
        sys.exit(1)

    # Banner
    print("=" * 80)
    print("SigilYX Multi-Format Write Benchmark")
    print("=" * 80)
    print(f"  Python:        {sys.version.split()[0]}")
    print(f"  sigilyx:       {sigilyx_version}")
    print(f"  polars:        {pl.__version__}")
    if HAS_PYARROW:
        print(f"  pyarrow:       {pyarrow.__version__}")
    else:
        print(f"  pyarrow:       not installed")
    if HAS_PANDAS:
        print(f"  pandas:        {pandas.__version__}")
    else:
        print(f"  pandas:        not installed")
    if alteryx_write_exe:
        print(f"  alteryx C++:   {alteryx_write_exe.name}")
    else:
        print(f"  alteryx C++:   not available")
    print(f"  Warmup runs:   {WARMUP_RUNS}")
    print(f"  Timed runs:    {args.runs}")
    print(f"  GC:            disabled during timed runs")
    print(f"  Timer:         time.perf_counter()")
    print(f"  Cache:         {'enabled (use --no-cache to force re-run)' if use_cache else 'disabled'}")
    print(f"  Test files:    {len(test_files)}")
    print()

    available_targets = [name for name, t in TARGETS.items() if t["available"]]
    print(f"  Targets: {', '.join(available_targets)}")
    print()

    # Main benchmark loop
    all_results = []

    for file_path in test_files:
        fname = os.path.basename(file_path)
        fsize = os.path.getsize(file_path)
        print("-" * 80)
        print(f"  File: {fname}  ({fsize / 1024:.1f} KB)")
        print("-" * 80)

        # Pre-load data into all formats (NOT timed)
        print(f"    Pre-loading data...", end=" ", flush=True)
        preloaded = preload_data(file_path)
        polars_df = preloaded.get("polars")
        if polars_df is not None:
            print(f"{polars_df.height:,} rows x {polars_df.width} cols")
        else:
            print("FAILED")
            continue

        for target_name in available_targets:
            fname_base = os.path.basename(file_path)
            print(f"    {target_name}... ", end="", flush=True)

            # Check cache first
            if use_cache:
                cached = load_cached_result(target_name, fname_base, args.runs)
                if cached is not None:
                    median = cached["median_s"]
                    cv = cached["cv"]
                    rows = cached["rows"]
                    tp_str = format_throughput(rows, median)
                    mb_str = format_throughput_mb(cached.get("throughput_mb_per_s", 0))
                    print(f"{format_time(median):>12}  CV={cv:.3f}  {tp_str}  {mb_str}  [cached]")
                    all_results.append(cached)
                    continue

            target = TARGETS[target_name]

            # External targets (C++ exe) handle their own warmup/timing
            if target["external"]:
                result = run_external_write_benchmark(
                    alteryx_write_exe, file_path, args.runs
                )
                if result is None:
                    print("SKIPPED")
                    continue
                result["target"] = target_name
            else:
                result = benchmark_target(target_name, file_path, preloaded, args.runs)
                if result is None:
                    print("SKIPPED")
                    continue

            median = result["median_s"]
            cv = result["cv"]
            rows = result["rows"]
            tp_str = format_throughput(rows, median)
            mb_str = format_throughput_mb(result.get("throughput_mb_per_s", 0))
            print(f"{format_time(median):>12}  CV={cv:.3f}  {tp_str}  {mb_str}")
            all_results.append(result)

            save_cached_result(target_name, fname_base, args.runs, result)

        print()

    # Summary table
    if all_results:
        print("=" * 80)
        print("SUMMARY")
        print("=" * 80)
        print()

        # Collect unique files in order
        files_seen = []
        for r in all_results:
            if r["file"] not in files_seen:
                files_seen.append(r["file"])

        # Short names for column headers
        short_names = {
            "sigilyx-write-polars": "write-polars",
            "sigilyx-write-arrow": "write-arrow",
            "sigilyx-write-pandas": "write-pandas",
            "alteryx-openyxdb-write": "write-cpp",
        }

        header = f"  {'File':<32}"
        for t in available_targets:
            header += f" {short_names.get(t, t):>16}"
        print(header)
        print(f"  {'-' * (32 + 17 * len(available_targets))}")

        for fname in files_seen:
            row = f"  {fname:<32}"
            for t in available_targets:
                # Match by library+input_type for SigilYX targets, by library for C++
                target_info = TARGETS[t]
                if target_info["external"]:
                    match = [r for r in all_results if r["file"] == fname
                             and r.get("library") == target_info["library"]
                             and r.get("output_type") == "write"]
                else:
                    match = [r for r in all_results if r["file"] == fname
                             and r.get("input_type") == target_info["input_type"]
                             and r.get("library") != "alteryx-openyxdb"]
                if match:
                    row += f" {format_time(match[0]['median_s']):>16}"
                else:
                    row += f" {'---':>16}"
            print(row)

        print()

    # Write JSON results
    output_path = args.output or str(Path(__file__).resolve().parent / "results_write_formats.json")
    with open(output_path, "w") as f:
        json.dump({
            "benchmark": "sigilyx-write-formats",
            "python_version": sys.version,
            "sigilyx_version": sigilyx_version,
            "polars_version": pl.__version__,
            "pyarrow_version": pyarrow.__version__ if HAS_PYARROW else None,
            "pandas_version": pandas.__version__ if HAS_PANDAS else None,
            "timed_runs": args.runs,
            "warmup_runs": WARMUP_RUNS,
            "results": all_results,
        }, f, indent=2)

    print(f"  Results written to: {output_path}")
    print("=" * 80)


if __name__ == "__main__":
    main()
