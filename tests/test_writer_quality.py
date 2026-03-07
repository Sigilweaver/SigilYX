"""Writer quality tests: file size regression, type fidelity, cross-impl validation.

These tests verify that the writer produces compact, correct output that is
compatible with the Alteryx ecosystem. File size baselines catch compression
regressions. Type fidelity tests ensure the correct YXDB field types are
chosen. Cross-implementation tests validate output against the Alteryx C++
dump tool.
"""

import os
import subprocess
import tempfile
from pathlib import Path

import polars as pl
import pytest

import sigilyx

TEST_DIR = Path(__file__).parent.parent / "sigilyx" / "test_files"
DUMP_EXE = Path(__file__).parent.parent / "benchmarks" / "cpp" / "alteryx_openyxdb_dump.exe"

HAS_DUMP_TOOL = DUMP_EXE.exists()


def _yxdb(name: str) -> str:
    return str(TEST_DIR / name)


def _write_roundtrip(src: str) -> tuple[pl.DataFrame, str]:
    """Read a test file, write roundtrip to temp, return (df, temp_path)."""
    df = sigilyx.read_yxdb(src)
    tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
    tmp.close()
    sigilyx.write_yxdb(tmp.name, df)
    return df, tmp.name


def _run_dump(path: str) -> tuple[list[str], list[str], list[list[str]]]:
    """Run the Alteryx C++ dump tool and return (names, types, rows)."""
    r = subprocess.run(
        [str(DUMP_EXE), path], capture_output=True, timeout=60
    )
    assert r.returncode == 0, f"dump failed: {r.stderr.decode('utf-8', errors='replace')}"
    lines = r.stdout.decode("utf-8").splitlines()
    names = lines[0].split("\t")
    types = lines[1].split("\t")
    rows = [l.split("\t") for l in lines[2:]]
    return names, types, rows


# ── File size baselines ─────────────────────────────────────────────────


# Baselines are the SigilYX roundtrip file sizes after all optimisations.
# Update these when intentional format changes occur.
SIZE_BASELINES = {
    "AllTypes.yxdb": 3_200,
    "NullValues.yxdb": 2_050,
    "ManyRecords.yxdb": 535_000,
    "Strings.yxdb": 1_600,
    "People.yxdb": 8_400,
    "SingleColumn.yxdb": 750,
}

# Allow 10% tolerance in either direction
SIZE_TOLERANCE = 0.10


class TestFileSizeBaselines:
    """Ensure roundtrip file sizes stay within known baselines."""

    @pytest.fixture(params=list(SIZE_BASELINES.keys()))
    def test_file(self, request):
        return request.param

    def test_roundtrip_size_within_baseline(self, test_file):
        src = _yxdb(test_file)
        _, tmp = _write_roundtrip(src)
        try:
            actual = os.path.getsize(tmp)
            baseline = SIZE_BASELINES[test_file]
            lo = baseline * (1 - SIZE_TOLERANCE)
            hi = baseline * (1 + SIZE_TOLERANCE)
            assert lo <= actual <= hi, (
                f"{test_file}: roundtrip size {actual:,} outside "
                f"[{lo:,.0f}, {hi:,.0f}] (baseline {baseline:,})"
            )
        finally:
            os.unlink(tmp)

    def test_roundtrip_not_larger_than_original(self, test_file):
        """Roundtrip should generally not be larger than the original file."""
        src = _yxdb(test_file)
        orig_size = os.path.getsize(src)
        _, tmp = _write_roundtrip(src)
        try:
            rt_size = os.path.getsize(tmp)
            # Allow up to 5% larger (different encoding decisions may cause minor growth)
            assert rt_size <= orig_size * 1.05, (
                f"{test_file}: roundtrip {rt_size:,} is more than 5% larger "
                f"than original {orig_size:,} ({rt_size/orig_size:.2f}x)"
            )
        finally:
            os.unlink(tmp)


# ── Type fidelity tests ─────────────────────────────────────────────────


