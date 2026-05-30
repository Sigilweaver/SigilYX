"""Alteryx OpenYXDB cross-implementation round-trip tests.

These tests write YXDB files with SigilYX and verify them against the official
Alteryx OpenYXDB C++ library (via the dump tool). This is the ground-truth
validation: if Alteryx's own library can read our files and produce identical
values, we know the binary format is correct.

Tests are organized into 4 tiers:
1. Read verification — both implementations read the same test files
2. Write verification — SigilYX writes, Alteryx reads
3. Full roundtrip — SigilYX write→read, verified against Alteryx read→read
4. Targeted type coverage — specific field types that are tricky

Requirements:
    - C++ dump tool (built separately, not bundled with the repo)
    - Build with the C++ harness from an earlier benchmark release

All tests are skipped if the dump tool is not available, so CI can run
without the C++ toolchain.
"""

import os
import struct
import subprocess
import tempfile
import warnings
from pathlib import Path

import polars as pl
import pytest

import sigilyx

# ── Configuration ──────────────────────────────────────────────────────

SCRIPT_DIR = Path(__file__).parent
PROJECT_ROOT = SCRIPT_DIR.parent
BENCHMARK_DIR = PROJECT_ROOT / "benchmarks"
ALTERYX_DUMP_EXE = BENCHMARK_DIR / "cpp" / "alteryx_openyxdb_dump.exe"
TEST_FILES_DIR = PROJECT_ROOT / "sigilyx" / "test_files"

# Skip everything if the C++ dump tool isn't built
_DUMP_TOOL_MISSING = not ALTERYX_DUMP_EXE.exists()
pytestmark = pytest.mark.skipif(
    _DUMP_TOOL_MISSING,
    reason=f"Alteryx OpenYXDB dump tool not found at {ALTERYX_DUMP_EXE}. "
           f"Build with: benchmarks\\cpp\\build_alteryx_dump.bat",
)

# Warn once when running locally (not CI) without the dump tool
if _DUMP_TOOL_MISSING and not os.environ.get("CI"):
    warnings.warn(
        f"Alteryx cross-implementation tests skipped: dump tool not found at "
        f"{ALTERYX_DUMP_EXE}. Build with: benchmarks\\cpp\\build_alteryx_dump.bat",
        stacklevel=1,
    )


def _yxdb(name: str) -> str:
    return str(TEST_FILES_DIR / name)


# ── C++ dump tool interface ───────────────────────────────────────────


def run_alteryx_dump(yxdb_path: str) -> dict:
    """Run the Alteryx OpenYXDB dump tool and parse TSV output.

    Returns a dict: {field_names, field_types, rows}
    Each row is a list of string|None values.
    """
    result = subprocess.run(
        [str(ALTERYX_DUMP_EXE), yxdb_path],
        capture_output=True,
        timeout=60,
    )
    if result.returncode != 0:
        stderr = result.stderr.decode("utf-8", errors="replace")
        raise RuntimeError(
            f"Alteryx dump tool failed (exit {result.returncode}): {stderr}"
        )

    lines = result.stdout.decode("utf-8").splitlines()
    assert len(lines) >= 2, f"Dump tool output too short: {len(lines)} lines"

    field_names = lines[0].split("\t")
    field_types = lines[1].split("\t")

    rows = []
    for line in lines[2:]:
        values = []
        for cell in line.split("\t"):
            if cell == "\\N":
                values.append(None)
            else:
                cell = cell.replace("\\t", "\t")
                cell = cell.replace("\\n", "\n")
                cell = cell.replace("\\r", "\r")
                cell = cell.replace("\\\\", "\\")
                values.append(cell)
        rows.append(values)

    return {"field_names": field_names, "field_types": field_types, "rows": rows}


def read_sigilyx_rows(yxdb_path: str) -> dict:
    """Read via SigilYX row reader, returning same format as C++ dump."""
    reader = sigilyx.YxdbRowReader(yxdb_path)
    fields = reader.fields

    field_names = [f.name for f in fields]
    field_types = [f.field_type for f in fields]

    rows = []
    while reader.next():
        row_dict = reader.read_dict()
        row = []
        for field in fields:
            val = row_dict[field.name]
            row.append(_format_value(val, field.field_type))
        rows.append(row)

    reader.close()
    return {"field_names": field_names, "field_types": field_types, "rows": rows}


