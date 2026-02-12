"""
Cross-implementation round-trip test for SigilYX.

Tests that YXDB files written by SigilYX can be read correctly by the
official Alteryx OpenYXDB C++ library (and optionally the NedHarding fork),
and that files in the test suite produce identical values across both
implementations.

Approach:
1. Read test YXDB files with SigilYX (row reader) and C++ dump tool(s)
2. Compare field names, types, and all values
3. Write new YXDB files with SigilYX, read back with C++ dump tool(s)
4. Verify the C++ libraries read them identically

C++ dump tools:
- alteryx_openyxdb_dump.exe  (official Alteryx OpenYXDB — primary verifier)
- open_yxdb_dump.exe         (NedHarding fork — optional secondary verifier)

Requirements:
- SigilYX Python package installed (pip install -e .)
- At least one C++ dump tool built (see benchmarks/cpp/)
- Run with: python benchmarks/test_cross_impl.py
"""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

import polars as pl

# Add parent to path for sigilyx import
sys.path.insert(0, str(Path(__file__).parent.parent))
import sigilyx as yx

# ── Configuration ──────────────────────────────────────────────────────

SCRIPT_DIR = Path(__file__).parent
ALTERYX_DUMP_EXE = SCRIPT_DIR / "cpp" / "alteryx_openyxdb_dump.exe"
NEDHARDING_DUMP_EXE = SCRIPT_DIR / "cpp" / "open_yxdb_dump.exe"
TEST_FILES_DIR = SCRIPT_DIR.parent / "sigilyx" / "test_files"

# Populated in main() after checking which executables exist
DUMP_TOOLS: dict[str, Path] = {}

# Test files that both implementations should be able to read
READ_TEST_FILES = [
    "AllTypes.yxdb",
    "NullValues.yxdb",
    "ManyRecords.yxdb",
    "Strings.yxdb",
    "People.yxdb",
    "SingleColumn.yxdb",
]


# ── Parse C++ dump output ─────────────────────────────────────────────

def run_cpp_dump(yxdb_path: str, dump_exe: str | Path | None = None) -> dict:
    """Run a C++ dump tool and parse its TSV output.

    Args:
        yxdb_path: Path to the YXDB file to dump.
        dump_exe: Path to the dump executable. If None, uses the first
                  available tool from DUMP_TOOLS.

    Returns a dict with keys: field_names, field_types, rows.
    Each row is a list of string values (or None for null).
    """
    if dump_exe is None:
        if not DUMP_TOOLS:
            raise RuntimeError("No C++ dump tools available")
        dump_exe = next(iter(DUMP_TOOLS.values()))

    result = subprocess.run(
        [str(dump_exe), yxdb_path],
        capture_output=True,
        timeout=60,
    )
    if result.returncode != 0:
        stderr = result.stderr.decode("utf-8", errors="replace")
        raise RuntimeError(f"C++ dump tool failed (exit {result.returncode}): {stderr}")

    lines = result.stdout.decode("utf-8").splitlines()
    if len(lines) < 2:
        raise RuntimeError(f"C++ dump tool produced too few lines: {len(lines)}")

    field_names = lines[0].split("\t")
    field_types = lines[1].split("\t")

    rows = []
    for line in lines[2:]:
        # Don't skip empty lines — they represent rows where all values are empty
        values = []
        for cell in line.split("\t"):
            if cell == "\\N":
                values.append(None)
            else:
                # Unescape TSV escapes
                cell = cell.replace("\\t", "\t")
                cell = cell.replace("\\n", "\n")
                cell = cell.replace("\\r", "\r")
                cell = cell.replace("\\\\", "\\")
                values.append(cell)
        rows.append(values)

    return {
        "field_names": field_names,
        "field_types": field_types,
        "rows": rows,
    }


# ── Read with SigilYX row reader ──────────────────────────────────────

def read_sigilyx_rows(yxdb_path: str) -> dict:
    """Read a YXDB file using SigilYX's row reader, returning same format as C++ dump.

    Returns a dict with keys: field_names, field_types, rows.
    Values are converted to strings matching C++ dump output format.
    """
    reader = yx.YxdbRowReader(yxdb_path)
    fields = reader.fields

    field_names = [f.name for f in fields]
    field_types = [f.field_type for f in fields]

    rows = []
    while reader.next():
        row_dict = reader.read_dict()
        row = []
        for field in fields:
            val = row_dict[field.name]
            row.append(format_value(val, field.field_type))
        rows.append(row)

    reader.close()
    return {
        "field_names": field_names,
        "field_types": field_types,
        "rows": rows,
    }


