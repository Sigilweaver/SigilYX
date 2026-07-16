---
sidebar_position: 11
description: "Complete reference for every public class and function in the sigilyx Python package."
---

# API Reference

Complete reference for the `sigilyx` Python package's public surface. This page lists every name exported from `sigilyx.__all__` with its signature, parameters, and return type. For narrative walkthroughs, see the guide pages linked from each section.

All of the functions below are also reachable as `yx.<name>` after `import sigilyx as yx`.

## Read Functions

| Function | Returns | Description |
| --- | --- | --- |
| `read_yxdb(path, *, spatial="wkb", allow_unverified_e2_types=False)` | `polars.DataFrame` | Read a full YXDB file. See [Polars](/python/polars). |
| `read_yxdb_columns(path, columns, *, spatial="wkb", allow_unverified_e2_types=False)` | `polars.DataFrame` | Read only the given columns. |
| `read_yxdb_arrow(path, *, spatial="wkb", allow_unverified_e2_types=False)` | `pyarrow.Table` | Read into a PyArrow Table. See [PyArrow](/python/pyarrow). |
| `read_yxdb_pandas(path, *, spatial="wkb", allow_unverified_e2_types=False)` | `pandas.DataFrame` | Read into a pandas DataFrame. See [Pandas](/python/pandas). |
| `scan_yxdb(path)` | `polars.LazyFrame` | Lazy scan with projection and n_rows pushdown. See [Lazy Scan](/python/lazy-scan). |
| `read_yxdb_batches(path, batch_size=65536, *, columns=None, n_rows=None)` | `Iterator[polars.DataFrame]` | Streaming batched reads. See [Streaming](/python/streaming). |
| `read_schema(path)` | `list[dict]` | Field metadata as raw dicts (`name`, `type`, `size`, `scale`). |
| `read_yxdb_fields(path)` | `list[FieldInfo]` | Field metadata as [`FieldInfo`](#fieldinfo) objects. See [Metadata](/python/metadata). |
| `record_count(path)` | `int` | Number of records, from the header only. |

Common parameters:

| Parameter | Type | Description |
| --- | --- | --- |
| `path` | `str \| Path` | Path to the `.yxdb` file. |
| `spatial` | `str` | `"wkb"` (decode `SpatialObj` to ISO WKB) or `"raw"` (keep raw SHP bytes). |
| `allow_unverified_e2_types` | `bool` | Allow E2 field types whose decoders are unverified against real data (Time, WString, Blob, SpatialObj). |

Module-level aliases: `read = read_yxdb`, `scan = scan_yxdb`.

## Row Reader

### `YxdbRowReader`

Cursor-style row-by-row reader. Implements the context manager and iterator protocols. See [Row Reader](/python/row-reader).

| Member | Returns | Description |
| --- | --- | --- |
| `YxdbRowReader(path)` | - | Open a reader over `path`. |
| `.next()` | `bool` | Advance to the next record; `False` when exhausted. |
| `.read_index(index)` | value | Read a field by 0-based column index. |
| `.read_name(name)` | value | Read a field by column name. |
| `.read_all()` | `tuple` | All field values from the current record. |
| `.read_dict()` | `dict` | All field values as `{name: value}`. |
| `.fields` | `list[FieldInfo]` | Field metadata (property). |
| `.num_records` | `int` | Total records in the file, from the header (property). |
| `.close()` | `None` | Release resources. |

## Write Functions

| Function | Returns | Description |
| --- | --- | --- |
| `write_yxdb(path, df, *, spatial_columns=None)` | `None` | Write a `polars.DataFrame`. See [Writing](/python/writing). |
| `write_yxdb_with_overrides(path, df, type_overrides, *, spatial_columns=None)` | `None` | Write with explicit per-column YXDB type overrides. |
| `write_yxdb_pandas(path, df)` | `None` | Write a `pandas.DataFrame`. |
| `write_yxdb_arrow(path, table)` | `None` | Write a `pyarrow.Table`. |
| `sink_yxdb(path, lf)` | `None` | Collect a `polars.LazyFrame` (streaming engine when available) and write it. |
| `write_yxdb_batches(path, batches)` | `int` | Write an iterator of same-schema `polars.DataFrame` batches; returns total records written. |

`type_overrides` maps a column name to a dict with `type` (one of `String`, `WString`, `V_String`, `V_WString`, `Bool`, `Byte`, `Int16`, `Int32`, `Int64`, `Float`, `Double`, `FixedDecimal`, `Date`, `Time`, `DateTime`, `Blob`, `SpatialObj`), and optional `size` / `scale`.

`spatial_columns` names Binary columns holding WKB geometry to be written as `SpatialObj` fields.

Module-level alias: `write = write_yxdb`, `sink = sink_yxdb`.

## Spatial

| Function | Returns | Description |
| --- | --- | --- |
| `shp_to_wkb(shp)` | `bytes \| None` | Convert raw SHP geometry bytes to ISO WKB (`None` for null shapes). |
| `wkb_to_shp(wkb)` | `bytes` | Convert ISO WKB geometry bytes to SHP format. |
| `read_spatial_info(path)` | `dict` | Header spatial metadata: `has_spatial_index`, `spatial_index_pos`, `file_id`, `spatial_columns`. |
| `read_yxdb_geoarrow(path, *, columns=None, allow_unverified_e2_types=False)` | `pyarrow.Table` | Read with `SpatialObj` columns tagged as `geoarrow.wkb` extension type. |
| `read_yxdb_geo(path, *, columns=None, geometry_column=None, allow_unverified_e2_types=False)` | `geopandas.GeoDataFrame` | Read with the first (or named) `SpatialObj` column set as active geometry. Requires `geopandas`/`shapely`. |
| `write_yxdb_geo(path, gdf, *, spatial_columns=None)` | `None` | Write a `GeoDataFrame`, converting geometry columns to `SpatialObj`. Requires `geopandas`. |

See [Spatial & GeoArrow](/python/spatial) for a full walkthrough.

## Polars Integration

Importing `sigilyx` auto-registers Polars integration; see [Polars](/python/polars).

| Member | Description |
| --- | --- |
| `register_polars()` | Registers `pl.read_yxdb` / `pl.scan_yxdb` top-level aliases and the `df.yxdb` / `lf.yxdb` namespaces. Returns `bool` (`True` if Polars is available). Called automatically on import. |
| `YxdbDataFrameNamespace.write(path)` | Accessed as `df.yxdb.write(path)`. Writes the DataFrame to a YXDB file. |
| `YxdbLazyFrameNamespace.sink(path)` | Accessed as `lf.yxdb.sink(path)`. Collects the LazyFrame and writes it to a YXDB file. |

`DataFrame.write_yxdb()` and `LazyFrame.sink_yxdb()` also exist for backward compatibility but are deprecated in favor of the `.yxdb` namespace.

## Types

### `FieldInfo`

Metadata for a single field (column) in a YXDB file.

| Attribute | Type | Description |
| --- | --- | --- |
| `name` | `str` | Column name. |
| `field_type` | `str` | YXDB field type (e.g. `"Int32"`, `"V_WString"`, `"Date"`). |
| `size` | `int` | Declared size (max chars for strings, precision for decimals). |
| `scale` | `int` | Scale (decimal places for `FixedDecimal`, 0 otherwise). |

## Module Attributes

| Attribute | Type | Description |
| --- | --- | --- |
| `__version__` | `str` | Installed `sigilyx` package version (`"0.0.0-dev"` if not installed). |
