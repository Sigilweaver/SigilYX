# SigilYX

*The open-source, high-performance bridge for YXDB data.*

---

> **⚠️ Disclaimer:** This project is an independent open-source tool and is **not affiliated with, endorsed by, or sponsored by Alteryx, Inc.** "Alteryx" and "YXDB" are trademarks of Alteryx, Inc.

---

## Overview

SigilYX is a fast, cross-platform YXDB file reader and writer written in Rust, with first-class Python bindings. Read and write Alteryx `.yxdb` files using [Polars](https://pola.rs/) DataFrames, [PyArrow](https://arrow.apache.org/docs/python/) Tables, or [Pandas](https://pandas.pydata.org/) DataFrames - without Alteryx Designer installed, and on any OS.

### Why SigilYX?

- **Fast.** High-performance Rust core. See [PERFORMANCE.md](PERFORMANCE.md).
- **Cross-platform.** Runs on Windows, macOS, and Linux (x64 and ARM). No Alteryx installation required.
- **Read and Write.** Full round-trip support for YXDB files.
- **Multiple output formats.** Get your data as Polars, PyArrow, or Pandas — whichever fits your workflow.
- **Streaming.** Read large files in batches with constant memory usage, or scan lazily with Polars LazyFrames.
- **Complete.** All 17 YXDB field types supported, including variable-length strings, dates, blobs, and spatial objects.

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
import sigilyx  # Importing registers pl.read_yxdb(), df.write_yxdb(), etc.

# Read using Polars-style API
df = pl.read_yxdb("data.yxdb")

# Write using method syntax
df.write_yxdb("output.yxdb")

# Or use sigilyx directly
import sigilyx as yx
df = yx.read("data.yxdb")
yx.write("output.yxdb", df)
```

## API

### Polars Integration

Importing `sigilyx` automatically registers methods on Polars classes:

```python
import polars as pl
import sigilyx  # Just importing is enough

# Read
df = pl.read_yxdb("data.yxdb")           # Returns pl.DataFrame
lf = pl.scan_yxdb("data.yxdb")           # Returns pl.LazyFrame

# Write
df.write_yxdb("output.yxdb")             # DataFrame method
lf.sink_yxdb("output.yxdb")              # LazyFrame method (collects first)
```

### Reading

```python
import sigilyx as yx

# Polars DataFrame (fastest — no serialization overhead)
df = yx.read_yxdb("data.yxdb")

# PyArrow Table (zero-copy Arrow IPC deserialization)
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

### Streaming / Batched read

```python
# Iterate in batches (constant memory for large files)
for batch in yx.read_yxdb_batches("data.yxdb", batch_size=100_000):
    process(batch)  # each batch is a Polars DataFrame
```

### Lazy scan

```python
# Returns a Polars LazyFrame — execution deferred
lf = yx.scan("data.yxdb")
result = lf.filter(pl.col("amount") > 100).collect()
```

### Metadata

```python
# Get field metadata without reading data
fields = yx.read_yxdb_fields("data.yxdb")
for f in fields:
    print(f.name, f.field_type, f.size)
```

## Supported Field Types

| YXDB Type | Arrow Type | Notes |
|-----------|------------|-------|
| Boolean | Boolean | |
| Byte | Int8 | |
| Int16 | Int16 | |
| Int32 | Int32 | |
| Int64 | Int64 | |
| Float | Float32 | |
| Double | Float64 | |
| FixedDecimal | Float64 | Decimal precision preserved |
| String | Utf8 | Fixed-width, ASCII/Latin-1 |
| WString | Utf8 | Fixed-width, UTF-16 decoded |
| V_String | LargeUtf8 | Variable-length, ASCII/Latin-1 |
| V_WString | LargeUtf8 | Variable-length, UTF-16 decoded |
| Date | Date32 | Days since epoch |
| DateTime | Datetime(us) | Microsecond precision |
| Time | Time64(us) | Microsecond precision |
| Blob | LargeBinary | Variable-length binary |
| SpatialObj | LargeBinary | Geometry as raw bytes |

## Performance

SigilYX achieves ~1.8 million rows/sec average throughput, with peak performance of 2+ million rows/sec on narrow numeric data.

| Data Shape | Rows | Cols | Time | Throughput |
|---|--:|--:|--:|--:|
| Medium volume | 50,000 | 3 | 24ms | 2,092,259 rows/sec |
| Mixed types | 200 | 8 | 0.4ms | 517,759 rows/sec |
| String-heavy | 6 | 5 | 0.2ms | 30,157 rows/sec |
| All 16 field types | 2 | 16 | 0.3ms | 7,101 rows/sec |

See [PERFORMANCE.md](PERFORMANCE.md) for detailed benchmarks.

## Development

### Setup

```bash
git clone https://github.com/yourusername/sigilyx.git
cd sigilyx
python -m venv .venv
.venv/Scripts/activate
pip install -r requirements-dev.txt
maturin develop --release
```

### Testing

```bash
# Rust tests
cargo test

# Python tests
pytest tests/ -v
```

### Benchmarks

```bash
python benchmarks/benchmark_suite.py
```

## Project Structure

```
sigilyx/
├── sigilyx/                # Core Rust library
│   └── src/
│       ├── lib.rs          # Public API
│       ├── reader.rs       # YXDB reader
│       ├── field.rs        # Field definitions
│       ├── record.rs       # Record parsing
│       ├── lzf.rs          # LZF decompression
│       └── error.rs        # Error types
├── sigilyx-python/         # PyO3 bindings
│   └── src/lib.rs          # Python-facing functions
├── python/sigilyx/         # Python wrapper module
│   └── __init__.py         # Polars / PyArrow / Pandas API
├── benchmarks/             # Benchmark scripts
├── tests/                  # Python test suite
├── Cargo.toml              # Workspace config
├── pyproject.toml          # Maturin build config
└── PERFORMANCE.md          # Benchmark methodology and results
```

## Licensing

This project is licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0-only)**.

