"""Tests for sigilyx Python bindings.

Expected values are independently known from the test data generation process.
"""

import datetime
import os
from decimal import Decimal
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
        assert abs(df["DecimalCol"][0] - Decimal("1234.5678")) < Decimal("0.001")
        assert abs(df["DecimalCol"][1] - Decimal("-9999.9999")) < Decimal("0.001")

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
            "WString", "V_String", "V_WString", "Date",
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
        with pytest.raises(ValueError, match="(?i)invalid|not a valid"):
            sigilyx.read_yxdb(_yxdb("not_a_yxdb.txt"))

    def test_too_small_file(self):
        with pytest.raises((OSError, ValueError)):
            sigilyx.read_yxdb(_yxdb("too_small.bin"))

    def test_nonexistent_file(self):
        with pytest.raises(FileNotFoundError):
            sigilyx.read_yxdb(_yxdb("does_not_exist.yxdb"))

    def test_invalid_schema(self):
        with pytest.raises(FileNotFoundError):
            sigilyx.read_schema(_yxdb("does_not_exist.yxdb"))


class TestPolarsOperations:
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

    def test_columns_projection(self):
        """Only requested columns should appear in yielded batches."""
        cols = ["Id", "Value"]
        batches = list(
            sigilyx.read_yxdb_batches(
                _yxdb("ManyRecords.yxdb"), batch_size=500, columns=cols
            )
        )
        for batch in batches:
            assert batch.columns == cols
        total = sum(b.shape[0] for b in batches)
        assert total == sigilyx.record_count(_yxdb("ManyRecords.yxdb"))

    def test_n_rows_limit(self):
        """Iteration should stop after n_rows total rows."""
        limit = 100
        batches = list(
            sigilyx.read_yxdb_batches(
                _yxdb("ManyRecords.yxdb"), batch_size=30, n_rows=limit
            )
        )
        total = sum(b.shape[0] for b in batches)
        assert total == limit

    def test_columns_and_n_rows_combined(self):
        """columns and n_rows should work together."""
        cols = ["Id"]
        limit = 50
        batches = list(
            sigilyx.read_yxdb_batches(
                _yxdb("ManyRecords.yxdb"),
                batch_size=20,
                columns=cols,
                n_rows=limit,
            )
        )
        total = sum(b.shape[0] for b in batches)
        assert total == limit
        for batch in batches:
            assert batch.columns == cols


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
        """df.write_yxdb() should still work but emit a DeprecationWarning."""
        import warnings
        df = pl.DataFrame({"x": [1, 2, 3], "y": ["a", "b", "c"]})
        path = str(tmp_path / "test.yxdb")
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            df.write_yxdb(path)
            assert len(w) == 1
            assert issubclass(w[0].category, DeprecationWarning)
            assert "df.yxdb.write" in str(w[0].message)
        df2 = pl.read_yxdb(path)
        assert df.equals(df2)

    def test_lazyframe_sink_yxdb_works(self, tmp_path):
        """lf.sink_yxdb() should still work but emit a DeprecationWarning."""
        import warnings
        lf = pl.LazyFrame({"a": [10, 20, 30], "b": [1.1, 2.2, 3.3]})
        path = str(tmp_path / "test.yxdb")
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            lf.sink_yxdb(path)
            assert len(w) == 1
            assert issubclass(w[0].category, DeprecationWarning)
            assert "lf.yxdb.sink" in str(w[0].message)
        df = pl.read_yxdb(path)
        assert df.shape == (3, 2)
        assert df["a"].to_list() == [10, 20, 30]

    def test_namespace_write(self, tmp_path):
        """df.yxdb.write() should write YXDB files (official namespace API)."""
        df = pl.DataFrame({"x": [1, 2, 3], "y": ["a", "b", "c"]})
        path = str(tmp_path / "ns_test.yxdb")
        df.yxdb.write(path)
        df2 = pl.read_yxdb(path)
        assert df.equals(df2)

    def test_namespace_sink(self, tmp_path):
        """lf.yxdb.sink() should collect and write YXDB files (official namespace API)."""
        lf = pl.LazyFrame({"a": [10, 20, 30], "b": [1.1, 2.2, 3.3]})
        path = str(tmp_path / "ns_test.yxdb")
        lf.yxdb.sink(path)
        df = pl.read_yxdb(path)
        assert df.shape == (3, 2)
        assert df["a"].to_list() == [10, 20, 30]

    def test_roundtrip_via_polars_methods(self, tmp_path):
        """Full round-trip using pl.read_yxdb() and df.yxdb.write()."""
        # Read original
        df1 = pl.read_yxdb(_yxdb("People.yxdb"))
        
        # Write using namespace method
        path = str(tmp_path / "roundtrip.yxdb")
        df1.yxdb.write(path)
        
        # Read back using function
        df2 = pl.read_yxdb(path)
        
        assert df1.equals(df2)