class TestTypeFidelity:
    """Verify that infer_schema produces the correct YXDB field types."""

    def test_ascii_string_uses_vstring(self):
        """ASCII-only string columns should use V_String, not V_WString."""
        df = pl.DataFrame({"name": ["Alice", "Bob", "Charlie"]})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "V_String"
        finally:
            os.unlink(tmp.name)

    def test_non_latin1_string_uses_vwstring(self):
        """Strings with non-Latin-1 characters should use V_WString."""
        df = pl.DataFrame({"text": ["hello", "world", "\u4e16\u754c"]})  # contains CJK
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "V_WString"
        finally:
            os.unlink(tmp.name)

    def test_vstring_size_reflects_max_length(self):
        """V_String size should match the actual max string byte length."""
        df = pl.DataFrame({"short": ["a", "bb", "ccc"], "long": ["x" * 100, "y", "z"]})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].size == 3  # max of "ccc"
            assert fields[1].size == 100  # max of "x" * 100
        finally:
            os.unlink(tmp.name)

    def test_vwstring_size_reflects_max_char_count(self):
        """V_WString size should match the max UTF-16 code-unit count."""
        # Each CJK char is 1 UTF-16 code unit, "hello" is 5
        df = pl.DataFrame({"text": ["\u4e16\u754c\u4f60\u597d", "hello"]})  # 4 chars, 5 chars
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "V_WString"
            assert fields[0].size == 5  # max of 4 and 5 UTF-16 code units
        finally:
            os.unlink(tmp.name)

    def test_int_types_produce_correct_fields(self):
        """Integer Polars dtypes should map to correct YXDB types."""
        df = pl.DataFrame({
            "i16": pl.Series([1], dtype=pl.Int16),
            "i32": pl.Series([1], dtype=pl.Int32),
            "i64": pl.Series([1], dtype=pl.Int64),
        })
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "Int16"
            assert fields[1].field_type == "Int32"
            assert fields[2].field_type == "Int64"
        finally:
            os.unlink(tmp.name)

    def test_float_types_produce_correct_fields(self):
        """Float Polars dtypes should map to correct YXDB types."""
        df = pl.DataFrame({
            "f32": pl.Series([1.0], dtype=pl.Float32),
            "f64": pl.Series([1.0], dtype=pl.Float64),
        })
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "Float"
            assert fields[1].field_type == "Double"
        finally:
            os.unlink(tmp.name)

    def test_bool_produces_bool_field(self):
        df = pl.DataFrame({"flag": [True, False, None]})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "Bool"
        finally:
            os.unlink(tmp.name)

    def test_date_produces_date_field(self):
        df = pl.DataFrame({"d": pl.Series([18628, 19889], dtype=pl.Date)})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            fields = sigilyx.read_yxdb_fields(tmp.name)
            assert fields[0].field_type == "Date"
        finally:
            os.unlink(tmp.name)

    def test_vstring_smaller_than_vwstring(self):
        """V_String file should be smaller than forced V_WString for ASCII data."""
        data = ["hello world " * 10] * 500
        df = pl.DataFrame({"text": data})

        tmp_narrow = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp_narrow.close()
        try:
            sigilyx.write_yxdb(tmp_narrow.name, df)
            size_narrow = os.path.getsize(tmp_narrow.name)
        finally:
            os.unlink(tmp_narrow.name)

        # Force V_WString by adding a non-Latin-1 char in one row
        data_wide = data.copy()
        data_wide[0] = data_wide[0] + "\u4e16"
        df_wide = pl.DataFrame({"text": data_wide})

        tmp_wide = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp_wide.close()
        try:
            sigilyx.write_yxdb(tmp_wide.name, df_wide)
            size_wide = os.path.getsize(tmp_wide.name)
        finally:
            os.unlink(tmp_wide.name)

        assert size_narrow < size_wide, (
            f"V_String ({size_narrow:,}) should be smaller than V_WString ({size_wide:,})"
        )


# ── Data roundtrip correctness ──────────────────────────────────────────


ROUNDTRIP_FILES = [
    "AllTypes.yxdb",
    "NullValues.yxdb",
    "ManyRecords.yxdb",
    "Strings.yxdb",
    "People.yxdb",
    "SingleColumn.yxdb",
]


