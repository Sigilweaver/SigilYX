"""YXDB write functions and DataFrame validation helpers."""

from __future__ import annotations

import io
import warnings
from pathlib import Path
from typing import Union

import polars as pl

from sigilyx.sigilyx import (
    write_yxdb_df as _write_yxdb_df,
    write_yxdb_df_with_overrides as _write_yxdb_df_with_overrides,
    write_yxdb_ipc_with_overrides as _write_yxdb_ipc_with_overrides,
    write_yxdb as _write_yxdb_ipc,
    YxdbStreamWriter as _YxdbStreamWriter,
)


def _validate_df_for_write(df: object) -> pl.DataFrame:
    """Validate and pre-process a DataFrame for YXDB writing.

    Raises TypeError for non-DataFrame inputs and unsupported column types.
    Casts incompatible types (Categorical → String, UInt8/UInt16 → wider signed types)
    and emits warnings for lossy conversions (timezone-aware datetimes).
    """
    if not isinstance(df, pl.DataFrame):
        raise TypeError(
            f"write_yxdb expects a polars.DataFrame, got {type(df).__name__}. "
            f"Use write_yxdb_pandas() for pandas DataFrames or write_yxdb_arrow() for PyArrow Tables."
        )

    # Unsupported compound types — reject with a clear message
    _UNSUPPORTED_TYPES = (pl.List, pl.Struct, pl.Array, pl.Object, pl.Duration)
    for col_name in df.columns:
        dtype = df[col_name].dtype
        base = dtype.base_type()
        if base in _UNSUPPORTED_TYPES:
            raise TypeError(
                f"Column '{col_name}' has type {dtype} which is not supported by YXDB. "
                f"Consider flattening structs, exploding lists, or casting to a supported type."
            )
        if base == pl.Null:
            raise TypeError(
                f"Column '{col_name}' has Null dtype (all values are null with no type information). "
                f"Cast to a concrete type before writing, e.g.: "
                f"df.with_columns(pl.col('{col_name}').cast(pl.String))"
            )

    # Cast incompatible types before sending to Rust
    cast_exprs = []
    for col_name in df.columns:
        dtype = df[col_name].dtype

        # Categorical / Enum → String (avoids Rust panic from dtype-categorical)
        if dtype == pl.Categorical or dtype.base_type() == pl.Categorical:
            cast_exprs.append(pl.col(col_name).cast(pl.String))
        elif isinstance(dtype, pl.Enum):
            cast_exprs.append(pl.col(col_name).cast(pl.String))

        # Timezone-aware datetimes: warn about timezone loss
        elif isinstance(dtype, pl.Datetime) and dtype.time_zone is not None:
            tz = dtype.time_zone
            warnings.warn(
                f"Column '{col_name}' has timezone '{tz}' which will be lost on write. "
                f"Values are stored as UTC. Use "
                f".dt.convert_time_zone('UTC').dt.replace_time_zone(None) "
                f"to make this explicit.",
                UserWarning,
                stacklevel=3,
            )

        # Empty column names
        if col_name == "":
            idx = df.columns.index(col_name)
            raise ValueError(
                f"Column name cannot be empty (column index {idx})"
            )

    if cast_exprs:
        df = df.with_columns(cast_exprs)

    return df


def _prepare_df_for_ipc(df: pl.DataFrame) -> pl.DataFrame:
    """Pre-cast types that the IPC fallback path cannot handle."""
    cast_exprs = []
    for col_name in df.columns:
        dtype = df[col_name].dtype
        # Categorical / Enum → String (avoids Rust panic)
        if dtype == pl.Categorical or dtype.base_type() == pl.Categorical:
            cast_exprs.append(pl.col(col_name).cast(pl.String))
        elif isinstance(dtype, pl.Enum):
            cast_exprs.append(pl.col(col_name).cast(pl.String))
        # UInt8/UInt16 → wider signed (IPC fallback fails on these)
        elif dtype == pl.UInt8:
            cast_exprs.append(pl.col(col_name).cast(pl.Int16))
        elif dtype == pl.UInt16:
            cast_exprs.append(pl.col(col_name).cast(pl.Int32))
    if cast_exprs:
        df = df.with_columns(cast_exprs)
    return df