def _format_value(val, field_type: str) -> str | None:
    """Convert Python value to string matching C++ dump output."""
    if val is None:
        return None
    if field_type == "Bool":
        return "true" if val else "false"
    if field_type in ("Byte", "Int16", "Int32"):
        return str(int(val))
    if field_type == "Int64":
        return str(int(val))
    if field_type in ("Float", "Double"):
        return f"{float(val):.17g}"
    if field_type == "FixedDecimal":
        return str(val)
    if field_type in ("Date", "Time", "DateTime"):
        return str(val)
    if field_type in ("Blob", "SpatialObj"):
        if isinstance(val, (bytes, bytearray)):
            return f"[blob:{len(val)}]"
        return str(val)
    return str(val)


# ── Type mapping ──────────────────────────────────────────────────────

SIGILYX_TO_CPP_TYPE = {
    "Bool": "Bool", "Byte": "Byte", "Int16": "Int16",
    "Int32": "Int32", "Int64": "Int64", "Float": "Float",
    "Double": "Double", "FixedDecimal": "FixedDecimal",
    "String": "String", "WString": "WString",
    "VString": "V_String", "VWString": "V_WString",
    "Date": "Date", "Time": "Time", "DateTime": "DateTime",
    "Blob": "Blob", "SpatialObj": "SpatialObj",
}


def _types_match(sigilyx_type: str, cpp_type: str) -> bool:
    expected = SIGILYX_TO_CPP_TYPE.get(sigilyx_type, sigilyx_type)
    return expected == cpp_type


# ── Comparison engine ─────────────────────────────────────────────────


def _compare_values(
    s_val: str | None, c_val: str | None, field_type: str
) -> bool:
    """Compare a single value from SigilYX vs C++."""
    if s_val is None and c_val is None:
        return True
    if s_val is None or c_val is None:
        return False

    if field_type in ("Float", "Double"):
        try:
            s, c = float(s_val), float(c_val)
            if s == c:
                return True
            if abs(s) > 0:
                return abs(s - c) / abs(s) < 1e-12
            return abs(s - c) < 1e-15
        except ValueError:
            return s_val == c_val

    if field_type == "FixedDecimal":
        try:
            return abs(float(s_val) - float(c_val)) < 1e-6
        except ValueError:
            return s_val == c_val

    return s_val == c_val


def assert_results_match(
    sigilyx_data: dict,
    cpp_data: dict,
    context: str = "",
    strict_types: bool = True,
):
    """Assert that SigilYX and C++ dump outputs are identical.

    Raises AssertionError with details on first mismatch.
    """
    prefix = f"[{context}] " if context else ""

    assert len(sigilyx_data["field_names"]) == len(cpp_data["field_names"]), (
        f"{prefix}Field count: SigilYX={len(sigilyx_data['field_names'])}, "
        f"C++={len(cpp_data['field_names'])}"
    )

    for i, (sn, cn) in enumerate(
        zip(sigilyx_data["field_names"], cpp_data["field_names"])
    ):
        assert sn == cn, f"{prefix}Field {i} name: SigilYX={sn!r}, C++={cn!r}"

    if strict_types:
        for i, (st, ct) in enumerate(
            zip(sigilyx_data["field_types"], cpp_data["field_types"])
        ):
            assert _types_match(st, ct), (
                f"{prefix}Field {i} type: SigilYX={st!r}, C++={ct!r}"
            )

    assert len(sigilyx_data["rows"]) == len(cpp_data["rows"]), (
        f"{prefix}Row count: SigilYX={len(sigilyx_data['rows'])}, "
        f"C++={len(cpp_data['rows'])}"
    )

    for row_idx, (s_row, c_row) in enumerate(
        zip(sigilyx_data["rows"], cpp_data["rows"])
    ):
        assert len(s_row) == len(c_row), (
            f"{prefix}Row {row_idx} col count: SigilYX={len(s_row)}, C++={len(c_row)}"
        )
        for col_idx, (sv, cv) in enumerate(zip(s_row, c_row)):
            cpp_type = cpp_data["field_types"][col_idx]
            fname = cpp_data["field_names"][col_idx]
            assert _compare_values(sv, cv, cpp_type), (
                f"{prefix}Row {row_idx}, col {col_idx} ({fname}): "
                f"SigilYX={sv!r} vs C++={cv!r}"
            )


# ============================================================================
#  Tier 1: Read existing test files — both impls should agree
# ============================================================================

READ_TEST_FILES = [
    "AllTypes.yxdb",
    "NullValues.yxdb",
    "ManyRecords.yxdb",
    "Strings.yxdb",
    "People.yxdb",
    "SingleColumn.yxdb",
]


