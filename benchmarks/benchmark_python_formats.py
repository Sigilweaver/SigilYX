#!/usr/bin/env python3
"""
SigilYX Python Multi-Format Read Benchmark
============================================

Benchmarks SigilYX read performance across all Python output formats,
plus yxdb-py as a baseline. Measures only the read call itself.

Targets:
  - sigilyx-py-polars:  sigilyx.read_yxdb()       -> polars.DataFrame
  - sigilyx-py-arrow:   sigilyx.read_yxdb_arrow()  -> pyarrow.Table
  - sigilyx-py-pandas:  sigilyx.read_yxdb_pandas() -> pandas.DataFrame
  - yxdb-py:            yxdb row-by-row iteration   -> dict[str, list]

Methodology:
  - 10 warmup runs per target per file
  - GC disabled during timed runs
  - time.perf_counter() (monotonic, ~100 ns on Windows)
  - Each run opens the file fresh (no cached reader)
  - Timer wraps only the read call (no setup, no post-processing)

Result caching:
  - Results are cached to .bench_cache/ keyed by (target, file, runs)
  - Subsequent runs load cached results instead of re-benchmarking
  - Use --no-cache to force a fresh run
  - Delete .bench_cache/ to clear all cached results

Usage:
    python benchmarks/benchmark_python_formats.py
    python benchmarks/benchmark_python_formats.py --runs 200
    python benchmarks/benchmark_python_formats.py --files bench_mixed_100000.yxdb
    python benchmarks/benchmark_python_formats.py --data-dir benchmarks/data
    python benchmarks/benchmark_python_formats.py --no-cache
"""

from __future__ import annotations

import argparse
import gc
import hashlib
import json
import math
import os
import sys
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

# Check for yxdb-py
try:
    from yxdb.yxdb_reader import YxdbReader
    HAS_YXDB_PY = True
except ImportError:
    HAS_YXDB_PY = False


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


# ---------------------------------------------------------------------------
# Read functions — each wraps ONLY the read call
# ---------------------------------------------------------------------------

def read_sigilyx_polars(path: str) -> tuple[int, int]:
    df = yx.read_yxdb(path)
    return df.height, df.width


def read_sigilyx_arrow(path: str) -> tuple[int, int]:
    table = yx.read_yxdb_arrow(path)
    return table.num_rows, table.num_columns


def read_sigilyx_pandas(path: str) -> tuple[int, int]:
    pdf = yx.read_yxdb_pandas(path)
    return len(pdf), len(pdf.columns)


def read_yxdb_py(path: str) -> tuple[int, int] | None:
    try:
        reader = YxdbReader(path=path)
    except Exception:
        return None

    num_cols = len(reader._fields)
    num_rows = 0

    while reader.next():
        for i in range(num_cols):
            _ = reader.read_index(i)
        num_rows += 1

    return num_rows, num_cols


def read_sigilyx_rows(path: str) -> tuple[int, int]:
    reader = yx.YxdbRowReader(path)
    num_cols = len(reader.fields)
    num_rows = 0
    while reader.next():
        _ = reader.read_all()
        num_rows += 1
    reader.close()
    return num_rows, num_cols


# ---------------------------------------------------------------------------
# Benchmark runner
# ---------------------------------------------------------------------------

TARGETS = {}


def register_target(name, read_fn, language, library, output_type, version, available=True):
    TARGETS[name] = {
        "read_fn": read_fn,
        "language": language,
        "library": library,
        "output_type": output_type,
        "version": version,
        "available": available,
    }


