# sigilyx

*YXDB reader and writer for Python.*

[![PyPI](https://img.shields.io/pypi/v/sigilyx)](https://pypi.org/project/sigilyx/)
[![Python](https://img.shields.io/pypi/pyversions/sigilyx)](https://pypi.org/project/sigilyx/)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Sigilweaver/SigilYX/blob/main/LICENSE)

YXDB is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. `sigilyx` reads and writes `.yxdb` files using [Polars](https://pola.rs/) DataFrames, [PyArrow](https://arrow.apache.org/docs/python/) Tables, or [Pandas](https://pandas.pydata.org/) DataFrames.

The core is written in Rust. No native Alteryx Designer installation is required.

> **Format scope:** `sigilyx` has full read/write support for the **E1** (original engine) YXDB layout. **Experimental** read support for **E2** (AMP engine) is included - 13 field types have been verified against real E2 files; 4 rare types (Blob, SpatialObj, Time, WString) have speculative decoders behind an opt-in flag. E2 writing is not yet supported. See [SPECIFICATION-E2.md](https://github.com/Sigilweaver/SigilYX/blob/main/SPECIFICATION-E2.md) for details.

## Installation

```bash
pip install sigilyx                     # Polars only (default)
pip install "sigilyx[arrow]"            # + PyArrow
pip install "sigilyx[pandas]"           # + Pandas + PyArrow
pip install "sigilyx[all]"              # all extras
```

Requires Python 3.9+. Pre-built wheels for Windows, macOS, and Linux (x64 and ARM).

## Quick Start

```python
import polars as pl
import sigilyx  # importing registers pl.read_yxdb(), df.yxdb, etc.

# Read
df = pl.read_yxdb("data.yxdb")

# Write
df.yxdb.write("output.yxdb")
```

## API

### Polars Integration

Importing `sigilyx` registers official [Polars namespace plugins](https://docs.pola.rs/api/python/stable/reference/api.html) and top-level IO aliases. No extra calls needed — just `import sigilyx`.

```python
import polars as pl
import sigilyx

# Top-level IO (mirrors pl.read_parquet / pl.scan_parquet style)
df = pl.read_yxdb("data.yxdb")           # returns pl.DataFrame
lf = pl.scan_yxdb("data.yxdb")           # returns pl.LazyFrame (IO plugin)

# Namespace API on DataFrame / LazyFrame
df.yxdb.write("output.yxdb")             # pl.DataFrame → .yxdb file
lf.yxdb.sink("output.yxdb")              # pl.LazyFrame → .yxdb file (streaming)
```

### Reading

```python
import sigilyx as yx

# Polars DataFrame — fastest, zero-copy via Arrow C Data Interface
df = yx.read_yxdb("data.yxdb")

# PyArrow Table
table = yx.read_yxdb_arrow("data.yxdb")

# Pandas DataFrame (via PyArrow)
pdf = yx.read_yxdb_pandas("data.yxdb")
```

### Writing

```python
import sigilyx as yx

# Polars DataFrame
yx.write_yxdb("output.yxdb", df)

# PyArrow Table
yx.write_yxdb_arrow("output.yxdb", table)

# Pandas DataFrame
yx.write_yxdb_pandas("output.yxdb", pdf)
```

### Streaming / Batched Read

Iterate over large files with constant memory usage:

```python
import sigilyx as yx

# Basic iteration — each batch is a Polars DataFrame
for batch in yx.read_yxdb_batches("data.yxdb", batch_size=100_000):
    process(batch)

# Column projection — only materialise the columns you need
for batch in yx.read_yxdb_batches("data.yxdb", columns=["Id", "Name", "Amount"]):
    process(batch)

# Row limit — stop after N total rows
for batch in yx.read_yxdb_batches("data.yxdb", n_rows=5_000):
    process(batch)
```

### Lazy Scan

```python
import polars as pl
import sigilyx as yx

# Returns a Polars LazyFrame backed by a native Rust streaming reader.
# Only the YXDB header is read on construction; data streams on .collect().
lf = yx.scan("data.yxdb")

result = lf.filter(pl.col("amount") > 100).collect()

# Projection pushdown — only selected columns are materialised in Rust
top10 = lf.select("id", "name").head(10).collect()
```

> **Pushdown support:** projection (`select` / `with_columns`) and row-limit
> (`n_rows` / `.head()`) are pushed down to the Rust reader.
> Predicate pushdown is not possible — YXDB rows are LZF-compressed with no
> block-level statistics, so `.filter()` is applied after the scan.

### Metadata

```python
import sigilyx as yx

# Inspect schema without reading any row data
fields = yx.read_yxdb_fields("data.yxdb")
for f in fields:
    print(f.name, f.field_type, f.size)

# Record count from the file header — no data read
n = yx.record_count("data.yxdb")
```

## Field Types

| YXDB Type | Polars / Arrow Type | Notes |
|-----------|---------------------|-------|
| `Bool` | `Boolean` | |
| `Byte` | `Int16` | Unsigned byte stored as Int16 |
| `Int16` | `Int16` | |
| `Int32` | `Int32` | |
| `Int64` | `Int64` | |
| `Float` | `Float32` | |
| `Double` | `Float64` | |
| `FixedDecimal` | `Decimal` | Precision and scale preserved |
| `String` | `String` / `Utf8` | Fixed-width, ASCII/Latin-1 |
| `WString` | `String` / `Utf8` | Fixed-width, UTF-16 decoded |
| `V_String` | `String` / `LargeUtf8` | Variable-length, ASCII/Latin-1 |
| `V_WString` | `String` / `LargeUtf8` | Variable-length, UTF-16 decoded |
| `Date` | `Date` | Days since Unix epoch |
| `DateTime` | `Datetime(us)` | Microsecond precision |
| `Time` | `Time` | Nanosecond precision |
| `Blob` | `Binary` / `LargeBinary` | Variable-length binary |
| `SpatialObj` | `Binary` / `LargeBinary` | Geometry as ISO WKB or raw SHP bytes |

## Links

- **GitHub:** https://github.com/Sigilweaver/SigilYX
- **Documentation:** https://sigilweaver.app/sigilyx/
- **Rust crate (crates.io):** https://crates.io/crates/sigilyx
- **Changelog:** https://github.com/Sigilweaver/SigilYX/blob/main/CHANGELOG.md
- **Issues:** https://github.com/Sigilweaver/SigilYX/issues

## License

[Apache License 2.0](https://github.com/Sigilweaver/SigilYX/blob/main/LICENSE).