def format_value(val, field_type: str) -> str | None:
    """Convert a Python value to string matching C++ dump output."""
    if val is None:
        return None

    if field_type == "Bool":
        return "true" if val else "false"

    if field_type in ("Byte", "Int16", "Int32"):
        return str(int(val))

    if field_type == "Int64":
        return str(int(val))

    if field_type in ("Float", "Double"):
        # Match C++ %.17g format
        return f"{float(val):.17g}"

    if field_type == "FixedDecimal":
        return str(val)

    if field_type in ("Date", "Time", "DateTime"):
        return str(val)

    if field_type in ("Blob", "SpatialObj"):
        if isinstance(val, (bytes, bytearray)):
            return f"[blob:{len(val)}]"
        return str(val)

    # String types
    return str(val)


# ── Map between SigilYX type names and C++ type names ─────────────────

SIGILYX_TO_CPP_TYPE = {
    "Bool": "Bool",
    "Byte": "Byte",
    "Int16": "Int16",
    "Int32": "Int32",
    "Int64": "Int64",
    "Float": "Float",
    "Double": "Double",
    "FixedDecimal": "FixedDecimal",
    "String": "String",
    "WString": "WString",
    "VString": "V_String",
    "VWString": "V_WString",
    "Date": "Date",
    "Time": "Time",
    "DateTime": "DateTime",
    "Blob": "Blob",
    "SpatialObj": "SpatialObj",
}


def types_match(sigilyx_type: str, cpp_type: str) -> bool:
    """Check if SigilYX and C++ type names refer to the same type."""
    expected = SIGILYX_TO_CPP_TYPE.get(sigilyx_type, sigilyx_type)
    return expected == cpp_type


# ── Comparison logic ──────────────────────────────────────────────────

def compare_values(
    sigilyx_val: str | None,
    cpp_val: str | None,
    field_type: str,
) -> bool:
    """Compare a single value from SigilYX and C++ outputs."""
    if sigilyx_val is None and cpp_val is None:
        return True
    if sigilyx_val is None or cpp_val is None:
        return False

    # For floating-point types, compare numerically with tolerance
    if field_type in ("Float", "Double"):
        try:
            s = float(sigilyx_val)
            c = float(cpp_val)
            if s == c:
                return True
            # Relative tolerance
            if abs(s) > 0:
                return abs(s - c) / abs(s) < 1e-12
            return abs(s - c) < 1e-15
        except ValueError:
            return sigilyx_val == cpp_val

    # For FixedDecimal, compare as floating-point values
    if field_type == "FixedDecimal":
        try:
            s = float(sigilyx_val)
            c = float(cpp_val)
            # FixedDecimal has limited precision
            return abs(s - c) < 1e-6
        except ValueError:
            return sigilyx_val == cpp_val

    # For everything else, exact string match
    return sigilyx_val == cpp_val