class TestReadExistingFiles:
    """Read each test file with both SigilYX and Alteryx, compare all values."""

    @pytest.mark.parametrize("filename", READ_TEST_FILES)
    def test_read_matches_alteryx(self, filename):
        path = _yxdb(filename)
        if not Path(path).exists():
            pytest.skip(f"{filename} not found")

        cpp_data = run_alteryx_dump(path)
        sigilyx_data = read_sigilyx_rows(path)
        assert_results_match(sigilyx_data, cpp_data, context=filename)

    @pytest.mark.parametrize("filename", READ_TEST_FILES)
    def test_field_names_match(self, filename):
        """Field names from both impls should be identical."""
        path = _yxdb(filename)
        if not Path(path).exists():
            pytest.skip(f"{filename} not found")

        cpp_data = run_alteryx_dump(path)
        sigilyx_data = read_sigilyx_rows(path)
        assert sigilyx_data["field_names"] == cpp_data["field_names"]

    @pytest.mark.parametrize("filename", READ_TEST_FILES)
    def test_row_counts_match(self, filename):
        """Row counts from both impls should be identical."""
        path = _yxdb(filename)
        if not Path(path).exists():
            pytest.skip(f"{filename} not found")

        cpp_data = run_alteryx_dump(path)
        sigilyx_data = read_sigilyx_rows(path)
        assert len(sigilyx_data["rows"]) == len(cpp_data["rows"])

    def test_large_blob_file(self):
        """LargeBlob.yxdb — verify blob sizes match between impls."""
        path = _yxdb("LargeBlob.yxdb")
        if not Path(path).exists():
            pytest.skip("LargeBlob.yxdb not found")

        cpp_data = run_alteryx_dump(path)
        sigilyx_data = read_sigilyx_rows(path)
        assert_results_match(sigilyx_data, cpp_data, context="LargeBlob.yxdb")


# ============================================================================
#  Tier 2: Write with SigilYX, read with Alteryx
# ============================================================================


