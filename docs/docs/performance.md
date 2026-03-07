---
sidebar_position: 10
description: "Benchmark results for SigilYX vs C++, Go, .NET, and Python YXDB readers."
---

# Performance

All benchmarks: 100,000 rows, 100 runs, median time reported. Test machine: Windows 11 Pro x64, Intel i5-12500H, 16 GB RAM, NVMe SSD, Rust release build (`lto = "fat"`, `codegen-units = 1`).

## Rust: SigilYX vs All Open-Source Readers

| Shape | SigilYX (Rust) | NedHarding C++ | Alteryx C++ | Go | .NET | vs best |
| --- | --: | --: | --: | --: | --: | --: |
| Narrow (2 cols, 1.4 MB) | **2.23 ms** | 2.23 ms | 3.15 ms | 4.53 ms | 8.70 ms | **1.00x** |
| Numeric (5 cols, 2.9 MB) | **4.17 ms** | 4.29 ms | 5.90 ms | 7.20 ms | 11.63 ms | **1.03x** |
| Mixed (8 cols, 16.3 MB) | **21.51 ms** | 44.78 ms | 39.88 ms | 130.28 ms | 108.44 ms | **1.85x** |
| String-heavy (5 cols, 51.3 MB) | **52.01 ms** | 85.25 ms | 85.91 ms | 344.57 ms | 204.65 ms | **1.64x** |
| Wide (50 cols, 62.2 MB) | **71.04 ms** | 149.31 ms | 139.56 ms | 438.97 ms | 336.55 ms | **1.96x** |

### Throughput (rows/sec)

| Shape | SigilYX (Rust) | NedHarding C++ | Alteryx C++ | Go | .NET |
| --- | --: | --: | --: | --: | --: |
| Narrow | 44.8M | 44.8M | 31.7M | 22.1M | 11.5M |
| Numeric | 24.0M | 23.3M | 16.9M | 13.9M | 8.6M |
| Mixed | 4.6M | 2.2M | 2.5M | 768K | 922K |
| String-heavy | 1.9M | 1.2M | 1.2M | 290K | 489K |
| Wide | 1.4M | 670K | 717K | 228K | 297K |

The advantage grows with schema complexity. For string-heavy and wide files, SigilYX's parallel column build and SIMD UTF-16 transcoding provide a ~2x advantage over the best C++ reader.

## Columnar vs Row Reader

| Shape | Columnar | Row | Columnar speedup |
| --- | --: | --: | --: |
| Narrow | 2.23 ms | 6.85 ms | 3.1x |
| Numeric | 4.17 ms | 9.56 ms | 2.3x |
| Mixed | 21.51 ms | 72.66 ms | 3.4x |
| String-heavy | 52.01 ms | 166.38 ms | 3.2x |
| Wide | 71.04 ms | 274.90 ms | 3.9x |

Use the columnar reader (default) unless you need row-level control.

## Python: SigilYX vs yxdb-py

SigilYX Python bindings use the Rust core via pyo3-polars (zero-copy Arrow C Data Interface). Compared against yxdb-py, the only other Python YXDB reader:

| Shape | Polars | Arrow | Pandas | Row | yxdb-py | vs yxdb-py |
| --- | --: | --: | --: | --: | --: | --: |
| Narrow | **2.79 ms** | 2.83 ms | 3.75 ms | 18.56 ms | 308.53 ms | **111x** |
| Numeric | **5.12 ms** | 5.24 ms | 6.32 ms | 26.62 ms | 362.03 ms | **71x** |
| Mixed | **22.21 ms** | 24.60 ms | 26.95 ms | 120.59 ms | 4,333 ms | **195x** |
| String-heavy | **52.25 ms** | 59.36 ms | 62.13 ms | 224.13 ms | 10,659 ms | **204x** |
| Wide | **74.01 ms** | 79.76 ms | 89.67 ms | 411.91 ms | 14,019 ms | **189x** |

### Python Format Comparison

For the Polars path, the Rust-to-Python overhead is ~0.5 ms (the Arrow C Data Interface is virtually free). The Arrow and Pandas paths add serialization overhead but remain 71-204x faster than the pure-Python alternative.

## Why Is It Fast?

| Optimization | Impact |
| --- | --- |
| Memory-mapped I/O | No heap allocation for file data |
| C LZF FFI | Decompression uses liblzf compiled with `-O3` |
| Parallel decompression | All LZF blocks decompressed in parallel (Rayon) |
| SIMD UTF-16 transcoding | SSE2-accelerated ASCII fast path for UTF-16 to UTF-8 |
| Direct Arrow arrays | Numeric columns built as raw buffer + validity bitmap |
| pyo3-polars | Zero-copy Python bridge via Arrow C Data Interface |
| Pipelined writes | Compression overlaps with serialization on a background thread |

## Running Benchmarks

See the [Benchmarks](/developer/benchmarks) page in the developer guide for instructions on reproducing these results.