def compare_results(
    name: str,
    sigilyx_data: dict,
    cpp_data: dict,
    strict_types: bool = True,
) -> tuple[bool, list[str]]:
    """Compare SigilYX and C++ dump results. Returns (passed, errors).

    If strict_types is False, type mismatches are reported as warnings
    but don't cause test failure (for roundtrip tests through Polars
    which normalizes types).
    """
    errors = []

    # Compare field count
    if len(sigilyx_data["field_names"]) != len(cpp_data["field_names"]):
        errors.append(
            f"Field count mismatch: SigilYX={len(sigilyx_data['field_names'])}, "
            f"C++={len(cpp_data['field_names'])}"
        )
        return False, errors

    # Compare field names
    for i, (sn, cn) in enumerate(
        zip(sigilyx_data["field_names"], cpp_data["field_names"])
    ):
        if sn != cn:
            errors.append(f"Field {i} name mismatch: SigilYX={sn!r}, C++={cn!r}")

    # Compare field types
    for i, (st, ct) in enumerate(
        zip(sigilyx_data["field_types"], cpp_data["field_types"])
    ):
        if not types_match(st, ct):
            msg = f"Field {i} type mismatch: SigilYX={st!r}, C++={ct!r}"
            if strict_types:
                errors.append(msg)

    # Compare row count
    if len(sigilyx_data["rows"]) != len(cpp_data["rows"]):
        errors.append(
            f"Row count mismatch: SigilYX={len(sigilyx_data['rows'])}, "
            f"C++={len(cpp_data['rows'])}"
        )
        return False, errors

    # Compare values row by row
    mismatches = 0
    max_reported = 10  # limit error output
    for row_idx, (s_row, c_row) in enumerate(
        zip(sigilyx_data["rows"], cpp_data["rows"])
    ):
        if len(s_row) != len(c_row):
            errors.append(
                f"Row {row_idx}: column count mismatch "
                f"(SigilYX={len(s_row)}, C++={len(c_row)})"
            )
            mismatches += 1
            continue

        for col_idx, (sv, cv) in enumerate(zip(s_row, c_row)):
            cpp_type = cpp_data["field_types"][col_idx]
            if not compare_values(sv, cv, cpp_type):
                if mismatches < max_reported:
                    fname = cpp_data["field_names"][col_idx]
                    errors.append(
                        f"Row {row_idx}, col {col_idx} ({fname}): "
                        f"SigilYX={sv!r} vs C++={cv!r}"
                    )
                mismatches += 1

    if mismatches > max_reported:
        errors.append(f"... and {mismatches - max_reported} more mismatches")

    passed = len(errors) == 0
    return passed, errors


# ── Test functions ────────────────────────────────────────────────────

def test_read_existing_files() -> tuple[int, int]:
    """Test 1: Read existing test files with SigilYX and each C++ dump tool."""
    print("=" * 60)
    print("Test 1: Read existing YXDB files with both implementations")
    print("=" * 60)

    passed = 0
    failed = 0

    for filename in READ_TEST_FILES:
        path = TEST_FILES_DIR / filename
        if not path.exists():
            print(f"  SKIP {filename} (not found)")
            continue

        for tool_name, dump_exe in DUMP_TOOLS.items():
            print(f"  Testing {filename} [{tool_name}]...")

            try:
                cpp_data = run_cpp_dump(str(path), dump_exe)
                sigilyx_data = read_sigilyx_rows(str(path))

                ok, errors = compare_results(filename, sigilyx_data, cpp_data)
                if ok:
                    n = len(cpp_data["rows"])
                    print(f"    PASS ({n} rows, {len(cpp_data['field_names'])} cols)")
                    passed += 1
                else:
                    print(f"    FAIL:")
                    for err in errors:
                        print(f"      - {err}")
                    failed += 1

            except Exception as e:
                print(f"    ERROR: {e}")
                failed += 1

    return passed, failed


