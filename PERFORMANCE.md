# Performance

> **Disclaimer:** This project is an independent open-source tool and is not affiliated with, endorsed by, or sponsored by Alteryx, Inc.

## Summary

SigilYX achieves **~1.8 million rows/sec** average throughput reading YXDB files, with performance varying by data shape and type.

| Data Shape | Rows | Cols | Time | Throughput |
|---|--:|--:|--:|--:|
| Medium volume | 50,000 | 3 | 24ms | 2,092,259 rows/sec |
| Mixed types | 200 | 8 | 0.4ms | 517,759 rows/sec |
| String-heavy | 6 | 5 | 0.2ms | 30,157 rows/sec |
| Nullable fields | 3 | 11 | 0.2ms | 14,487 rows/sec |
| All 16 field types | 2 | 16 | 0.3ms | 7,101 rows/sec |
| Binary/blob data | 4 | 2 | 3.6ms | 1,111 rows/sec |

**Benchmark: 50,215 rows in 28.6 ms = 1,757,459 rows/sec**

---

## Benchmark Results

### Test Files (Included)

| File | Description | Rows | Cols | Time | Throughput |
|---|---|--:|--:|--:|--:|
| ManyRecords.yxdb | Numeric columns | 50,000 | 3 | 23.9ms | 2,092,259 rows/sec |
| People.yxdb | Mixed types | 200 | 8 | 0.39ms | 517,759 rows/sec |
| Strings.yxdb | String-heavy | 6 | 5 | 0.20ms | 30,157 rows/sec |
| NullValues.yxdb | Nullable fields | 3 | 11 | 0.21ms | 14,487 rows/sec |
| AllTypes.yxdb | All 16 field types | 2 | 16 | 0.28ms | 7,101 rows/sec |
| LargeBlob.yxdb | Binary/blob data | 4 | 2 | 3.60ms | 1,111 rows/sec |

---

## Performance by Data Shape

### Narrow Tables (few columns)

- Pure numeric columns (Int64, Float): **2+ million rows/sec**
- Fixed-width numerics are the fastest to parse

### Wide Tables (many columns)

- Mixed types with strings/dates: **500K-1M rows/sec**
- Performance scales with column count and type complexity

### String-Heavy Data

- Variable-length strings (V_WString) are slower due to UTF-16 decoding
- Fixed-width numerics achieve 2-4x higher throughput than string columns

### Binary/Blob Data

- Large binary blobs are the slowest to process
- Blob I/O dominates read time for blob-heavy files

### Key Insight

**Column count and data types matter more than row count.** Narrow numeric tables read significantly faster than wide string-heavy or blob tables.

---

## Running Benchmarks

### Full Benchmark Suite

```bash
python benchmarks/benchmark_suite.py
```

### Single File

```bash
python benchmarks/benchmark.py path/to/file.yxdb
```

### Creating Test Data

To create additional YXDB test files, use Alteryx Designer's Python Tool:

1. Generate synthetic data:
   ```bash
   python benchmarks/generate_clean_data.py --rows 100000
   ```

2. In Alteryx Designer, create a workflow with a Python Tool and use:
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

3. Connect the Python Tool output to an Output Data tool configured to write YXDB.

---

## Test Environment

- **OS:** Windows 10 Pro x64
- **Python:** 3.11.9
- **Rust:** 1.75+ (release build)
- **Polars:** 1.x (Python), 0.46 (Rust)
- **Storage:** SSD

---

## Why SigilYX Is Fast

1. **Direct column builders** — Values push directly into Arrow-backed vectors
2. **Zero-copy date parsing** — ASCII to epoch integer without String allocation
3. **LZF buffer reuse** — Single buffer reused across all compressed blocks
4. **Arrow IPC bridge** — Efficient binary serialization to Python

---

*This project is not affiliated with Alteryx, Inc.*