class TestWriteAndVerifyWithAlteryx:
    """Write YXDB with SigilYX → Read back with Alteryx C++ → compare."""

    def _write_and_verify(self, df: pl.DataFrame, test_name: str):
        """Helper: write df, read with both impls, compare."""
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb(tmp_path, df)
            sigilyx_data = read_sigilyx_rows(tmp_path)
            cpp_data = run_alteryx_dump(tmp_path)
            assert_results_match(
                sigilyx_data, cpp_data, context=test_name, strict_types=False
            )
        finally:
            os.unlink(tmp_path)

    def test_integers(self):
        df = pl.DataFrame({
            "i16": pl.Series([1, -32768, 32767, 0], dtype=pl.Int16),
            "i32": pl.Series([1, -2147483648, 2147483647, 42], dtype=pl.Int32),
            "i64": pl.Series(
                [1, -9223372036854775808, 9223372036854775807, 0], dtype=pl.Int64
            ),
        })
        self._write_and_verify(df, "integers")

    def test_floats(self):
        df = pl.DataFrame({
            "f32": pl.Series([1.5, -0.5, 0.0, 3.14], dtype=pl.Float32),
            "f64": pl.Series(
                [3.141592653589793, -1e308, 0.0, 1e-300], dtype=pl.Float64
            ),
        })
        self._write_and_verify(df, "floats")

    def test_strings(self):
        df = pl.DataFrame({
            "name": ["Alice", "Bob", "Charlie", "David"],
            "unicode": ["café", "日本語", "Ünïcödé", "simple"],
        })
        self._write_and_verify(df, "strings")

    def test_booleans(self):
        df = pl.DataFrame({"flag": [True, False, True, False]})
        self._write_and_verify(df, "booleans")

    def test_dates_and_datetimes(self):
        import datetime

        df = pl.DataFrame({
            "d": pl.Series([
                "2025-01-15", "1999-12-31", "2000-06-15", "2025-03-01"
            ]).str.to_date(),
            "dt": pl.Series([
                "2025-01-15 08:30:00", "1999-12-31 23:59:59",
                "2000-06-15 12:00:00", "2025-03-01 00:00:00",
            ]).str.to_datetime(),
        })
        self._write_and_verify(df, "dates")

    def test_times(self):
        import datetime

        df = pl.DataFrame({
            "t": pl.Series([
                datetime.time(0, 0, 0),
                datetime.time(12, 0, 0),
                datetime.time(23, 59, 59),
                datetime.time(8, 30, 0),
            ], dtype=pl.Time),
        })
        self._write_and_verify(df, "times")

    def test_mixed_types(self):
        df = pl.DataFrame({
            "id": pl.Series([1, 2, 3], dtype=pl.Int32),
            "name": ["Alice", "Bob", "Charlie"],
            "score": pl.Series([95.5, 87.3, 92.1], dtype=pl.Float64),
            "active": [True, False, True],
        })
        self._write_and_verify(df, "mixed")

    def test_nullable_integers(self):
        df = pl.DataFrame({
            "val": pl.Series([1, None, 3, None], dtype=pl.Int32),
            "big": pl.Series([100, None, 300, None], dtype=pl.Int64),
        })
        self._write_and_verify(df, "nullable_int")

    def test_nullable_strings(self):
        df = pl.DataFrame({
            "name": pl.Series(["Alice", None, "Charlie", None], dtype=pl.String),
        })
        self._write_and_verify(df, "nullable_str")

    def test_nullable_floats(self):
        df = pl.DataFrame({
            "val": pl.Series([1.5, None, 3.14, None], dtype=pl.Float64),
        })
        self._write_and_verify(df, "nullable_float")

    def test_empty_strings(self):
        df = pl.DataFrame({"text": ["", "hello", "", "world"]})
        self._write_and_verify(df, "empty_strings")

    def test_all_null_column(self):
        df = pl.DataFrame({
            "v": pl.Series([None, None, None], dtype=pl.Int32),
        })
        self._write_and_verify(df, "all_null")

    def test_single_row(self):
        df = pl.DataFrame({"x": [42], "s": ["only"]})
        self._write_and_verify(df, "single_row")

    def test_long_strings(self):
        df = pl.DataFrame({
            "short": ["hi"],
            "medium": ["M" * 500],
            "long": ["L" * 10_000],
        })
        self._write_and_verify(df, "long_strings")

    def test_binary_blobs(self):
        df = pl.DataFrame({
            "b": pl.Series(
                [b"\x00\x01\x02\x03", b"\xFF" * 1000, b"tiny"],
                dtype=pl.Binary,
            )
        })
        self._write_and_verify(df, "blobs")

    def test_many_rows(self):
        """Write 10k rows, verify all with Alteryx."""
        n = 10_000
        df = pl.DataFrame({
            "id": pl.Series(list(range(n)), dtype=pl.Int32),
            "label": [f"row_{i:05d}" for i in range(n)],
        })
        self._write_and_verify(df, "many_rows_10k")


# ============================================================================
#  Tier 3: Full roundtrip (SigilYX read → write → Alteryx read)
# ============================================================================


class TestFullRoundtripWithAlteryx:
    """Read test file → write via SigilYX → Alteryx reads the written file.
    
    The Alteryx output for the original file and the SigilYX-written copy
    should produce identical values.
    """

    ROUNDTRIP_FILES = [
        "AllTypes.yxdb",
        "NullValues.yxdb",
        "ManyRecords.yxdb",
        "Strings.yxdb",
        "People.yxdb",
        "SingleColumn.yxdb",
    ]

    @pytest.mark.parametrize("filename", ROUNDTRIP_FILES)
    def test_roundtrip_verified_by_alteryx(self, filename):
        path = _yxdb(filename)
        if not Path(path).exists():
            pytest.skip(f"{filename} not found")

        # Read original with SigilYX (columnar)
        df = sigilyx.read_yxdb(path)

        # Write to temp file
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb(tmp_path, df)

            # Read both original and roundtrip with Alteryx
            cpp_original = run_alteryx_dump(path)
            cpp_roundtrip = run_alteryx_dump(tmp_path)

            assert_results_match(
                cpp_original,
                cpp_roundtrip,
                context=f"{filename} roundtrip",
                strict_types=False,  # Polars normalizes types on roundtrip
            )
        finally:
            os.unlink(tmp_path)

    @pytest.mark.parametrize("filename", ROUNDTRIP_FILES)
    def test_roundtrip_header_valid(self, filename):
        """Written file should have valid YXDB header."""
        path = _yxdb(filename)
        if not Path(path).exists():
            pytest.skip(f"{filename} not found")

        df = sigilyx.read_yxdb(path)
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb(tmp_path, df)

            with open(tmp_path, "rb") as f:
                header = f.read(512)

            # Check magic
            assert header[:21] == b"Alteryx Database File"

            # Check record count
            num_records = struct.unpack_from("<Q", header, 104)[0]
            assert num_records == len(df)

            # Check metadata size > 0
            meta_size = struct.unpack_from("<I", header, 80)[0]
            assert meta_size > 0
        finally:
            os.unlink(tmp_path)