def test_write_read_roundtrip() -> tuple[int, int]:
    """Test 2: Write YXDB with SigilYX, read back with each C++ dump tool."""
    print()
    print("=" * 60)
    print("Test 2: Write with SigilYX, verify with C++ dump tool(s)")
    print("=" * 60)

    passed = 0
    failed = 0

    test_cases = [
        (
            "integers",
            pl.DataFrame({
                "int16_col": pl.Series([1, -32768, 32767, 0], dtype=pl.Int16),
                "int32_col": pl.Series([1, -2147483648, 2147483647, 42], dtype=pl.Int32),
                "int64_col": pl.Series([1, -9223372036854775808, 9223372036854775807, 0], dtype=pl.Int64),
            }),
        ),
        (
            "floats",
            pl.DataFrame({
                "float_col": pl.Series([1.5, -0.5, 0.0, 3.14], dtype=pl.Float32),
                "double_col": pl.Series([3.141592653589793, -1e308, 0.0, 1e-300], dtype=pl.Float64),
            }),
        ),
        (
            "strings",
            pl.DataFrame({
                "name": ["Alice", "Bob", "Charlie", "David"],
                "unicode": ["hello", "world", "test", "data"],
            }),
        ),
        (
            "booleans",
            pl.DataFrame({
                "flag": [True, False, True, False],
            }),
        ),
        (
            "dates_times",
            pl.DataFrame({
                "dt_col": pl.Series(["2025-01-15", "1999-12-31", "2000-06-15", "2025-03-01"]).str.to_date(),
                "dtm_col": pl.Series(["2025-01-15 08:30:00", "1999-12-31 23:59:59", "2000-06-15 12:00:00", "2025-03-01 00:00:00"]).str.to_datetime(),
            }),
        ),
        (
            "mixed_types",
            pl.DataFrame({
                "id": pl.Series([1, 2, 3], dtype=pl.Int32),
                "name": ["Alice", "Bob", "Charlie"],
                "score": pl.Series([95.5, 87.3, 92.1], dtype=pl.Float64),
                "active": [True, False, True],
            }),
        ),
        (
            "nullable_integers",
            pl.DataFrame({
                "val": pl.Series([1, None, 3, None], dtype=pl.Int32),
                "big_val": pl.Series([100, None, 300, None], dtype=pl.Int64),
            }),
        ),
        (
            "nullable_strings",
            pl.DataFrame({
                "name": pl.Series(["Alice", None, "Charlie", None], dtype=pl.String),
            }),
        ),
        (
            "nullable_floats",
            pl.DataFrame({
                "val": pl.Series([1.5, None, 3.14, None], dtype=pl.Float64),
            }),
        ),
        (
            "empty_strings",
            pl.DataFrame({
                "text": ["", "hello", "", "world"],
            }),
        ),
    ]

    for test_name, df in test_cases:
        try:
            with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
                tmp_path = f.name

            # Write with SigilYX
            yx.write_yxdb(tmp_path, df)

            # Read back with SigilYX
            sigilyx_data = read_sigilyx_rows(tmp_path)

            for tool_name, dump_exe in DUMP_TOOLS.items():
                print(f"  Testing {test_name} [{tool_name}]...")
                try:
                    # Read back with C++
                    cpp_data = run_cpp_dump(tmp_path, dump_exe)

                    ok, errors = compare_results(test_name, sigilyx_data, cpp_data)
                    if ok:
                        n = len(cpp_data["rows"])
                        print(f"    PASS ({n} rows, {len(cpp_data['field_names'])} cols)")
                        passed += 1
                    else:
                        print(f"    FAIL:")
                        for err in errors:
                            print(f"      - {err}")
                        failed += 1
                except Exception as e:
                    print(f"    ERROR: {e}")
                    import traceback
                    traceback.print_exc()
                    failed += 1

        except Exception as e:
            print(f"  ERROR writing {test_name}: {e}")
            import traceback
            traceback.print_exc()
            failed += 1

        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    return passed, failed


def test_sigilyx_roundtrip_with_cpp_verify() -> tuple[int, int]:
    """Test 3: SigilYX write->read roundtrip, verified by each C++ dump tool."""
    print()
    print("=" * 60)
    print("Test 3: SigilYX roundtrip + C++ verification on test files")
    print("=" * 60)

    passed = 0
    failed = 0

    for filename in READ_TEST_FILES:
        path = TEST_FILES_DIR / filename
        if not path.exists():
            continue

        # Skip blob files for roundtrip (blob columns may differ in representation)
        if filename == "LargeBlob.yxdb":
            continue

        try:
            # Read original with SigilYX (columnar)
            df = yx.read_yxdb(str(path))

            # Write to temp file
            with tempfile.NamedTemporaryFile(suffix=".yxdb", delete=False) as f:
                tmp_path = f.name

            yx.write_yxdb(tmp_path, df)

            for tool_name, dump_exe in DUMP_TOOLS.items():
                print(f"  Testing roundtrip for {filename} [{tool_name}]...")
                try:
                    # Read original and roundtrip with C++
                    cpp_original = run_cpp_dump(str(path), dump_exe)
                    cpp_roundtrip = run_cpp_dump(tmp_path, dump_exe)

                    ok, errors = compare_results(
                        f"{filename} (roundtrip)",
                        cpp_original,
                        cpp_roundtrip,
                        strict_types=False,  # Polars normalizes types on roundtrip
                    )
                    if ok:
                        n = len(cpp_original["rows"])
                        print(f"    PASS ({n} rows roundtripped)")
                        passed += 1
                    else:
                        print(f"    FAIL:")
                        for err in errors:
                            print(f"      - {err}")
                        failed += 1
                except Exception as e:
                    print(f"    ERROR: {e}")
                    import traceback
                    traceback.print_exc()
                    failed += 1

        except Exception as e:
            print(f"  ERROR: {e}")
            import traceback
            traceback.print_exc()
            failed += 1

        finally:
            if "tmp_path" in locals() and os.path.exists(tmp_path):
                os.unlink(tmp_path)

    return passed, failed


