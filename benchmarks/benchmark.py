#!/usr/bin/env python3
"""
SigilYX Public Benchmark Script

This script benchmarks SigilYX read performance on YXDB files.

Usage:
    python benchmarks/benchmark_public.py [path/to/file.yxdb]

If no file is provided, it will use the generated benchmark data.

"""

from __future__ import annotations

import argparse
import os
import sys
import time
from pathlib import Path

# Ensure sigilyx is importable
project_root = Path(__file__).parent.parent
sys.path.insert(0, str(project_root / "python"))

try:
    import sigilyx as yx
except ImportError:
    print("ERROR: sigilyx not installed. Run `maturin develop --release` first.")
    sys.exit(1)


def benchmark_sigilyx(file_path: str, warmup_runs: int = 1, timed_runs: int = 3) -> dict:
    """Benchmark SigilYX read performance."""
    
    print(f"\n{'='*60}")
    print(f"SigilYX Benchmark: {file_path}")
    print(f"{'='*60}")
    
    # Warmup
    print(f"\nWarmup ({warmup_runs} run(s))...")
    for _ in range(warmup_runs):
        _ = yx.read(file_path)
    
    # Get record count
    record_count = yx.record_count(file_path)
    print(f"Records: {record_count:,}")
    
    # Timed runs
    print(f"\nTimed runs ({timed_runs} run(s))...")
    times = []
    for i in range(timed_runs):
        start = time.perf_counter()
        df = yx.read(file_path)
        elapsed = time.perf_counter() - start
        times.append(elapsed)
        rows_per_sec = record_count / elapsed
        print(f"  Run {i+1}: {elapsed:.3f}s ({rows_per_sec:,.0f} rows/sec)")
    
    avg_time = sum(times) / len(times)
    avg_throughput = record_count / avg_time
    
    print(f"\nResults:")
    print(f"  Average time: {avg_time:.3f}s")
    print(f"  Throughput:   {avg_throughput:,.0f} rows/sec")
    print(f"  DataFrame shape: {df.shape}")
    
    return {
        "file": file_path,
        "records": record_count,
        "avg_time_sec": avg_time,
        "throughput_rows_per_sec": avg_throughput,
    }


def main():
    parser = argparse.ArgumentParser(description="Benchmark SigilYX YXDB reader")
    parser.add_argument(
        "file",
        nargs="?",
        default=None,
        help="Path to YXDB file to benchmark",
    )
    parser.add_argument(
        "--warmup",
        type=int,
        default=1,
        help="Number of warmup runs (default: 1)",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=3,
        help="Number of timed runs (default: 3)",
    )
    args = parser.parse_args()
    
    # Find test file
    if args.file:
        test_file = args.file
    else:
        # Try to find generated benchmark file
        benchmark_dir = Path(__file__).parent
        potential_files = list(benchmark_dir.glob("*.yxdb"))
        if not potential_files:
            # Fall back to test files
            test_dir = project_root / "sigilyx" / "test_files"
            potential_files = list(test_dir.glob("*.yxdb"))
        
        if not potential_files:
            print("ERROR: No YXDB file found. Please provide a file path.")
            print("       Run `python benchmarks/generate_clean_data.py` first.")
            sys.exit(1)
        
        # Use the largest file
        test_file = str(max(potential_files, key=lambda f: f.stat().st_size))
    
    if not os.path.exists(test_file):
        print(f"ERROR: File not found: {test_file}")
        sys.exit(1)
    
    result = benchmark_sigilyx(test_file, args.warmup, args.runs)
    
    print("\n" + "="*60)
    print("Benchmark complete!")
    print("="*60)


if __name__ == "__main__":
    main()
