"""Tests for sigilyx Python bindings.

All test data generated from scratch using Alteryx native PyYXDBReader API.
Expected values are independently known from the test data generation process.
"""

import datetime
import os
from pathlib import Path

import polars as pl
import pyarrow as pa
import pandas as pd
import pytest

import sigilyx

TEST_DIR = Path(__file__).parent.parent / "sigilyx" / "test_files"


def _yxdb(name: str) -> str:
    return str(TEST_DIR / name)


class TestAllTypes:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (2, 16)

    def test_column_names(self, df: pl.DataFrame):
        expected = [
            "ByteCol", "BoolCol", "Int16Col", "Int32Col", "Int64Col",
            "FloatCol", "DoubleCol", "DecimalCol", "StringCol", "WStringCol",
            "VStringCol", "VWStringCol", "DateCol", "TimeCol", "DateTimeCol",
            "BlobCol",
        ]
        assert df.columns == expected

    def test_byte_field(self, df: pl.DataFrame):
        vals = df["ByteCol"].to_list()
        assert vals[0] == 7
        assert vals[1] == 255

    def test_int16_field(self, df: pl.DataFrame):
        vals = df["Int16Col"].to_list()
        assert vals[0] == -1234
        assert vals[1] == 32767

    def test_int32_field(self, df: pl.DataFrame):
        vals = df["Int32Col"].to_list()
        assert vals[0] == 42000
        assert vals[1] == -1

    def test_int64_field(self, df: pl.DataFrame):
        vals = df["Int64Col"].to_list()
        assert vals[0] == 9_000_000_000
        assert vals[1] == -9_000_000_000

    def test_bool_field(self, df: pl.DataFrame):
        assert df["BoolCol"][0] is True
        assert df["BoolCol"][1] is False

    def test_float_field(self, df: pl.DataFrame):
        assert abs(df["FloatCol"][0] - 2.5) < 0.01
        assert abs(df["FloatCol"][1] - (-0.5)) < 0.01

    def test_double_field(self, df: pl.DataFrame):
        assert abs(df["DoubleCol"][0] - 3.141592653589793) < 1e-10
        assert abs(df["DoubleCol"][1] - 0.0) < 1e-10

    def test_decimal_field(self, df: pl.DataFrame):
        assert abs(df["DecimalCol"][0] - 1234.5678) < 0.001
        assert abs(df["DecimalCol"][1] - (-9999.9999)) < 0.001

    def test_fixed_string(self, df: pl.DataFrame):
        assert df["StringCol"][0] == "Alteryx"
        assert df["StringCol"][1] == "Test"

    def test_fixed_wstring_unicode(self, df: pl.DataFrame):
        assert df["WStringCol"][0] == "\u00dc\u006e\u00ef\u0063\u00f6\u0064\u00e9"
        assert df["WStringCol"][1] == "W\u00efd\u00e9"

    def test_var_string_short(self, df: pl.DataFrame):
        assert df["VStringCol"][0] == "short var"

    def test_var_string_long(self, df: pl.DataFrame):
        assert df["VStringCol"][1] == "y" * 800

    def test_var_wstring_long(self, df: pl.DataFrame):
        val = df["VWStringCol"][0]
        assert len(val) == 600
        assert val == "x" * 600

    def test_var_wstring_very_long(self, df: pl.DataFrame):
        val = df["VWStringCol"][1]
        assert len(val) == 1200
        assert val == "z" * 1200

    def test_date_field(self, df: pl.DataFrame):
        assert df["DateCol"][0] == datetime.date(2025, 3, 15)
        assert df["DateCol"][1] == datetime.date(1999, 1, 1)

    def test_datetime_field(self, df: pl.DataFrame):
        assert df["DateTimeCol"][0] == datetime.datetime(2025, 3, 15, 8, 30, 0)
        assert df["DateTimeCol"][1] == datetime.datetime(1999, 1, 1, 23, 59, 59)

    def test_blob_pattern(self, df: pl.DataFrame):
        blob0 = df["BlobCol"][0]
        assert len(blob0) == 1024
        assert blob0[0] == 0x00
        assert blob0[1] == 0x01
        assert blob0[255] == 0xFF
        assert blob0[256] == 0x00

    def test_blob_all_ff(self, df: pl.DataFrame):
        blob1 = df["BlobCol"][1]
        assert len(blob1) == 512
        assert all(b == 0xFF for b in blob1)

    def test_dtypes(self, df: pl.DataFrame):
        assert df["BoolCol"].dtype == pl.Boolean
        assert df["Int32Col"].dtype == pl.Int32
        assert df["Int64Col"].dtype == pl.Int64
        assert df["FloatCol"].dtype == pl.Float32
        assert df["DoubleCol"].dtype == pl.Float64
        assert df["StringCol"].dtype == pl.String
        assert df["DateCol"].dtype == pl.Date


