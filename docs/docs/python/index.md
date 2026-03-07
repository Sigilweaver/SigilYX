---
sidebar_position: 1
---

# Python Guide

SigilYX provides first-class Python bindings for reading and writing YXDB files. The Rust core does the heavy lifting - your Python code gets native performance with zero-copy data transfer.

## Supported Output Formats

| Format | Function | Best for |
| --- | --- | --- |
| **Polars** (default) | `read_yxdb()` | Fastest path, modern DataFrame API |
| **PyArrow** | `read_yxdb_arrow()` | Interop with Arrow ecosystem |
| **Pandas** | `read_yxdb_pandas()` | Legacy codebases, broad library support |

All three formats use the same Rust reader under the hood. Polars is the fastest because data transfers via the Arrow C Data Interface with zero copies. PyArrow and Pandas add a conversion step but are still orders of magnitude faster than pure-Python alternatives.

## Installation

See the [Installation](/python/installation) page for full details, including optional extras and building from source.

```bash
pip install sigilyx
```

## Two Ways to Use It

### 1. Polars Plugin API (Recommended)

Just importing `sigilyx` registers official Polars namespace plugins:

```python
import polars as pl
import sigilyx  # That's all it takes

df = pl.read_yxdb("data.yxdb")
df.yxdb.write("output.yxdb")
```

### 2. Direct API

```python
import sigilyx as yx

df = yx.read_yxdb("data.yxdb")       # Polars DataFrame
yx.write_yxdb("output.yxdb", df)
```

Both approaches use the same Rust reader. The Polars plugin API is syntactically nicer if you're already in a Polars workflow.

## Next Steps

- [Installation](/python/installation) - Requirements and optional extras
- [Polars](/python/polars) - Full Polars integration guide
- [Pandas](/python/pandas) - Working with Pandas DataFrames
- [PyArrow](/python/pyarrow) - Arrow Tables and interop
- [Streaming](/python/streaming) - Batched reads for large files
- [Lazy Scan](/python/lazy-scan) - Deferred execution with LazyFrames
- [Writing](/python/writing) - All write paths including streaming batch writes
- [Metadata](/python/metadata) - Inspect file structure without reading data
- [Spatial & GeoArrow](/python/spatial) - Geospatial data, GeoArrow, and GeoPandas
- [Row Reader](/python/row-reader) - Row-by-row iteration
