"""
SigilYX -- High-performance YXDB file reader and writer.

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

import io
import warnings
from importlib.metadata import version as _pkg_version, PackageNotFoundError
from pathlib import Path
from typing import Iterator, Union

import polars as pl

try:
    __version__ = _pkg_version("sigilyx")
except PackageNotFoundError:
    __version__ = "0.0.0-dev"

from sigilyx.sigilyx import (
    read_yxdb as _read_yxdb_ipc,
    read_yxdb_df as _read_yxdb_df,
    read_yxdb_df_columns as _read_yxdb_df_columns,
    write_yxdb_df as _write_yxdb_df,
    shp_to_wkb_py as _shp_to_wkb,
    wkb_to_shp_py as _wkb_to_shp,
    read_yxdb_schema as _read_yxdb_schema,
    read_yxdb_record_count as _read_yxdb_record_count,
    read_yxdb_spatial_info as _read_yxdb_spatial_info,
    write_yxdb as _write_yxdb_ipc,
    YxdbStreamWriter as _YxdbStreamWriter,
    _YxdbRowReader as _YxdbRowReaderRust,
    _YxdbBatchReader as _YxdbBatchReaderRust,
)


# ── YXDB field type → Polars dtype mapping ──────────────────────────────

# Maps the canonical YXDB XML type name (from FieldType::Display) to the
# corresponding Polars data type.
_YXDB_TYPE_MAP: dict[str, "pl.DataType"] = {
    "Bool": pl.Boolean,
    "Byte": pl.Int16,
    "Int16": pl.Int16,
    "Int32": pl.Int32,
    "Int64": pl.Int64,
    "Float": pl.Float32,
    "Double": pl.Float64,
    "FixedDecimal": pl.Decimal,   # precision/scale filled per-column
    "String": pl.String,
    "WString": pl.String,
    "V_String": pl.String,
    "V_WString": pl.String,
    "Date": pl.Date,
    "Time": pl.Time,
    "DateTime": pl.Datetime("us"),
    "Blob": pl.Binary,
    "SpatialObj": pl.Binary,
}


def _yxdb_schema_to_polars(schema_info: list[dict]) -> dict[str, "pl.DataType"]:
    """Convert YXDB field metadata (from Rust) to a Polars SchemaDict."""
    result: dict[str, pl.DataType] = {}
    for field in schema_info:
        name = field["name"]
        ft = field["type"]
        if ft == "FixedDecimal":
            result[name] = pl.Decimal(
                precision=field.get("size", 18),
                scale=field.get("scale", 0),
            )
        else:
            result[name] = _YXDB_TYPE_MAP.get(ft, pl.String)
    return result


def read_yxdb(
    path: Union[str, Path],
    *,
    spatial: str = "wkb",
) -> pl.DataFrame:
    """Read an Alteryx YXDB file and return a Polars DataFrame.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    spatial : str, default ``"wkb"``
        How to handle ``SpatialObj`` columns:

        - ``"wkb"`` — decode the internal SHP format to ISO Well-Known
          Binary, consumable by Shapely, GeoPandas, PostGIS, GDAL, etc.
        - ``"raw"`` — keep the raw SHP bytes for expert/debug use.

    Returns
    -------
    polars.DataFrame
        The contents of the YXDB file as a Polars DataFrame.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb("data.yxdb")
    >>> df = yx.read_yxdb("data.yxdb", spatial="raw")
    """
    return _read_yxdb_df(str(path), spatial=spatial)


def read_yxdb_columns(
    path: Union[str, Path],
    columns: list[str],
    *,
    spatial: str = "wkb",
) -> pl.DataFrame:
    """Read only the specified columns from an Alteryx YXDB file.

    This is faster than reading the full file when you only need a subset
    of columns, because it skips parsing and allocating the unused fields.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    columns : list[str]
        Column names to read.
    spatial : str, default ``"wkb"``
        How to handle ``SpatialObj`` columns (see :func:`read_yxdb`).

    Returns
    -------
    polars.DataFrame
        A DataFrame containing only the requested columns.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb_columns("data.yxdb", ["Id", "Name"])
    """
    return _read_yxdb_df_columns(str(path), columns, spatial=spatial)


def shp_to_wkb(shp: bytes) -> bytes | None:
    """Convert SHP geometry bytes (Alteryx SpatialObj) to WKB.

    Parameters
    ----------
    shp : bytes
        Raw SHP geometry bytes (as stored in a YXDB SpatialObj field).

    Returns
    -------
    bytes or None
        ISO WKB bytes, or ``None`` for null shapes (SHP type 0).
    """
    return _shp_to_wkb(shp)


def wkb_to_shp(wkb: bytes) -> bytes:
    """Convert WKB geometry bytes to SHP format (Alteryx SpatialObj).

    Parameters
    ----------
    wkb : bytes
        ISO WKB geometry bytes.

    Returns
    -------
    bytes
        SHP geometry bytes suitable for writing to a YXDB SpatialObj field.
    """
    return _wkb_to_shp(wkb)


# ── Spatial Index Info ──────────────────────────────────────────────────


def read_spatial_info(path: Union[str, Path]) -> dict:
    """Read spatial index metadata from a YXDB file header.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    dict
        A dict with keys:

        - ``has_spatial_index`` (bool) — whether the file has a spatial index
        - ``spatial_index_pos`` (int) — file offset of the spatial index (0 if absent)
        - ``file_id`` (int) — raw file ID / version from the header
        - ``spatial_columns`` (list[str]) — names of SpatialObj columns

    Examples
    --------
    >>> import sigilyx as yx
    >>> info = yx.read_spatial_info("data.yxdb")
    >>> if info["has_spatial_index"]:
    ...     print("File has spatial index at offset", info["spatial_index_pos"])
    >>> print("Spatial columns:", info["spatial_columns"])
    """
    return _read_yxdb_spatial_info(str(path))


# ── GeoArrow Support ───────────────────────────────────────────────────


def _apply_geoarrow_metadata(
    table: "pyarrow.Table",
    spatial_columns: list[str],
) -> "pyarrow.Table":
    """Annotate WKB binary columns with GeoArrow extension type metadata.

    Adds ``ARROW:extension:name = "geoarrow.wkb"`` to the Arrow field
    metadata of each spatial column, making the table compatible with
    GeoArrow-aware tools (lonboard, leafmap, DuckDB Spatial, etc.).
    """
    import pyarrow as pa

    schema = table.schema
    new_fields = []
    changed = False

    for i, field in enumerate(schema):
        if field.name in spatial_columns and (pa.types.is_binary(field.type) or pa.types.is_large_binary(field.type)):
            geo_meta = {
                b"ARROW:extension:name": b"geoarrow.wkb",
                b"ARROW:extension:metadata": b'{}',
            }
            existing = field.metadata or {}
            merged = {**existing, **geo_meta}
            new_fields.append(field.with_metadata(merged))
            changed = True
        else:
            new_fields.append(field)

    if changed:
        return table.cast(pa.schema(new_fields, metadata=schema.metadata))
    return table


def read_yxdb_geoarrow(
    path: Union[str, Path],
    *,
    columns: list[str] | None = None,
) -> "pyarrow.Table":
    """Read a YXDB file and return a PyArrow Table with GeoArrow metadata.

    SpatialObj columns are decoded to ISO WKB and tagged with
    ``ARROW:extension:name = "geoarrow.wkb"`` in the Arrow schema,
    making them compatible with GeoArrow-aware libraries.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    columns : list[str] or None, default None
        If provided, only read these columns.

    Returns
    -------
    pyarrow.Table
        A PyArrow Table with GeoArrow extension type metadata on
        spatial columns.

    Examples
    --------
    >>> import sigilyx as yx
    >>> table = yx.read_yxdb_geoarrow("spatial_data.yxdb")
    >>> # Spatial columns now have ARROW:extension:name = "geoarrow.wkb"
    """
    info = read_spatial_info(path)
    spatial_cols = info["spatial_columns"]

    if columns is not None:
        df = read_yxdb_columns(path, columns, spatial="wkb")
    else:
        df = read_yxdb(path, spatial="wkb")

    table = df.to_arrow()
    return _apply_geoarrow_metadata(table, spatial_cols)


# ── GeoPandas / Shapely Integration ────────────────────────────────────


def read_yxdb_geo(
    path: Union[str, Path],
    *,
    columns: list[str] | None = None,
    geometry_column: str | None = None,
) -> "geopandas.GeoDataFrame":
    """Read a YXDB file and return a GeoPandas GeoDataFrame.

    SpatialObj columns are decoded from SHP to WKB, then converted to
    Shapely geometry objects. The first spatial column (or the one named
    by *geometry_column*) is set as the active geometry.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    columns : list[str] or None, default None
        If provided, only read these columns.
    geometry_column : str or None, default None
        Name of the column to use as the active geometry. If ``None``,
        the first ``SpatialObj`` column is used.

    Returns
    -------
    geopandas.GeoDataFrame

    Raises
    ------
    ImportError
        If geopandas or shapely is not installed.
    ValueError
        If no SpatialObj columns are found in the file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> gdf = yx.read_yxdb_geo("parcels.yxdb")
    >>> gdf.plot()
    """
    try:
        import geopandas as gpd
        from shapely import from_wkb
    except ImportError as exc:
        raise ImportError(
            "geopandas and shapely are required for read_yxdb_geo(). "
            "Install them with: pip install geopandas shapely"
        ) from exc

    info = read_spatial_info(path)
    spatial_cols = info["spatial_columns"]

    if not spatial_cols:
        raise ValueError(
            f"No SpatialObj columns found in {path}. "
            "Use read_yxdb() for non-spatial files."
        )

    if columns is not None:
        df = read_yxdb_columns(path, columns, spatial="wkb")
        # Filter spatial columns to only those actually requested
        spatial_cols = [c for c in spatial_cols if c in columns]
    else:
        df = read_yxdb(path, spatial="wkb")

    if not spatial_cols:
        raise ValueError(
            "None of the requested columns are SpatialObj columns. "
            "Use read_yxdb() for non-spatial queries."
        )

    # Convert to pandas
    pdf = df.to_pandas()

    # Convert WKB binary columns to Shapely geometry
    for col in spatial_cols:
        if col in pdf.columns:
            pdf[col] = gpd.GeoSeries.from_wkb(pdf[col])

    # Choose the active geometry column
    geom_col = geometry_column or spatial_cols[0]
    if geom_col not in pdf.columns:
        raise ValueError(
            f"Geometry column {geom_col!r} not found. "
            f"Available spatial columns: {spatial_cols}"
        )

    return gpd.GeoDataFrame(pdf, geometry=geom_col)


def write_yxdb_geo(
    path: Union[str, Path],
    gdf: "geopandas.GeoDataFrame",
    *,
    spatial_columns: list[str] | None = None,
) -> None:
    """Write a GeoPandas GeoDataFrame to a YXDB file.

    Geometry columns are converted from Shapely objects to WKB, then
    written as ``SpatialObj`` fields in the YXDB file.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    gdf : geopandas.GeoDataFrame
        The GeoDataFrame to write.
    spatial_columns : list[str] or None, default None
        Names of geometry columns to write as SpatialObj. If ``None``,
        all geometry-dtype columns are auto-detected.

    Raises
    ------
    ImportError
        If geopandas is not installed.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import geopandas as gpd
    >>> gdf = gpd.read_file("parcels.shp")
    >>> yx.write_yxdb_geo("parcels.yxdb", gdf)
    """
    try:
        import geopandas as gpd
    except ImportError as exc:
        raise ImportError(
            "geopandas is required for write_yxdb_geo(). "
            "Install it with: pip install geopandas"
        ) from exc

    # Find geometry columns
    if spatial_columns is None:
        spatial_columns = [
            col for col in gdf.columns
            if isinstance(gdf[col].dtype, gpd.array.GeometryDtype)
        ]

    # Convert GeoDataFrame to a regular pandas DataFrame with WKB
    pdf = gdf.copy()
    for col in spatial_columns:
        if col in pdf.columns:
            pdf[col] = pdf[col].to_wkb()

    # Convert to Polars and write
    pl_df = pl.from_pandas(pdf)
    write_yxdb(path, pl_df, spatial_columns=spatial_columns)


def read_schema(path: Union[str, Path]) -> list[dict]:
    """Read field metadata from a YXDB file without loading data.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    list[dict]
        List of field metadata dicts with keys: name, type, size, scale.
    """
    return _read_yxdb_schema(str(path))


class FieldInfo:
    """Metadata for a single field (column) in a YXDB file.

    Attributes
    ----------
    name : str
        Column name.
    field_type : str
        YXDB field type (e.g. 'Int32', 'V_WString', 'Date').
    size : int
        Declared size (max chars for strings, precision for decimals).
    scale : int
        Scale (decimal places for FixedDecimal, 0 otherwise).
    """

    __slots__ = ("name", "field_type", "size", "scale")

    def __init__(self, d: dict):
        self.name: str = d["name"]
        self.field_type: str = d["type"]
        self.size: int = d.get("size", 0)
        self.scale: int = d.get("scale", 0)

    def __repr__(self) -> str:
        parts = f"name={self.name!r}, type={self.field_type!r}"
        if self.size:
            parts += f", size={self.size}"
        if self.scale:
            parts += f", scale={self.scale}"
        return f"FieldInfo({parts})"

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, FieldInfo):
            return NotImplemented
        return (
            self.name == other.name
            and self.field_type == other.field_type
            and self.size == other.size
            and self.scale == other.scale
        )


def read_yxdb_fields(path: Union[str, Path]) -> list["FieldInfo"]:
    """Read field metadata from a YXDB file without loading data.

    Returns a list of :class:`FieldInfo` objects with ``.name``,
    ``.field_type``, ``.size``, and ``.scale`` attributes.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    list[FieldInfo]
        List of field metadata objects.

    Examples
    --------
    >>> import sigilyx as yx
    >>> fields = yx.read_yxdb_fields("data.yxdb")
    >>> for f in fields:
    ...     print(f.name, f.field_type, f.size)
    """
    return [FieldInfo(d) for d in _read_yxdb_schema(str(path))]


class YxdbRowReader:
    """A row-by-row YXDB file reader.

    Provides a cursor-style API for iterating records one at a time
    and extracting typed field values, without building columnar data.
    This is useful for streaming processing or when you only need a
    subset of records.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> reader = yx.YxdbRowReader("data.yxdb")
    >>> while reader.next():
    ...     name = reader.read_name("Name")
    ...     print(name)
    >>> reader.close()

    As a context manager:

    >>> with yx.YxdbRowReader("data.yxdb") as reader:
    ...     for row in reader:
    ...         print(row)

    As an iterator (yields tuples):

    >>> for row in yx.YxdbRowReader("data.yxdb"):
    ...     print(row)
    """

    __slots__ = ("_reader", "_fields")

    def __init__(self, path: Union[str, Path]):
        self._reader = _YxdbRowReaderRust(str(path))
        self._fields: list[FieldInfo] | None = None

    def next(self) -> bool:
        """Advance to the next record.

        Returns True if a record is available, False if all records
        have been consumed.
        """
        return self._reader.next_record()

    def read_index(self, index: int):
        """Read a field value by column index (0-based)."""
        return self._reader.read_index(index)

    def read_name(self, name: str):
        """Read a field value by column name."""
        return self._reader.read_name(name)

    def read_all(self) -> tuple:
        """Read all field values from the current record as a tuple."""
        return self._reader.read_all()

    def read_dict(self) -> dict:
        """Read all field values as a dict {name: value}."""
        return self._reader.read_dict()

    @property
    def fields(self) -> list[FieldInfo]:
        """Field metadata for the YXDB file."""
        if self._fields is None:
            self._fields = [FieldInfo(d) for d in self._reader.fields()]
        return self._fields

    @property
    def num_records(self) -> int:
        """Total number of records in the file (from header)."""
        return self._reader.num_records()

    def close(self):
        """Close the reader and release resources."""
        self._reader.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False

    def __iter__(self):
        return self

    def __next__(self) -> tuple:
        if self.next():
            return self.read_all()
        raise StopIteration


def record_count(path: Union[str, Path]) -> int:
    """Return the number of records in a YXDB file (header-only read).

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    int
        Number of records in the file.
    """
    return _read_yxdb_record_count(str(path))


def scan_yxdb(path: Union[str, Path]) -> pl.LazyFrame:
    """Lazily scan an Alteryx YXDB file, returning a Polars LazyFrame.

    This is a *true* lazy scan: only the file header is read upfront
    to determine the schema. Data is streamed in constant-memory batches
    when ``.collect()`` is called, and Polars' projection pushdown
    (``with_columns``) and row-limit pushdown (``n_rows``) are respected.

    Predicate pushdown is **not** supported by the YXDB format (rows are
    interleaved and LZF-compressed with no block statistics), so any
    ``.filter()`` is applied after reading.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    polars.LazyFrame
        A lazy view over the YXDB data.

    Examples
    --------
    >>> import sigilyx as yx
    >>> lf = yx.scan_yxdb("data.yxdb")
    >>> result = lf.filter(pl.col("status") == "active").collect()

    Only the projected columns and the requested row limit are read:

    >>> lf = yx.scan_yxdb("data.yxdb")
    >>> top10 = lf.select("id", "name").head(10).collect()
    """
    from polars.io.plugins import register_io_source

    path_str = str(path)

    # Build the Polars schema from the YXDB header (fast, no data read).
    schema_info = _read_yxdb_schema(path_str)
    polars_schema = _yxdb_schema_to_polars(schema_info)

    def _source(
        with_columns: "list[str] | None",
        predicate: "pl.Expr | None",
        n_rows: "int | None",
        batch_size: "int | None",
    ) -> "Iterator[pl.DataFrame]":
        effective_batch = batch_size if batch_size is not None else 65_536
        reader = _YxdbBatchReaderRust(
            path_str,
            effective_batch,
            with_columns,
            n_rows,
        )
        for batch in reader:
            if predicate is not None:
                batch = batch.filter(predicate)
            yield batch

    return register_io_source(
        io_source=_source,
        schema=polars_schema,
        is_pure=True,
    )


def read_yxdb_batches(
    path: Union[str, Path],
    batch_size: int = 65_536,
    *,
    columns: "list[str] | None" = None,
    n_rows: "int | None" = None,
) -> "Iterator[pl.DataFrame]":
    """Read an Alteryx YXDB file in batches (streaming / memory-efficient).

    Yields one ``polars.DataFrame`` per batch of up to *batch_size* rows.
    This is truly streaming — only one batch is in memory at a time.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    batch_size : int, default 65 536
        Maximum number of rows per batch.
    columns : list[str] or None, default None
        If provided, only materialise these columns (projection pushdown).
    n_rows : int or None, default None
        If provided, stop after reading this many rows total.

    Yields
    ------
    polars.DataFrame
        A DataFrame containing up to *batch_size* rows.

    Examples
    --------
    >>> import sigilyx as yx
    >>> for batch in yx.read_yxdb_batches("big_file.yxdb", batch_size=100_000):
    ...     process(batch)
    """
    reader = _YxdbBatchReaderRust(str(path), batch_size, columns, n_rows)
    yield from reader


def read_yxdb_arrow(
    path: Union[str, Path],
    *,
    spatial: str = "wkb",
) -> "pyarrow.Table":
    """Read an Alteryx YXDB file and return a PyArrow Table.

    This reads the file via the native Rust reader into a Polars DataFrame,
    then converts to PyArrow with zero-copy. This avoids an expensive IPC
    serialization round-trip.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    spatial : str, default ``"wkb"``
        How to handle ``SpatialObj`` columns (see :func:`read_yxdb`).

    Returns
    -------
    pyarrow.Table
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> table = yx.read_yxdb_arrow("data.yxdb")
    """
    return read_yxdb(path, spatial=spatial).to_arrow()


def read_yxdb_pandas(
    path: Union[str, Path],
    *,
    spatial: str = "wkb",
) -> "pandas.DataFrame":
    """Read an Alteryx YXDB file and return a pandas DataFrame.

    Internally reads via the native Rust reader into a Polars DataFrame,
    converts to a PyArrow Table (zero-copy), then to pandas.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    spatial : str, default ``"wkb"``
        How to handle ``SpatialObj`` columns (see :func:`read_yxdb`).

    Returns
    -------
    pandas.DataFrame
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb_pandas("data.yxdb")
    """
    return read_yxdb_arrow(path, spatial=spatial).to_pandas()


# Convenience aliases for top-level usage: import sigilyx as yx; yx.read(...)
read = read_yxdb
scan = scan_yxdb


def write_yxdb(
    path: Union[str, Path],
    df: pl.DataFrame,
    *,
    spatial_columns: list[str] | None = None,
) -> None:
    """Write a Polars DataFrame to an Alteryx YXDB file.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    df : polars.DataFrame
        The DataFrame to write.
    spatial_columns : list[str] or None, default None
        Names of Binary columns containing WKB geometry data. These
        will be written as ``SpatialObj`` fields (WKB → SHP conversion).
        ``None`` or empty list means no spatial columns.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import polars as pl
    >>> df = pl.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
    >>> yx.write_yxdb("output.yxdb", df)

    With spatial data:

    >>> yx.write_yxdb("output.yxdb", df, spatial_columns=["geometry"])
    """
    try:
        _write_yxdb_df(str(path), df, spatial_columns=spatial_columns)
    except TypeError as exc:
        # pyo3-polars compat_level mismatch: fall back to IPC serialization.
        if "compat_level" in str(exc) or "argument" in str(exc).lower():
            buf = io.BytesIO()
            df.write_ipc(buf)
            _write_yxdb_ipc(str(path), buf.getvalue(), spatial_columns=spatial_columns)
        else:
            raise


def write_yxdb_pandas(path: Union[str, Path], df: "pandas.DataFrame") -> None:
    """Write a pandas DataFrame to an Alteryx YXDB file.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    df : pandas.DataFrame
        The DataFrame to write.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import pandas as pd
    >>> df = pd.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
    >>> yx.write_yxdb_pandas("output.yxdb", df)
    """
    # Convert pandas to polars, then write
    pl_df = pl.from_pandas(df)
    write_yxdb(path, pl_df)


def write_yxdb_arrow(path: Union[str, Path], table: "pyarrow.Table") -> None:
    """Write a PyArrow Table to an Alteryx YXDB file.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    table : pyarrow.Table
        The table to write.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import pyarrow as pa
    >>> table = pa.table({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
    >>> yx.write_yxdb_arrow("output.yxdb", table)
    """
    # Convert PyArrow to polars, then write
    pl_df = pl.from_arrow(table)
    write_yxdb(path, pl_df)


def sink_yxdb(path: Union[str, Path], lf: pl.LazyFrame) -> None:
    """Write a Polars LazyFrame to an Alteryx YXDB file.

    This collects the LazyFrame and writes the result to YXDB.
    For very large datasets that don't fit in memory, consider using
    `write_yxdb_batches` with a batched data source instead.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    lf : polars.LazyFrame
        The LazyFrame to collect and write.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import polars as pl
    >>> lf = pl.scan_csv("large_file.csv")
    >>> lf_filtered = lf.filter(pl.col("status") == "active")
    >>> yx.sink_yxdb("output.yxdb", lf_filtered)

    Notes
    -----
    Unlike Polars' native `sink_parquet` which streams data directly to disk,
    this function collects the LazyFrame first because YXDB format requires
    knowing the record count before writing the header. For datasets larger
    than available RAM, use chunked processing with `write_yxdb_batches`.
    """
    df = lf.collect()
    write_yxdb(path, df)


def write_yxdb_batches(
    path: Union[str, Path],
    batches: "Iterator[pl.DataFrame]",
) -> int:
    """Write an iterator of DataFrames to a YXDB file in streaming fashion.

    This enables memory-efficient writing of large datasets that are
    processed in chunks. Each batch is written incrementally without
    holding the entire dataset in memory.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    batches : Iterator[polars.DataFrame]
        An iterator yielding DataFrames to write. All DataFrames must
        have the same schema.

    Returns
    -------
    int
        The total number of records written.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import polars as pl
    >>>
    >>> # Stream from CSV in chunks
    >>> def read_chunks():
    ...     for chunk in pl.read_csv_batched("huge_file.csv", batch_size=100_000):
    ...         yield chunk.filter(pl.col("value") > 0)  # process each chunk
    >>>
    >>> n_written = yx.write_yxdb_batches("output.yxdb", read_chunks())
    >>> print(f"Wrote {n_written} records")

    Notes
    -----
    Uses the Rust streaming writer under the hood, which writes LZF-compressed
    record blocks incrementally and seeks back to update the header record count
    upon finalization. Only one batch needs to be in memory at a time.
    """
    path_str = str(path)
    writer = None

    for batch in batches:
        buf = io.BytesIO()
        batch.write_ipc(buf)
        ipc_bytes = buf.getvalue()

        if writer is None:
            # First batch: create writer (uses IPC for schema inference only)
            writer = _YxdbStreamWriter(path_str, ipc_bytes)

        writer.write_batch(ipc_bytes)

    if writer is None:
        raise ValueError(
            "Cannot write empty batch iterator — need at least one batch for schema"
        )

    return writer.finish()


# Convenience aliases
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


# ── Polars Integration ──────────────────────────────────────────────────
#
# Uses Polars' official plugin APIs:
#   - pl.api.register_dataframe_namespace  (df.yxdb.write)
#   - pl.api.register_lazyframe_namespace  (lf.yxdb.sink)
#   - polars.io.plugins.register_io_source (scan_yxdb, already used above)
#
# Top-level aliases (pl.read_yxdb, pl.scan_yxdb) are added as module
# attributes since Polars has no official API for top-level functions.


@pl.api.register_dataframe_namespace("yxdb")
class YxdbDataFrameNamespace:
    """YXDB operations on a Polars DataFrame.

    Accessed via the ``.yxdb`` namespace on any DataFrame::

        import polars as pl
        import sigilyx  # registers the namespace

        df = pl.read_yxdb("data.yxdb")
        df.yxdb.write("output.yxdb")
    """

    def __init__(self, df: pl.DataFrame) -> None:
        self._df = df

    def write(self, path: Union[str, Path]) -> None:
        """Write this DataFrame to a YXDB file.

        Parameters
        ----------
        path : str or Path
            Output file path.

        Examples
        --------
        >>> df.yxdb.write("output.yxdb")
        """
        write_yxdb(path, self._df)


@pl.api.register_lazyframe_namespace("yxdb")
class YxdbLazyFrameNamespace:
    """YXDB operations on a Polars LazyFrame.

    Accessed via the ``.yxdb`` namespace on any LazyFrame::

        import polars as pl
        import sigilyx  # registers the namespace

        lf = pl.scan_yxdb("data.yxdb")
        lf.yxdb.sink("output.yxdb")
    """

    def __init__(self, lf: pl.LazyFrame) -> None:
        self._lf = lf

    def sink(self, path: Union[str, Path]) -> None:
        """Collect this LazyFrame and write to a YXDB file.

        Parameters
        ----------
        path : str or Path
            Output file path.

        Examples
        --------
        >>> lf.yxdb.sink("output.yxdb")
        """
        sink_yxdb(path, self._lf)


def register_polars() -> bool:
    """Register YXDB integration with Polars.

    After calling this (or simply importing sigilyx), you can use:

        pl.read_yxdb("data.yxdb")        # top-level alias
        pl.scan_yxdb("data.yxdb")        # top-level alias
        df.yxdb.write("output.yxdb")     # official namespace plugin
        lf.yxdb.sink("output.yxdb")      # official namespace plugin

    The ``df.yxdb`` and ``lf.yxdb`` namespaces are registered via
    ``@pl.api.register_dataframe_namespace`` / ``register_lazyframe_namespace``
    (Polars' official plugin API) and are available as soon as sigilyx
    is imported.

    Returns
    -------
    bool
        True if registration succeeded, False if Polars not available.

    Examples
    --------
    >>> import polars as pl
    >>> import sigilyx  # Auto-registers on import
    >>> df = pl.read_yxdb("data.yxdb")
    >>> df.yxdb.write("output.yxdb")
    """
    try:
        import polars as pl

        # Top-level aliases (no official API for these).
        if not hasattr(pl, "read_yxdb"):
            pl.read_yxdb = read_yxdb
        if not hasattr(pl, "scan_yxdb"):
            pl.scan_yxdb = scan_yxdb

        # Backward-compat: keep the old monkey-patched methods but
        # emit a deprecation warning pointing to the namespace API.
        if not hasattr(pl.DataFrame, "write_yxdb"):
            def _df_write_yxdb_deprecated(
                self: pl.DataFrame, path: Union[str, Path]
            ) -> None:
                warnings.warn(
                    "DataFrame.write_yxdb() is deprecated. "
                    "Use df.yxdb.write(path) instead.",
                    DeprecationWarning,
                    stacklevel=2,
                )
                write_yxdb(path, self)
            pl.DataFrame.write_yxdb = _df_write_yxdb_deprecated  # type: ignore[attr-defined]

        if not hasattr(pl.LazyFrame, "sink_yxdb"):
            def _lf_sink_yxdb_deprecated(
                self: pl.LazyFrame, path: Union[str, Path]
            ) -> None:
                warnings.warn(
                    "LazyFrame.sink_yxdb() is deprecated. "
                    "Use lf.yxdb.sink(path) instead.",
                    DeprecationWarning,
                    stacklevel=2,
                )
                sink_yxdb(path, self)
            pl.LazyFrame.sink_yxdb = _lf_sink_yxdb_deprecated  # type: ignore[attr-defined]

        # Namespace plugins (df.yxdb / lf.yxdb) are registered via
        # the @pl.api decorators above — nothing more needed here.
        return True

    except ImportError:
        return False


# Auto-register on import
register_polars()
