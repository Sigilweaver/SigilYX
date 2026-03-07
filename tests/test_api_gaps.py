"""Tests covering API gaps not addressed by other test files.

Covers:
- read_yxdb_fields() and FieldInfo
- sink_yxdb() standalone function
- pathlib.Path arguments across the API surface
- YxdbRowReader as context manager and iterator protocol
- write_yxdb_batches with empty iterator (ValueError)
- write_yxdb_pandas / write_yxdb_arrow edge cases (nulls, unicode)
- Sub-second Time truncation behavior
"""

import datetime
import warnings
from pathlib import Path

import polars as pl
import pyarrow as pa
import pandas as pd
import pytest

import sigilyx

TEST_DIR = Path(__file__).parent.parent / "sigilyx" / "test_files"


def _yxdb(name: str) -> str:
    return str(TEST_DIR / name)


# ============================================================================
#  read_yxdb_fields() and FieldInfo
# ============================================================================


class TestReadYxdbFields:
    """Tests for read_yxdb_fields() and the FieldInfo class."""

    def test_returns_list_of_field_info(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        assert isinstance(fields, list)
        assert len(fields) == 16
        assert all(isinstance(f, sigilyx.FieldInfo) for f in fields)

    def test_field_info_attributes(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        first = fields[0]
        assert first.name == "ByteCol"
        assert first.field_type == "Byte"
        assert isinstance(first.size, int)
        assert isinstance(first.scale, int)

    def test_field_info_repr(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        r = repr(fields[0])
        assert "FieldInfo(" in r
        assert "ByteCol" in r
        assert "Byte" in r

    def test_field_info_equality(self):
        fields1 = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        fields2 = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        assert fields1[0] == fields2[0]
        assert fields1[0] != fields2[1]

    def test_field_info_equality_wrong_type(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        assert fields[0] != "not a FieldInfo"

    def test_all_types_present(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        types = {f.field_type for f in fields}
        for expected in [
            "Byte", "Bool", "Int16", "Int32", "Int64",
            "Float", "Double", "FixedDecimal", "String",
            "WString", "V_String", "V_WString", "Date",
            "Time", "DateTime", "Blob",
        ]:
            assert expected in types, f"{expected} missing"

    def test_decimal_field_has_scale(self):
        fields = sigilyx.read_yxdb_fields(_yxdb("AllTypes.yxdb"))
        dec = next(f for f in fields if f.field_type == "FixedDecimal")
        assert dec.scale > 0

    def test_nonexistent_file_raises(self):
        with pytest.raises(FileNotFoundError):
            sigilyx.read_yxdb_fields(_yxdb("does_not_exist.yxdb"))


# ============================================================================
#  sink_yxdb() standalone function
# ============================================================================


class TestSinkYxdb:
    """Tests for the standalone sink_yxdb() function."""

    def test_sink_basic(self, tmp_path):
        lf = pl.LazyFrame({"a": [1, 2, 3], "b": ["x", "y", "z"]})
        path = str(tmp_path / "sink.yxdb")
        sigilyx.sink_yxdb(path, lf)
        df = sigilyx.read_yxdb(path)
        assert df["a"].to_list() == [1, 2, 3]
        assert df["b"].to_list() == ["x", "y", "z"]

    def test_sink_with_filter(self, tmp_path):
        lf = pl.LazyFrame({"x": [1, 2, 3, 4, 5]}).filter(pl.col("x") > 3)
        path = str(tmp_path / "sink_filtered.yxdb")
        sigilyx.sink_yxdb(path, lf)
        df = sigilyx.read_yxdb(path)
        assert df["x"].to_list() == [4, 5]

    def test_sink_with_projection(self, tmp_path):
        lf = pl.LazyFrame({"a": [1], "b": [2], "c": [3]}).select("a", "c")
        path = str(tmp_path / "sink_proj.yxdb")
        sigilyx.sink_yxdb(path, lf)
        df = sigilyx.read_yxdb(path)
        assert df.columns == ["a", "c"]


# ============================================================================
#  pathlib.Path arguments
# ============================================================================


class TestPathlibPathArguments:
    """Verify that every API accepting a path works with pathlib.Path objects."""

    def test_read_yxdb_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        df = sigilyx.read_yxdb(p)
        assert df.shape == (5, 1)

    def test_read_yxdb_columns_with_path(self):
        p = TEST_DIR / "People.yxdb"
        df = sigilyx.read_yxdb_columns(p, ["PersonId"])
        assert df.columns == ["PersonId"]

    def test_read_schema_with_path(self):
        p = TEST_DIR / "AllTypes.yxdb"
        schema = sigilyx.read_schema(p)
        assert len(schema) == 16

    def test_read_yxdb_fields_with_path(self):
        p = TEST_DIR / "AllTypes.yxdb"
        fields = sigilyx.read_yxdb_fields(p)
        assert len(fields) == 16

    def test_record_count_with_path(self):
        p = TEST_DIR / "ManyRecords.yxdb"
        assert sigilyx.record_count(p) == 50_000

    def test_scan_yxdb_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        df = sigilyx.scan_yxdb(p).collect()
        assert df.shape == (5, 1)

    def test_read_yxdb_batches_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        batches = list(sigilyx.read_yxdb_batches(p))
        total = sum(b.shape[0] for b in batches)
        assert total == 5

    def test_read_yxdb_arrow_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        tbl = sigilyx.read_yxdb_arrow(p)
        assert tbl.num_rows == 5

    def test_read_yxdb_pandas_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        pdf = sigilyx.read_yxdb_pandas(p)
        assert len(pdf) == 5

    def test_write_yxdb_with_path(self, tmp_path):
        df = pl.DataFrame({"v": [1, 2, 3]})
        p = tmp_path / "path_test.yxdb"
        sigilyx.write_yxdb(p, df)
        df2 = sigilyx.read_yxdb(p)
        assert df2["v"].to_list() == [1, 2, 3]

    def test_write_yxdb_pandas_with_path(self, tmp_path):
        pdf = pd.DataFrame({"v": [1, 2]})
        p = tmp_path / "pd_path.yxdb"
        sigilyx.write_yxdb_pandas(p, pdf)
        df2 = sigilyx.read_yxdb(p)
        assert df2["v"].to_list() == [1, 2]

    def test_write_yxdb_arrow_with_path(self, tmp_path):
        tbl = pa.table({"v": [10, 20]})
        p = tmp_path / "pa_path.yxdb"
        sigilyx.write_yxdb_arrow(p, tbl)
        df2 = sigilyx.read_yxdb(p)
        assert df2["v"].to_list() == [10, 20]

    def test_sink_yxdb_with_path(self, tmp_path):
        lf = pl.LazyFrame({"v": [42]})
        p = tmp_path / "sink_path.yxdb"
        sigilyx.sink_yxdb(p, lf)
        df = sigilyx.read_yxdb(p)
        assert df["v"][0] == 42

    def test_write_yxdb_batches_with_path(self, tmp_path):
        batches = [pl.DataFrame({"x": [1, 2]})]
        p = tmp_path / "batch_path.yxdb"
        sigilyx.write_yxdb_batches(p, iter(batches))
        df = sigilyx.read_yxdb(p)
        assert df["x"].to_list() == [1, 2]

    def test_row_reader_with_path(self):
        p = TEST_DIR / "SingleColumn.yxdb"
        reader = sigilyx.YxdbRowReader(p)
        count = 0
        while reader.next():
            count += 1
        reader.close()
        assert count == 5


# ============================================================================
#  YxdbRowReader context manager and iterator protocol
# ============================================================================


class TestYxdbRowReaderProtocols:
    """Test context manager and iterator protocol for YxdbRowReader."""

    def test_context_manager(self):
        """Reader works with 'with' statement."""
        with sigilyx.YxdbRowReader(_yxdb("SingleColumn.yxdb")) as reader:
            rows = []
            while reader.next():
                rows.append(reader.read_all())
        assert len(rows) == 5
        assert rows[0] == (10,)

    def test_context_manager_fields_accessible(self):
        """Fields are accessible inside context manager."""
        with sigilyx.YxdbRowReader(_yxdb("AllTypes.yxdb")) as reader:
            assert len(reader.fields) == 16
            assert reader.num_records == 2

    def test_iterator_protocol(self):
        """Reader works as a Python iterator yielding tuples."""
        rows = list(sigilyx.YxdbRowReader(_yxdb("SingleColumn.yxdb")))
        assert len(rows) == 5
        assert rows[0] == (10,)
        assert rows[4] == (50,)

    def test_iterator_with_for_loop(self):
        """Reader works in a for loop."""
        values = []
        for row in sigilyx.YxdbRowReader(_yxdb("SingleColumn.yxdb")):
            values.append(row[0])
        assert values == [10, 20, 30, 40, 50]

    def test_iterator_in_context_manager(self):
        """Iterator protocol works inside context manager."""
        with sigilyx.YxdbRowReader(_yxdb("SingleColumn.yxdb")) as reader:
            rows = list(reader)
        assert len(rows) == 5

    def test_iterator_with_multi_column(self):
        """Iterator yields tuples with correct number of fields."""
        first_row = next(iter(sigilyx.YxdbRowReader(_yxdb("AllTypes.yxdb"))))
        assert len(first_row) == 16

    def test_context_manager_dict_read(self):
        """read_dict works inside context manager iteration."""
        with sigilyx.YxdbRowReader(_yxdb("SingleColumn.yxdb")) as reader:
            reader.next()
            d = reader.read_dict()
            assert d["Value"] == 10


# ============================================================================
#  write_yxdb_batches with empty iterator
# ============================================================================


class TestWriteBatchesEmptyIterator:
    """Confirm that write_yxdb_batches raises on empty iterator."""

    def test_empty_iterator_raises_value_error(self, tmp_path):
        path = str(tmp_path / "empty_iter.yxdb")
        with pytest.raises(ValueError, match="(?i)empty|schema"):
            sigilyx.write_yxdb_batches(path, iter([]))


# ============================================================================
#  write_yxdb_pandas / write_yxdb_arrow edge cases
# ============================================================================


class TestWritePandasEdgeCases:
    """Edge cases for write_yxdb_pandas beyond basic happy path."""

    def test_pandas_with_nulls(self, tmp_path):
        pdf = pd.DataFrame({
            "id": [1, 2, 3],
            "name": ["Alice", None, "Charlie"],
            "score": [95.5, None, 87.0],
        })
        path = str(tmp_path / "pd_nulls.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf)
        df = sigilyx.read_yxdb(path)
        assert df["name"][1] is None
        assert df["score"][1] is None
        assert df["name"][0] == "Alice"

    def test_pandas_with_unicode(self, tmp_path):
        pdf = pd.DataFrame({"text": ["café", "日本語", "Ünïcödé"]})
        path = str(tmp_path / "pd_unicode.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf)
        df = sigilyx.read_yxdb(path)
        assert df["text"].to_list() == ["café", "日本語", "Ünïcödé"]

    def test_pandas_roundtrip_with_dates(self, tmp_path):
        pdf = pd.DataFrame({
            "d": pd.to_datetime(["2025-01-15", "2000-06-01"]).date,
        })
        path = str(tmp_path / "pd_dates.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf)
        pdf2 = sigilyx.read_yxdb_pandas(path)
        assert list(pdf2["d"]) == list(pdf["d"])

    def test_pandas_empty_dataframe(self, tmp_path):
        """Writing an empty pandas DataFrame creates a valid file with 0 records."""
        pdf = pd.DataFrame({"x": pd.Series([], dtype="int64")})
        path = str(tmp_path / "pd_empty.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf)
        # Verify header reports 0 records
        assert sigilyx.record_count(path) == 0


class TestWriteArrowEdgeCases:
    """Edge cases for write_yxdb_arrow beyond basic happy path."""

    def test_arrow_with_nulls(self, tmp_path):
        tbl = pa.table({
            "id": [1, 2, 3],
            "val": pa.array([10.0, None, 30.0], type=pa.float64()),
        })
        path = str(tmp_path / "pa_nulls.yxdb")
        sigilyx.write_yxdb_arrow(path, tbl)
        df = sigilyx.read_yxdb(path)
        assert df["val"][1] is None
        assert df["val"][0] == 10.0

    def test_arrow_with_unicode(self, tmp_path):
        tbl = pa.table({"s": ["ĀĂĄĆĈ", "αβγδε", "🎉🚀"]})
        path = str(tmp_path / "pa_unicode.yxdb")
        sigilyx.write_yxdb_arrow(path, tbl)
        df = sigilyx.read_yxdb(path)
        assert df["s"].to_list() == ["ĀĂĄĆĈ", "αβγδε", "🎉🚀"]

    def test_arrow_binary_column(self, tmp_path):
        tbl = pa.table({
            "b": pa.array([b"\x00\x01", b"\xFF" * 200, None], type=pa.binary()),
        })
        path = str(tmp_path / "pa_binary.yxdb")
        sigilyx.write_yxdb_arrow(path, tbl)
        df = sigilyx.read_yxdb(path)
        assert df["b"][0] == b"\x00\x01"
        assert df["b"][1] == b"\xFF" * 200
        assert df["b"][2] is None

    def test_arrow_roundtrip_preserves_values(self, tmp_path):
        tbl_orig = pa.table({
            "i": [1, 2, 3],
            "s": ["a", "b", "c"],
            "f": [1.1, 2.2, 3.3],
        })
        path = str(tmp_path / "pa_roundtrip.yxdb")
        sigilyx.write_yxdb_arrow(path, tbl_orig)
        tbl_read = sigilyx.read_yxdb_arrow(path)
        assert tbl_orig.to_pydict() == tbl_read.to_pydict()


# ============================================================================
#  Sub-second Time precision
# ============================================================================


class TestTimeSubSecondPrecision:
    """YXDB Time is HH:MM:SS (no fractional seconds).

    Verify that sub-second precision is truncated on roundtrip, matching
    the format's limitation.
    """

    def test_whole_second_times_preserved(self, tmp_path):
        times = [
            datetime.time(0, 0, 0),
            datetime.time(12, 30, 45),
            datetime.time(23, 59, 59),
        ]
        df = pl.DataFrame({"t": pl.Series(times, dtype=pl.Time)})
        path = str(tmp_path / "time_whole.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["t"].to_list() == times

    def test_subsecond_times_truncated(self, tmp_path):
        """Times with microsecond/nanosecond parts lose sub-second precision."""
        times_with_us = [
            datetime.time(10, 30, 15, 123456),  # .123456 seconds
            datetime.time(8, 0, 0, 500000),      # .500000 seconds
        ]
        expected_truncated = [
            datetime.time(10, 30, 15),
            datetime.time(8, 0, 0),
        ]
        df = pl.DataFrame({"t": pl.Series(times_with_us, dtype=pl.Time)})
        path = str(tmp_path / "time_subsec.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["t"].to_list() == expected_truncated


# ============================================================================
#  write_yxdb_with_overrides()
# ============================================================================


class TestWriteYxdbWithOverrides:
    """Tests for write_yxdb_with_overrides()."""

    def test_string_size_override(self, tmp_path):
        """Override a String column to a fixed-size String with explicit size."""
        df = pl.DataFrame({"name": ["Alice", "Bob"]})
        path = str(tmp_path / "override_str.yxdb")
        sigilyx.write_yxdb_with_overrides(
            path, df, {"name": {"type": "String", "size": 64}}
        )
        df2 = sigilyx.read_yxdb(path)
        assert df2["name"].to_list() == ["Alice", "Bob"]
        # Verify the schema has the override applied
        fields = sigilyx.read_yxdb_fields(path)
        name_field = [f for f in fields if f.name == "name"][0]
        assert name_field.field_type == "String"
        assert name_field.size == 64

    def test_wstring_override(self, tmp_path):
        """Override to WString type."""
        df = pl.DataFrame({"text": ["Hello", "World"]})
        path = str(tmp_path / "override_wstr.yxdb")
        sigilyx.write_yxdb_with_overrides(
            path, df, {"text": {"type": "WString", "size": 128}}
        )
        df2 = sigilyx.read_yxdb(path)
        assert df2["text"].to_list() == ["Hello", "World"]
        fields = sigilyx.read_yxdb_fields(path)
        text_field = [f for f in fields if f.name == "text"][0]
        assert text_field.field_type == "WString"
        assert text_field.size == 128

    def test_fixed_decimal_override(self, tmp_path):
        """Override a float column to FixedDecimal with precision and scale."""
        df = pl.DataFrame({"price": [1.99, 2.50, 3.00]})
        path = str(tmp_path / "override_decimal.yxdb")
        sigilyx.write_yxdb_with_overrides(
            path, df, {"price": {"type": "FixedDecimal", "size": 10, "scale": 2}}
        )
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape[0] == 3
        fields = sigilyx.read_yxdb_fields(path)
        price_field = [f for f in fields if f.name == "price"][0]
        assert price_field.field_type == "FixedDecimal"
        assert price_field.size == 10
        assert price_field.scale == 2

    def test_multiple_overrides(self, tmp_path):
        """Override multiple columns at once."""
        df = pl.DataFrame({
            "id": [1, 2, 3],
            "name": ["A", "B", "C"],
            "value": pl.Series([1.0, 2.0, 3.0], dtype=pl.Float32),
        })
        path = str(tmp_path / "override_multi.yxdb")
        sigilyx.write_yxdb_with_overrides(path, df, {
            "name": {"type": "WString", "size": 50},
            "value": {"type": "Float"},
        })
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == (3, 3)
        fields = sigilyx.read_yxdb_fields(path)
        fields_dict = {f.name: f for f in fields}
        assert fields_dict["name"].field_type == "WString"
        assert fields_dict["value"].field_type == "Float"

    def test_override_preserves_data(self, tmp_path):
        """Data values are preserved through overrides."""
        df = pl.DataFrame({
            "x": [10, 20, 30],
            "label": ["alpha", "beta", "gamma"],
        })
        path = str(tmp_path / "override_preserve.yxdb")
        sigilyx.write_yxdb_with_overrides(
            path, df, {"label": {"type": "V_WString", "size": 256}}
        )
        df2 = sigilyx.read_yxdb(path)
        assert df2["x"].to_list() == [10, 20, 30]
        assert df2["label"].to_list() == ["alpha", "beta", "gamma"]

    def test_no_overrides_same_as_write(self, tmp_path):
        """Empty overrides dict behaves like write_yxdb."""
        df = pl.DataFrame({"a": [1, 2], "b": ["x", "y"]})
        path_override = str(tmp_path / "override_empty.yxdb")
        path_normal = str(tmp_path / "normal.yxdb")
        sigilyx.write_yxdb_with_overrides(path_override, df, {})
        sigilyx.write_yxdb(path_normal, df)
        df_o = sigilyx.read_yxdb(path_override)
        df_n = sigilyx.read_yxdb(path_normal)
        assert df_o.equals(df_n)

    def test_pathlib_path_argument(self, tmp_path):
        """Accepts pathlib.Path as the path argument."""
        df = pl.DataFrame({"v": [1, 2, 3]})
        path = tmp_path / "override_pathlib.yxdb"
        sigilyx.write_yxdb_with_overrides(
            path, df, {"v": {"type": "Int64"}}
        )
        df2 = sigilyx.read_yxdb(str(path))
        assert df2["v"].to_list() == [1, 2, 3]

    def test_validates_dataframe(self):
        """Rejects non-DataFrame input."""
        with pytest.raises(TypeError, match="polars.DataFrame"):
            sigilyx.write_yxdb_with_overrides("out.yxdb", "not_a_df", {})

    def test_duration_rejected(self, tmp_path):
        """Duration columns are rejected with a clear error."""
        import datetime as dt
        df = pl.DataFrame({"d": [dt.timedelta(days=1)]})
        with pytest.raises(TypeError, match="not supported by YXDB"):
            sigilyx.write_yxdb_with_overrides(
                str(tmp_path / "dur.yxdb"), df, {}
            )
