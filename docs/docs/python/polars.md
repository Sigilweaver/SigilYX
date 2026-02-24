---
sidebar_position: 2
description: "Read and write YXDB files with Polars DataFrames and LazyFrames."
---

# Polars

SigilYX is built for Polars. Importing `sigilyx` registers official Polars namespace plugins that make YXDB files feel like a native Polars format.

## Reading

### DataFrame

```python
import polars as pl
import sigilyx

# Read the entire file into a DataFrame
df = pl.read_yxdb("data.yxdb")
```

This is the fastest read path. Data transfers from Rust to Python via the Arrow C Data Interface with zero copies.

### LazyFrame

```python
# Returns a LazyFrame -- only the header is read upfront
lf = pl.scan_yxdb("data.yxdb")

# Data is streamed from Rust on .collect()
result = lf.filter(pl.col("amount") > 100).select("id", "name").collect()
```

Projection pushdown and row-limit pushdown are supported. See [Lazy Scan](/python/lazy-scan) for details.

### Equivalent Direct API

```python
import sigilyx as yx

df = yx.read_yxdb("data.yxdb")     # Same as pl.read_yxdb()
lf = yx.scan("data.yxdb")          # Same as pl.scan_yxdb()
```

## Writing

### From a DataFrame

```python
# Using the namespace plugin
df.yxdb.write("output.yxdb")

# Or using the direct API
import sigilyx as yx
yx.write_yxdb("output.yxdb", df)
```

### From a LazyFrame

```python
# Collect and write in one step
lf.yxdb.sink("output.yxdb")
```

This collects the LazyFrame and writes the result to a YXDB file.

## Type Mapping

Polars types are mapped to YXDB field types as follows:

| Polars Type | YXDB Type | Notes |
| --- | --- | --- |
| `Boolean` | Boolean | |
| `Int16` | Int16 | `Byte` maps to Int16 on read |
| `Int32` | Int32 | |
| `Int64` | Int64 | |
| `Float32` | Float | |
| `Float64` | Double | |
| `Decimal` | FixedDecimal | Precision and scale preserved |
| `String` | String / WString | Fixed-width strings |
| `String` | V_String / V_WString | Variable-length strings |
| `Date` | Date | Days since epoch |
| `Datetime` | DateTime | Microsecond precision |
| `Time` | Time | Nanosecond precision |
| `Binary` | Blob | Raw bytes |

## Common Patterns

### Read, transform, write

```python
import polars as pl
import sigilyx

df = pl.read_yxdb("input.yxdb")

result = (
    df
    .filter(pl.col("status") == "active")
    .with_columns(
        pl.col("revenue").cast(pl.Float64).alias("revenue_float"),
    )
    .group_by("region")
    .agg(pl.col("revenue_float").sum().alias("total_revenue"))
    .sort("total_revenue", descending=True)
)

result.yxdb.write("output.yxdb")
```

### Convert YXDB to Parquet

```python
import polars as pl
import sigilyx

pl.read_yxdb("data.yxdb").write_parquet("data.parquet")
```

### Convert Parquet to YXDB

```python
import polars as pl
import sigilyx

pl.read_parquet("data.parquet").yxdb.write("data.yxdb")
```

### Read only specific columns (lazy)

```python
import polars as pl
import sigilyx

result = (
    pl.scan_yxdb("large_file.yxdb")
    .select("id", "name", "email")
    .head(1000)
    .collect()
)
```

Only the three selected columns are materialized, and reading stops after 1,000 rows.