def benchmark_target(target_name: str, file_path: str, runs: int) -> dict | None:
    target = TARGETS[target_name]
    if not target["available"]:
        return None

    read_fn = target["read_fn"]

    # Warmup
    rows, cols = 0, 0
    for _ in range(WARMUP_RUNS):
        result = read_fn(file_path)
        if result is None:
            return None
        rows, cols = result

    # Timed runs
    gc.collect()
    gc.disable()
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        result = read_fn(file_path)
        elapsed = time.perf_counter() - start
        if result is None:
            gc.enable()
            return None
        times.append(elapsed)
    gc.enable()

    stats = compute_stats(times)
    file_size = os.path.getsize(file_path)
    fname = os.path.basename(file_path)

    throughput = rows / stats["median_s"] if stats["median_s"] > 0 else 0

    return {
        "library": target["library"],
        "version": target["version"],
        "language": target["language"],
        "file": fname,
        "rows": rows,
        "cols": cols,
        "file_size_bytes": file_size,
        "output_type": target["output_type"],
        "throughput_rows_per_s": throughput,
        **stats,
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="SigilYX Python multi-format read benchmark",
    )
    parser.add_argument("--runs", type=int, default=100,
                        help="Number of timed runs per target per file (default: 100)")
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
        sigilyx_version = "0.1.0"

    # Register targets
    register_target("sigilyx-py-polars", read_sigilyx_polars,
                     "Python (Rust backend)", "sigilyx", "Polars DataFrame",
                     sigilyx_version)
    register_target("sigilyx-py-arrow", read_sigilyx_arrow,
                     "Python (Rust backend)", "sigilyx", "PyArrow Table",
                     sigilyx_version, available=HAS_PYARROW)
    register_target("sigilyx-py-pandas", read_sigilyx_pandas,
                     "Python (Rust backend)", "sigilyx", "Pandas DataFrame",
                     sigilyx_version, available=HAS_PANDAS)
    register_target("sigilyx-py-rows", read_sigilyx_rows,
                     "Python (Rust backend)", "sigilyx", "tuple (row-by-row)",
                     sigilyx_version)

    yxdb_py_version = "unknown"
    if HAS_YXDB_PY:
        try:
            import importlib.metadata
            yxdb_py_version = importlib.metadata.version("yxdb")
        except Exception:
            yxdb_py_version = "1.1.1"
    register_target("yxdb-py", read_yxdb_py,
                     "Pure Python", "yxdb-py", "dict[str, list]",
                     yxdb_py_version, available=HAS_YXDB_PY)

    # Resolve data files
    if args.data_dir:
        data_dir = Path(args.data_dir)
    else:
        data_dir = Path(__file__).resolve().parent / "data"

    if args.files:
        file_names = args.files
    else:
        # Auto-discover .yxdb files in data dir
        if data_dir.exists():
            file_names = sorted(f.name for f in data_dir.glob("*.yxdb"))
        else:
            # Fallback to test_files
            test_dir = PROJECT_ROOT / "sigilyx" / "test_files"
            file_names = ["ManyRecords.yxdb"]
            data_dir = test_dir

    test_files = []
    for f in file_names:
        p = data_dir / f
        if p.exists():
            test_files.append(str(p))
        else:
            # Try sigilyx test_files as fallback
            alt = PROJECT_ROOT / "sigilyx" / "test_files" / f
            if alt.exists():
                test_files.append(str(alt))
            else:
                print(f"  WARNING: File not found: {f}")

    if not test_files:
        print("  ERROR: No test files found.")
        sys.exit(1)

    # Banner
    print("=" * 80)
    print("SigilYX Python Multi-Format Read Benchmark")
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
    if HAS_YXDB_PY:
        print(f"  yxdb-py:       {yxdb_py_version}")
    else:
        print(f"  yxdb-py:       not installed")
    print(f"  Warmup runs:   {WARMUP_RUNS}")
    print(f"  Timed runs:    {args.runs}")
    print(f"  GC:            disabled during timed runs")
    print(f"  Timer:         time.perf_counter()")
    print(f"  Cache:         {'enabled (use --no-cache to force re-run)' if use_cache else 'disabled'}")
    print(f"  Test files:    {len(test_files)}")
    print()

    # Available targets
    available_targets = [name for name, t in TARGETS.items() if t["available"]]
    print(f"  Targets: {', '.join(available_targets)}")
    print()

    all_results = []

    for file_path in test_files:
        fname = os.path.basename(file_path)
        fsize = os.path.getsize(file_path)
        print("-" * 80)
        print(f"  File: {fname}  ({fsize / 1024:.1f} KB)")
        print("-" * 80)

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
                    print(f"{format_time(median):>12}  CV={cv:.3f}  {tp_str}  [cached]")
                    all_results.append(cached)
                    continue

            result = benchmark_target(target_name, file_path, args.runs)
            if result is None:
                print("SKIPPED (cannot read file)")
                continue

            median = result["median_s"]
            cv = result["cv"]
            rows = result["rows"]
            tp_str = format_throughput(rows, median)
            print(f"{format_time(median):>12}  CV={cv:.3f}  {tp_str}")
            all_results.append(result)

            # Save to cache
            save_cached_result(target_name, fname_base, args.runs, result)

        print()

    # Summary table
    if all_results:
        print("=" * 80)
        print("SUMMARY")
        print("=" * 80)
        print()

        # Group by file
        files_seen = []
        for r in all_results:
            if r["file"] not in files_seen:
                files_seen.append(r["file"])

        header = f"  {'File':<30}"
        for t in available_targets:
            header += f" {t:>16}"
        print(header)
        print(f"  {'-' * (30 + 17 * len(available_targets))}")

        for fname in files_seen:
            row = f"  {fname:<30}"
            for t in available_targets:
                match = [r for r in all_results if r["file"] == fname and r.get("library") == TARGETS[t]["library"]
                         and r.get("output_type") == TARGETS[t]["output_type"]]
                if match:
                    row += f" {format_time(match[0]['median_s']):>16}"
                else:
                    row += f" {'N/A':>16}"
            print(row)

        print()

    # Write JSON
    output_path = args.output or str(Path(__file__).resolve().parent / "results_python_formats.json")
    with open(output_path, "w") as f:
        json.dump({
            "benchmark": "sigilyx-python-formats",
            "python_version": sys.version,
            "sigilyx_version": sigilyx_version,
            "polars_version": pl.__version__,
            "pyarrow_version": pyarrow.__version__ if HAS_PYARROW else None,
            "pandas_version": pandas.__version__ if HAS_PANDAS else None,
            "yxdb_py_version": yxdb_py_version if HAS_YXDB_PY else None,
            "timed_runs": args.runs,
            "warmup_runs": WARMUP_RUNS,
            "results": all_results,
        }, f, indent=2)

    print(f"  Results written to: {output_path}")
    print("=" * 80)


if __name__ == "__main__":
    main()
