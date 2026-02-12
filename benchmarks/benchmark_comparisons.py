#!/usr/bin/env python3
"""
SigilYX Comparative Read Benchmark
====================================

Benchmarks SigilYX against other Python-accessible YXDB reading libraries:
  - sigilyx  (Rust-backed, returns Polars DataFrame via Arrow IPC)
  - yxdb     (pure Python, row-by-row iteration)

Cross-language benchmarks (yxdb-go, yxdb-net, Open_AlteryxYXDB) are handled
by the separate orchestrator script: benchmark_cross_language.py

Methodology
-----------
See METHODOLOGY.md for full details. Key points:

  - Minimum 100 timed runs per library per file (configurable via --runs).
  - 10 warmup runs before measurement to stabilize OS page cache and
    Python import / JIT / allocation caches.
  - Wall-clock time measured with time.perf_counter() (sub-microsecond
    resolution on Windows, nanosecond on Linux).
  - Garbage collection is disabled during timed runs and forced between
    libraries to prevent GC pauses from skewing results.
  - Both libraries read ALL records and ALL fields from each file.
    SigilYX materializes a Polars DataFrame; yxdb-py accumulates values
    into a dict-of-lists (its natural output).
  - Statistical output: mean, median, std dev, min, max, p5, p95, IQR,
    coefficient of variation (CV).
  - Results are written to JSON for downstream analysis and cross-language
    comparison.

Usage
-----
    python benchmarks/benchmark_comparisons.py
    python benchmarks/benchmark_comparisons.py --runs 200
    python benchmarks/benchmark_comparisons.py --runs 100 --output results.json
    python benchmarks/benchmark_comparisons.py --files ManyRecords.yxdb People.yxdb
"""

from __future__ import annotations

import argparse
import gc
import json
import os
import statistics
import sys
import time
from pathlib import Path

# ---------------------------------------------------------------------------
# Project setup
# ---------------------------------------------------------------------------
PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "python"))

TEST_FILES_DIR = PROJECT_ROOT / "sigilyx" / "test_files"

# Default test files ordered by expected runtime (smallest first)
DEFAULT_TEST_FILES = [
    "AllTypes.yxdb",        # 2 rows × 16 cols      — type coverage
    "Strings.yxdb",         # 6 rows × 5 cols       — string-heavy
    "NullValues.yxdb",      # 3 rows × 11 cols      — nullable fields
    "People.yxdb",          # 200 rows × 8 cols     — mixed types
    "ManyRecords.yxdb",     # 50 000 rows × 3 cols  — primary throughput test
]

WARMUP_RUNS = 10


# ═══════════════════════════════════════════════════════════════════════════
# Statistical helpers
# ═══════════════════════════════════════════════════════════════════════════

def compute_stats(times: list[float]) -> dict:
    """Compute descriptive statistics for a list of timing measurements.

    Returns a dict with: count, mean, median, stdev, min, max,
    p5, p25, p75, p95, iqr, cv (coefficient of variation).
    """
    n = len(times)
    if n == 0:
        return {}

    sorted_t = sorted(times)
    mean = statistics.mean(sorted_t)
    median = statistics.median(sorted_t)
    stdev = statistics.stdev(sorted_t) if n > 1 else 0.0
    p5 = sorted_t[max(0, int(n * 0.05))]
    p25 = sorted_t[max(0, int(n * 0.25))]
    p75 = sorted_t[min(n - 1, int(n * 0.75))]
    p95 = sorted_t[min(n - 1, int(n * 0.95))]
    iqr = p75 - p25
    cv = (stdev / mean) if mean > 0 else 0.0

    return {
        "count": n,
        "mean_s": mean,
        "median_s": median,
        "stdev_s": stdev,
        "min_s": min(sorted_t),
        "max_s": max(sorted_t),
        "p5_s": p5,
        "p25_s": p25,
        "p75_s": p75,
        "p95_s": p95,
        "iqr_s": iqr,
        "cv": cv,
    }


