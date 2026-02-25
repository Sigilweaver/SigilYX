# SigilYX

*High-performance YXDB reader and writer in Rust, with Python bindings.*

## Overview

SigilYX reads and writes `.yxdb` files using [Polars](https://pola.rs/) DataFrames, [PyArrow](https://arrow.apache.org/docs/python/) Tables, or [Pandas](https://pandas.pydata.org/) DataFrames. Cross-platform (Windows, macOS, Linux).

- **Fast.** Rust core with parallel decompression, SIMD transcoding, and direct Arrow array construction. 1.2--3x faster than the fastest open-source C++ readers. See [PERFORMANCE.md](PERFORMANCE.md).
- **Cross-platform.** Windows, macOS, Linux (x64 and ARM).
- **Read and Write.** Full round-trip support for all 17 YXDB field types.
- **Multiple output formats.** Polars, PyArrow, or Pandas.
- **Streaming.** Batched reads with constant memory, or lazy scans with Polars LazyFrames.

## Installation

Requires [Rust](https://rustup.rs/) and Python 3.9+.

```bash
python -m venv .venv
.venv/Scripts/activate        # Windows
# source .venv/bin/activate   # macOS / Linux
pip install maturin polars pyarrow pandas
maturin develop --release
```

## Quick Start

```python
import polars as pl
import sigilyx  # Importing registers pl.read_yxdb(), df.yxdb, lf.yxdb, etc.

# Read using Polars-style API
df = pl.read_yxdb("data.yxdb")

# Write via the official namespace plugin
df.yxdb.write("output.yxdb")

# Or use sigilyx directly
import sigilyx as yx
df = yx.read("data.yxdb")
yx.write("output.yxdb", df)
```

## API

### Polars Integration

Importing `sigilyx` registers official Polars namespace plugins and top-level aliases:

```python
import polars as pl
import sigilyx  # Just importing is enough

# Read
df = pl.read_yxdb("data.yxdb")           # Returns pl.DataFrame
lf = pl.scan_yxdb("data.yxdb")           # Returns pl.LazyFrame (IO plugin)

# Write (official namespace API)
df.yxdb.write("output.yxdb")             # @pl.api.register_dataframe_namespace
lf.yxdb.sink("output.yxdb")              # @pl.api.register_lazyframe_namespace
```

### Reading

```python
import sigilyx as yx

# Polars DataFrame (fastest -- zero-copy via Arrow C Data Interface)
df = yx.read_yxdb("data.yxdb")

# PyArrow Table
table = yx.read_yxdb_arrow("data.yxdb")

# Pandas DataFrame (via PyArrow)
pdf = yx.read_yxdb_pandas("data.yxdb")
```

### Writing

```python
# Write Polars DataFrame
yx.write_yxdb("output.yxdb", df)

# Write Pandas DataFrame
yx.write_yxdb_pandas("output.yxdb", pdf)

# Write PyArrow Table
yx.write_yxdb_arrow("output.yxdb", table)
```

### Streaming / Batched Read

```python
# Iterate in batches (constant memory for large files)
for batch in yx.read_yxdb_batches("data.yxdb", batch_size=100_000):
    process(batch)  # each batch is a Polars DataFrame

# Column projection -- only materialise the columns you need
for batch in yx.read_yxdb_batches("data.yxdb", columns=["Id", "Name"]):
    process(batch)

# Row limit -- stop after reading N total rows
for batch in yx.read_yxdb_batches("data.yxdb", n_rows=1000):
    process(batch)
```

### Lazy Scan

```python
# Returns a Polars LazyFrame backed by a native Rust streaming reader.
# Only the YXDB header is read upfront; data is streamed on .collect().
lf = yx.scan("data.yxdb")
result = lf.filter(pl.col("amount") > 100).collect()

# Projection pushdown: only the selected columns are materialised.
top10 = lf.select("id", "name").head(10).collect()
```

> **Pushdown support:** projection (``with_columns``) and row-limit
> (``n_rows`` / ``.head()``) are pushed down to the Rust reader.
> Predicate pushdown is not possible (YXDB rows are LZF-compressed
> with no block statistics), so ``.filter()`` is applied post-scan.

### Metadata

```python
# Get field metadata without reading data
fields = yx.read_yxdb_fields("data.yxdb")
for f in fields:
    print(f.name, f.field_type, f.size)

# Record count from header (no data read)
n = yx.record_count("data.yxdb")
```

## Supported Field Types

| YXDB Type | Arrow Type | Notes |
|-----------|------------|-------|
| Boolean | Boolean | |
| Byte | Int16 | Unsigned byte stored as Int16 |
| Int16 | Int16 | |
| Int32 | Int32 | |
| Int64 | Int64 | |
| Float | Float32 | |
| Double | Float64 | |
| FixedDecimal | Decimal | Precision and scale preserved |
| String | Utf8 | Fixed-width, ASCII/Latin-1 |
| WString | Utf8 | Fixed-width, UTF-16 decoded |
| V_String | LargeUtf8 | Variable-length, ASCII/Latin-1 |
| V_WString | LargeUtf8 | Variable-length, UTF-16 decoded |
| Date | Date32 | Days since epoch |
| DateTime | Datetime(us) | Microsecond precision |
| Time | Time | Nanosecond precision |
| Blob | LargeBinary | Variable-length binary |
| SpatialObj | LargeBinary | Geometry as raw bytes |

## Performance

100,000 rows, 50 runs, median. SigilYX columnar reader vs all open-source YXDB readers:

| Shape | SigilYX (Rust) | Best C++ | Go | .NET | vs C++ |
|---|--:|--:|--:|--:|--:|
| Narrow (2 cols) | 2.9ms | 4.1ms | 7.8ms | 13.9ms | **1.5x** |
| Numeric (5 cols) | 4.6ms | 5.4ms | 10.8ms | 17.7ms | **1.2x** |
| Mixed (8 cols) | 18.9ms | 56.5ms | 202.7ms | 152.0ms | **3.0x** |
| String-heavy (5 cols) | 42.4ms | 126.5ms | 638.9ms | 287.3ms | **3.0x** |
| Wide (50 cols) | 66.9ms | 192.3ms | 672.2ms | 470.6ms | **2.9x** |

Python via pyo3-polars vs pure-Python yxdb-py:

| Shape | SigilYX (Polars) | yxdb-py | Speedup |
|---|--:|--:|--:|
| Narrow | 3.3ms | 508ms | **153x** |
| Mixed | 20.5ms | 6,922ms | **337x** |
| String-heavy | 47.2ms | 17,613ms | **373x** |

See [PERFORMANCE.md](PERFORMANCE.md) for full results and methodology.

## Development

### Setup

```bash
git clone https://github.com/sigilweaver/sigilyx.git
cd sigilyx
python -m venv .venv && .venv/Scripts/activate  # or source .venv/bin/activate
pip install maturin polars pyarrow pandas
maturin develop --release
```

### Testing

```bash
# Rust tests (65 tests + doc-tests)
cargo test --workspace

# Python tests
pytest tests/ -v

# Cross-implementation tests (requires C++ dump tool, see benchmarks/README.md)
python benchmarks/test_cross_impl.py
```

### Benchmarks

See [benchmarks/README.md](benchmarks/README.md) for full environment setup (Windows, Linux, pixi) and how to run the cross-language benchmark suite.

```bash
# Generate benchmark data
python benchmarks/generate_benchmark_data.py

# Run cross-language benchmarks (auto-detects available toolchains)
python benchmarks/benchmark_cross_language.py --runs 50
```

## Project Structure

```
sigilyx/
├── sigilyx/                # Core Rust library
│   └── src/
│       ├── lib.rs          # Public API
│       ├── reader.rs       # Columnar + row reader, parallel decompression
│       ├── writer.rs       # Pipelined writer with streaming support
│       ├── field.rs        # Field type definitions and parsing
│       ├── record.rs       # Record-level extraction
│       ├── header.rs       # YXDB header parsing
│       ├── lzf.rs          # LZF compression
│       └── error.rs        # Error types
├── sigilyx-python/         # PyO3 + pyo3-polars bindings
│   └── src/lib.rs
├── python/sigilyx/         # Python wrapper module
│   └── __init__.py         # Polars / PyArrow / Pandas API + Polars registration
├── benchmarks/             # Cross-language benchmark suite
│   ├── README.md           # Setup and usage
│   └── ...                 # Rust, Go, C#, C++ benchmark harnesses
├── tests/                  # Python test suite
├── PERFORMANCE.md          # Benchmark results
└── SPECIFICATION.md        # YXDB format specification
```

## Licensing

Licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0-only)**. See [LICENSE](LICENSE).

## Acknowledgments

Format specification derived from existing open-source implementations. See [SPECIFICATION.md](SPECIFICATION.md) for references.