def _is_pyo3_compat_error(exc: TypeError) -> bool:
    """Check if a TypeError is from a pyo3-polars version mismatch."""
    msg = str(exc)
    # pyo3-polars compat_level mismatch surfaces as one of:
    #   - explicit "compat_level" mention
    #   - "cannot be converted" from pyo3 type conversion failure
    return "compat_level" in msg or "cannot be converted" in msg


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

    Notes
    -----
    YXDB DateTime fields have second-level precision — sub-second data
    is truncated. Timezone information is not stored; timezone-aware
    columns are written as UTC values with the timezone stripped.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import polars as pl
    >>> df = pl.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
    >>> yx.write_yxdb("output.yxdb", df)

    With spatial data:

    >>> yx.write_yxdb("output.yxdb", df, spatial_columns=["geometry"])
    """
    df = _validate_df_for_write(df)
    try:
        _write_yxdb_df(str(path), df, spatial_columns=spatial_columns)
    except TypeError as exc:
        if _is_pyo3_compat_error(exc):
            df = _prepare_df_for_ipc(df)
            buf = io.BytesIO()
            df.write_ipc(buf)
            _write_yxdb_ipc(str(path), buf.getvalue(), spatial_columns=spatial_columns)
        else:
            raise


def write_yxdb_with_overrides(
    path: Union[str, Path],
    df: pl.DataFrame,
    type_overrides: dict[str, dict],
    *,
    spatial_columns: list[str] | None = None,
) -> None:
    """Write a Polars DataFrame to YXDB with explicit field type overrides.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    df : polars.DataFrame
        The DataFrame to write.
    type_overrides : dict[str, dict]
        Maps column name to override dict with keys:
          - ``type``: YXDB type name (``"String"``, ``"WString"``, ``"V_String"``,
            ``"V_WString"``, ``"Bool"``, ``"Byte"``, ``"Int16"``, ``"Int32"``,
            ``"Int64"``, ``"Float"``, ``"Double"``, ``"FixedDecimal"``,
            ``"Date"``, ``"Time"``, ``"DateTime"``, ``"Blob"``, ``"SpatialObj"``)
          - ``size`` (optional): field size (max chars for strings, precision for FixedDecimal)
          - ``scale`` (optional): scale (only for FixedDecimal)
    spatial_columns : list[str] or None
        Names of Binary columns containing WKB geometry data.

    Examples
    --------
    >>> yx.write_yxdb_with_overrides("out.yxdb", df, {"name": {"type": "String", "size": 64}})
    """
    df = _validate_df_for_write(df)
    try:
        _write_yxdb_df_with_overrides(str(path), df, type_overrides, spatial_columns=spatial_columns)
    except TypeError as exc:
        if _is_pyo3_compat_error(exc):
            df = _prepare_df_for_ipc(df)
            buf = io.BytesIO()
            df.write_ipc(buf)
            _write_yxdb_ipc_with_overrides(
                str(path), buf.getvalue(), type_overrides, spatial_columns=spatial_columns,
            )
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
    pl_df = pl.from_arrow(table)
    write_yxdb(path, pl_df)


def sink_yxdb(path: Union[str, Path], lf: pl.LazyFrame) -> None:
    """Write a Polars LazyFrame to an Alteryx YXDB file.

    This collects the LazyFrame and writes the result to YXDB.
    For very large datasets that don't fit in memory, consider using
    ``write_yxdb_batches`` with a batched data source instead.

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
    Unlike Polars' native ``sink_parquet`` which streams data directly to disk,
    this function collects the LazyFrame first because YXDB format requires
    knowing the record count before writing the header. For datasets larger
    than available RAM, use chunked processing with ``write_yxdb_batches``.
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
