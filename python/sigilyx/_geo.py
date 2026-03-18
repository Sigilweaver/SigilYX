"""Spatial/GeoArrow/GeoPandas integration for YXDB files."""

from __future__ import annotations

from pathlib import Path
from typing import Union, TYPE_CHECKING

import polars as pl

from sigilyx.sigilyx import (
    shp_to_wkb_py as _shp_to_wkb,
    wkb_to_shp_py as _wkb_to_shp,
    read_yxdb_spatial_info as _read_yxdb_spatial_info,
)

from sigilyx._readers import read_yxdb, read_yxdb_columns
from sigilyx._writers import write_yxdb

if TYPE_CHECKING:
    import geopandas
    import pyarrow


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


def _apply_geoarrow_metadata(
    table: pyarrow.Table,
    spatial_columns: list[str],
) -> pyarrow.Table:
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
    allow_unverified_e2_types: bool = False,
) -> pyarrow.Table:
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
    allow_unverified_e2_types : bool, default False
        If ``True``, attempt to read E2 files with unverified field types
        (see :func:`read_yxdb`).

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
        df = read_yxdb_columns(
            path, columns, spatial="wkb",
            allow_unverified_e2_types=allow_unverified_e2_types,
        )
    else:
        df = read_yxdb(
            path, spatial="wkb",
            allow_unverified_e2_types=allow_unverified_e2_types,
        )

    table = df.to_arrow()
    return _apply_geoarrow_metadata(table, spatial_cols)


def read_yxdb_geo(
    path: Union[str, Path],
    *,
    columns: list[str] | None = None,
    geometry_column: str | None = None,
    allow_unverified_e2_types: bool = False,
) -> geopandas.GeoDataFrame:
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
        from shapely import from_wkb  # noqa: F401
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
        df = read_yxdb_columns(
            path, columns, spatial="wkb",
            allow_unverified_e2_types=allow_unverified_e2_types,
        )
        # Filter spatial columns to only those actually requested
        spatial_cols = [c for c in spatial_cols if c in columns]
    else:
        df = read_yxdb(
            path, spatial="wkb",
            allow_unverified_e2_types=allow_unverified_e2_types,
        )

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
    gdf: geopandas.GeoDataFrame,
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
