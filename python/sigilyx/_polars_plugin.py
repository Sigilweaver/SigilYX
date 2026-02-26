"""Polars namespace plugins and integration registration for YXDB."""

from __future__ import annotations

import warnings
from pathlib import Path
from typing import Union

import polars as pl

from sigilyx._readers import read_yxdb, scan_yxdb
from sigilyx._writers import write_yxdb, sink_yxdb


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
            pl.read_yxdb = read_yxdb  # type: ignore[attr-defined]
        if not hasattr(pl, "scan_yxdb"):
            pl.scan_yxdb = scan_yxdb  # type: ignore[attr-defined]

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
