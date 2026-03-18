"""
SigilYX -- High-performance YXDB (E1) file reader and writer.

Format scope: supports the E1 (original engine) YXDB layout.
Files produced by the AMP engine (E2)
use a different binary layout and are not yet supported.

Usage::

    import sigilyx as yx
    df = yx.read_yxdb("path/to/file.yxdb")           # polars.DataFrame
    df = yx.read_yxdb_pandas("path/to/file.yxdb")     # pandas.DataFrame
    tbl = yx.read_yxdb_arrow("path/to/file.yxdb")     # pyarrow.Table

    yx.write_yxdb("output.yxdb", df)                  # Write to YXDB

Polars Integration (official plugin API)::

    import polars as pl
    import sigilyx  # Auto-registers on import

    df = pl.read_yxdb("data.yxdb")       # Read using Polars-style API
    df.yxdb.write("output.yxdb")         # Write via registered namespace

    lf = pl.scan_yxdb("data.yxdb")       # LazyFrame for deferred execution
    lf.yxdb.sink("output.yxdb")          # Collect and write via namespace

Streaming & Lazy Scan:

    ``scan_yxdb()`` returns a Polars **LazyFrame** backed by a native
    Rust streaming reader.  Polars' **projection pushdown** (only
    materialise requested columns) and **n_rows pushdown** (stop early)
    are honoured.  The YXDB format does not support predicate pushdown
    (rows are LZF-compressed with no statistics), so filters are
    applied post-scan.

    ``read_yxdb_batches()`` yields DataFrames one batch at a time in
    constant memory, with optional *columns* and *n_rows* arguments.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _pkg_version

try:
    __version__ = _pkg_version("sigilyx")
except PackageNotFoundError:
    __version__ = "0.0.0-dev"

# ── Public API re-exports ───────────────────────────────────────────────
# The implementation is split across private sub-modules for
# maintainability.  Everything listed in __all__ is importable
# directly from ``sigilyx``.

from sigilyx._geo import (  # noqa: E402
    _apply_geoarrow_metadata,
    read_spatial_info,
    read_yxdb_geo,
    read_yxdb_geoarrow,
    shp_to_wkb,
    wkb_to_shp,
    write_yxdb_geo,
)
from sigilyx._polars_plugin import (  # noqa: E402
    YxdbDataFrameNamespace,
    YxdbLazyFrameNamespace,
    register_polars,
)
from sigilyx._readers import (  # noqa: E402
    YxdbRowReader,
    read_schema,
    read_yxdb,
    read_yxdb_arrow,
    read_yxdb_batches,
    read_yxdb_columns,
    read_yxdb_fields,
    read_yxdb_pandas,
    record_count,
    scan_yxdb,
)
from sigilyx._types import FieldInfo  # noqa: E402
from sigilyx._writers import (  # noqa: E402
    sink_yxdb,
    write_yxdb,
    write_yxdb_arrow,
    write_yxdb_batches,
    write_yxdb_pandas,
    write_yxdb_with_overrides,
)

# Convenience aliases
read = read_yxdb
scan = scan_yxdb
write = write_yxdb
sink = sink_yxdb

__all__ = [
    "__version__",
    "read",
    "read_yxdb",
    "read_yxdb_columns",
    "read_yxdb_pandas",
    "read_yxdb_arrow",
    "read_yxdb_geoarrow",
    "read_yxdb_geo",
    "scan",
    "scan_yxdb",
    "read_yxdb_batches",
    "read_schema",
    "read_yxdb_fields",
    "read_spatial_info",
    "record_count",
    "FieldInfo",
    "YxdbRowReader",
    "write",
    "write_yxdb",
    "write_yxdb_with_overrides",
    "write_yxdb_batches",
    "write_yxdb_pandas",
    "write_yxdb_arrow",
    "write_yxdb_geo",
    "sink",
    "sink_yxdb",
    "shp_to_wkb",
    "wkb_to_shp",
    "register_polars",
    "YxdbDataFrameNamespace",
    "YxdbLazyFrameNamespace",
]

# Auto-register Polars integration on import
register_polars()
