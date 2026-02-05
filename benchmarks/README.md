# SigilYX Benchmarks

> **Disclaimer:** This project is not affiliated with Alteryx, Inc.

## Overview

This directory contains benchmarking tools for measuring SigilYX read performance across different data shapes and types.

## Scripts

### `benchmark_suite.py`

Runs benchmarks against the included test files in `sigilyx/test_files/`.

```bash
python benchmarks/benchmark_suite.py
```

**Output:** Performance summary across all test files with throughput metrics.

### `benchmark.py`

Benchmark a single YXDB file.

```bash
python benchmarks/benchmark.py path/to/file.yxdb
```

**Options:**
- `--iterations N` — Number of iterations (default: 3)

### `generate_clean_data.py`

Generate synthetic benchmark data in Arrow IPC format.

```bash
python benchmarks/generate_clean_data.py --rows 100000 --output my_data.arrow
```

**Options:**
- `--rows N` — Number of rows to generate (default: 1,000,000)
- `--output PATH` — Output file path (default: `benchmarks/clean_benchmark.arrow`)
- `--seed N` — Random seed for reproducibility (default: 42)

## Creating YXDB Test Files

YXDB files can only be created using Alteryx Designer. To create test files from generated data:

1. **Generate synthetic data:**
   ```bash
   python benchmarks/generate_clean_data.py --rows 100000
   ```

2. **In Alteryx Designer**, create a workflow with a Python Tool:
   ```python
   import pandas as pd
   import pyarrow.ipc as ipc
   from ayx import Alteryx
   
   with open("benchmarks/clean_benchmark.arrow", "rb") as f:
       reader = ipc.open_file(f)
       table = reader.read_all()
   df = table.to_pandas()
   Alteryx.write(df, 1)
   ```

3. **Connect** the Python Tool output to an Output Data tool configured to write `.yxdb`.

## Included Test Files

The benchmark suite uses test files from `sigilyx/test_files/`:

| File | Description | Rows | Cols |
|------|-------------|-----:|-----:|
| `AllTypes.yxdb` | All 16 YXDB field types | 2 | 16 |
| `People.yxdb` | Mixed types (strings, dates, numbers) | 200 | 8 |
| `Strings.yxdb` | String-heavy data | 6 | 5 |
| `NullValues.yxdb` | Nullable fields | 3 | 11 |
| `ManyRecords.yxdb` | Medium volume numeric data | 50,000 | 3 |
| `LargeBlob.yxdb` | Binary/blob data | 4 | 2 |
| `SingleColumn.yxdb` | Single column file | varies | 1 |

## Dependencies

```bash
pip install polars faker
```

Or use the dev requirements:

```bash
pip install -r requirements-dev.txt
```

## Interpreting Results

**Throughput** is measured in rows/sec and varies by:

- **Column count** — Fewer columns = faster
- **Data types** — Numeric types are fastest; variable-length strings are slowest
- **Compression** — LZF-compressed blocks add decompression overhead
- **Blob/spatial data** — Binary data is I/O-bound

Typical results on modern hardware:

| Data Shape | Expected Throughput |
|------------|--------------------:|
| Narrow numeric (3 cols) | 2+ million rows/sec |
| Mixed types (8 cols) | 500K–1M rows/sec |
| String-heavy | 30K–500K rows/sec |
| Blob-heavy | 1K–10K rows/sec |

---

*See [PERFORMANCE.md](../PERFORMANCE.md) for detailed methodology and results.*