# ============================================================================
#  Tier 4: Targeted type-specific round-trips verified by Alteryx
# ============================================================================


class TestTypeSpecificAlteryx:
    """One-off tests targeting specific field types that need C++ verification."""

    def _roundtrip_verify(self, df: pl.DataFrame, test_name: str):
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb(tmp_path, df)
            sigilyx_data = read_sigilyx_rows(tmp_path)
            cpp_data = run_alteryx_dump(tmp_path)
            assert_results_match(
                sigilyx_data, cpp_data, context=test_name, strict_types=False
            )
        finally:
            os.unlink(tmp_path)

    def test_int_boundary_values(self):
        """Min/max values for all integer types."""
        df = pl.DataFrame({
            "i16": pl.Series([-(2**15), 2**15 - 1, 0], dtype=pl.Int16),
            "i32": pl.Series([-(2**31), 2**31 - 1, 0], dtype=pl.Int32),
            "i64": pl.Series([-(2**63), 2**63 - 1, 0], dtype=pl.Int64),
        })
        self._roundtrip_verify(df, "int_boundaries")

    def test_float_special_values(self):
        """Infinity, negative infinity, zero."""
        import math

        df = pl.DataFrame({
            "f64": [float("inf"), float("-inf"), 0.0, -0.0, 1e-300, 1e308],
        })
        self._roundtrip_verify(df, "float_specials")

    def test_bool_all_states(self):
        """true, false, null."""
        df = pl.DataFrame({
            "b": pl.Series([True, False, None, True, None, False], dtype=pl.Boolean),
        })
        self._roundtrip_verify(df, "bool_states")

    def test_empty_string_vs_null(self):
        """Alteryx must distinguish empty string from null."""
        df = pl.DataFrame({
            "s": pl.Series(["hello", "", None, "world", None, ""], dtype=pl.String),
        })
        self._roundtrip_verify(df, "empty_vs_null")

    def test_unicode_strings(self):
        """Various Unicode ranges verified by C++."""
        df = pl.DataFrame({
            "s": [
                "ASCII only",
                "café au lait",
                "Ünïcödé",
                "日本語テスト",
                "ĀĂĄĆĈ",  # Latin Extended-A (SIMD bug range)
                "αβγδε",
                "Привет",
            ]
        })
        self._roundtrip_verify(df, "unicode")

    def test_long_variable_strings(self):
        """Strings crossing the 127-byte variable threshold, verified by C++."""
        df = pl.DataFrame({
            "s": ["A" * 126, "B" * 127, "C" * 128, "D" * 200, "E" * 1000],
        })
        self._roundtrip_verify(df, "var_threshold")

    def test_dates_leap_years(self):
        """Leap year dates verified by Alteryx."""
        import datetime

        df = pl.DataFrame({
            "d": [
                datetime.date(2000, 2, 29),
                datetime.date(2004, 2, 29),
                datetime.date(1970, 1, 1),
                datetime.date(2025, 12, 31),
            ]
        })
        self._roundtrip_verify(df, "leap_dates")

    def test_datetime_pre_epoch(self):
        """Pre-1970 datetimes verified by Alteryx."""
        import datetime

        df = pl.DataFrame({
            "dt": [
                datetime.datetime(1960, 6, 15, 12, 30, 0),
                datetime.datetime(1969, 12, 31, 23, 59, 59),
                datetime.datetime(1900, 1, 1, 0, 0, 0),
            ]
        })
        self._roundtrip_verify(df, "pre_epoch_dt")

    def test_blob_sizes(self):
        """Various blob sizes verified by Alteryx."""
        df = pl.DataFrame({
            "b": pl.Series(
                [b"", b"\x42", b"\x00" * 127, b"\xFF" * 128, b"\xAB" * 10_000],
                dtype=pl.Binary,
            )
        })
        self._roundtrip_verify(df, "blob_sizes")

    def test_mixed_null_patterns(self):
        """Alternating null/non-null across multiple columns."""
        n = 100
        df = pl.DataFrame({
            "a": pl.Series(
                [i if i % 2 == 0 else None for i in range(n)], dtype=pl.Int32
            ),
            "b": pl.Series(
                [f"row_{i}" if i % 3 == 0 else None for i in range(n)],
                dtype=pl.String,
            ),
            "c": pl.Series(
                [float(i) if i % 5 == 0 else None for i in range(n)],
                dtype=pl.Float64,
            ),
        })
        self._roundtrip_verify(df, "mixed_null_patterns")

    def test_wide_schema(self):
        """50 columns of mixed types verified by Alteryx."""
        data = {}
        for i in range(10):
            data[f"int_{i}"] = pl.Series([i * 10, i * 20, i * 30], dtype=pl.Int32)
        for i in range(10):
            data[f"str_{i}"] = [f"val_{i}_0", f"val_{i}_1", f"val_{i}_2"]
        for i in range(10):
            data[f"f64_{i}"] = pl.Series(
                [float(i), float(i) + 0.5, float(i) + 1.0], dtype=pl.Float64
            )
        for i in range(10):
            data[f"bool_{i}"] = [i % 2 == 0, i % 2 == 1, i % 3 == 0]
        df = pl.DataFrame(data)
        self._roundtrip_verify(df, "wide_schema")

    def test_streaming_write_verified(self):
        """Streaming writer output verified by Alteryx."""
        batches = [
            pl.DataFrame({"id": pl.Series([1, 2], dtype=pl.Int32), "s": ["a", "b"]}),
            pl.DataFrame({"id": pl.Series([3, 4], dtype=pl.Int32), "s": ["c", "d"]}),
            pl.DataFrame({"id": pl.Series([5], dtype=pl.Int32), "s": ["e"]}),
        ]
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb_batches(tmp_path, iter(batches))
            sigilyx_data = read_sigilyx_rows(tmp_path)
            cpp_data = run_alteryx_dump(tmp_path)
            assert_results_match(
                sigilyx_data, cpp_data,
                context="streaming_write", strict_types=False
            )
        finally:
            os.unlink(tmp_path)

    def test_many_rows_cross_impl(self):
        """50k rows verified by Alteryx — stress test for block boundaries."""
        n = 50_000
        df = pl.DataFrame({
            "id": pl.Series(list(range(n)), dtype=pl.Int32),
            "v": pl.Series([float(i) * 0.1 for i in range(n)], dtype=pl.Float64),
            "s": [f"row_{i:06d}" for i in range(n)],
        })
        with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
            tmp_path = f.name
        try:
            sigilyx.write_yxdb(tmp_path, df)

            # Verify count via C++
            cpp_data = run_alteryx_dump(tmp_path)
            assert len(cpp_data["rows"]) == n

            # Spot-check first, last, and middle rows
            sigilyx_data = read_sigilyx_rows(tmp_path)
            for idx in [0, n // 2, n - 1]:
                for col_idx in range(3):
                    assert _compare_values(
                        sigilyx_data["rows"][idx][col_idx],
                        cpp_data["rows"][idx][col_idx],
                        cpp_data["field_types"][col_idx],
                    ), (
                        f"Mismatch at row {idx}, col {col_idx}: "
                        f"SigilYX={sigilyx_data['rows'][idx][col_idx]!r} vs "
                        f"C++={cpp_data['rows'][idx][col_idx]!r}"
                    )
        finally:
            os.unlink(tmp_path)


# ============================================================================
#  Alteryx-written files read by SigilYX (if benchmark data exists)
# ============================================================================


class TestBenchmarkDataFiles:
    """Spot-check benchmark data files that were generated by Alteryx/other tools."""

    @pytest.fixture
    def bench_dir(self):
        d = BENCHMARK_DIR / "data"
        if not d.exists():
            pytest.skip("No benchmark data directory")
        return d

    def test_bench_files_readable(self, bench_dir):
        """All benchmark YXDB files should be readable by both impls."""
        files = sorted(bench_dir.glob("bench_*_1000.yxdb"))[:3]
        if not files:
            pytest.skip("No benchmark data files found")

        for path in files:
            cpp_data = run_alteryx_dump(str(path))
            sigilyx_data = read_sigilyx_rows(str(path))

            # Compare first 100 rows for speed
            limit = min(100, len(cpp_data["rows"]))
            for row_idx in range(limit):
                for col_idx in range(len(cpp_data["field_names"])):
                    sv = sigilyx_data["rows"][row_idx][col_idx]
                    cv = cpp_data["rows"][row_idx][col_idx]
                    ft = cpp_data["field_types"][col_idx]
                    assert _compare_values(sv, cv, ft), (
                        f"{path.name} row {row_idx}, col {col_idx}: "
                        f"SigilYX={sv!r} vs C++={cv!r}"
                    )
