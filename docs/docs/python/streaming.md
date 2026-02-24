---
sidebar_position: 5
---

# Streaming

For files that are too large to fit in memory, SigilYX provides batched reading. Each batch is a Polars DataFrame, and memory usage stays constant regardless of file size.

## Basic Batched Read

```python
import sigilyx as yx

for batch in yx.read_yxdb_batches("large_file.yxdb", batch_size=100_000):
    print(f"Batch: {batch.shape}")
    process(batch)
```

Each `batch` is a `polars.DataFrame` with at most `batch_size` rows. The default batch size is 100,000 rows if not specified.

## Column Projection

Only materialize the columns you need:

```python
for batch in yx.read_yxdb_batches("data.yxdb", columns=["Id", "Name", "Email"]):
    process(batch)
```

Columns not listed in the `columns` argument are never decoded. This saves both memory and CPU time, especially for wide files with many string columns.

## Row Limit

Stop after reading a fixed number of rows:

```python
for batch in yx.read_yxdb_batches("data.yxdb", n_rows=10_000):
    process(batch)
```

This reads exactly 10,000 rows (or fewer if the file is smaller), split across batches.

## Combining Options

All options compose:

```python
for batch in yx.read_yxdb_batches(
    "data.yxdb",
    batch_size=50_000,
    columns=["Id", "Amount"],
    n_rows=200_000,
):
    process(batch)
```

This reads 200,000 rows in batches of 50,000, materializing only the `Id` and `Amount` columns.

## Use Cases

### Aggregation over a large file

```python
import polars as pl
import sigilyx as yx

running_total = 0
row_count = 0

for batch in yx.read_yxdb_batches("huge_file.yxdb", columns=["revenue"]):
    running_total += batch["revenue"].sum()
    row_count += len(batch)

print(f"Average revenue: {running_total / row_count:.2f}")
```

### Write filtered subset

```python
import sigilyx as yx

filtered_frames = []
for batch in yx.read_yxdb_batches("data.yxdb", batch_size=100_000):
    filtered = batch.filter(batch["status"] == "active")
    if len(filtered) > 0:
        filtered_frames.append(filtered)

import polars as pl
result = pl.concat(filtered_frames)
yx.write_yxdb("active_only.yxdb", result)
```

### Progress reporting

```python
import sigilyx as yx

total_rows = yx.record_count("data.yxdb")
processed = 0

for batch in yx.read_yxdb_batches("data.yxdb", batch_size=100_000):
    processed += len(batch)
    pct = processed / total_rows * 100
    print(f"\r{processed:,}/{total_rows:,} ({pct:.1f}%)", end="", flush=True)
    process(batch)

print("\nDone.")
```

## Memory Characteristics

Batched reading uses constant memory proportional to `batch_size * row_width`. The full file is memory-mapped but only the current batch's decoded data lives on the heap.

For a file with 100 million rows and an 80-byte fixed record width:

| batch_size | Approximate heap per batch |
| --: | --: |
| 10,000 | ~0.8 MB |
| 100,000 | ~8 MB |
| 1,000,000 | ~80 MB |

Variable-length fields (V_WString, Blob, etc.) add to the per-batch memory proportionally to their content size.
