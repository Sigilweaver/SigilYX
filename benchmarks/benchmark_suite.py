#!/usr/bin/env python3
"""
SigilYX Benchmark Suite

Tests read performance across different data shapes using the included
test files. All test data is either minimal synthetic data or generated
at runtime.

Usage:
    python benchmarks/benchmark_suite.py

Not affiliated with Alteryx, Inc.
"""

import os
import sys
import time
from pathlib import Path

# Add project root to path
project_root = Path(__file__).parent.parent
sys.path.insert(0, str(project_root / "python"))

try:
    import sigilyx as yx
except ImportError as e:
    print(f"ERROR: {e}")
    print("Run `maturin develop --release` first.")
    sys.exit(1)


def benchmark_file(path: str, name: str, runs: int = 5) -> dict:
    """Benchmark a single YXDB file."""
    file_size = os.path.getsize(path) / 1024  # KB
    record_count = yx.record_count(path)
    col_count = len(yx.read_schema(path))
    
    # Warmup
    _ = yx.read(path)
    
    # Timed runs
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        df = yx.read(path)
        elapsed = time.perf_counter() - start
        times.append(elapsed)
    
    avg_time = sum(times) / len(times)
    min_time = min(times)
    throughput = record_count / avg_time if avg_time > 0 else 0
    
    return {
        "name": name,
        "rows": record_count,
        "cols": col_count,
        "size_kb": file_size,
        "avg_time_ms": avg_time * 1000,
        "min_time_ms": min_time * 1000,
        "throughput": throughput,
    }


def print_result(result: dict):
    """Print a single benchmark result."""
    print(f"  {result['name']:28} | {result['rows']:>8,} × {result['cols']:<2} | "
          f"{result['avg_time_ms']:>8.2f} ms | {result['throughput']:>12,.0f} rows/sec")


def main():
    print("=" * 78)
    print("SigilYX Benchmark Suite")
    print("=" * 78)
    print("Testing read performance across different data shapes and types.\n")
    
    test_files_dir = project_root / "sigilyx" / "test_files"
    results = []
    
    # ===== TEST FILES =====
    # These files are included in the repo and contain synthetic test data
    print("[Test Files] Included YXDB files with varied types")
    print("-" * 78)
    print(f"  {'File':<28} | {'Shape':>12} | {'Time':>11} | {'Throughput':>16}")
    print("-" * 78)
    
    test_files = [
        ("AllTypes.yxdb", "All 16 field types"),
        ("NullValues.yxdb", "Nullable fields"),
        ("Strings.yxdb", "String-heavy"),
        ("People.yxdb", "Mixed types"),
        ("LargeBlob.yxdb", "Binary/blob data"),
        ("ManyRecords.yxdb", "Medium volume"),
    ]
    
    for filename, description in test_files:
        path = str(test_files_dir / filename)
        if os.path.exists(path):
            result = benchmark_file(path, description)
            results.append(result)
            print_result(result)
    
    # ===== SUMMARY =====
    print("\n" + "=" * 78)
    print("SUMMARY")
    print("=" * 78)
    
    if results:
        total_rows = sum(r["rows"] for r in results)
        total_time_ms = sum(r["avg_time_ms"] for r in results)
        overall_throughput = total_rows / (total_time_ms / 1000) if total_time_ms > 0 else 0
        
        print(f"  Files tested:       {len(results)}")
        print(f"  Total rows:         {total_rows:,}")
        print(f"  Total time:         {total_time_ms:.2f} ms")
        print(f"  Avg throughput:     {overall_throughput:,.0f} rows/sec")
        
        # Breakdown by type
        print("\n  Performance by data characteristic:")
        
        # Find fastest (likely numeric-heavy)
        fastest = max(results, key=lambda r: r["throughput"])
        print(f"    Fastest: {fastest['name']} ({fastest['throughput']:,.0f} rows/sec)")
        
        # Find most rows
        largest = max(results, key=lambda r: r["rows"])
        print(f"    Largest: {largest['name']} ({largest['rows']:,} rows in {largest['avg_time_ms']:.1f}ms)")
    
    print("\n" + "=" * 78)
    print("Benchmark complete.")
    print("=" * 78)


if __name__ == "__main__":
    main()
