# Performance

## Read Performance: Cross-Language Comparison

All benchmarks: 100,000 rows, 50 runs, median time reported. Test machine: Windows 10 Pro x64, SSD, Rust release build (`lto = "fat"`, `codegen-units = 1`).

### SigilYX (Rust) vs All Open-Source Readers

| Shape | SigilYX Rust | NedHarding C++ | Alteryx C++ | Go | .NET | vs best C++ |
|---|--:|--:|--:|--:|--:|--:|
| Narrow (2 cols, 1.4 MB) | **2.86ms** | 4.14ms | 4.86ms | 7.81ms | 13.91ms | **1.45x** |
| Numeric (5 cols, 2.9 MB) | **4.61ms** | 5.39ms | 6.88ms | 10.79ms | 17.67ms | **1.17x** |
| Mixed (8 cols, 16.3 MB) | **18.91ms** | 63.52ms | 56.52ms | 202.69ms | 152.03ms | **2.99x** |
| String-heavy (5 cols, 51.3 MB) | **42.45ms** | 126.50ms | 127.71ms | 638.87ms | 287.31ms | **2.98x** |
| Wide (50 cols, 62.2 MB) | **66.85ms** | 204.95ms | 192.26ms | 672.23ms | 470.64ms | **2.88x** |

Throughput (rows/sec):

| Shape | SigilYX Rust | NedHarding C++ | Alteryx C++ | Go | .NET |
|---|--:|--:|--:|--:|--:|
| Narrow | 35.0M | 24.1M | 20.6M | 12.8M | 7.2M |
| Numeric | 21.7M | 18.6M | 14.5M | 9.3M | 5.7M |
| Mixed | 5.3M | 1.6M | 1.8M | 493K | 658K |
| String-heavy | 2.4M | 791K | 783K | 157K | 348K |
| Wide | 1.5M | 488K | 520K | 149K | 212K |

### SigilYX Row Reader vs Columnar

The columnar reader (default) builds a Polars DataFrame in parallel. The row reader iterates record-by-record, useful for streaming or filtered reads.

| Shape | Columnar | Row | Columnar speedup |
|---|--:|--:|--:|
| Narrow | 2.86ms | 9.36ms | 3.3x |
| Numeric | 4.61ms | 11.28ms | 2.4x |
| Mixed | 18.91ms | 117.77ms | 6.2x |
| String-heavy | 42.45ms | 302.99ms | 7.1x |
| Wide | 66.85ms | 441.41ms | 6.6x |

### Python: SigilYX vs yxdb-py

SigilYX Python bindings use the Rust core via pyo3-polars (zero-copy Arrow C Data Interface). Compared against yxdb-py, the only other Python YXDB reader.

| Shape | Polars | Arrow | Pandas | Row | yxdb-py | vs yxdb-py |
|---|--:|--:|--:|--:|--:|--:|
| Narrow | **3.32ms** | 5.64ms | 6.94ms | 29.87ms | 508.35ms | **153x** |
| Numeric | **5.63ms** | 9.59ms | 11.43ms | 38.47ms | 541.36ms | **96x** |
| Mixed | **20.52ms** | 38.37ms | 41.68ms | 160.68ms | 6,922ms | **337x** |
| String-heavy | **47.22ms** | 102.18ms | 111.65ms | 335.00ms | 17,613ms | **373x** |
| Wide | **76.72ms** | 172.00ms | 193.89ms | 606.55ms | 22,523ms | **294x** |

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
| alteryx-openyxdb | C++ | Alteryx OpenYXDB | C++ typed values |

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

1. **Memory-mapped I/O** -- File data is memory-mapped (no heap copy)
2. **Parallel decompression** -- LZF block boundaries parsed, then all blocks decompressed in parallel (Rayon)
3. **Block compaction** -- Decompressed blocks compacted into a contiguous buffer
4. **Record boundary scan** -- Walk the buffer to locate record starts (arithmetic for fixed-size, sequential for variable-length)
5. **Parallel column build** -- Build Polars Series in parallel, one task per column, using direct Arrow array construction (value buffer + validity bitmap, no `Vec<Option<T>>` intermediate)

### Write Pipeline

1. **Record serialization** -- Fixed + variable-length records built from DataFrame columns
2. **Pipelined compression** -- Background thread compresses blocks via `mpsc::sync_channel` while the main thread serializes the next block
3. **Sequential I/O** -- Compressed blocks written in order for block index tracking

### Key Optimizations

- **Memory-mapped I/O** -- Avoids heap allocation for file data
- **C LZF FFI** -- Decompression uses the liblzf C library compiled with `-O3`
- **SIMD UTF-16 transcoding** -- SSE2-accelerated ASCII-path for UTF-16-to-UTF-8 conversion
- **Direct Arrow arrays** -- Numeric columns built as raw value buffer + validity bitmap, bypassing `Vec<Option<T>>`
- **pyo3-polars** -- Zero-copy Python bridge via the Arrow C Data Interface
- **Pipelined writes** -- Compression overlaps with serialization on a background thread

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

Demonstrates `scan_yxdb()` and `read_yxdb_batches()` processing a file that **exceeds available RAM** with constant memory overhead — a capability no other YXDB library offers.

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
| Any full-materialising library | ✗ DNF | — | — | — | Must load entire file into RAM |

**Headline:** 32.5 GB file processed in 52.6 seconds using only 1.1 GB peak RAM (3.4% of file size). Any library that calls the equivalent of `read()` on this file will OOM.

> **Note on Alteryx OpenYXDB C++:** The Alteryx OpenYXDB library provides only a row-by-row reader with no batching or streaming API. While it uses O(1) memory per row, it cannot perform vectorised queries and would process this file orders of magnitude slower than the Polars streaming engine. The Alteryx Designer desktop application handles large files by compressing/swapping to disk, but that capability is not exposed in the open-source library.

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
python benchmarks/benchmark_cross_language.py --runs 50

# Python-only benchmark (SigilYX formats + yxdb-py)
python benchmarks/benchmark_python_formats.py --runs 50

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

- **OS:** Windows 10 Pro x64
- **CPU:** (results are relative; absolute times depend on hardware)
- **Storage:** NVMe SSD
- **Python:** 3.11.9
- **Rust:** 1.86 (release, `lto = "fat"`, `codegen-units = 1`)
- **Polars:** 1.38 (Python) / 0.46 (Rust)
- **Go:** latest stable
- **.NET:** 8.0
- **C++:** MSVC 2022 (cl.exe, `/O2`)