def format_time(seconds: float) -> str:
    """Format a time value for display."""
    if seconds < 0.001:
        return f"{seconds * 1_000_000:.1f} us"
    elif seconds < 1.0:
        return f"{seconds * 1_000:.3f} ms"
    else:
        return f"{seconds:.3f} s"


def format_throughput(rows_per_sec: float) -> str:
    """Format throughput for display."""
    if rows_per_sec >= 1_000_000:
        return f"{rows_per_sec / 1_000_000:.2f}M rows/s"
    elif rows_per_sec >= 1_000:
        return f"{rows_per_sec / 1_000:.1f}K rows/s"
    else:
        return f"{rows_per_sec:.0f} rows/s"


# ═══════════════════════════════════════════════════════════════════════════
# Benchmark runners
# ═══════════════════════════════════════════════════════════════════════════

def benchmark_sigilyx(file_path: str, runs: int) -> dict:
    """Benchmark SigilYX read performance.

    Measures the time to call yx.read(path), which:
      1. Opens the file
      2. Parses the 512-byte header and XML metadata
      3. Decompresses all LZF blocks
      4. Extracts all field values into Arrow columnar arrays
      5. Serializes to Arrow IPC
      6. Deserializes in Python to a Polars DataFrame

    The result is a fully materialized in-memory DataFrame.
    """
    import sigilyx as yx

    record_count = yx.record_count(file_path)
    schema = yx.read_schema(file_path)
    col_count = len(schema)

    # Warmup: populate OS page cache + Python caches
    for _ in range(WARMUP_RUNS):
        _ = yx.read(file_path)

    # Timed runs with GC disabled
    gc.collect()
    gc.disable()
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        df = yx.read(file_path)
        elapsed = time.perf_counter() - start
        times.append(elapsed)
    gc.enable()

    stats = compute_stats(times)
    throughput = record_count / stats["median_s"] if stats["median_s"] > 0 else 0

    return {
        "library": "sigilyx",
        "language": "Rust (Python bindings)",
        "file": os.path.basename(file_path),
        "rows": record_count,
        "cols": col_count,
        "file_size_bytes": os.path.getsize(file_path),
        "output_type": "polars.DataFrame",
        "throughput_rows_per_s": throughput,
        **stats,
    }


def benchmark_yxdb_py(file_path: str, runs: int) -> dict | None:
    """Benchmark yxdb (pure Python) read performance.

    Measures the time to:
      1. Construct a YxdbReader (opens file, parses header + XML metadata)
      2. Iterate all records via reader.next()
      3. Extract every field value via reader.read_index()
      4. Accumulate values into a dict-of-lists

    The dict-of-lists output is the natural equivalent of a DataFrame's
    underlying columnar storage. This is the fairest end-to-end comparison:
    both libraries start from the same file on disk and end with all data
    materialized in RAM in a structured columnar form.

    Note: yxdb-py does not offer a bulk/columnar read mode. The row-by-row
    iteration with per-field extraction is its only API. This is a
    fundamental design difference, not a benchmark artifact.
    """
    try:
        from yxdb.yxdb_reader import YxdbReader
    except ImportError:
        return None

    # Pre-read to get metadata.
    # yxdb-py does not support all YXDB field types; if it fails to open
    # a file, we skip it and return None rather than crashing.
    try:
        reader = YxdbReader(path=file_path)
    except Exception:
        return None
    fields = reader.list_fields()
    col_count = len(fields)
    # Count records by iterating (yxdb-py has no header record count API)
    record_count = 0
    while reader.next():
        record_count += 1
    reader.close()

    # Warmup
    for _ in range(WARMUP_RUNS):
        r = YxdbReader(path=file_path)
        flds = r.list_fields()
        data = {f.name: [] for f in flds}
        while r.next():
            for j in range(len(flds)):
                data[flds[j].name].append(r.read_index(j))
        r.close()

    # Timed runs with GC disabled
    gc.collect()
    gc.disable()
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        r = YxdbReader(path=file_path)
        flds = r.list_fields()
        data = {f.name: [] for f in flds}
        while r.next():
            for j in range(len(flds)):
                data[flds[j].name].append(r.read_index(j))
        r.close()
        elapsed = time.perf_counter() - start
        times.append(elapsed)
    gc.enable()

    stats = compute_stats(times)
    throughput = record_count / stats["median_s"] if stats["median_s"] > 0 else 0

    return {
        "library": "yxdb",
        "language": "Pure Python",
        "file": os.path.basename(file_path),
        "rows": record_count,
        "cols": col_count,
        "file_size_bytes": os.path.getsize(file_path),
        "output_type": "dict[str, list]",
        "throughput_rows_per_s": throughput,
        **stats,
    }


