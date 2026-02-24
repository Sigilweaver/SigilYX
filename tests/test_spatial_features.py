"""Tests for spatial features: GeoArrow, GeoPandas/Shapely, spatial index info.

Covers:
- read_spatial_info() on real and synthetic YXDB files
- read_yxdb_geoarrow() with GeoArrow metadata verification
- read_yxdb_geo() / write_yxdb_geo() roundtrip with GeoPandas
- _apply_geoarrow_metadata() internal helper
- spatial="geoarrow" mode via read_yxdb()
- shp_to_wkb / wkb_to_shp low-level utilities
- Edge cases: nulls, multiple spatial columns, no spatial columns
"""

import struct
import tempfile
from pathlib import Path

import polars as pl
import pyarrow as pa
import pytest

import sigilyx

TEST_DIR = Path(__file__).parent.parent / "sigilyx" / "test_files"


def _yxdb(name: str) -> str:
    return str(TEST_DIR / name)


# ── Helpers to build WKB geometries ────────────────────────────────────


def _wkb_point(x: float, y: float) -> bytes:
    """Build a little-endian WKB Point."""
    return struct.pack("<BIdd", 1, 1, x, y)


def _wkb_linestring(coords: list[tuple[float, float]]) -> bytes:
    """Build a little-endian WKB LineString."""
    buf = struct.pack("<BI", 1, 2)  # LE, type=LineString
    buf += struct.pack("<I", len(coords))
    for x, y in coords:
        buf += struct.pack("<dd", x, y)
    return buf


def _wkb_polygon(rings: list[list[tuple[float, float]]]) -> bytes:
    """Build a little-endian WKB Polygon."""
    buf = struct.pack("<BI", 1, 3)  # LE, type=Polygon
    buf += struct.pack("<I", len(rings))
    for ring in rings:
        buf += struct.pack("<I", len(ring))
        for x, y in ring:
            buf += struct.pack("<dd", x, y)
    return buf


def _wkb_multipoint(points: list[tuple[float, float]]) -> bytes:
    """Build a little-endian WKB MultiPoint."""
    buf = struct.pack("<BI", 1, 4)  # LE, type=MultiPoint
    buf += struct.pack("<I", len(points))
    for x, y in points:
        buf += _wkb_point(x, y)
    return buf


def _write_spatial_yxdb(
    path: str,
    geom_col: str = "geom",
    geom_data: list = None,
    extra_cols: dict = None,
    spatial_cols: list[str] | None = None,
):
    """Helper: write a YXDB with a spatial column."""
    if geom_data is None:
        geom_data = [_wkb_point(1.0, 2.0), _wkb_point(3.0, 4.0)]

    cols = {}
    if extra_cols:
        cols.update(extra_cols)
    cols[geom_col] = geom_data

    df = pl.DataFrame(cols)
    spatial = spatial_cols or [geom_col]
    sigilyx.write_yxdb(path, df, spatial_columns=spatial)


# ── read_spatial_info() ────────────────────────────────────────────────


class TestReadSpatialInfo:
    """Test read_spatial_info() on existing and synthetic files."""

    def test_existing_non_spatial_files(self):
        """Files without SpatialObj columns should report no spatial info."""
        for name in ["People.yxdb", "AllTypes.yxdb", "Strings.yxdb"]:
            info = sigilyx.read_spatial_info(_yxdb(name))
            assert isinstance(info, dict)
            assert info["spatial_columns"] == [], f"unexpected spatial in {name}"
            assert isinstance(info["has_spatial_index"], bool)
            assert isinstance(info["spatial_index_pos"], int)
            assert isinstance(info["file_id"], int)

    def test_synthetic_spatial_file(self):
        """A file we write with spatial columns should report them."""
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            path = f.name
        _write_spatial_yxdb(path, geom_col="location")
        info = sigilyx.read_spatial_info(path)
        assert info["spatial_columns"] == ["location"]
        # Written files don't have a spatial index
        assert info["has_spatial_index"] is False
        assert info["spatial_index_pos"] == 0

    def test_multiple_spatial_columns(self):
        """File with two spatial columns should list both."""
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            path = f.name
        pt1 = _wkb_point(1, 2)
        pt2 = _wkb_point(10, 20)
        df = pl.DataFrame({
            "id": [1],
            "origin": [pt1],
            "dest": [pt2],
        })
        sigilyx.write_yxdb(path, df, spatial_columns=["origin", "dest"])
        info = sigilyx.read_spatial_info(path)
        assert info["spatial_columns"] == ["origin", "dest"]

    def test_file_id_is_integer(self):
        """file_id should be a positive integer."""
        info = sigilyx.read_spatial_info(_yxdb("People.yxdb"))
        assert info["file_id"] > 0


