# Performance

## Read Performance: Cross-Language Comparison

All benchmarks: 100,000 rows, 100 runs, median time reported. Test machine: Windows 11 Pro x64, Intel i5-12500H, 16 GB RAM, NVMe SSD, Rust release build (`lto = "fat"`, `codegen-units = 1`).

### SigilYX (Rust) vs All Open-Source Readers

| Shape | SigilYX Rust | NedHarding C++ | OpenYXDB C++ | Go | .NET | vs best |
|---|--:|--:|--:|--:|--:|--:|
| Narrow (2 cols, 1.4 MB) | **2.23ms** | 2.23ms | 3.15ms | 4.53ms | 8.70ms | **1.00x** |
| Numeric (5 cols, 2.9 MB) | **4.17ms** | 4.29ms | 5.90ms | 7.20ms | 11.63ms | **1.03x** |
| Mixed (8 cols, 16.3 MB) | **21.51ms** | 44.78ms | 39.88ms | 130.28ms | 108.44ms | **1.85x** |
| String-heavy (5 cols, 51.3 MB) | **52.01ms** | 85.25ms | 85.91ms | 344.57ms | 204.65ms | **1.64x** |
| Wide (50 cols, 62.2 MB) | **71.04ms** | 149.31ms | 139.56ms | 438.97ms | 336.55ms | **1.96x** |

Throughput (rows/sec):

| Shape | SigilYX Rust | NedHarding C++ | OpenYXDB C++ | Go | .NET |
|---|--:|--:|--:|--:|--:|
| Narrow | 44.8M | 44.8M | 31.7M | 22.1M | 11.5M |
| Numeric | 24.0M | 23.3M | 16.9M | 13.9M | 8.6M |
| Mixed | 4.6M | 2.2M | 2.5M | 768K | 922K |
| String-heavy | 1.9M | 1.2M | 1.2M | 290K | 489K |
| Wide | 1.4M | 670K | 717K | 228K | 297K |

### SigilYX Row Reader vs Columnar

The columnar reader (default) builds a Polars DataFrame in parallel. The row reader iterates record-by-record, useful for streaming or filtered reads.

| Shape | Columnar | Row | Columnar speedup |
|---|--:|--:|--:|
| Narrow | 2.23ms | 6.85ms | 3.1x |
| Numeric | 4.17ms | 9.56ms | 2.3x |
| Mixed | 21.51ms | 72.66ms | 3.4x |
| String-heavy | 52.01ms | 166.38ms | 3.2x |
| Wide | 71.04ms | 274.90ms | 3.9x |

### Python: SigilYX vs yxdb-py

SigilYX Python bindings use the Rust core via pyo3-polars (zero-copy Arrow C Data Interface). Compared against yxdb-py, the only other Python YXDB reader.

| Shape | Polars | Arrow | Pandas | Row | yxdb-py | vs yxdb-py |
|---|--:|--:|--:|--:|--:|--:|
| Narrow | **2.79ms** | 2.83ms | 3.75ms | 18.56ms | 308.53ms | **111x** |
| Numeric | **5.12ms** | 5.24ms | 6.32ms | 26.62ms | 362.03ms | **71x** |
| Mixed | **22.21ms** | 24.60ms | 26.95ms | 120.59ms | 4,333ms | **195x** |
| String-heavy | **52.25ms** | 59.36ms | 62.13ms | 224.13ms | 10,659ms | **204x** |
| Wide | **74.01ms** | 79.76ms | 89.67ms | 411.91ms | 14,019ms | **189x** |

---

## Benchmark Targets

| Target | Language | Library | Output |
|--------|----------|---------|--------|
| sigilyx-rust | Rust | `sigilyx` (this crate) | Polars DataFrame |
| sigilyx-rust-row | Rust | `sigilyx` row reader | Typed values per record |
| sigilyx-py-polars | Python/Rust | `sigilyx` via pyo3-polars | `polars.DataFrame` |
| sigilyx-py-arrow | Python/Rust | `sigilyx` via Arrow IPC | `pyarrow.Table` |
| sigilyx-py-pandas | Python/Rust | `sigilyx` via PyArrow | `pandas.DataFrame` |
| sigilyx-py-rows | Python/Rust | `sigilyx` row reader | Python dicts |
| yxdb-py | Pure Python | `pip install yxdb` | Python lists |
| yxdb-go | Go | `yxdb-go` | Go structs |
| yxdb-net | C# (.NET 8) | NuGet `yxdb` | .NET objects |
| nedharding-openyxdb | C++ | Open_AlteryxYXDB | C++ typed values |
| alteryx-openyxdb | C++ | OpenYXDB (Alteryx) | C++ typed values |