# ═══════════════════════════════════════════════════════════════════════════
# Display
# ═══════════════════════════════════════════════════════════════════════════

def print_header():
    """Print benchmark banner."""
    print("=" * 80)
    print("SigilYX Comparative Read Benchmark")
    print("=" * 80)
    print()
    print(f"  Platform:      {sys.platform}")
    print(f"  Python:        {sys.version.split()[0]}")
    print(f"  Warmup runs:   {WARMUP_RUNS}")
    print(f"  Timer:         time.perf_counter()")
    print(f"  GC:            disabled during timed runs")
    print()


def print_file_header(file_path: str, rows: int, cols: int):
    """Print header for a test file."""
    fname = os.path.basename(file_path)
    fsize = os.path.getsize(file_path)
    print("-" * 80)
    print(f"  File: {fname}  ({fsize / 1024:.1f} KB, {rows:,} rows x {cols} cols)")
    print("-" * 80)


def print_result(result: dict, indent: str = "    "):
    """Print a single library result."""
    lib = result["library"]
    lang = result["language"]
    mean = result["mean_s"]
    median = result["median_s"]
    stdev = result["stdev_s"]
    cv = result["cv"]
    p5 = result["p5_s"]
    p95 = result["p95_s"]
    tp = result["throughput_rows_per_s"]

    print(f"{indent}{lib} ({lang})")
    print(f"{indent}  Median:     {format_time(median)}")
    print(f"{indent}  Mean:       {format_time(mean)}  (stdev: {format_time(stdev)}, CV: {cv:.3f})")
    print(f"{indent}  Range:      [{format_time(result['min_s'])} .. {format_time(result['max_s'])}]")
    print(f"{indent}  P5..P95:    [{format_time(p5)} .. {format_time(p95)}]")
    print(f"{indent}  IQR:        {format_time(result['iqr_s'])}")
    print(f"{indent}  Throughput:  {format_throughput(tp)}")
    print()


def print_comparison(sigilyx_result: dict, yxdb_result: dict | None, yxdb_py_available: bool = False, indent: str = "    "):
    """Print head-to-head comparison."""
    if yxdb_result is None:
        if not yxdb_py_available:
            print(f"{indent}yxdb-py not installed -- skipping comparison")
            print(f"{indent}Install with: pip install yxdb")
        return

    sig_median = sigilyx_result["median_s"]
    yxdb_median = yxdb_result["median_s"]

    if sig_median > 0:
        speedup = yxdb_median / sig_median
        print(f"{indent}Speedup (median): {speedup:.1f}x faster (sigilyx vs yxdb-py)")
    print()


def print_summary(all_results: list[dict]):
    """Print overall summary."""
    print()
    print("=" * 80)
    print("SUMMARY")
    print("=" * 80)
    print()

    # Group by file
    files = {}
    for r in all_results:
        fname = r["file"]
        if fname not in files:
            files[fname] = {}
        files[fname][r["library"]] = r

    print(f"  {'File':<22} {'sigilyx':>14} {'yxdb-py':>14} {'Speedup':>10}")
    print(f"  {'-' * 22} {'-' * 14} {'-' * 14} {'-' * 10}")

    for fname, libs in files.items():
        sig = libs.get("sigilyx")
        yxdb = libs.get("yxdb")
        sig_str = format_time(sig["median_s"]) if sig else "N/A"
        yxdb_str = format_time(yxdb["median_s"]) if yxdb else "N/A"
        if sig and yxdb and sig["median_s"] > 0:
            speedup = yxdb["median_s"] / sig["median_s"]
            speedup_str = f"{speedup:.1f}x"
        else:
            speedup_str = "N/A"
        print(f"  {fname:<22} {sig_str:>14} {yxdb_str:>14} {speedup_str:>10}")

    print()
    print("  Metric: median wall-clock time over N runs (lower is better)")
    print("  Speedup: how many times faster sigilyx is vs yxdb-py")
    print()


