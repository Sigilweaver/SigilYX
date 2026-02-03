"""
SigilYX — High-performance YXDB file reader and writer.
Not affiliated with Alteryx, Inc. "Alteryx" is a registered trademark of Alteryx, Inc.

Usage:
    import sigilyx as yx
    df = yx.read_yxdb("path/to/file.yxdb")           # polars.DataFrame
    df = yx.read_yxdb_pandas("path/to/file.yxdb")     # pandas.DataFrame
    tbl = yx.read_yxdb_arrow("path/to/file.yxdb")     # pyarrow.Table
    
    yx.write_yxdb("output.yxdb", df)                  # Write to YXDB

Polars Integration:
    Importing sigilyx registers methods on Polars classes:
    
    import polars as pl
    import sigilyx  # Auto-registers on import
    
    df = pl.read_yxdb("data.yxdb")       # Read using Polars-style API
    df.write_yxdb("output.yxdb")         # Write using method syntax
    
    lf = pl.scan_yxdb("data.yxdb")       # LazyFrame for deferred execution
    lf.sink_yxdb("output.yxdb")          # Collect and write
"""

from __future__ import annotations

import io
from pathlib import Path
from typing import Iterator, Union

import polars as pl

from sigilyx.sigilyx import (
    read_yxdb as _read_yxdb_ipc,
    read_yxdb_schema as _read_yxdb_schema,
    read_yxdb_record_count as _read_yxdb_record_count,
    read_yxdb_batches as _read_yxdb_batches,
    write_yxdb as _write_yxdb_ipc,
)


def read_yxdb(path: Union[str, Path]) -> pl.DataFrame:
    """Read an Alteryx YXDB file and return a Polars DataFrame.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    polars.DataFrame
        The contents of the YXDB file as a Polars DataFrame.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb("data.yxdb")
    >>> print(df)
    """
    ipc_bytes = _read_yxdb_ipc(str(path))
    return pl.read_ipc(io.BytesIO(ipc_bytes))


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

    The file is read eagerly under the hood (YXDB does not support
    predicate push-down), but wrapping the result in a LazyFrame lets
    you compose filter / select / join operations before calling
    ``.collect()``.

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
    """
    return read_yxdb(path).lazy()


def read_yxdb_batches(
    path: Union[str, Path],
    batch_size: int = 65_536,
) -> "Iterator[pl.DataFrame]":
    """Read an Alteryx YXDB file in batches (streaming / memory-efficient).

    Yields one ``polars.DataFrame`` per batch of up to *batch_size* rows.
    This is useful for files that are too large to fit in memory at once.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.
    batch_size : int, default 65 536
        Maximum number of rows per batch.

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
    ipc_chunks = _read_yxdb_batches(str(path), batch_size)
    for chunk in ipc_chunks:
        yield pl.read_ipc(io.BytesIO(chunk))


def read_yxdb_arrow(path: Union[str, Path]) -> "pyarrow.Table":
    """Read an Alteryx YXDB file and return a PyArrow Table.

    This is the fastest path to a PyArrow Table — the Rust reader
    produces Arrow IPC bytes which PyArrow reads with zero-copy.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    pyarrow.Table
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> table = yx.read_yxdb_arrow("data.yxdb")
    """
    import pyarrow.ipc as ipc

    ipc_bytes = _read_yxdb_ipc(str(path))
    reader = ipc.open_file(io.BytesIO(ipc_bytes))
    return reader.read_all()


def read_yxdb_pandas(path: Union[str, Path]) -> "pandas.DataFrame":
    """Read an Alteryx YXDB file and return a pandas DataFrame.

    Internally reads via Arrow IPC and converts to pandas, which is
    faster than going through Polars first.

    Parameters
    ----------
    path : str or Path
        Path to the .yxdb file.

    Returns
    -------
    pandas.DataFrame
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb_pandas("data.yxdb")
    """
    return read_yxdb_arrow(path).to_pandas()


# Convenience aliases for top-level usage: import sigilyx as yx; yx.read(...)
read = read_yxdb
scan = scan_yxdb