---

## Data Profiles

Five profiles designed to stress different parts of the read pipeline:

| Profile | Cols | Types | Tests |
|---------|-----:|-------|-------|
| Narrow | 2 | Int64, Float64 | Minimal overhead, max row throughput |
| Numeric | 5 | Int32, Int64, Float32, Float64, Int16 | Pure numeric decode |
| Mixed | 8 | Int64, Float64, Utf8, Bool, Date, DateTime, Int16, Utf8 | Real-world schema |
| String-heavy | 5 | V_WString (varying lengths + nulls) | UTF-16 transcode stress |
| Wide | 50 | 15x Int64, 15x Float64, 10x V_WString, 5x Bool, 5x Date | Column-count stress |

All files contain 100,000 rows generated deterministically (seed=42).

---

## Architecture

### Read Pipeline

1. **Memory-mapped I/O** - File data is memory-mapped (no heap copy)
2. **Parallel decompression** - LZF block boundaries parsed, then all blocks decompressed in parallel (Rayon)
3. **Block compaction** - Decompressed blocks compacted into a contiguous buffer
4. **Record boundary scan** - Walk the buffer to locate record starts (arithmetic for fixed-size, sequential for variable-length)
5. **Parallel column build** - Build Polars Series in parallel, one task per column, using direct Arrow array construction (value buffer + validity bitmap, no `Vec<Option<T>>` intermediate)

### Write Pipeline

1. **Record serialization** - Fixed + variable-length records built from DataFrame columns
2. **Pipelined compression** - Background thread compresses blocks via `mpsc::sync_channel` while the main thread serializes the next block
3. **Sequential I/O** - Compressed blocks written in order for block index tracking

### Key Optimizations

- **Memory-mapped I/O** - Avoids heap allocation for file data
- **C LZF FFI** - Decompression uses the liblzf C library compiled with `-O3`
- **SIMD UTF-16 transcoding** - SSE2-accelerated ASCII-path for UTF-16-to-UTF-8 conversion
- **Direct Arrow arrays** - Numeric columns built as raw value buffer + validity bitmap, bypassing `Vec<Option<T>>`
- **pyo3-polars** - Zero-copy Python bridge via the Arrow C Data Interface
- **Pipelined writes** - Compression overlaps with serialization on a background thread

---

## Read Performance: Per-Type Breakdown

Single-column files, 10.1 M rows each (except Blob at 7.3 M due to size).
Measures the raw decode throughput for every YXDB field type independently.

| Type | Rows | File (MB) | Read (s) | MB/s | Mrows/s | Notes |
|------|-----:|----------:|---------:|-----:|--------:|-------|
| Byte | 10.1M | 101 | 0.24 | 413 | 41.5 | Fastest per-row — trivial decode |
| Int16 | 10.1M | 152 | 0.28 | 540 | 36.0 | |
| Int32 | 10.1M | 253 | 0.43 | 588 | 23.5 | |
| Bool | 10.1M | 42 | 0.51 | 82 | 19.7 | Small file, low MB/s but high Mrows/s |
| Float | 10.1M | 253 | 0.48 | 525 | 21.0 | |
| Double | 10.1M | 455 | 0.72 | 634 | 14.1 | **Highest MB/s** — direct memcpy |
| Int64 | 10.1M | 442 | 0.79 | 557 | 12.7 | |
| Date | 10.1M | 461 | 0.98 | 472 | 10.3 | |
| Time | 10.1M | 406 | 1.01 | 400 | 10.0 | |
| DateTime | 10.1M | 874 | 1.69 | 518 | 6.0 | |
| FixedDecimal | 10.1M | 963 | 2.19 | 440 | 4.6 | String-encoded decimal parse |
| V_String | 10.1M | 1,295 | 3.55 | 365 | 2.9 | Variable-length UTF-8 |
| String | 10.1M | 1,054 | 5.57 | 189 | 1.8 | Fixed-width, null-pad strip |
| V_WString | 10.1M | 2,826 | 6.92 | 409 | 1.5 | Variable-length UTF-16→UTF-8 |
| SpatialObj | 10.1M | 909 | 9.09 | 100 | 1.1 | WKB binary blobs |
| Blob | 7.3M | 4,058 | 11.64 | 349 | 0.6 | Large binary (avg 583 bytes/row) |
| WString | 10.1M | 2,261 | 97.60 | 23 | 0.1 | ⚠ Fixed-width UTF-16, known bottleneck |
| **Total** | — | **16,802** | **143.7** | **117** | — | |