def test_benchmark_files() -> tuple[int, int]:
    """Test 4: Spot-check benchmark data files with each C++ dump tool."""
    print()
    print("=" * 60)
    print("Test 4: Spot-check benchmark data files (first 100 rows)")
    print("=" * 60)

    data_dir = SCRIPT_DIR / "data"
    if not data_dir.exists():
        print("  SKIP (no benchmark data directory)")
        return 0, 0

    bench_files = sorted(data_dir.glob("bench_*_1000.yxdb"))[:3]
    if not bench_files:
        print("  SKIP (no benchmark data files)")
        return 0, 0

    passed = 0
    failed = 0

    for path in bench_files:
        sigilyx_data = read_sigilyx_rows(str(path))

        for tool_name, dump_exe in DUMP_TOOLS.items():
            print(f"  Testing {path.name} [{tool_name}]...")

            try:
                cpp_data = run_cpp_dump(str(path), dump_exe)

                # Compare a subset (first 100 rows) for speed on large files
                if len(cpp_data["rows"]) > 100:
                    cpp_subset = {
                        "field_names": cpp_data["field_names"],
                        "field_types": cpp_data["field_types"],
                        "rows": cpp_data["rows"][:100],
                    }
                    sigilyx_subset = {
                        "field_names": sigilyx_data["field_names"],
                        "field_types": sigilyx_data["field_types"],
                        "rows": sigilyx_data["rows"][:100],
                    }
                else:
                    cpp_subset = cpp_data
                    sigilyx_subset = sigilyx_data

                ok, errors = compare_results(path.name, sigilyx_subset, cpp_subset)
                if ok:
                    n = len(cpp_data["rows"])
                    print(f"    PASS ({n} total rows, checked 100)")
                    passed += 1
                else:
                    print(f"    FAIL:")
                    for err in errors:
                        print(f"      - {err}")
                    failed += 1

            except Exception as e:
                print(f"    ERROR: {e}")
                failed += 1

    return passed, failed


# ── Main ──────────────────────────────────────────────────────────────

def main():
    print("SigilYX Cross-Implementation Round-Trip Test")
    print("=" * 60)

    # Detect available C++ dump tools (Alteryx official is primary)
    if ALTERYX_DUMP_EXE.exists():
        DUMP_TOOLS["alteryx-openyxdb"] = ALTERYX_DUMP_EXE
    if NEDHARDING_DUMP_EXE.exists():
        DUMP_TOOLS["nedharding"] = NEDHARDING_DUMP_EXE

    if not DUMP_TOOLS:
        print("ERROR: No C++ dump tools found. Build at least one:")
        print(f"  Alteryx:    benchmarks/cpp/build_alteryx_dump.bat")
        print(f"  NedHarding: benchmarks/cpp/build_dump.bat")
        sys.exit(1)

    print(f"C++ dump tools:")
    for name, path in DUMP_TOOLS.items():
        print(f"  {name:>20}: {path}")
    print(f"Test files dir: {TEST_FILES_DIR}")
    print()

    total_passed = 0
    total_failed = 0

    p, f = test_read_existing_files()
    total_passed += p
    total_failed += f

    p, f = test_write_read_roundtrip()
    total_passed += p
    total_failed += f

    p, f = test_sigilyx_roundtrip_with_cpp_verify()
    total_passed += p
    total_failed += f

    p, f = test_benchmark_files()
    total_passed += p
    total_failed += f

    # Summary
    print()
    print("=" * 60)
    print("SUMMARY")
    print("=" * 60)
    print(f"  Passed: {total_passed}")
    print(f"  Failed: {total_failed}")
    print(f"  Total:  {total_passed + total_failed}")
    print()

    if total_failed == 0:
        print("All cross-implementation tests PASSED!")
    else:
        print("Some tests FAILED!")
        sys.exit(1)


if __name__ == "__main__":
    main()