### What this means:

- Use freely — Use SigilYX for any purpose, including commercial.
- Modify freely — Fork and adapt to your needs.
- Share freely — Distribute copies.
- Copyleft — If you distribute modified versions (including as a network service), you must release your modifications under AGPL-3.0 and make source code available.
- Attribution — You must retain copyright and license notices.

---

## Development Approach

SigilYX was developed by studying **existing open-source YXDB implementations**:

### Reference Implementations

- [yxdb-go](https://github.com/tlarsendataguy-yxdb/yxdb-go) — Go reader by @tlarsendataguy (MIT)
- [yxdb-py](https://github.com/tlarsendataguy-yxdb/yxdb-py) — Python reader by @tlarsendataguy (MIT)
- [OpenYXDB](https://github.com/alteryx/OpenYXDB) — Official Alteryx C++ implementation (GPL-3.0, format study only)
- [Open_AlteryxYXDB](https://github.com/AlteryxNed/Open_AlteryxYXDB) — NedHarding C++ implementation (GPL-3.0, format study only)

**Note:** The GPL-3.0 C++ implementations were studied for format understanding
only — no code is derived from them. Our implementation references the
MIT-licensed Go and Python readers as primary references.

### Compatibility Verification

We tested SigilYX output against Alteryx-generated files:

- **Data integrity**: 100% pass — all test files round-trip correctly
- **Byte-for-byte identical**: No (and not expected)
- **Differences**: Header metadata only (version bytes, timestamps)

Files written by SigilYX are format-compatible but not bit-identical to Alteryx
output. This is expected for an independent implementation — we produce
**compatible** files, not **identical** files.

For the full derived specification, see [SPECIFICATION.md](SPECIFICATION.md).---

## Acknowledgments

### Prior Art

We gratefully acknowledge the work of **[@tlarsendataguy](https://github.com/tlarsendataguy-yxdb)** and the [yxdb-go](https://github.com/tlarsendataguy-yxdb/yxdb-go) project family, which pioneered open-source YXDB file access. Their implementations (MIT licensed) served as valuable reference material for validating our independent understanding of the file format.

The tlarsendataguy-yxdb organization provides YXDB libraries for multiple platforms:
- [yxdb-go](https://github.com/tlarsendataguy-yxdb/yxdb-go) (Go)
- [yxdb-py](https://github.com/tlarsendataguy-yxdb/yxdb-py) (Python)
- [yxdb-java](https://github.com/tlarsendataguy-yxdb/yxdb-java) (Java/JVM)
- [yxdb-net](https://github.com/tlarsendataguy-yxdb/yxdb-net) (.NET/C#)

### Note on Implementation

SigilYX is an independent implementation written in Rust. Our code was developed from scratch based on our own specification research. Any structural similarities to other implementations arise from the constraints of the YXDB format itself, not from code copying.

### Our Perspective on YXDB

We built SigilYX to enable interoperability, not because we believe YXDB is a superior format. For our thoughts on YXDB versus modern alternatives like Parquet and Arrow, see [PHILOSOPHY.md](PHILOSOPHY.md).

---

### Trademark Notice

"Alteryx" and "YXDB" are trademarks of Alteryx, Inc. This project is not affiliated with Alteryx, Inc.

---

*Built with Rust.*