# ---------------------------------------------------------------------------
#  Edge-case & stress tests
# ---------------------------------------------------------------------------


class TestWriteEdgeCases:
    """Boundary conditions, unusual data, and adversarial inputs for the writer."""

    def test_empty_dataframe(self, tmp_path):
        """Writing an empty DataFrame should create a valid file."""
        df = pl.DataFrame({"x": pl.Series([], dtype=pl.Int32)})
        path = str(tmp_path / "empty.yxdb")
        sigilyx.write_yxdb(path, df)
        # Verify the file was created and is valid by checking header
        import struct
        with open(path, "rb") as f:
            header = f.read(512)
        assert header[:21] == b"Alteryx Database File"
        num_records = struct.unpack_from("<Q", header, 104)[0]
        assert num_records == 0

    def test_single_row(self, tmp_path):
        """Single-row DataFrame round-trip."""
        df = pl.DataFrame({"a": [42], "b": ["only"]})
        path = str(tmp_path / "single.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.equals(df)

    def test_single_column_single_row(self, tmp_path):
        """Minimal DataFrame: 1 column, 1 row."""
        df = pl.DataFrame({"v": [True]})
        path = str(tmp_path / "tiny.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["v"][0] is True

    def test_all_null_column(self, tmp_path):
        """Column where every value is null."""
        df = pl.DataFrame({"n": [None, None, None]}).cast({"n": pl.Int64})
        path = str(tmp_path / "all_null.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["n"].null_count() == 3
        assert df2["n"].to_list() == [None, None, None]

    def test_all_null_string_column(self, tmp_path):
        """String column where every value is null."""
        df = pl.DataFrame({"s": [None, None]}).cast({"s": pl.String})
        path = str(tmp_path / "null_str.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].null_count() == 2

    def test_empty_strings(self, tmp_path):
        """Empty strings should round-trip as empty strings, not nulls."""
        df = pl.DataFrame({"s": ["", "", ""]})
        path = str(tmp_path / "empty_str.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == ["", "", ""]
        assert df2["s"].null_count() == 0

    def test_mixed_empty_and_null_strings(self, tmp_path):
        """Empty string and null should be distinguishable."""
        df = pl.DataFrame({"s": ["hello", "", None, "world", None, ""]})
        path = str(tmp_path / "mixed_str.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = df2["s"].to_list()
        assert vals[0] == "hello"
        assert vals[1] == ""
        assert vals[2] is None
        assert vals[3] == "world"
        assert vals[4] is None
        assert vals[5] == ""

    def test_unicode_column_names(self, tmp_path):
        """Column names with non-ASCII chars."""
        df = pl.DataFrame({"Ñame": [1], "Größe": [2], "日付": [3]})
        path = str(tmp_path / "unicode_cols.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.columns == ["Ñame", "Größe", "日付"]
        assert df2["Ñame"][0] == 1

    def test_column_name_with_xml_special_chars(self, tmp_path):
        """Column names with XML special chars (&, <, >, etc.) must be escaped."""
        df = pl.DataFrame({"A&B": [1], "C<D": [2], 'E"F': [3]})
        path = str(tmp_path / "xml_cols.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.columns == ["A&B", "C<D", 'E"F']

    def test_very_long_string(self, tmp_path):
        """String exceeding typical buffer sizes."""
        long_str = "A" * 100_000
        df = pl.DataFrame({"s": [long_str]})
        path = str(tmp_path / "long_str.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == long_str

    def test_unicode_strings_roundtrip(self, tmp_path):
        """Various Unicode planes round-trip correctly."""
        strings = [
            "Hello",                   # ASCII
            "café",                    # Latin-1 supplement
            "Ünïcödé",                 # Latin Extended
            "日本語",                   # CJK
            "العربية",                  # Arabic (RTL)
            "ĀĂĄĆĈ",                  # Latin Extended-A (U+0100 range - the SSE2 bug range)
            "αβγδε",                   # Greek
            "Привет",                  # Cyrillic
            "🎉🚀💻",                   # Emoji (supplementary plane)
            "\u0000",                  # Null character embedded in string
        ]
        df = pl.DataFrame({"text": strings})
        path = str(tmp_path / "unicode.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        for i, expected in enumerate(strings):
            assert df2["text"][i] == expected, f"Mismatch at index {i}: {df2['text'][i]!r} != {expected!r}"

    def test_int_boundary_values(self, tmp_path):
        """Min/max values for integer types."""
        df = pl.DataFrame({
            "i16": pl.Series([-(2**15), 2**15 - 1, 0], dtype=pl.Int16),
            "i32": pl.Series([-(2**31), 2**31 - 1, 0], dtype=pl.Int32),
            "i64": pl.Series([-(2**63), 2**63 - 1, 0], dtype=pl.Int64),
        })
        path = str(tmp_path / "int_bounds.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["i16"].to_list() == [-(2**15), 2**15 - 1, 0]
        assert df2["i32"].to_list() == [-(2**31), 2**31 - 1, 0]
        assert df2["i64"].to_list() == [-(2**63), 2**63 - 1, 0]

    def test_float_special_values(self, tmp_path):
        """NaN, inf, -inf, and denormalized floats."""
        import math
        df = pl.DataFrame({
            "f64": [float("inf"), float("-inf"), float("nan"), 0.0, -0.0, 5e-324],
        })
        path = str(tmp_path / "float_specials.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = df2["f64"].to_list()
        assert vals[0] == float("inf")
        assert vals[1] == float("-inf")
        assert math.isnan(vals[2])
        assert vals[3] == 0.0
        # -0.0 and 0.0 are equal in Python, check sign via copysign
        assert math.copysign(1, vals[4]) == math.copysign(1, -0.0)
        assert vals[5] == 5e-324

    def test_f32_special_values(self, tmp_path):
        """Float32 special values."""
        import math
        df = pl.DataFrame({
            "f32": pl.Series([float("inf"), float("-inf"), float("nan"), 0.0], dtype=pl.Float32),
        })
        path = str(tmp_path / "f32_specials.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = df2["f32"].to_list()
        assert vals[0] == float("inf")
        assert vals[1] == float("-inf")
        assert math.isnan(vals[2])
        assert vals[3] == 0.0

    def test_date_boundaries(self, tmp_path):
        """Edge dates: epoch, far past, far future."""
        df = pl.DataFrame({
            "d": [
                datetime.date(1970, 1, 1),   # epoch
                datetime.date(1, 1, 1),       # year 1
                datetime.date(9999, 12, 31),  # max year
                datetime.date(2000, 2, 29),   # leap day
            ],
        })
        path = str(tmp_path / "dates.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["d"].to_list() == df["d"].to_list()

    def test_datetime_boundaries(self, tmp_path):
        """Edge datetimes."""
        df = pl.DataFrame({
            "dt": [
                datetime.datetime(1970, 1, 1, 0, 0, 0),
                datetime.datetime(1969, 12, 31, 23, 59, 59),  # pre-epoch
                datetime.datetime(2000, 2, 29, 12, 0, 0),     # leap day noon
                datetime.datetime(2099, 12, 31, 23, 59, 59),   # far future
            ],
        })
        path = str(tmp_path / "datetimes.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["dt"].to_list() == df["dt"].to_list()

    def test_bool_with_nulls(self, tmp_path):
        """Bool column with all three states: true, false, null."""
        df = pl.DataFrame({"b": [True, False, None, True, None, False]})
        path = str(tmp_path / "bool_null.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["b"].to_list() == [True, False, None, True, None, False]

    def test_wide_dataframe(self, tmp_path):
        """DataFrame with many columns (100+)."""
        data = {f"col_{i:03d}": [i * 10] for i in range(100)}
        df = pl.DataFrame(data)
        path = str(tmp_path / "wide.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == (1, 100)
        assert df2.columns == df.columns
        for col in df.columns:
            assert df2[col][0] == df[col][0]

    def test_many_rows_varied_types(self, tmp_path):
        """10k rows across multiple types to stress block boundaries."""
        n = 10_000
        df = pl.DataFrame({
            "id": list(range(n)),
            "val": [float(i) * 0.1 for i in range(n)],
            "flag": [i % 3 == 0 for i in range(n)],
            "text": [f"row_{i:05d}" for i in range(n)],
        })
        path = str(tmp_path / "many_varied.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape
        assert df2["id"].to_list() == df["id"].to_list()
        assert df2["text"].to_list() == df["text"].to_list()


class TestReadEdgeCases:
    """Edge-case tests for the reader side."""

    def test_record_count_matches_data(self):
        """record_count should match actual row count for all test files."""
        for name in ["AllTypes.yxdb", "NullValues.yxdb", "ManyRecords.yxdb",
                      "LargeBlob.yxdb", "People.yxdb", "Strings.yxdb",
                      "SingleColumn.yxdb"]:
            expected_count = sigilyx.record_count(_yxdb(name))
            df = sigilyx.read_yxdb(_yxdb(name))
            assert df.height == expected_count, f"{name}: height {df.height} != count {expected_count}"

    def test_batched_vs_full_read_all_files(self):
        """Batched read should produce identical data to full read for all test files."""
        for name in ["AllTypes.yxdb", "NullValues.yxdb", "People.yxdb",
                      "SingleColumn.yxdb"]:
            df_full = sigilyx.read_yxdb(_yxdb(name))
            batches = list(sigilyx.read_yxdb_batches(_yxdb(name), batch_size=1))
            df_batched = pl.concat(batches) if batches else pl.DataFrame()
            assert df_full.equals(df_batched), f"{name}: batched read differs from full read"

    def test_batch_size_one(self):
        """batch_size=1 should yield one row per batch."""
        batches = list(sigilyx.read_yxdb_batches(_yxdb("SingleColumn.yxdb"), batch_size=1))
        assert len(batches) == 5
        for b in batches:
            assert b.shape[0] == 1

    def test_batch_size_larger_than_file(self):
        """batch_size > total rows should yield a single batch."""
        batches = list(sigilyx.read_yxdb_batches(_yxdb("SingleColumn.yxdb"), batch_size=999_999))
        assert len(batches) == 1
        assert batches[0].shape[0] == 5

    def test_n_rows_zero(self):
        """n_rows=0 should yield nothing."""
        batches = list(sigilyx.read_yxdb_batches(_yxdb("ManyRecords.yxdb"), n_rows=0))
        total = sum(b.shape[0] for b in batches)
        assert total == 0

    def test_n_rows_one(self):
        """n_rows=1 should yield exactly 1 row."""
        batches = list(sigilyx.read_yxdb_batches(_yxdb("ManyRecords.yxdb"), batch_size=1, n_rows=1))
        total = sum(b.shape[0] for b in batches)
        assert total == 1

    def test_schema_dtypes_stable(self):
        """Schema types should be deterministic across repeated reads."""
        df1 = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        df2 = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        assert df1.dtypes == df2.dtypes

    def test_read_columns_nonexistent_column(self):
        """Requesting a column that doesn't exist should raise ValueError."""
        with pytest.raises(ValueError, match="not found"):
            sigilyx.read_yxdb_columns(_yxdb("SingleColumn.yxdb"), ["NoSuchColumn"])

    def test_read_columns_empty_list(self):
        """Requesting zero columns should either raise or return 0-width frame."""
        try:
            df = sigilyx.read_yxdb_columns(_yxdb("SingleColumn.yxdb"), [])
            # If it succeeds, it should have no columns
            assert df.width == 0
        except Exception:
            # Also acceptable to raise
            pass

    def test_scan_head_zero(self):
        """Scanning with head(0) should return 0 rows."""
        df = sigilyx.scan_yxdb(_yxdb("ManyRecords.yxdb")).head(0).collect()
        assert df.height == 0

    def test_scan_select_single_column(self):
        """Projection pushdown to a single column."""
        df = sigilyx.scan_yxdb(_yxdb("People.yxdb")).select("FirstName").collect()
        assert df.columns == ["FirstName"]
        assert df.height == 200


class TestStreamingWriterEdgeCases:
    """Edge cases for the batched/streaming writer."""

    def test_write_batches_simple(self, tmp_path):
        """write_yxdb_batches with a list of DataFrames."""
        batches = [
            pl.DataFrame({"x": [1, 2]}),
            pl.DataFrame({"x": [3, 4]}),
            pl.DataFrame({"x": [5]}),
        ]
        path = str(tmp_path / "batched.yxdb")
        n = sigilyx.write_yxdb_batches(path, iter(batches))
        assert n == 5
        df = sigilyx.read_yxdb(path)
        assert df["x"].to_list() == [1, 2, 3, 4, 5]

    def test_write_batches_single_batch(self, tmp_path):
        """write_yxdb_batches with exactly one batch."""
        batches = [pl.DataFrame({"a": [10, 20, 30]})]
        path = str(tmp_path / "one_batch.yxdb")
        n = sigilyx.write_yxdb_batches(path, iter(batches))
        assert n == 3
        df = sigilyx.read_yxdb(path)
        assert df["a"].to_list() == [10, 20, 30]

    def test_write_batches_empty_batches_interspersed(self, tmp_path):
        """Empty batches in the stream should be silently skipped."""
        batches = [
            pl.DataFrame({"v": pl.Series([], dtype=pl.Int64)}),
            pl.DataFrame({"v": [1, 2]}),
            pl.DataFrame({"v": pl.Series([], dtype=pl.Int64)}),
            pl.DataFrame({"v": [3]}),
            pl.DataFrame({"v": pl.Series([], dtype=pl.Int64)}),
        ]
        path = str(tmp_path / "with_empties.yxdb")
        n = sigilyx.write_yxdb_batches(path, iter(batches))
        assert n == 3
        df = sigilyx.read_yxdb(path)
        assert df["v"].to_list() == [1, 2, 3]

    def test_write_then_read_batches_consistency(self, tmp_path):
        """Data written via batches should match when read back via batches."""
        write_batches = [
            pl.DataFrame({"id": list(range(i * 100, (i + 1) * 100))})
            for i in range(10)
        ]
        path = str(tmp_path / "batch_consistency.yxdb")
        sigilyx.write_yxdb_batches(path, iter(write_batches))

        read_batches = list(sigilyx.read_yxdb_batches(path, batch_size=100))
        df = pl.concat(read_batches)
        assert df["id"].to_list() == list(range(1000))


class TestCrossFormatConsistency:
    """Verify Polars, Arrow, and Pandas readers all agree."""

    def test_all_formats_same_shape(self):
        for name in ["AllTypes.yxdb", "People.yxdb", "SingleColumn.yxdb"]:
            df_pl = sigilyx.read_yxdb(_yxdb(name))
            tbl_pa = sigilyx.read_yxdb_arrow(_yxdb(name))
            df_pd = sigilyx.read_yxdb_pandas(_yxdb(name))
            assert df_pl.shape == (tbl_pa.num_rows, tbl_pa.num_columns), \
                f"{name}: Polars shape != Arrow shape"
            assert df_pl.shape == df_pd.shape, \
                f"{name}: Polars shape != Pandas shape"

    def test_all_formats_same_column_names(self):
        for name in ["AllTypes.yxdb", "People.yxdb"]:
            df_pl = sigilyx.read_yxdb(_yxdb(name))
            tbl_pa = sigilyx.read_yxdb_arrow(_yxdb(name))
            df_pd = sigilyx.read_yxdb_pandas(_yxdb(name))
            assert df_pl.columns == tbl_pa.column_names
            assert df_pl.columns == list(df_pd.columns)

    def test_int_values_agree(self):
        """Integer values should be identical across formats."""
        df_pl = sigilyx.read_yxdb(_yxdb("SingleColumn.yxdb"))
        tbl_pa = sigilyx.read_yxdb_arrow(_yxdb("SingleColumn.yxdb"))
        df_pd = sigilyx.read_yxdb_pandas(_yxdb("SingleColumn.yxdb"))
        expected = [10, 20, 30, 40, 50]
        assert df_pl["Value"].to_list() == expected
        assert tbl_pa.column("Value").to_pylist() == expected
        assert list(df_pd["Value"]) == expected


class TestVersionAndMeta:
    """Package metadata sanity checks."""

    def test_version_exists(self):
        assert hasattr(sigilyx, "__version__")
        assert isinstance(sigilyx.__version__, str)
        assert len(sigilyx.__version__) > 0

    def test_all_exports_exist(self):
        """Every name in __all__ should actually be importable."""
        for name in sigilyx.__all__:
            assert hasattr(sigilyx, name), f"{name} in __all__ but not in module"

    def test_read_alias_equals_read_yxdb(self):
        assert sigilyx.read is sigilyx.read_yxdb

    def test_write_alias_equals_write_yxdb(self):
        assert sigilyx.write is sigilyx.write_yxdb

    def test_scan_alias_equals_scan_yxdb(self):
        assert sigilyx.scan is sigilyx.scan_yxdb


class TestRobustness:
    """Stress tests and unusual but valid usage patterns."""

    def test_read_same_file_many_times(self):
        """Repeated reads should always return the same result."""
        results = [sigilyx.read_yxdb(_yxdb("SingleColumn.yxdb")) for _ in range(20)]
        for df in results[1:]:
            assert results[0].equals(df)

    def test_write_read_cycle_ten_times(self, tmp_path):
        """Repeated write -> read should be stable (no drift)."""
        df = pl.DataFrame({"x": [1, 2, 3], "y": [1.5, 2.5, 3.5], "z": ["a", "b", "c"]})
        for i in range(10):
            path = str(tmp_path / f"cycle_{i}.yxdb")
            sigilyx.write_yxdb(path, df)
            df = sigilyx.read_yxdb(path)
        assert df["x"].to_list() == [1, 2, 3]
        assert df["z"].to_list() == ["a", "b", "c"]

    def test_concurrent_reads(self):
        """Multiple threads reading the same file simultaneously."""
        import concurrent.futures
        path = _yxdb("People.yxdb")
        def read_it():
            return sigilyx.read_yxdb(path).shape
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda _: read_it(), range(8)))
        assert all(r == (200, 8) for r in results)

    def test_large_batch_count(self, tmp_path):
        """Writing 500 tiny batches via streaming writer."""
        batches = [pl.DataFrame({"v": [i]}) for i in range(500)]
        path = str(tmp_path / "many_batches.yxdb")
        n = sigilyx.write_yxdb_batches(path, iter(batches))
        assert n == 500
        df = sigilyx.read_yxdb(path)
        assert df["v"].to_list() == list(range(500))

    def test_alternating_null_values(self, tmp_path):
        """Every other value is null - exercises null bitmask handling."""
        n = 200
        vals = [i if i % 2 == 0 else None for i in range(n)]
        df = pl.DataFrame({"v": vals}).cast({"v": pl.Int64})
        path = str(tmp_path / "alternating_nulls.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["v"].to_list() == vals

    def test_all_types_columns_projection(self):
        """Column projection should work for every column individually."""
        df_full = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        for col_name in df_full.columns:
            df_proj = sigilyx.read_yxdb_columns(_yxdb("AllTypes.yxdb"), [col_name])
            assert df_proj.columns == [col_name]
            assert df_proj.height == df_full.height

    def test_row_reader_all_types(self):
        """Row reader should return values for every field in AllTypes."""
        reader = sigilyx.YxdbRowReader(_yxdb("AllTypes.yxdb"))
        rows = []
        while reader.next():
            rows.append(reader.read_dict())
        assert len(rows) == 2
        # First row should have known values
        assert rows[0]["Int32Col"] == 42000
        assert rows[0]["StringCol"] == "Alteryx"
        assert rows[0]["BoolCol"] is True

    def test_row_reader_sequential_iteration(self):
        """Row reader count matches record_count."""
        reader = sigilyx.YxdbRowReader(_yxdb("ManyRecords.yxdb"))
        count = 0
        while reader.next():
            count += 1
        assert count == 50_000
