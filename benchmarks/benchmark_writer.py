"""
Benchmark SigilYX Writer Performance

Tests write throughput for various data shapes and sizes.
"""

import os
import sys
import tempfile
import time
from pathlib import Path

import polars as pl

sys.path.insert(0, str(Path(__file__).parent.parent))
import sigilyx as yx

# Benchmark configurations
BENCHMARKS = [
    {
        "name": "Small simple",
        "rows": 1_000,
        "schema": {"id": pl.Int64, "value": pl.Float64, "name": pl.String},
    },
    {
        "name": "Medium volume",
        "rows": 50_000,
        "schema": {"id": pl.Int64, "value": pl.Float64, "label": pl.String},
    },
    {
        "name": "Large volume",
        "rows": 500_000,
        "schema": {"id": pl.Int64, "value": pl.Float64, "label": pl.String},
    },
    {
        "name": "Very large",
        "rows": 1_000_000,
        "schema": {"id": pl.Int64, "value": pl.Float64},
    },
    {
        "name": "Wide table (20 cols)",
        "rows": 100_000,
        "schema": {f"col_{i}": pl.Int64 for i in range(20)},
    },
    {
        "name": "String-heavy",
        "rows": 50_000,
        "schema": {"id": pl.Int64, "s1": pl.String, "s2": pl.String, "s3": pl.String, "s4": pl.String},
    },
    {
        "name": "Mixed types",
        "rows": 100_000,
        "schema": {
            "id": pl.Int64,
            "flag": pl.Boolean,
            "score": pl.Float64,
            "name": pl.String,
            "count": pl.Int32,
        },
    },
]


def generate_data(rows: int, schema: dict) -> pl.DataFrame:
    """Generate test data matching the schema."""
    data = {}
    for col, dtype in schema.items():
        if dtype == pl.Int64:
            data[col] = list(range(rows))
        elif dtype == pl.Int32:
            data[col] = [i % 1000 for i in range(rows)]
        elif dtype == pl.Float64:
            data[col] = [i * 1.5 for i in range(rows)]
        elif dtype == pl.String:
            data[col] = [f"value_{i}" for i in range(rows)]
        elif dtype == pl.Boolean:
            data[col] = [i % 2 == 0 for i in range(rows)]
    return pl.DataFrame(data)


def benchmark_write(df: pl.DataFrame, iterations: int = 5) -> dict:
    """Benchmark write performance."""
    times = []
    file_size = 0
    
    with tempfile.TemporaryDirectory() as tmpdir:
        for i in range(iterations):
            path = os.path.join(tmpdir, f"bench_{i}.yxdb")
            
            start = time.perf_counter()
            yx.write_yxdb(path, df)
            elapsed = time.perf_counter() - start
            
            times.append(elapsed)
            file_size = os.path.getsize(path)
    
    avg_time = sum(times) / len(times)
    min_time = min(times)
    
    return {
        "avg_time_ms": avg_time * 1000,
        "min_time_ms": min_time * 1000,
        "rows_per_sec": df.height / avg_time,
        "file_size_kb": file_size / 1024,
    }


def benchmark_read(path: str, iterations: int = 5) -> dict:
    """Benchmark read performance for comparison."""
    times = []
    
    for _ in range(iterations):
        start = time.perf_counter()
        df = yx.read_yxdb(path)
        elapsed = time.perf_counter() - start
        times.append(elapsed)
    
    avg_time = sum(times) / len(times)
    return {
        "avg_time_ms": avg_time * 1000,
        "rows_per_sec": df.height / avg_time,
    }


def benchmark_roundtrip(df: pl.DataFrame, iterations: int = 5) -> dict:
    """Benchmark full write + read roundtrip."""
    times = []
    
    with tempfile.TemporaryDirectory() as tmpdir:
        for i in range(iterations):
            path = os.path.join(tmpdir, f"roundtrip_{i}.yxdb")
            
            start = time.perf_counter()
            yx.write_yxdb(path, df)
            df2 = yx.read_yxdb(path)
            elapsed = time.perf_counter() - start
            
            times.append(elapsed)
    
    avg_time = sum(times) / len(times)
    return {
        "avg_time_ms": avg_time * 1000,
        "rows_per_sec": df.height / avg_time,
    }


def run_benchmarks():
    """Run all benchmarks."""
    print("=" * 80)
    print("SigilYX Writer Benchmark Suite")
    print("=" * 80)
    print()
    
    results = []
    
    for bench in BENCHMARKS:
        print(f"Benchmarking: {bench['name']} ({bench['rows']:,} rows, {len(bench['schema'])} cols)")
        
        # Generate data
        df = generate_data(bench["rows"], bench["schema"])
        
        # Benchmark write
        write_result = benchmark_write(df)
        
        # Benchmark roundtrip
        roundtrip_result = benchmark_roundtrip(df)
        
        result = {
            "name": bench["name"],
            "rows": bench["rows"],
            "cols": len(bench["schema"]),
            **write_result,
            "roundtrip_ms": roundtrip_result["avg_time_ms"],
        }
        results.append(result)
        
        print(f"  Write: {write_result['avg_time_ms']:.1f}ms ({write_result['rows_per_sec']:,.0f} rows/sec)")
        print(f"  File size: {write_result['file_size_kb']:.1f} KB")
        print(f"  Roundtrip: {roundtrip_result['avg_time_ms']:.1f}ms")
        print()
    
    # Summary table
    print("=" * 80)
    print("SUMMARY")
    print("=" * 80)
    print()
    print(f"{'Benchmark':<22} {'Rows':>10} {'Cols':>5} {'Write (ms)':>12} {'Rows/sec':>15} {'Size (KB)':>10}")
    print("-" * 80)
    
    for r in results:
        print(f"{r['name']:<22} {r['rows']:>10,} {r['cols']:>5} {r['avg_time_ms']:>12.1f} {r['rows_per_sec']:>15,.0f} {r['file_size_kb']:>10.1f}")
    
    print()
    
    # Calculate overall throughput
    total_rows = sum(r["rows"] for r in results)
    total_time = sum(r["avg_time_ms"] for r in results) / 1000
    overall_throughput = total_rows / total_time
    print(f"Overall throughput: {overall_throughput:,.0f} rows/sec")


def compare_read_vs_write():
    """Compare read vs write performance using existing test files."""
    print()
    print("=" * 80)
    print("Read vs Write Comparison (using test files)")
    print("=" * 80)
    print()
    
    test_files = [
        ("ManyRecords.yxdb", "sigilyx/test_files/ManyRecords.yxdb"),
        ("People.yxdb", "sigilyx/test_files/People.yxdb"),
    ]
    
    for name, path in test_files:
        if not os.path.exists(path):
            print(f"Skipping {name} - file not found")
            continue
        
        # Read benchmark
        read_result = benchmark_read(path)
        
        # Read the file and benchmark writing it back
        df = yx.read_yxdb(path)
        write_result = benchmark_write(df)
        
        print(f"{name}:")
        print(f"  Read:  {read_result['avg_time_ms']:.1f}ms ({read_result['rows_per_sec']:,.0f} rows/sec)")
        print(f"  Write: {write_result['avg_time_ms']:.1f}ms ({write_result['rows_per_sec']:,.0f} rows/sec)")
        print(f"  Write/Read ratio: {write_result['avg_time_ms'] / read_result['avg_time_ms']:.2f}x")
        print()


if __name__ == "__main__":
    run_benchmarks()
    compare_read_vs_write()
