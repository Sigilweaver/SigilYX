---
sidebar_position: 9
description: "Write YXDB files from Polars, PyArrow, Pandas, and LazyFrames, including streaming batch writes."
---

# Writing

SigilYX provides multiple ways to write YXDB files from Python. All write paths use the same Rust writer with pipelined LZF compression.

## Write a Polars DataFrame

```python
import sigilyx as yx
import polars as pl

df = pl.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})

# Direct API
yx.write_yxdb("output.yxdb", df)

# Or via the Polars namespace plugin
import sigilyx
df.yxdb.write("output.yxdb")
```

### With Spatial Columns

If your DataFrame contains Binary columns with WKB geometry data, mark them as spatial so they are written as `SpatialObj` fields:

```python
yx.write_yxdb("output.yxdb", df, spatial_columns=["geometry"])
```

## Write a LazyFrame

Collect and write a LazyFrame in one step:

```python
import sigilyx as yx
import polars as pl

lf = pl.scan_csv("large_file.csv").filter(pl.col("status") == "active")

# Direct API
yx.sink_yxdb("output.yxdb", lf)

# Or via the Polars namespace plugin
lf.yxdb.sink("output.yxdb")
```

:::note
Unlike Polars' native `sink_parquet` which streams directly to disk, `sink_yxdb` collects the LazyFrame first because the YXDB header requires the record count upfront. For datasets larger than available RAM, use `write_yxdb_batches` with a batched data source instead.
:::

## Write a PyArrow Table

```python
import sigilyx as yx
import pyarrow as pa

table = pa.table({"id": [1, 2, 3], "name": ["a", "b", "c"]})
yx.write_yxdb_arrow("output.yxdb", table)
```

## Write a Pandas DataFrame

```python
import sigilyx as yx
import pandas as pd

df = pd.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
yx.write_yxdb_pandas("output.yxdb", df)
```

## Streaming Batch Writes

For datasets too large to hold in memory, `write_yxdb_batches` writes from an iterator of DataFrames:

```python
import sigilyx as yx
import polars as pl

def read_chunks():
    reader = pl.read_csv_batched("huge_file.csv", batch_size=100_000)
    while True:
        batch = reader.next_batches(1)
        if batch is None:
            break
        yield batch[0]

n_written = yx.write_yxdb_batches("output.yxdb", read_chunks())
print(f"Wrote {n_written:,} records")
```

Only one batch is in memory at a time. The Rust streaming writer writes LZF-compressed record blocks incrementally and updates the header record count upon finalization.

### ETL Pipeline Example

```python
import sigilyx as yx
import polars as pl

def transform(batch: pl.DataFrame) -> pl.DataFrame:
    return (
        batch
        .filter(pl.col("amount") > 0)
        .with_columns(pl.col("amount").cast(pl.Float64).alias("amount_f"))
    )

def pipeline():
    for batch in yx.read_yxdb_batches("input.yxdb", batch_size=100_000):
        yield transform(batch)

n = yx.write_yxdb_batches("output.yxdb", pipeline())
print(f"Transformed {n:,} records")
```

## Write API Summary

| Function | Input | Use Case |
| --- | --- | --- |
| `yx.write_yxdb(path, df)` | `polars.DataFrame` | Default write path |
| `df.yxdb.write(path)` | `polars.DataFrame` | Polars namespace plugin |
| `yx.sink_yxdb(path, lf)` | `polars.LazyFrame` | Collect + write |
| `lf.yxdb.sink(path)` | `polars.LazyFrame` | Namespace plugin sink |
| `yx.write_yxdb_arrow(path, table)` | `pyarrow.Table` | Arrow ecosystem |
| `yx.write_yxdb_pandas(path, df)` | `pandas.DataFrame` | Pandas ecosystem |
| `yx.write_yxdb_batches(path, iter)` | `Iterator[polars.DataFrame]` | Streaming / large files |
| `yx.write_yxdb_geo(path, gdf)` | `geopandas.GeoDataFrame` | Geospatial (see [Spatial](/python/spatial)) |
