---
sidebar_position: 1
slug: /
description: "YXDB reader and writer in Rust, with Python bindings for Polars, PyArrow, and Pandas."
---

# SigilYX

**YXDB reader and writer in Rust, with Python bindings.**

SigilYX reads and writes `.yxdb` files using [Polars](https://pola.rs/) DataFrames, [PyArrow](https://arrow.apache.org/docs/python/) Tables, or [Pandas](https://pandas.pydata.org/) DataFrames. It works on Windows, macOS, and Linux (x64 and ARM).

## Why SigilYX?

- **Standalone.** No native Alteryx Designer installation required.
- **Cross-platform.** Windows, macOS, Linux (x64 and ARM).
- **Read and Write.** Full round-trip support for all 17 YXDB field types.
- **Multiple output formats.** Polars, PyArrow, or Pandas - pick the DataFrame library you already use.
- **Streaming.** Batched reads with constant memory, or lazy scans with Polars LazyFrames.

## Quick Start

```bash
pip install sigilyx
```

```python
import polars as pl
import sigilyx  # Importing registers pl.read_yxdb() and more

# Read
df = pl.read_yxdb("data.yxdb")

# Write
df.yxdb.write("output.yxdb")
```

That's it. Jump to the [Python guide](/python) or the [Rust guide](/rust) for full details.

## At a Glance

| Feature | Details |
| --- | --- |
| Rust core | Memory-mapped I/O with parallel block decompression |
| Polars integration | Official namespace plugin - `pl.read_yxdb()`, `df.yxdb.write()`, lazy `pl.scan_yxdb()` |
| PyArrow support | Zero-copy `pyarrow.Table` via Arrow C Data Interface |
| Pandas support | `pandas.DataFrame` via PyArrow bridge |
| Streaming | Batched reads in constant memory for arbitrarily large files |
| Field types | All 17 YXDB types including `SpatialObj`, `Blob`, `FixedDecimal` |
| License | Apache-2.0 |

:::info Part of the Sigilweaver ecosystem
SigilYX is developed as part of [Sigilweaver](https://sigilweaver.app), the visual data pipeline platform. Sigilweaver uses SigilYX under the hood to read and write YXDB files at scale. If you're building data pipelines and want a visual, no-row-limit alternative to legacy ETL tools, check out [sigilweaver.app](https://sigilweaver.app).
:::

## Choose Your Path

| I want to... | Go here |
| --- | --- |
| Read/write YXDB in Python with Polars | [Python - Polars](/python/polars) |
| Read/write YXDB in Python with Pandas | [Python - Pandas](/python/pandas) |
| Read/write YXDB in Python with PyArrow | [Python - PyArrow](/python/pyarrow) |
| Stream large YXDB files in Python | [Streaming](/python/streaming) |
| Use lazy evaluation with Polars | [Lazy Scan](/python/lazy-scan) |
| Write YXDB files (all formats) | [Writing](/python/writing) |
| Work with geospatial / SpatialObj data | [Spatial & GeoArrow](/python/spatial) |
| Iterate rows one at a time | [Row Reader](/python/row-reader) |
| Use the Rust crate directly | [Rust Guide](/rust/getting-started) |
| Contribute to SigilYX | [Developer Guide](/developer) |