class TestRoundtripCorrectness:
    """Verify data integrity across read-write-read cycles."""

    @pytest.fixture(params=ROUNDTRIP_FILES)
    def test_file(self, request):
        return request.param

    def test_polars_df_equals(self, test_file):
        """Read -> write -> read should produce identical DataFrames."""
        src = _yxdb(test_file)
        df, tmp = _write_roundtrip(src)
        try:
            df2 = sigilyx.read_yxdb(tmp)
            assert df.equals(df2), f"{test_file}: roundtrip data mismatch"
        finally:
            os.unlink(tmp)

    @pytest.mark.skipif(not HAS_DUMP_TOOL, reason="Alteryx C++ dump tool not available")
    def test_cpp_dump_values_match(self, test_file):
        """Values should match when read by the Alteryx C++ dump tool."""
        src = _yxdb(test_file)
        _, tmp = _write_roundtrip(src)
        try:
            orig_names, orig_types, orig_rows = _run_dump(src)
            rt_names, rt_types, rt_rows = _run_dump(tmp)

            assert len(orig_rows) == len(rt_rows), (
                f"{test_file}: row count mismatch {len(orig_rows)} vs {len(rt_rows)}"
            )

            mismatches = 0
            for ri, (r1, r2) in enumerate(zip(orig_rows, rt_rows)):
                for ci, (v1, v2) in enumerate(zip(r1, r2)):
                    if v1 != v2:
                        try:
                            if abs(float(v1) - float(v2)) < 1e-10:
                                continue
                        except (ValueError, TypeError):
                            pass
                        mismatches += 1

            assert mismatches == 0, (
                f"{test_file}: {mismatches} value mismatches in C++ dump comparison"
            )
        finally:
            os.unlink(tmp)


# ── LZF compression quality ─────────────────────────────────────────────


class TestLzfCompression:
    """Verify LZF compression produces reasonably compact output."""

    def test_highly_compressible_data(self):
        """Repeating patterns should compress well."""
        # 10,000 rows of the same short string = highly compressible
        df = pl.DataFrame({"text": ["hello world"] * 10_000})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            size = os.path.getsize(tmp.name)
            # Uncompressed variable data alone would be ~120KB (11 bytes * 10000)
            # plus fixed portion ~40KB. With good LZF compression this should
            # be well under 10KB.
            assert size < 10_000, (
                f"Highly compressible data produced {size:,} bytes (expected < 10,000)"
            )
        finally:
            os.unlink(tmp.name)

    def test_numeric_data_compresses(self):
        """Numeric columns should compress reasonably."""
        df = pl.DataFrame({
            "id": list(range(50_000)),
            "value": [float(i) * 0.1 for i in range(50_000)],
        })
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            size = os.path.getsize(tmp.name)
            # Raw data: 50000 * (5 + 9) = 700KB. Some compression expected.
            assert size < 600_000, (
                f"Numeric data produced {size:,} bytes (expected < 600,000)"
            )
        finally:
            os.unlink(tmp.name)


# ── Cross-implementation type validation ─────────────────────────────────


# Expected YXDB type for each Polars dtype when written by SigilYX
EXPECTED_TYPE_MAP = {
    "Int16": "Int16",
    "Int32": "Int32",
    "Int64": "Int64",
    "Float": "Float",
    "Double": "Double",
    "Bool": "Bool",
    "Date": "Date",
    "V_String": "V_String",
    "V_WString": "V_WString",
}


@pytest.mark.skipif(not HAS_DUMP_TOOL, reason="Alteryx C++ dump tool not available")
class TestCrossImplTypeValidation:
    """Validate that Alteryx C++ reads our field types correctly."""

    def test_basic_types_roundtrip(self):
        """Write known types and verify C++ reads them as expected."""
        df = pl.DataFrame({
            "int_col": pl.Series([1, 2, 3], dtype=pl.Int32),
            "float_col": pl.Series([1.0, 2.0, 3.0], dtype=pl.Float64),
            "str_col": ["hello", "world", "test"],
            "bool_col": [True, False, True],
        })
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            names, types, rows = _run_dump(tmp.name)

            type_map = dict(zip(names, types))
            assert type_map["int_col"] == "Int32"
            assert type_map["float_col"] == "Double"
            assert type_map["str_col"] == "V_String"
            assert type_map["bool_col"] == "Bool"

            assert len(rows) == 3
        finally:
            os.unlink(tmp.name)

    def test_unicode_string_type(self):
        """Non-Latin-1 strings should be V_WString in C++ dump."""
        df = pl.DataFrame({"text": ["hello", "\u4e16\u754c"]})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            names, types, _ = _run_dump(tmp.name)
            type_map = dict(zip(names, types))
            assert type_map["text"] == "V_WString"
        finally:
            os.unlink(tmp.name)

    def test_date_type(self):
        """Date columns should be Date in C++ dump."""
        df = pl.DataFrame({"d": pl.Series([18628, 19889], dtype=pl.Date)})
        tmp = tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False)
        tmp.close()
        try:
            sigilyx.write_yxdb(tmp.name, df)
            names, types, _ = _run_dump(tmp.name)
            type_map = dict(zip(names, types))
            assert type_map["d"] == "Date"
        finally:
            os.unlink(tmp.name)
