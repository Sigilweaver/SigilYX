---
sidebar_position: 4
---

# Benchmarks

SigilYX includes a comprehensive cross-language benchmark suite that compares read performance against every known open-source YXDB reader.

## Quick Start

```bash
# Generate benchmark data (100K rows, 5 profiles)
python benchmarks/generate_benchmark_data.py

# Run cross-language benchmarks
python benchmarks/benchmark_cross_language.py --runs 50

# Run Python format comparison
python benchmarks/benchmark_python_formats.py --runs 50
```

## Data Profiles

Five profiles are designed to stress different parts of the read pipeline:

| Profile | Cols | Types | What it tests |
| --- | ---: | --- | --- |
| Narrow | 2 | Int64, Float64 | Minimal overhead, max row throughput |
| Numeric | 5 | Int32, Int64, Float32, Float64, Int16 | Pure numeric decode |
| Mixed | 8 | Int64, Float64, Utf8, Bool, Date, DateTime, Int16, Utf8 | Real-world schema |
| String-heavy | 5 | V_WString (varying lengths + nulls) | UTF-16 transcode stress |
| Wide | 50 | 15x Int64, 15x Float64, 10x V_WString, 5x Bool, 5x Date | Column-count stress |

All files contain 100,000 rows generated deterministically (seed=42).

## Benchmark Targets

| Target | Language | Output |
| --- | --- | --- |
| sigilyx-rust | Rust | Polars DataFrame |
| sigilyx-rust-row | Rust | Per-record values |
| sigilyx-py-polars | Python/Rust | `polars.DataFrame` |
| sigilyx-py-arrow | Python/Rust | `pyarrow.Table` |
| sigilyx-py-pandas | Python/Rust | `pandas.DataFrame` |
| sigilyx-py-rows | Python/Rust | Python dicts |
| yxdb-py | Pure Python | Python lists |
| yxdb-go | Go | Go structs |
| yxdb-net | C# (.NET 8) | .NET objects |
| nedharding-openyxdb | C++ | C++ typed values |
| alteryx-openyxdb | C++ | C++ typed values |

The cross-language benchmark auto-detects which toolchains are installed and skips unavailable targets.

## Methodology

- **Timing**: `time.perf_counter()` (Python), `std::time::Instant` (Rust), language-native high-resolution timers for others
- **Warmup**: 10 untimed iterations before measurement
- **GC**: Python garbage collection disabled during timed runs
- **Statistics**: Median reported (robust to outliers). Full output includes mean, std dev, percentiles (p5/p25/p75/p95), coefficient of variation, and IQR.
- **Consistency**: CV < 0.10 for most targets

## Single-File Benchmark

```bash
python benchmarks/benchmark.py path/to/file.yxdb --iterations 10
```

## Interpreting Results

The benchmark outputs JSON with per-target timing data. Key things to look for:

- **Median vs Mean**: If they differ significantly, outliers are present. Trust the median.
- **CV (coefficient of variation)**: Values > 0.10 suggest inconsistent results. Try increasing `--runs` or ensuring the machine is idle.
- **Columnar vs Row**: The columnar reader should be 3--7x faster than the row reader. If not, something may be wrong with parallelism (check thread count).

## Environment Setup

For the full cross-language benchmark including C++ and Go targets, see `benchmarks/README.md` in the repository. This requires:

- Rust toolchain
- Go 1.21+
- .NET 8 SDK
- C/C++ compiler (for building OpenYXDB)
- pixi (for C++ toolchain management on Linux)
