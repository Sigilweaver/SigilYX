---
sidebar_position: 10
description: "Benchmark results for SigilYX vs C++, Go, .NET, and Python YXDB readers."
---

# Performance

All benchmarks: 100,000 rows, 50 runs, median time reported. Test machine: Windows 10 Pro x64, NVMe SSD, Rust release build (`lto = "fat"`, `codegen-units = 1`).

## Rust: SigilYX vs All Open-Source Readers

| Shape | SigilYX (Rust) | NedHarding C++ | Alteryx C++ | Go | .NET | vs best C++ |
| --- | --: | --: | --: | --: | --: | --: |
| Narrow (2 cols, 1.4 MB) | **2.86 ms** | 4.14 ms | 4.86 ms | 7.81 ms | 13.91 ms | **1.45x** |
| Numeric (5 cols, 2.9 MB) | **4.61 ms** | 5.39 ms | 6.88 ms | 10.79 ms | 17.67 ms | **1.17x** |
| Mixed (8 cols, 16.3 MB) | **18.91 ms** | 63.52 ms | 56.52 ms | 202.69 ms | 152.03 ms | **2.99x** |
| String-heavy (5 cols, 51.3 MB) | **42.45 ms** | 126.50 ms | 127.71 ms | 638.87 ms | 287.31 ms | **2.98x** |
| Wide (50 cols, 62.2 MB) | **66.85 ms** | 204.95 ms | 192.26 ms | 672.23 ms | 470.64 ms | **2.88x** |

### Throughput (rows/sec)

| Shape | SigilYX (Rust) | NedHarding C++ | Alteryx C++ | Go | .NET |
| --- | --: | --: | --: | --: | --: |
| Narrow | 35.0M | 24.1M | 20.6M | 12.8M | 7.2M |
| Numeric | 21.7M | 18.6M | 14.5M | 9.3M | 5.7M |
| Mixed | 5.3M | 1.6M | 1.8M | 493K | 658K |
| String-heavy | 2.4M | 791K | 783K | 157K | 348K |
| Wide | 1.5M | 488K | 520K | 149K | 212K |

The advantage grows with schema complexity. For string-heavy and wide files, SigilYX's parallel column build and SIMD UTF-16 transcoding provide a nearly 3x advantage over the best C++ reader.

## Columnar vs Row Reader

| Shape | Columnar | Row | Columnar speedup |
| --- | --: | --: | --: |
| Narrow | 2.86 ms | 9.36 ms | 3.3x |
| Numeric | 4.61 ms | 11.28 ms | 2.4x |
| Mixed | 18.91 ms | 117.77 ms | 6.2x |
| String-heavy | 42.45 ms | 302.99 ms | 7.1x |
| Wide | 66.85 ms | 441.41 ms | 6.6x |

Use the columnar reader (default) unless you need row-level control.

## Python: SigilYX vs yxdb-py

SigilYX Python bindings use the Rust core via pyo3-polars (zero-copy Arrow C Data Interface). Compared against yxdb-py, the only other Python YXDB reader:

| Shape | Polars | Arrow | Pandas | Row | yxdb-py | vs yxdb-py |
| --- | --: | --: | --: | --: | --: | --: |
| Narrow | **3.32 ms** | 5.64 ms | 6.94 ms | 29.87 ms | 508.35 ms | **153x** |
| Numeric | **5.63 ms** | 9.59 ms | 11.43 ms | 38.47 ms | 541.36 ms | **96x** |
| Mixed | **20.52 ms** | 38.37 ms | 41.68 ms | 160.68 ms | 6,922 ms | **337x** |
| String-heavy | **47.22 ms** | 102.18 ms | 111.65 ms | 335.00 ms | 17,613 ms | **373x** |
| Wide | **76.72 ms** | 172.00 ms | 193.89 ms | 606.55 ms | 22,523 ms | **294x** |

### Python Format Comparison

For the Polars path, the Rust-to-Python overhead is ~0.5 ms (the Arrow C Data Interface is virtually free). The Arrow and Pandas paths add serialization overhead but remain 96--373x faster than the pure-Python alternative.

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