# ── read_yxdb_geoarrow() ──────────────────────────────────────────────


class TestReadYxdbGeoArrow:
    """Test read_yxdb_geoarrow() returns valid GeoArrow-annotated tables."""

    @pytest.fixture
    def spatial_file(self, tmp_path):
        path = str(tmp_path / "geo.yxdb")
        _write_spatial_yxdb(
            path,
            geom_col="geom",
            geom_data=[_wkb_point(-73.9857, 40.7484), _wkb_point(2.3522, 48.8566)],
            extra_cols={"name": ["New York", "Paris"]},
        )
        return path

    def test_returns_pyarrow_table(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        assert isinstance(table, pa.Table)

    def test_shape(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        assert table.num_rows == 2
        assert table.num_columns == 2

    def test_geoarrow_metadata(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        geom_field = table.schema.field("geom")
        assert geom_field.metadata is not None
        assert geom_field.metadata[b"ARROW:extension:name"] == b"geoarrow.wkb"

    def test_non_spatial_column_no_metadata(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        name_field = table.schema.field("name")
        # Non-spatial columns should NOT have geoarrow metadata
        meta = name_field.metadata or {}
        assert b"ARROW:extension:name" not in meta

    def test_wkb_data_is_valid(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        geom_col = table.column("geom")
        wkb_data = geom_col[0].as_py()
        assert isinstance(wkb_data, bytes)
        # Check it's a WKB point (LE byte order, type=1)
        assert wkb_data[0] == 1  # little-endian
        wkb_type = struct.unpack_from("<I", wkb_data, 1)[0]
        assert wkb_type == 1  # Point

    def test_coords_preserved(self, spatial_file):
        table = sigilyx.read_yxdb_geoarrow(spatial_file)
        wkb = table.column("geom")[0].as_py()
        x, y = struct.unpack_from("<dd", wkb, 5)
        assert abs(x - (-73.9857)) < 1e-10
        assert abs(y - 40.7484) < 1e-10

    def test_columns_filter(self, spatial_file):
        """columns= should work while still applying geoarrow metadata."""
        table = sigilyx.read_yxdb_geoarrow(spatial_file, columns=["geom"])
        assert table.num_columns == 1
        assert table.schema.field("geom").metadata[b"ARROW:extension:name"] == b"geoarrow.wkb"

    def test_non_spatial_file(self):
        """Non-spatial files should return a table with no geoarrow metadata."""
        table = sigilyx.read_yxdb_geoarrow(_yxdb("People.yxdb"))
        for field in table.schema:
            meta = field.metadata or {}
            assert b"ARROW:extension:name" not in meta


# ── _apply_geoarrow_metadata() ────────────────────────────────────────


class TestApplyGeoArrowMetadata:
    """Test the internal _apply_geoarrow_metadata helper."""

    def test_adds_metadata(self):
        table = pa.table({"geom": pa.array([b"\x01\x00\x00\x00"], type=pa.binary())})
        result = sigilyx._apply_geoarrow_metadata(table, ["geom"])
        meta = result.schema.field("geom").metadata
        assert meta[b"ARROW:extension:name"] == b"geoarrow.wkb"
        assert meta[b"ARROW:extension:metadata"] == b"{}"

    def test_no_change_when_no_spatial_cols(self):
        table = pa.table({"x": [1, 2, 3]})
        result = sigilyx._apply_geoarrow_metadata(table, [])
        assert result.schema == table.schema

    def test_skips_non_binary_columns(self):
        """Should not annotate string columns even if named as spatial."""
        table = pa.table({"geom": ["a", "b"]})
        result = sigilyx._apply_geoarrow_metadata(table, ["geom"])
        meta = result.schema.field("geom").metadata or {}
        assert b"ARROW:extension:name" not in meta

    def test_preserves_existing_metadata(self):
        field = pa.field("geom", pa.binary(), metadata={b"custom": b"value"})
        table = pa.table({"geom": pa.array([b"\x01"], type=pa.binary())},
                         schema=pa.schema([field]))
        result = sigilyx._apply_geoarrow_metadata(table, ["geom"])
        meta = result.schema.field("geom").metadata
        assert meta[b"custom"] == b"value"
        assert meta[b"ARROW:extension:name"] == b"geoarrow.wkb"


# ── read_yxdb() with spatial="geoarrow" ───────────────────────────────


class TestReadYxdbGeoArrowMode:
    """Test passing spatial='geoarrow' to read_yxdb()."""

    @pytest.fixture
    def spatial_file(self, tmp_path):
        path = str(tmp_path / "geo.yxdb")
        _write_spatial_yxdb(path)
        return path

    def test_geoarrow_mode_returns_dataframe(self, spatial_file):
        df = sigilyx.read_yxdb(spatial_file, spatial="geoarrow")
        assert isinstance(df, pl.DataFrame)
        assert df.height == 2

    def test_geoarrow_mode_wkb_data(self, spatial_file):
        """GeoArrow mode at Polars level produces WKB binary data."""
        df = sigilyx.read_yxdb(spatial_file, spatial="geoarrow")
        geom = df["geom"][0]
        assert isinstance(geom, bytes)
        wkb_type = struct.unpack_from("<I", geom, 1)[0]
        assert wkb_type == 1  # Point

    def test_geoarrow_matches_wkb(self, spatial_file):
        df_wkb = sigilyx.read_yxdb(spatial_file, spatial="wkb")
        df_geo = sigilyx.read_yxdb(spatial_file, spatial="geoarrow")
        assert df_wkb["geom"].to_list() == df_geo["geom"].to_list()


# ── read_yxdb_geo() with GeoPandas ────────────────────────────────────


class TestReadYxdbGeo:
    """Test read_yxdb_geo() with GeoPandas + Shapely."""

    @pytest.fixture
    def spatial_file(self, tmp_path):
        path = str(tmp_path / "geo.yxdb")
        _write_spatial_yxdb(
            path,
            geom_data=[_wkb_point(-73.9857, 40.7484), _wkb_point(2.3522, 48.8566)],
            extra_cols={"name": ["New York", "Paris"]},
        )
        return path

    def test_returns_geodataframe(self, spatial_file):
        import geopandas as gpd
        gdf = sigilyx.read_yxdb_geo(spatial_file)
        assert isinstance(gdf, gpd.GeoDataFrame)

    def test_geometry_column_set(self, spatial_file):
        gdf = sigilyx.read_yxdb_geo(spatial_file)
        assert gdf.geometry.name == "geom"

    def test_geometry_values(self, spatial_file):
        from shapely.geometry import Point
        gdf = sigilyx.read_yxdb_geo(spatial_file)
        pt = gdf.geometry.iloc[0]
        assert isinstance(pt, Point)
        assert abs(pt.x - (-73.9857)) < 1e-4
        assert abs(pt.y - 40.7484) < 1e-4

    def test_non_geometry_columns(self, spatial_file):
        gdf = sigilyx.read_yxdb_geo(spatial_file)
        assert "name" in gdf.columns
        assert gdf["name"].tolist() == ["New York", "Paris"]

    def test_row_count(self, spatial_file):
        gdf = sigilyx.read_yxdb_geo(spatial_file)
        assert len(gdf) == 2

    def test_columns_filter(self, spatial_file):
        gdf = sigilyx.read_yxdb_geo(spatial_file, columns=["geom", "name"])
        assert set(gdf.columns) == {"geom", "name"}

    def test_non_spatial_file_raises_valueerror(self):
        with pytest.raises(ValueError, match="No SpatialObj columns"):
            sigilyx.read_yxdb_geo(_yxdb("People.yxdb"))

    def test_custom_geometry_column(self, tmp_path):
        path = str(tmp_path / "multi_geo.yxdb")
        df = pl.DataFrame({
            "id": [1],
            "origin": [_wkb_point(0, 0)],
            "dest": [_wkb_point(10, 20)],
        })
        sigilyx.write_yxdb(path, df, spatial_columns=["origin", "dest"])

        gdf = sigilyx.read_yxdb_geo(path, geometry_column="dest")
        assert gdf.geometry.name == "dest"
        pt = gdf.geometry.iloc[0]
        assert abs(pt.x - 10.0) < 1e-10


# ── write_yxdb_geo() ──────────────────────────────────────────────────


class TestWriteYxdbGeo:
    """Test write_yxdb_geo() roundtrip with GeoPandas."""

    def test_roundtrip_points(self, tmp_path):
        import geopandas as gpd
        from shapely.geometry import Point

        gdf = gpd.GeoDataFrame(
            {"name": ["A", "B"]},
            geometry=[Point(1, 2), Point(3, 4)],
            crs=None,
        )
        path = str(tmp_path / "out.yxdb")
        sigilyx.write_yxdb_geo(path, gdf)

        # Read back
        gdf2 = sigilyx.read_yxdb_geo(path)
        assert len(gdf2) == 2
        assert abs(gdf2.geometry.iloc[0].x - 1.0) < 1e-10
        assert abs(gdf2.geometry.iloc[1].y - 4.0) < 1e-10
        assert gdf2["name"].tolist() == ["A", "B"]

    def test_roundtrip_linestrings(self, tmp_path):
        import geopandas as gpd
        from shapely.geometry import LineString

        gdf = gpd.GeoDataFrame(
            {"id": [1]},
            geometry=[LineString([(0, 0), (1, 1), (2, 0)])],
        )
        path = str(tmp_path / "lines.yxdb")
        sigilyx.write_yxdb_geo(path, gdf)

        gdf2 = sigilyx.read_yxdb_geo(path)
        assert len(gdf2) == 1
        # SHP roundtrip promotes LineString to MultiLineString
        geom = gdf2.geometry.iloc[0]
        assert geom.geom_type in ("LineString", "MultiLineString")

    def test_roundtrip_polygons(self, tmp_path):
        import geopandas as gpd
        from shapely.geometry import Polygon

        gdf = gpd.GeoDataFrame(
            {"id": [1]},
            geometry=[Polygon([(0, 0), (4, 0), (2, 3), (0, 0)])],
        )
        path = str(tmp_path / "polygons.yxdb")
        sigilyx.write_yxdb_geo(path, gdf)

        gdf2 = sigilyx.read_yxdb_geo(path)
        assert len(gdf2) == 1
        geom = gdf2.geometry.iloc[0]
        assert geom.geom_type in ("Polygon", "MultiPolygon")

    def test_roundtrip_with_none_geometry(self, tmp_path):
        import geopandas as gpd
        from shapely.geometry import Point

        gdf = gpd.GeoDataFrame(
            {"name": ["A", "B", "C"]},
            geometry=[Point(1, 2), None, Point(3, 4)],
        )
        path = str(tmp_path / "nulls.yxdb")
        sigilyx.write_yxdb_geo(path, gdf)

        gdf2 = sigilyx.read_yxdb_geo(path)
        assert len(gdf2) == 3
        assert gdf2.geometry.iloc[0] is not None
        assert gdf2.geometry.iloc[1] is None
        assert gdf2.geometry.iloc[2] is not None

    def test_auto_detect_geometry_columns(self, tmp_path):
        """When spatial_columns=None, auto-detect GeometryDtype columns."""
        import geopandas as gpd
        from shapely.geometry import Point

        gdf = gpd.GeoDataFrame(
            {"name": ["A"]},
            geometry=[Point(1, 2)],
        )
        path = str(tmp_path / "auto.yxdb")
        sigilyx.write_yxdb_geo(path, gdf)

        info = sigilyx.read_spatial_info(path)
        assert "geometry" in info["spatial_columns"]


# ── shp_to_wkb / wkb_to_shp low-level ─────────────────────────────────


class TestShpWkbConversion:
    """Test the low-level shp_to_wkb and wkb_to_shp functions."""

    def test_point_shp_to_wkb(self):
        """SHP Point → WKB Point."""
        shp = struct.pack("<i", 1)  # SHP_POINT
        shp += struct.pack("<dd", 42.0, 24.0)
        wkb = sigilyx.shp_to_wkb(shp)
        assert wkb[0] == 1  # LE
        wkb_type = struct.unpack_from("<I", wkb, 1)[0]
        assert wkb_type == 1  # WKB_POINT
        x, y = struct.unpack_from("<dd", wkb, 5)
        assert abs(x - 42.0) < 1e-10
        assert abs(y - 24.0) < 1e-10

    def test_wkb_to_shp_point(self):
        """WKB Point → SHP Point."""
        wkb = _wkb_point(42.0, 24.0)
        shp = sigilyx.wkb_to_shp(wkb)
        shape_type = struct.unpack_from("<i", shp, 0)[0]
        assert shape_type == 1  # SHP_POINT
        x, y = struct.unpack_from("<dd", shp, 4)
        assert abs(x - 42.0) < 1e-10
        assert abs(y - 24.0) < 1e-10

    def test_point_roundtrip(self):
        """WKB → SHP → WKB should preserve coordinates."""
        original_wkb = _wkb_point(-122.4194, 37.7749)
        shp = sigilyx.wkb_to_shp(original_wkb)
        back_wkb = sigilyx.shp_to_wkb(shp)
        x_orig, y_orig = struct.unpack_from("<dd", original_wkb, 5)
        x_back, y_back = struct.unpack_from("<dd", back_wkb, 5)
        assert abs(x_orig - x_back) < 1e-10
        assert abs(y_orig - y_back) < 1e-10

    def test_linestring_roundtrip(self):
        """WKB LineString → SHP Polyline → WKB (has coordinates)."""
        wkb = _wkb_linestring([(0, 0), (1, 1), (2, 0)])
        shp = sigilyx.wkb_to_shp(wkb)
        wkb2 = sigilyx.shp_to_wkb(shp)
        assert isinstance(wkb2, bytes)
        assert len(wkb2) > 5

    def test_polygon_roundtrip(self):
        """WKB Polygon → SHP Polygon → WKB (has coordinates)."""
        ring = [(0.0, 0.0), (4.0, 0.0), (2.0, 3.0), (0.0, 0.0)]
        wkb = _wkb_polygon([ring])
        shp = sigilyx.wkb_to_shp(wkb)
        wkb2 = sigilyx.shp_to_wkb(shp)
        assert isinstance(wkb2, bytes)
        assert len(wkb2) > 5


# ── Spatial write/read roundtrip via Polars ────────────────────────────


class TestSpatialPolarsRoundtrip:
    """Test writing with spatial_columns= and reading back."""

    def test_point_roundtrip_wkb(self, tmp_path):
        path = str(tmp_path / "pt.yxdb")
        pt = _wkb_point(-73.9857, 40.7484)
        df = pl.DataFrame({"id": [1, 2], "geom": [pt, pt]})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        df2 = sigilyx.read_yxdb(path, spatial="wkb")
        assert df2.height == 2
        wkb = df2["geom"][0]
        x, y = struct.unpack_from("<dd", wkb, 5)
        assert abs(x - (-73.9857)) < 1e-10
        assert abs(y - 40.7484) < 1e-10

    def test_raw_mode_returns_shp(self, tmp_path):
        path = str(tmp_path / "raw.yxdb")
        pt = _wkb_point(5.0, 10.0)
        df = pl.DataFrame({"geom": [pt]})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        df2 = sigilyx.read_yxdb(path, spatial="raw")
        raw = df2["geom"][0]
        shape_type = struct.unpack_from("<i", raw, 0)[0]
        assert shape_type == 1  # SHP Point

    def test_null_geometry(self, tmp_path):
        path = str(tmp_path / "null.yxdb")
        pt = _wkb_point(1.0, 2.0)
        df = pl.DataFrame({"geom": [pt, None, pt]})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        df2 = sigilyx.read_yxdb(path, spatial="wkb")
        assert df2["geom"][0] is not None
        assert df2["geom"][1] is None
        assert df2["geom"][2] is not None

    def test_many_points(self, tmp_path):
        """Stress test with 1000 rows."""
        path = str(tmp_path / "many.yxdb")
        pts = [_wkb_point(i * 0.01, i * 0.02) for i in range(1000)]
        df = pl.DataFrame({"geom": pts})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        df2 = sigilyx.read_yxdb(path, spatial="wkb")
        assert df2.height == 1000

        # Spot-check a few coordinates
        for idx in [0, 500, 999]:
            wkb = df2["geom"][idx]
            x, y = struct.unpack_from("<dd", wkb, 5)
            assert abs(x - idx * 0.01) < 1e-10
            assert abs(y - idx * 0.02) < 1e-10

    def test_all_spatial_modes_same_shape(self, tmp_path):
        """All three modes produce the same DataFrame shape."""
        path = str(tmp_path / "modes.yxdb")
        pts = [_wkb_point(1, 2), _wkb_point(3, 4)]
        df = pl.DataFrame({"id": [1, 2], "geom": pts})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        df_raw = sigilyx.read_yxdb(path, spatial="raw")
        df_wkb = sigilyx.read_yxdb(path, spatial="wkb")
        df_geo = sigilyx.read_yxdb(path, spatial="geoarrow")
        assert df_raw.shape == df_wkb.shape == df_geo.shape

    def test_multiple_geometry_types(self, tmp_path):
        """Write and read back different geometry types (as separate files)."""
        for label, geom in [
            ("point", _wkb_point(1, 2)),
            ("line", _wkb_linestring([(0, 0), (1, 1)])),
            ("polygon", _wkb_polygon([[(0, 0), (1, 0), (0, 1), (0, 0)]])),
            ("mpoint", _wkb_multipoint([(1, 2), (3, 4)])),
        ]:
            path = str(tmp_path / f"{label}.yxdb")
            df = pl.DataFrame({"geom": [geom]})
            sigilyx.write_yxdb(path, df, spatial_columns=["geom"])
            df2 = sigilyx.read_yxdb(path, spatial="wkb")
            assert df2.height == 1, f"Failed for {label}"
            wkb = df2["geom"][0]
            assert isinstance(wkb, bytes)
            assert len(wkb) > 5


# ── Existing files with all modes ──────────────────────────────────────


class TestExistingFilesAllModes:
    """Verify that all spatial modes work on every test YXDB file."""

    @pytest.mark.parametrize("name", [
        "AllTypes.yxdb", "People.yxdb", "Strings.yxdb",
        "NullValues.yxdb", "SingleColumn.yxdb", "ManyRecords.yxdb",
    ])
    def test_all_modes_same_shape(self, name):
        path = _yxdb(name)
        df_default = sigilyx.read_yxdb(path)
        df_raw = sigilyx.read_yxdb(path, spatial="raw")
        df_wkb = sigilyx.read_yxdb(path, spatial="wkb")
        df_geo = sigilyx.read_yxdb(path, spatial="geoarrow")
        assert df_default.shape == df_raw.shape == df_wkb.shape == df_geo.shape


# ── PyArrow read_yxdb_arrow with spatial ───────────────────────────────


class TestReadYxdbArrowSpatial:
    """Test read_yxdb_arrow with spatial mode."""

    def test_arrow_wkb_mode(self, tmp_path):
        path = str(tmp_path / "arrow.yxdb")
        pt = _wkb_point(1, 2)
        df = pl.DataFrame({"geom": [pt]})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        table = sigilyx.read_yxdb_arrow(path, spatial="wkb")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1

    def test_arrow_geoarrow_mode(self, tmp_path):
        path = str(tmp_path / "arrow_geo.yxdb")
        pt = _wkb_point(1, 2)
        df = pl.DataFrame({"geom": [pt]})
        sigilyx.write_yxdb(path, df, spatial_columns=["geom"])

        table = sigilyx.read_yxdb_arrow(path, spatial="geoarrow")
        assert isinstance(table, pa.Table)
