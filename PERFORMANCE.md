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

## Running Benchmarks

```bash
# Generate benchmark data (100K rows x 5 profiles)
python benchmarks/generate_benchmark_data.py

# Cross-language benchmark (auto-detects available toolchains)
python benchmarks/benchmark_cross_language.py --runs 50

# Python-only benchmark (SigilYX formats + yxdb-py)
python benchmarks/benchmark_python_formats.py --runs 50

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