class TestNullValues:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("NullValues.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (3, 11)

    def test_populated_row(self, df: pl.DataFrame):
        assert df["Id"][0] == 1
        assert df["NullByte"][0] == 42
        assert df["NullInt16"][0] == 100
        assert df["NullInt32"][0] == 200
        assert df["NullInt64"][0] == 300
        assert abs(df["NullFloat"][0] - 1.5) < 0.01
        assert abs(df["NullDouble"][0] - 2.5) < 0.01
        assert df["NullStr"][0] == "hello"

    def test_all_null_row(self, df: pl.DataFrame):
        assert df["Id"][1] == 2
        assert df["NullByte"][1] is None
        assert df["NullInt16"][1] is None
        assert df["NullInt32"][1] is None
        assert df["NullInt64"][1] is None
        assert df["NullFloat"][1] is None
        assert df["NullDouble"][1] is None
        assert df["NullStr"][1] is None
        assert df["NullDate"][1] is None
        assert df["NullDT"][1] is None
        assert df["NullBlob"][1] is None

    def test_mixed_null_row(self, df: pl.DataFrame):
        assert df["Id"][2] == 3
        assert df["NullByte"][2] is None
        assert df["NullInt16"][2] == 50
        assert df["NullInt32"][2] is None
        assert df["NullInt64"][2] == 600
        assert df["NullFloat"][2] is None
        assert abs(df["NullDouble"][2] - 9.9) < 0.01
        assert df["NullStr"][2] is None
        assert df["NullDate"][2] == datetime.date(2025, 12, 25)
        assert df["NullDT"][2] is None
        assert df["NullBlob"][2] is None

    def test_null_counts(self, df: pl.DataFrame):
        assert df["Id"].null_count() == 0
        assert df["NullByte"].null_count() == 2
        assert df["NullInt16"].null_count() == 1
        assert df["NullInt32"].null_count() == 2


class TestManyRecords:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("ManyRecords.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (50_000, 3)

    def test_columns(self, df: pl.DataFrame):
        assert df.columns == ["Id", "Value", "Label"]

    def test_id_sequential(self, df: pl.DataFrame):
        ids = df["Id"].to_list()
        assert ids[0] == 1
        assert ids[-1] == 50_000
        assert df["Id"].is_sorted()

    def test_id_sum(self, df: pl.DataFrame):
        assert df["Id"].cast(pl.Int64).sum() == 1_250_025_000

    def test_value_formula(self, df: pl.DataFrame):
        row_100 = df.row(99)
        assert row_100[0] == 100
        assert abs(row_100[1] - 150.0) < 0.01

    def test_label_format(self, df: pl.DataFrame):
        assert df["Label"][0] == "row_00001"
        assert df["Label"][49_999] == "row_50000"
        assert df["Label"][999] == "row_01000"


class TestLargeBlob:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("LargeBlob.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (4, 2)

    def test_large_blob(self, df: pl.DataFrame):
        blob = df["Data"][0]
        assert len(blob) == 512_000
        assert blob[0] == 0
        assert blob[255] == 255
        assert blob[256] == 0

    def test_null_blob(self, df: pl.DataFrame):
        assert df["Data"][1] is None

    def test_tiny_blob(self, df: pl.DataFrame):
        blob = df["Data"][2]
        assert blob == b"tiny"

    def test_second_large_blob(self, df: pl.DataFrame):
        blob = df["Data"][3]
        assert len(blob) == 500_000


class TestPeople:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("People.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (200, 8)

    def test_columns(self, df: pl.DataFrame):
        expected = [
            "PersonId", "FirstName", "LastName", "Age",
            "Salary", "Active", "JoinDate", "Notes",
        ]
        assert df.columns == expected

    def test_ids_complete(self, df: pl.DataFrame):
        ids = sorted(df["PersonId"].to_list())
        assert ids == list(range(1, 201))

    def test_no_null_ids(self, df: pl.DataFrame):
        assert df["PersonId"].null_count() == 0

    def test_age_range(self, df: pl.DataFrame):
        ages = df["Age"].to_list()
        assert all(18 <= a <= 75 for a in ages if a is not None)

    def test_salary_range(self, df: pl.DataFrame):
        salaries = df["Salary"].to_list()
        assert all(30000 <= s <= 180000 for s in salaries if s is not None)

    def test_active_is_boolean(self, df: pl.DataFrame):
        assert df["Active"].dtype == pl.Boolean

    def test_some_notes_are_null(self, df: pl.DataFrame):
        null_pct = df["Notes"].null_count() / len(df)
        assert 0.1 < null_pct < 0.6


class TestStrings:
    @pytest.fixture
    def df(self) -> pl.DataFrame:
        return sigilyx.read_yxdb(_yxdb("Strings.yxdb"))

    def test_shape(self, df: pl.DataFrame):
        assert df.shape == (6, 5)

    def test_normal_strings(self, df: pl.DataFrame):
        assert df["FixedStr"][0] == "hello"
        assert df["FixedWStr"][0] == "world"
        assert df["VarStr"][0] == "variable"
        assert df["VarWStr"][0] == "w\u00efd\u00e9"

    def test_empty_strings(self, df: pl.DataFrame):
        assert df["FixedStr"][1] == ""
        assert df["VarStr"][1] == ""

    def test_max_length_fixed(self, df: pl.DataFrame):
        assert df["FixedStr"][2] == "A" * 50
        assert df["FixedWStr"][2] == "B" * 50

    def test_long_variable_strings(self, df: pl.DataFrame):
        assert df["VarStr"][3] == "M" * 2000
        assert df["VarWStr"][3] == "N" * 3000

    def test_unicode_in_wstring(self, df: pl.DataFrame):
        val = df["VarWStr"][4]
        assert val is not None
        assert len(val) > 0

    def test_null_strings(self, df: pl.DataFrame):
        assert df["FixedStr"][5] is None
        assert df["FixedWStr"][5] is None
        assert df["VarStr"][5] is None
        assert df["VarWStr"][5] is None


class TestSingleColumn:
    def test_values(self):
        df = sigilyx.read_yxdb(_yxdb("SingleColumn.yxdb"))
        assert df.shape == (5, 1)
        assert df.columns == ["Value"]
        assert df["Value"].to_list() == [10, 20, 30, 40, 50]


class TestSchemaFunctions:
    def test_read_schema(self):
        schema = sigilyx.read_schema(_yxdb("AllTypes.yxdb"))
        assert isinstance(schema, list)
        assert len(schema) == 16
        assert schema[0]["name"] == "ByteCol"
        assert schema[0]["type"] == "Byte"

    def test_schema_all_types_present(self):
        schema = sigilyx.read_schema(_yxdb("AllTypes.yxdb"))
        types = [f["type"] for f in schema]
        for expected in [
            "Byte", "Bool", "Int16", "Int32", "Int64",
            "Float", "Double", "FixedDecimal", "String",
            "WString", "VString", "VWString", "Date",
            "Time", "DateTime", "Blob",
        ]:
            assert expected in types, f"{expected} missing from schema"

    def test_record_count_many(self):
        assert sigilyx.record_count(_yxdb("ManyRecords.yxdb")) == 50_000

    def test_record_count_small(self):
        assert sigilyx.record_count(_yxdb("SingleColumn.yxdb")) == 5

    def test_record_count_people(self):
        assert sigilyx.record_count(_yxdb("People.yxdb")) == 200


class TestErrorHandling:
    def test_invalid_text_file(self):
        with pytest.raises(RuntimeError, match="(?i)invalid|not a valid"):
            sigilyx.read_yxdb(_yxdb("not_a_yxdb.txt"))

    def test_too_small_file(self):
        with pytest.raises(RuntimeError):
            sigilyx.read_yxdb(_yxdb("too_small.bin"))

    def test_nonexistent_file(self):
        with pytest.raises(RuntimeError):
            sigilyx.read_yxdb(_yxdb("does_not_exist.yxdb"))

    def test_invalid_schema(self):
        with pytest.raises(RuntimeError):
            sigilyx.read_schema(_yxdb("does_not_exist.yxdb"))


class TestPolarsIntegration:
    def test_filter(self):
        df = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        active = df.filter(pl.col("Active") == True)
        assert len(active) > 0
        assert all(v is True for v in active["Active"].to_list())

    def test_group_by(self):
        df = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        grouped = df.group_by("Active").agg(
            pl.col("PersonId").count().alias("count")
        )
        assert len(grouped) == 2
        assert grouped["count"].sum() == 200

    def test_select_and_cast(self):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        result = df.select([
            pl.col("Int32Col").cast(pl.Float64),
            pl.col("StringCol"),
        ])
        assert result.shape == (2, 2)
        assert result["Int32Col"][0] == 42000.0

    def test_sort(self):
        df = sigilyx.read_yxdb(_yxdb("ManyRecords.yxdb"))
        sorted_df = df.sort("Id", descending=True)
        assert sorted_df["Id"][0] == 50_000
        assert sorted_df["Id"][-1] == 1

    def test_lazy_query(self):
        df = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        result = (
            df.lazy()
            .filter(pl.col("Active") == True)
            .select(["FirstName", "LastName", "Salary"])
            .collect()
        )
        assert len(result) > 0
        assert result.columns == ["FirstName", "LastName", "Salary"]

    def test_write_and_read_parquet(self, tmp_path):
        df = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        parquet_path = tmp_path / "people.parquet"
        df.write_parquet(str(parquet_path))
        df2 = pl.read_parquet(str(parquet_path))
        assert df.shape == df2.shape
        assert df.columns == df2.columns

    def test_write_csv(self, tmp_path):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        # Drop binary column â€” Polars can't write binary to CSV
        df = df.drop("BlobCol")
        csv_path = tmp_path / "all_types.csv"
        df.write_csv(str(csv_path))
        content = csv_path.read_text()
        assert "ByteCol" in content
        assert "Alteryx" in content

    def test_join(self):
        people = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        single = sigilyx.read_yxdb(_yxdb("SingleColumn.yxdb"))
        single = single.rename({"Value": "PersonId"})
        joined = people.join(single, on="PersonId", how="inner")
        assert len(joined) == 5


class TestScanYxdb:
    """Tests for scan_yxdb (LazyFrame API)."""

    def test_returns_lazyframe(self):
        lf = sigilyx.scan_yxdb(_yxdb("People.yxdb"))
        assert isinstance(lf, pl.LazyFrame)

    def test_collect_matches_read(self):
        df = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        lf_df = sigilyx.scan_yxdb(_yxdb("People.yxdb")).collect()
        assert df.shape == lf_df.shape
        assert df.columns == lf_df.columns

    def test_lazy_filter(self):
        result = (
            sigilyx.scan_yxdb(_yxdb("People.yxdb"))
            .filter(pl.col("Active") == True)  # noqa: E712
            .collect()
        )
        assert len(result) > 0
        assert result["Active"].to_list() == [True] * len(result)

    def test_lazy_select(self):
        result = (
            sigilyx.scan_yxdb(_yxdb("People.yxdb"))
            .select("FirstName", "LastName")
            .collect()
        )
        assert result.columns == ["FirstName", "LastName"]

    def test_lazy_with_columns(self):
        result = (
            sigilyx.scan_yxdb(_yxdb("ManyRecords.yxdb"))
            .with_columns((pl.col("Id") * 2).alias("DoubleId"))
            .head(3)
            .collect()
        )
        assert "DoubleId" in result.columns
        assert result["DoubleId"].to_list() == [2, 4, 6]


class TestReadYxdbBatches:
    """Tests for read_yxdb_batches (streaming/batched API)."""

    def test_yields_dataframes(self):
        batches = list(sigilyx.read_yxdb_batches(_yxdb("People.yxdb"), batch_size=100))
        for batch in batches:
            assert isinstance(batch, pl.DataFrame)

    def test_total_rows_match(self):
        expected = sigilyx.record_count(_yxdb("ManyRecords.yxdb"))
        batches = list(sigilyx.read_yxdb_batches(_yxdb("ManyRecords.yxdb"), batch_size=500))
        total = sum(b.shape[0] for b in batches)
        assert total == expected

    def test_batch_size_respected(self):
        batch_size = 300
        batches = list(sigilyx.read_yxdb_batches(_yxdb("ManyRecords.yxdb"), batch_size=batch_size))
        # All batches except possibly the last should have exactly batch_size rows
        for batch in batches[:-1]:
            assert batch.shape[0] == batch_size
        # Last batch should have <= batch_size rows
        assert batches[-1].shape[0] <= batch_size

    def test_columns_consistent(self):
        batches = list(sigilyx.read_yxdb_batches(_yxdb("AllTypes.yxdb"), batch_size=1))
        expected_cols = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb")).columns
        for batch in batches:
            assert batch.columns == expected_cols

    def test_concat_matches_full_read(self):
        df_full = sigilyx.read_yxdb(_yxdb("ManyRecords.yxdb"))
        batches = list(sigilyx.read_yxdb_batches(_yxdb("ManyRecords.yxdb"), batch_size=500))
        df_batched = pl.concat(batches)
        assert df_full.shape == df_batched.shape
        assert df_full.equals(df_batched)

    def test_single_batch_small_file(self):
        # File with only 2 rows â€” should yield a single batch
        batches = list(sigilyx.read_yxdb_batches(_yxdb("AllTypes.yxdb"), batch_size=65536))
        assert len(batches) == 1
        assert batches[0].shape[0] == 2

    def test_default_batch_size(self):
        # Just verify it works with the default batch_size
        batches = list(sigilyx.read_yxdb_batches(_yxdb("People.yxdb")))
        total = sum(b.shape[0] for b in batches)
        assert total == sigilyx.record_count(_yxdb("People.yxdb"))


# ---------------------------------------------------------------------------
#  PyArrow reader
# ---------------------------------------------------------------------------

class TestReadYxdbArrow:
    """Tests for read_yxdb_arrow() returning pyarrow.Table."""

    def test_returns_pyarrow_table(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        assert isinstance(tbl, pa.Table)

    def test_shape_matches_polars(self):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        assert tbl.num_rows == df.shape[0]
        assert tbl.num_columns == df.shape[1]

    def test_column_names_match_polars(self):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        assert tbl.column_names == df.columns

    def test_int_values(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        assert tbl.column("Int32Col").to_pylist() == [42000, -1]

    def test_string_values(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        assert tbl.column("StringCol").to_pylist() == ["Alteryx", "Test"]

    def test_people_file(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("People.yxdb"))
        assert tbl.num_rows == 200

    def test_many_records(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("ManyRecords.yxdb"))
        expected = sigilyx.record_count(_yxdb("ManyRecords.yxdb"))
        assert tbl.num_rows == expected

    def test_nulls_file(self):
        tbl = sigilyx.read_yxdb_arrow(_yxdb("NullValues.yxdb"))
        df = sigilyx.read_yxdb(_yxdb("NullValues.yxdb"))
        assert tbl.num_rows == df.shape[0]

    def test_empty_file(self):
        # SingleColumn is the smallest file â€” just verify arrow works on it
        tbl = sigilyx.read_yxdb_arrow(_yxdb("SingleColumn.yxdb"))
        assert tbl.num_rows > 0


# ---------------------------------------------------------------------------
#  Pandas reader
# ---------------------------------------------------------------------------

class TestReadYxdbPandas:
    """Tests for read_yxdb_pandas() returning pandas.DataFrame."""

    def test_returns_pandas_dataframe(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        assert isinstance(pdf, pd.DataFrame)

    def test_shape_matches_polars(self):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        assert pdf.shape == df.shape

    def test_column_names_match_polars(self):
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        assert list(pdf.columns) == df.columns

    def test_int_values(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        assert list(pdf["Int32Col"]) == [42000, -1]

    def test_string_values(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        assert list(pdf["StringCol"]) == ["Alteryx", "Test"]

    def test_people_file(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("People.yxdb"))
        assert len(pdf) == 200

    def test_many_records(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("ManyRecords.yxdb"))
        expected = sigilyx.record_count(_yxdb("ManyRecords.yxdb"))
        assert len(pdf) == expected

    def test_nulls_file(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("NullValues.yxdb"))
        df = sigilyx.read_yxdb(_yxdb("NullValues.yxdb"))
        assert pdf.shape == df.shape

    def test_small_file(self):
        pdf = sigilyx.read_yxdb_pandas(_yxdb("SingleColumn.yxdb"))
        assert len(pdf) > 0
        assert len(pdf.columns) > 0

    def test_roundtrip_matches_arrow(self):
        """Pandas DataFrame from read_yxdb_pandas matches converting arrow table."""
        pdf = sigilyx.read_yxdb_pandas(_yxdb("AllTypes.yxdb"))
        tbl = sigilyx.read_yxdb_arrow(_yxdb("AllTypes.yxdb"))
        pdf_from_arrow = tbl.to_pandas()
        pd.testing.assert_frame_equal(pdf, pdf_from_arrow)


class TestWriteYxdb:
    """Tests for write_yxdb functionality."""

    def test_write_and_read_simple(self, tmp_path):
        """Write and read back a simple DataFrame."""
        df = pl.DataFrame({
            "id": [1, 2, 3],
            "name": ["Alice", "Bob", "Charlie"],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape
        assert df2.columns == df.columns

    def test_write_preserves_data(self, tmp_path):
        """Written data matches original exactly."""
        df = pl.DataFrame({
            "id": [1, 2, 3],
            "value": [10.5, 20.5, 30.5],
            "flag": [True, False, True],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df["id"].to_list() == df2["id"].to_list()
        assert df["value"].to_list() == df2["value"].to_list()
        assert df["flag"].to_list() == df2["flag"].to_list()

    def test_write_with_nulls(self, tmp_path):
        """Null values are preserved correctly."""
        df = pl.DataFrame({
            "val": [10, None, 30],
        }).cast({"val": pl.Int32})
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = df2["val"].to_list()
        assert vals[0] == 10
        assert vals[1] is None
        assert vals[2] == 30

    def test_write_strings(self, tmp_path):
        """String values including unicode are preserved."""
        df = pl.DataFrame({
            "text": ["hello", "世界", "café", ""],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df["text"].to_list() == df2["text"].to_list()

    def test_roundtrip_alltypes(self, tmp_path):
        """Round-trip test with AllTypes.yxdb."""
        df_orig = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        path = str(tmp_path / "roundtrip.yxdb")
        sigilyx.write_yxdb(path, df_orig)
        df_read = sigilyx.read_yxdb(path)
        assert df_orig.equals(df_read)

    def test_roundtrip_people(self, tmp_path):
        """Round-trip test with People.yxdb."""
        df_orig = sigilyx.read_yxdb(_yxdb("People.yxdb"))
        path = str(tmp_path / "roundtrip.yxdb")
        sigilyx.write_yxdb(path, df_orig)
        df_read = sigilyx.read_yxdb(path)
        assert df_orig.equals(df_read)

    def test_roundtrip_many_records(self, tmp_path):
        """Round-trip test with ManyRecords.yxdb (50k rows)."""
        df_orig = sigilyx.read_yxdb(_yxdb("ManyRecords.yxdb"))
        path = str(tmp_path / "roundtrip.yxdb")
        sigilyx.write_yxdb(path, df_orig)
        df_read = sigilyx.read_yxdb(path)
        assert df_orig.equals(df_read)

    def test_roundtrip_null_values(self, tmp_path):
        """Round-trip test with NullValues.yxdb."""
        df_orig = sigilyx.read_yxdb(_yxdb("NullValues.yxdb"))
        path = str(tmp_path / "roundtrip.yxdb")
        sigilyx.write_yxdb(path, df_orig)
        df_read = sigilyx.read_yxdb(path)
        assert df_orig.equals(df_read)

    def test_write_creates_file(self, tmp_path):
        """Verify the file is actually created on disk."""
        df = pl.DataFrame({"x": [1, 2, 3]})
        path = tmp_path / "output.yxdb"
        sigilyx.write_yxdb(str(path), df)
        assert path.exists()
        assert path.stat().st_size > 0


class TestWriteYxdbPandas:
    """Tests for write_yxdb_pandas functionality."""

    def test_write_pandas_dataframe(self, tmp_path):
        """Write a pandas DataFrame to YXDB."""
        pdf = pd.DataFrame({
            "id": [1, 2, 3],
            "name": ["Alice", "Bob", "Charlie"],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == (3, 2)
        assert df2["id"].to_list() == [1, 2, 3]

    def test_roundtrip_pandas(self, tmp_path):
        """Round-trip: pandas -> YXDB -> pandas."""
        pdf_orig = pd.DataFrame({
            "a": [1.0, 2.0, 3.0],
            "b": ["x", "y", "z"],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb_pandas(path, pdf_orig)
        pdf_read = sigilyx.read_yxdb_pandas(path)
        pd.testing.assert_frame_equal(pdf_orig, pdf_read)


class TestWriteYxdbArrow:
    """Tests for write_yxdb_arrow functionality."""

    def test_write_arrow_table(self, tmp_path):
        """Write a PyArrow Table to YXDB."""
        table = pa.table({
            "id": [1, 2, 3],
            "name": ["Alice", "Bob", "Charlie"],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb_arrow(path, table)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == (3, 2)

    def test_roundtrip_arrow(self, tmp_path):
        """Round-trip: Arrow -> YXDB -> Arrow (data equality)."""
        table_orig = pa.table({
            "x": [10, 20, 30],
            "y": ["a", "b", "c"],
        })
        path = str(tmp_path / "test.yxdb")
        sigilyx.write_yxdb_arrow(path, table_orig)
        table_read = sigilyx.read_yxdb_arrow(path)
        # Compare data (not schema, as string type may differ)
        assert table_orig.to_pydict() == table_read.to_pydict()


class TestPolarsIntegration:
    """Tests for pl.read_yxdb(), df.write_yxdb(), etc."""

    def test_pl_read_yxdb_registered(self):
        """pl.read_yxdb() should be available after importing sigilyx."""
        assert hasattr(pl, 'read_yxdb')
        assert callable(pl.read_yxdb)

    def test_pl_scan_yxdb_registered(self):
        """pl.scan_yxdb() should be available after importing sigilyx."""
        assert hasattr(pl, 'scan_yxdb')
        assert callable(pl.scan_yxdb)

    def test_dataframe_write_yxdb_registered(self):
        """DataFrame.write_yxdb() should be available after importing sigilyx."""
        assert hasattr(pl.DataFrame, 'write_yxdb')
        assert callable(getattr(pl.DataFrame, 'write_yxdb'))

    def test_lazyframe_sink_yxdb_registered(self):
        """LazyFrame.sink_yxdb() should be available after importing sigilyx."""
        assert hasattr(pl.LazyFrame, 'sink_yxdb')
        assert callable(getattr(pl.LazyFrame, 'sink_yxdb'))

    def test_pl_read_yxdb_works(self):
        """pl.read_yxdb() should read YXDB files."""
        df = pl.read_yxdb(_yxdb("People.yxdb"))
        assert isinstance(df, pl.DataFrame)
        assert df.shape == (200, 8)

    def test_pl_scan_yxdb_works(self):
        """pl.scan_yxdb() should return a LazyFrame."""
        lf = pl.scan_yxdb(_yxdb("People.yxdb"))
        assert isinstance(lf, pl.LazyFrame)
        df = lf.filter(pl.col("Age") > 50).collect()
        assert isinstance(df, pl.DataFrame)
        assert len(df) < 200  # Filtered result

    def test_dataframe_write_yxdb_works(self, tmp_path):
        """df.write_yxdb() should write YXDB files."""
        df = pl.DataFrame({"x": [1, 2, 3], "y": ["a", "b", "c"]})
        path = str(tmp_path / "test.yxdb")
        df.write_yxdb(path)
        df2 = pl.read_yxdb(path)
        assert df.equals(df2)

    def test_lazyframe_sink_yxdb_works(self, tmp_path):
        """lf.sink_yxdb() should collect and write YXDB files."""
        lf = pl.LazyFrame({"a": [10, 20, 30], "b": [1.1, 2.2, 3.3]})
        path = str(tmp_path / "test.yxdb")
        lf.sink_yxdb(path)
        df = pl.read_yxdb(path)
        assert df.shape == (3, 2)
        assert df["a"].to_list() == [10, 20, 30]

    def test_roundtrip_via_polars_methods(self, tmp_path):
        """Full round-trip using pl.read_yxdb() and df.write_yxdb()."""
        # Read original
        df1 = pl.read_yxdb(_yxdb("People.yxdb"))
        
        # Write using method
        path = str(tmp_path / "roundtrip.yxdb")
        df1.write_yxdb(path)
        
        # Read back using function
        df2 = pl.read_yxdb(path)
        
        assert df1.equals(df2)
