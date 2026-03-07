---
sidebar_position: 6
---

# Lazy Scan

SigilYX integrates with Polars' lazy evaluation engine. A lazy scan reads only the YXDB header upfront - the actual data is streamed from Rust when you call `.collect()`.

## Basic Usage

```python
import polars as pl
import sigilyx

lf = pl.scan_yxdb("data.yxdb")
print(type(lf))  # <class 'polars.LazyFrame'>

# Build a query plan
query = (
    lf
    .filter(pl.col("amount") > 100)
    .select("id", "name", "amount")
    .sort("amount", descending=True)
    .head(50)
)

# Data is read and processed only now
result = query.collect()
```

The equivalent direct API:

```python
import sigilyx as yx

lf = yx.scan("data.yxdb")
```

## Pushdown Support

Polars pushes optimizations down into the SigilYX reader where possible:

| Optimization | Supported | Effect |
| --- | --- | --- |
| **Projection pushdown** | Yes | Only selected columns are decoded |
| **Row-limit pushdown** | Yes | Reading stops after `n_rows` / `.head()` |
| **Predicate pushdown** | No | Filters applied post-scan |

### Why no predicate pushdown?

YXDB rows are packed into LZF-compressed blocks with no block-level statistics (min/max, bloom filters, etc.). The reader must decompress and decode every block to evaluate filters. This is a format limitation, not a SigilYX limitation.

:::tip
For filter-heavy queries on large files, consider converting to Parquet first. Parquet supports predicate pushdown via row-group statistics, which can skip large portions of the file entirely.
:::

## Projection Pushdown in Action

```python
import polars as pl
import sigilyx

# Only "id" and "name" are decoded from the YXDB file
result = pl.scan_yxdb("wide_file.yxdb").select("id", "name").collect()
```

For a 50-column file where you only need 2 columns, this is dramatically faster and uses much less memory than reading all columns.

## Row-Limit Pushdown

```python
# Reading stops after 100 rows - the rest of the file is never touched
preview = pl.scan_yxdb("big_file.yxdb").head(100).collect()
```

## Combining with Other Lazy Sources

Polars LazyFrames compose naturally:

```python
import polars as pl
import sigilyx

yxdb_data = pl.scan_yxdb("legacy.yxdb")
parquet_data = pl.scan_parquet("modern.parquet")

# Join across formats - everything stays lazy until .collect()
result = (
    yxdb_data
    .join(parquet_data, on="customer_id", how="inner")
    .group_by("region")
    .agg(pl.col("revenue").sum())
    .collect()
)
```

## When to Use Lazy vs Eager

| Scenario | Recommendation |
| --- | --- |
| Small files (< 100 MB) | Eager `pl.read_yxdb()` is simpler |
| Large files, need all columns | Eager or streaming batches |
| Large files, need few columns | Lazy `pl.scan_yxdb()` - projection pushdown shines |
| Preview / sampling | Lazy with `.head()` - row-limit pushdown |
| Complex multi-source pipelines | Lazy - let Polars optimize the plan |