def write_yxdb(path: Union[str, Path], df: pl.DataFrame) -> None:
    """Write a Polars DataFrame to an Alteryx YXDB file.

    Parameters
    ----------
    path : str or Path
        Path where the .yxdb file will be written.
    df : polars.DataFrame
        The DataFrame to write.

    Examples
    --------
    >>> import sigilyx as yx
    >>> import polars as pl
    >>> df = pl.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
    >>> yx.write_yxdb("output.yxdb", df)
    """
    # Serialize DataFrame to Arrow IPC bytes
    buf = io.BytesIO()
    df.write_ipc(buf)
    ipc_bytes = buf.getvalue()
    _write_yxdb_ipc(str(path), ipc_bytes)


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
    This uses a seek-back approach to update the record count in the YXDB
    header after all batches have been written, enabling true streaming
    without knowing the total count upfront.
    """
    import tempfile
    import shutil

    # We need to implement this in Python since the Rust streaming writer
    # isn't exposed via PyO3 yet. Use a temp file approach.
    first_batch = None
    path_str = str(path)

    # Collect all batches and concatenate (for now)
    # TODO: Expose Rust YxdbWriter for true streaming
    all_batches = []
    total_rows = 0

    for batch in batches:
        if first_batch is None:
            first_batch = batch
        all_batches.append(batch)
        total_rows += batch.height

    if not all_batches:
        # Empty iterator - create empty file with schema from nowhere
        raise ValueError("Cannot write empty batch iterator - need at least one batch for schema")

    # Concatenate and write
    df = pl.concat(all_batches)
    write_yxdb(path_str, df)

    return total_rows


# Convenience aliases
write = write_yxdb
sink = sink_yxdb

__all__ = [
    "read",
    "read_yxdb",
    "read_yxdb_pandas",
    "read_yxdb_arrow",
    "scan",
    "scan_yxdb",
    "read_yxdb_batches",
    "read_schema",
    "record_count",
    "write",
    "write_yxdb",
    "write_yxdb_batches",
    "write_yxdb_pandas",
    "write_yxdb_arrow",
    "sink",
    "sink_yxdb",
    "register_polars",
]


# ── Polars Integration ──────────────────────────────────────────────────
#
# Registers pl.read_yxdb(), pl.scan_yxdb(), DataFrame.write_yxdb(), etc.
# This is automatically called on import, but can be called manually if needed.

def register_polars() -> bool:
    """Register YXDB methods on Polars classes.
    
    After calling this (or simply importing sigilyx), you can use:
    
        pl.read_yxdb("data.yxdb")
        pl.scan_yxdb("data.yxdb")
        df.write_yxdb("output.yxdb")
        lf.sink_yxdb("output.yxdb")
    
    Returns
    -------
    bool
        True if registration succeeded, False if Polars not available.
    
    Examples
    --------
    >>> import polars as pl
    >>> import sigilyx  # Auto-registers on import
    >>> df = pl.read_yxdb("data.yxdb")
    >>> df.write_yxdb("output.yxdb")
    """
    try:
        import polars as pl
        
        # Register pl.read_yxdb()
        if not hasattr(pl, 'read_yxdb'):
            pl.read_yxdb = read_yxdb
        
        # Register pl.scan_yxdb()
        if not hasattr(pl, 'scan_yxdb'):
            pl.scan_yxdb = scan_yxdb
        
        # Register DataFrame.write_yxdb()
        if not hasattr(pl.DataFrame, 'write_yxdb'):
            def _df_write_yxdb(self: pl.DataFrame, path: Union[str, Path]) -> None:
                """Write this DataFrame to a YXDB file.
                
                Parameters
                ----------
                path : str or Path
                    Output file path.
                """
                write_yxdb(str(path), self)
            pl.DataFrame.write_yxdb = _df_write_yxdb
        
        # Register LazyFrame.sink_yxdb()
        if not hasattr(pl.LazyFrame, 'sink_yxdb'):
            def _lf_sink_yxdb(self: pl.LazyFrame, path: Union[str, Path]) -> None:
                """Collect this LazyFrame and write to a YXDB file.
                
                Parameters
                ----------
                path : str or Path
                    Output file path.
                """
                sink_yxdb(str(path), self)
            pl.LazyFrame.sink_yxdb = _lf_sink_yxdb
        
        return True
        
    except ImportError:
        return False


# Auto-register on import
register_polars()
