---
sidebar_position: 4
---

# PyArrow

SigilYX can produce PyArrow Tables directly. This is useful if you're working in the Arrow ecosystem -- for example, writing to Parquet, feeding data into DuckDB, or bridging to other Arrow-compatible tools.

## Installation

```bash
pip install sigilyx[arrow]
```

## Reading

```python
import sigilyx as yx

table = yx.read_yxdb_arrow("data.yxdb")
print(type(table))  # <class 'pyarrow.lib.Table'>
print(table.schema)
```

The Arrow Table is built from the same Arrow arrays that the Rust reader produces. This is a near-zero-cost conversion.

## Writing

```python
import sigilyx as yx
import pyarrow as pa

# Write a PyArrow Table to YXDB
table = pa.table({"id": [1, 2, 3], "name": ["a", "b", "c"]})
yx.write_yxdb_arrow("output.yxdb", table)
```

## Type Mapping

| Arrow Type | YXDB Type |
| --- | --- |
| `bool` | Boolean |
| `int16` | Int16 |
| `int32` | Int32 |
| `int64` | Int64 |
| `float32` | Float |
| `float64` | Double |
| `decimal128` | FixedDecimal |
| `utf8` | String / WString |
| `large_utf8` | V_String / V_WString |
| `date32` | Date |
| `timestamp[us]` | DateTime |
| `time64[ns]` | Time |
| `binary` / `large_binary` | Blob |

## Common Patterns

### YXDB to Parquet via Arrow

```python
import sigilyx as yx
import pyarrow.parquet as pq

table = yx.read_yxdb_arrow("data.yxdb")
pq.write_table(table, "data.parquet")
```

### Query YXDB with DuckDB

```python
import sigilyx as yx
import duckdb

table = yx.read_yxdb_arrow("data.yxdb")

result = duckdb.sql("""
    SELECT region, SUM(revenue) as total
    FROM table
    GROUP BY region
    ORDER BY total DESC
""").arrow()
```

### Stream Arrow RecordBatches

If you need batch-level control, you can use `read_yxdb_batches()` and convert each batch:

```python
import sigilyx as yx

for polars_batch in yx.read_yxdb_batches("data.yxdb", batch_size=50_000):
    arrow_batch = polars_batch.to_arrow()
    # Process each batch individually
```

### IPC / Feather round-trip

```python
import sigilyx as yx
import pyarrow as pa
import pyarrow.feather as feather

table = yx.read_yxdb_arrow("data.yxdb")
feather.write_feather(table, "data.feather")

# Read back
table2 = feather.read_table("data.feather")
yx.write_yxdb_arrow("roundtrip.yxdb", table2)
```
