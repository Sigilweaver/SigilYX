---
sidebar_position: 8
description: "Read and write geospatial YXDB files with GeoArrow, GeoPandas, and WKB support."
---

# Spatial & GeoArrow

SigilYX can read and write YXDB files containing `SpatialObj` columns. The internal SHP geometry format is decoded to standard ISO Well-Known Binary (WKB), making data compatible with PostGIS, GDAL, Shapely, GeoPandas, and other geospatial tools.

## SpatialObj Handling Modes

Every read function that touches spatial data accepts a `spatial` parameter:

| Mode | Behavior |
| --- | --- |
| `"wkb"` (default) | Decode SHP geometry to ISO WKB |
| `"raw"` | Keep the raw SHP bytes as-is |

```python
import sigilyx as yx

# Default: SHP → WKB conversion
df = yx.read_yxdb("parcels.yxdb")

# Keep raw SHP bytes (expert/debug use)
df = yx.read_yxdb("parcels.yxdb", spatial="raw")
```

## Spatial Metadata

Inspect which columns are spatial and whether a spatial index exists, without reading any row data:

```python
import sigilyx as yx

info = yx.read_spatial_info("parcels.yxdb")
print(info)
# {
#     "has_spatial_index": True,
#     "spatial_index_pos": 123456,
#     "file_id": 21,
#     "spatial_columns": ["SpatialObj"]
# }
```

## GeoArrow Output

`read_yxdb_geoarrow()` returns a PyArrow Table where spatial columns are tagged with `ARROW:extension:name = "geoarrow.wkb"`. This makes the table compatible with GeoArrow-aware tools (lonboard, leafmap, DuckDB Spatial, etc.).

```bash
pip install sigilyx[arrow]
```

```python
import sigilyx as yx

table = yx.read_yxdb_geoarrow("parcels.yxdb")

# Spatial columns now have GeoArrow extension metadata
field = table.schema.field("SpatialObj")
print(field.metadata)
# {b'ARROW:extension:name': b'geoarrow.wkb', ...}
```

With column projection:

```python
table = yx.read_yxdb_geoarrow("parcels.yxdb", columns=["Id", "SpatialObj"])
```

## GeoPandas Integration

`read_yxdb_geo()` reads a YXDB file directly into a GeoPandas `GeoDataFrame`. Spatial columns are decoded from SHP to WKB, then converted to Shapely geometry objects.

```bash
pip install geopandas shapely
```

```python
import sigilyx as yx

gdf = yx.read_yxdb_geo("parcels.yxdb")
print(type(gdf))  # <class 'geopandas.GeoDataFrame'>
gdf.plot()
```

### Column Projection

```python
gdf = yx.read_yxdb_geo("parcels.yxdb", columns=["Id", "Name", "SpatialObj"])
```

### Choosing the Geometry Column

If a file has multiple `SpatialObj` columns, the first one is used as the active geometry by default. Override with `geometry_column`:

```python
gdf = yx.read_yxdb_geo("multi_spatial.yxdb", geometry_column="Boundary")
```

### Writing GeoPandas to YXDB

```python
import sigilyx as yx
import geopandas as gpd

gdf = gpd.read_file("parcels.shp")
yx.write_yxdb_geo("parcels.yxdb", gdf)
```

Auto-detects geometry columns. To force specific columns:

```python
yx.write_yxdb_geo("parcels.yxdb", gdf, spatial_columns=["geometry"])
```

## Low-Level Geometry Conversion

Convert individual geometry values between SHP and WKB formats:

```python
import sigilyx as yx

# SHP → WKB (returns None for null shapes)
wkb = yx.shp_to_wkb(shp_bytes)

# WKB → SHP
shp = yx.wkb_to_shp(wkb_bytes)
```

These are useful when processing spatial data row by row or building custom pipelines.

## Common Patterns

### YXDB to GeoJSON

```python
import sigilyx as yx

gdf = yx.read_yxdb_geo("parcels.yxdb")
gdf.to_file("parcels.geojson", driver="GeoJSON")
```

### YXDB to PostGIS

```python
import sigilyx as yx
from sqlalchemy import create_engine

engine = create_engine("postgresql://user:pass@localhost/db")
gdf = yx.read_yxdb_geo("parcels.yxdb")
gdf.to_postgis("parcels", engine, if_exists="replace")
```

### DuckDB Spatial Query via GeoArrow

```python
import sigilyx as yx
import duckdb

table = yx.read_yxdb_geoarrow("parcels.yxdb")

result = duckdb.sql("""
    SELECT Id, ST_Area(ST_GeomFromWKB(SpatialObj)) as area
    FROM table
    ORDER BY area DESC
    LIMIT 10
""").arrow()
```

### Shapefile to YXDB

```python
import sigilyx as yx
import geopandas as gpd

gdf = gpd.read_file("boundaries.shp")
yx.write_yxdb_geo("boundaries.yxdb", gdf)
```