# ═══════════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════════

def main():
    parser = argparse.ArgumentParser(
        description="Benchmark SigilYX against other YXDB readers",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--runs", type=int, default=100,
        help="Number of timed runs per library per file (default: 100)",
    )
    parser.add_argument(
        "--files", nargs="+", default=None,
        help="Specific test file names to benchmark (default: all)",
    )
    parser.add_argument(
        "--output", type=str, default=None,
        help="Write JSON results to this path",
    )
    parser.add_argument(
        "--skip-yxdb-py", action="store_true",
        help="Skip yxdb-py benchmark (only run SigilYX)",
    )
    args = parser.parse_args()

    # Resolve test files
    if args.files:
        test_files = args.files
    else:
        test_files = DEFAULT_TEST_FILES

    resolved_files = []
    for f in test_files:
        p = TEST_FILES_DIR / f
        if p.exists():
            resolved_files.append(str(p))
        else:
            print(f"WARNING: Test file not found: {p}")

    if not resolved_files:
        print("ERROR: No test files found.")
        sys.exit(1)

    # Check library availability
    sigilyx_available = True
    try:
        import sigilyx as yx
    except ImportError:
        print("ERROR: sigilyx not installed. Run `maturin develop --release` first.")
        sys.exit(1)

    yxdb_py_available = False
    if not args.skip_yxdb_py:
        try:
            from yxdb.yxdb_reader import YxdbReader
            yxdb_py_available = True
        except ImportError:
            pass

    # Print header
    print_header()
    print(f"  Timed runs:    {args.runs}")
    print(f"  Test files:    {len(resolved_files)}")
    print(f"  Libraries:     sigilyx" + (", yxdb-py" if yxdb_py_available else " (yxdb-py not installed)"))
    print()

    all_results = []

    for file_path in resolved_files:
        # Get file metadata from sigilyx (fast)
        rows = yx.record_count(file_path)
        cols = len(yx.read_schema(file_path))
        print_file_header(file_path, rows, cols)

        # Benchmark SigilYX
        print(f"    Benchmarking sigilyx ({args.runs} runs)...")
        sig_result = benchmark_sigilyx(file_path, args.runs)
        all_results.append(sig_result)
        print_result(sig_result)

        # Benchmark yxdb-py
        yxdb_result = None
        if yxdb_py_available:
            print(f"    Benchmarking yxdb-py ({args.runs} runs)...")
            yxdb_result = benchmark_yxdb_py(file_path, args.runs)
            if yxdb_result:
                all_results.append(yxdb_result)
                print_result(yxdb_result)
            else:
                print(f"    yxdb-py: cannot read this file (unsupported field type)")
                print()

        # Comparison
        print_comparison(sig_result, yxdb_result, yxdb_py_available)

    # Summary
    print_summary(all_results)

    # Write JSON output
    if args.output:
        output_path = args.output
    else:
        output_path = str(PROJECT_ROOT / "benchmarks" / "results_python.json")

    with open(output_path, "w") as f:
        json.dump({
            "benchmark": "sigilyx-comparative-read",
            "platform": sys.platform,
            "python_version": sys.version,
            "warmup_runs": WARMUP_RUNS,
            "timed_runs": args.runs,
            "gc_disabled": True,
            "timer": "time.perf_counter",
            "results": all_results,
        }, f, indent=2)

    print(f"  Results written to: {output_path}")
    print()


if __name__ == "__main__":
    main()
