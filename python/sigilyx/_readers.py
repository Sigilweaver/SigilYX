"""YXDB read functions, row/batch readers, and lazy scan support."""

from __future__ import annotations

from pathlib import Path
from typing import Iterator, Union, TYPE_CHECKING

import polars as pl

from sigilyx.sigilyx import (
    read_yxdb_df as _read_yxdb_df,
    read_yxdb_df_columns as _read_yxdb_df_columns,
    read_yxdb_schema as _read_yxdb_schema,
    read_yxdb_record_count as _read_yxdb_record_count,
    _YxdbRowReader as _YxdbRowReaderRust,
    _YxdbBatchReader as _YxdbBatchReaderRust,
)

from sigilyx._types import FieldInfo, _yxdb_schema_to_polars

if TYPE_CHECKING:
    import pyarrow


def read_yxdb(
    path: Union[str, Path],
    *,
    spatial: str = "wkb",
    allow_unverified_e2_types: bool = False,
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
    allow_unverified_e2_types : bool, default False
        If ``True``, attempt to read E2 files that contain field types
        whose decoders have never been verified against real data
        (Time, WString, Blob, SpatialObj). The decoders are speculative
        and may produce incorrect results.

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
    return _read_yxdb_df(
        str(path),
        spatial=spatial,
        allow_unverified_e2_types=allow_unverified_e2_types,
    )


def read_yxdb_columns(
    path: Union[str, Path],
    columns: list[str],
    *,
    spatial: str = "wkb",
    allow_unverified_e2_types: bool = False,
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
    allow_unverified_e2_types : bool, default False
        If ``True``, attempt to read E2 files with unverified field types
        (see :func:`read_yxdb`).

    Returns
    -------
    polars.DataFrame
        A DataFrame containing only the requested columns.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb_columns("data.yxdb", ["Id", "Name"])
    """
    return _read_yxdb_df_columns(
        str(path),
        columns,
        spatial=spatial,
        allow_unverified_e2_types=allow_unverified_e2_types,
    )


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


def read_yxdb_fields(path: Union[str, Path]) -> list[FieldInfo]:
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
        with_columns: list[str] | None,
        predicate: pl.Expr | None,
        n_rows: int | None,
        batch_size: int | None,
    ) -> Iterator[pl.DataFrame]:
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
    columns: list[str] | None = None,
    n_rows: int | None = None,
) -> Iterator[pl.DataFrame]:
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
    allow_unverified_e2_types: bool = False,
) -> pyarrow.Table:
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
    allow_unverified_e2_types : bool, default False
        If ``True``, attempt to read E2 files with unverified field types
        (see :func:`read_yxdb`).

    Returns
    -------
    pyarrow.Table
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> table = yx.read_yxdb_arrow("data.yxdb")
    """
    return read_yxdb(
        path, spatial=spatial, allow_unverified_e2_types=allow_unverified_e2_types
    ).to_arrow()


def read_yxdb_pandas(
    path: Union[str, Path],
    *,
    spatial: str = "wkb",
    allow_unverified_e2_types: bool = False,
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
    allow_unverified_e2_types : bool, default False
        If ``True``, attempt to read E2 files with unverified field types
        (see :func:`read_yxdb`).

    Returns
    -------
    pandas.DataFrame
        The contents of the YXDB file.

    Examples
    --------
    >>> import sigilyx as yx
    >>> df = yx.read_yxdb_pandas("data.yxdb")
    """
    return read_yxdb_arrow(
        path, spatial=spatial, allow_unverified_e2_types=allow_unverified_e2_types
    ).to_pandas()