Key takeaways:
- **Numeric types (Byte→Int64)** are memory-bandwidth limited at 400-634 MB/s
- **Variable-length strings** (V_String, V_WString) are 300-400 MB/s — the SIMD UTF-16 fast-path helps V_WString significantly
- **Fixed-width WString is the outlier** at 23 MB/s — each row requires scanning a fixed-width UTF-16 buffer and stripping null padding; this is the #1 optimisation target
- **SpatialObj** at 100 MB/s — WKB blob decode cost; acceptable for the data type

---

## Streaming: Larger-Than-RAM Processing

Demonstrates `scan_yxdb()` and `read_yxdb_batches()` processing a file that **exceeds available RAM** with constant memory overhead.

**Setup:** 32.5 GB file (90 M rows × 20 mixed-type columns) on a machine with 11.9 GB available / 16.9 GB total RAM.

**Task:** Single-pass aggregate query:
```sql
SELECT COUNT(*), SUM(f64_a), AVG(f64_b)
FROM   bench_giant_90000000.yxdb
WHERE  i32_a > 0
```

| Variant | Status | Time | GB/s | Peak RAM | Method |
|---------|--------|-----:|-----:|---------:|--------|
| **sigilyx-scan** | ✓ | **52.6s** | **0.62** | **1.1 GB** | `scan_yxdb()` + Polars streaming engine |
| **sigilyx-batches** | ✓ | **53.5s** | **0.61** | **1.1 GB** | `read_yxdb_batches()` + Python loop |
| sigilyx-full-read | ✗ DNF | — | — | — | `read()` — OOM (needs 32+ GB) |

**Headline:** 32.5 GB file processed in 52.6 seconds using only 1.1 GB peak RAM (3.4% of file size).

> **Note:** The open-source C++ readers (OpenYXDB, Open_AlteryxYXDB) provide row-by-row APIs with O(1) memory per row, but no batching or vectorised query interface. SigilYX's streaming mode combines constant-memory reading with Polars' vectorised engine.

---

## Running Benchmarks

```bash
# Generate benchmark data (100K rows x 5 profiles)
python benchmarks/generate_benchmark_data.py

# Generate per-type files (10.1M rows × 17 types, ~16.8 GB total)
python benchmarks/generate_benchmark_data.py --per-type

# Generate giant file (90M rows × 20 cols, ~32.5 GB)
python benchmarks/generate_benchmark_data.py --giant

# Cross-language benchmark (auto-detects available toolchains)
python benchmarks/benchmark_cross_language.py --runs 100

# Python-only benchmark (SigilYX formats + yxdb-py)
python benchmarks/benchmark_python_formats.py --runs 100

# Per-type read benchmark (all 17 YXDB field types)
python benchmarks/read_per_type.py

# Streaming benchmark (constant-memory processing of large files)
python benchmarks/benchmark_streaming.py
python benchmarks/benchmark_streaming.py --skip-full-read  # skip OOM variant

# Single file
python benchmarks/benchmark.py path/to/file.yxdb --iterations 10
```

See [benchmarks/README.md](benchmarks/README.md) for full environment setup instructions (Windows, Linux, pixi for C/C++ toolchains).

---

## Methodology

- **Timing:** `time.perf_counter()` (Python), `std::time::Instant` (Rust), `time.Now()` (Go), `Stopwatch` (.NET), `std::chrono::high_resolution_clock` (C++)
- **Warmup:** 10 untimed iterations before each measurement
- **GC:** Python GC disabled during timed runs
- **Statistics:** Median reported (robust to outliers). Full output includes mean, stdev, p5/p25/p75/p95, CV, IQR.
- **Consistency:** CV < 0.10 for most targets; outlier-heavy runs noted in JSON output

See [benchmarks/METHODOLOGY.md](benchmarks/METHODOLOGY.md) for detailed statistical methodology.

---

## Test Environment

- **OS:** Windows 11 Pro x64
- **CPU:** Intel Core i5-12500H (12C/16T)
- **RAM:** 16 GB DDR4
- **Storage:** NVMe SSD
- **Python:** 3.13.12
- **Rust:** 1.93.1 (release, `lto = "fat"`, `codegen-units = 1`)
- **Polars:** 1.38.0
- **Go:** latest stable
- **.NET:** 8.0
- **C++:** MSVC 2022 (cl.exe, `/O2`)
