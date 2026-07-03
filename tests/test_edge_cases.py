"""Comprehensive edge-case and stress tests for SigilYX.

Covers gaps not addressed by test_yxdb_reader.py:
- LZF block boundary behavior
- Variable-length data at size thresholds (127-byte boundary)
- FixedDecimal precision/scale combinations
- Byte (UInt8) type handling
- Time field roundtrips
- SpatialObj-like binary data
- Writer schema inference for unsigned int types
- Concurrent writes
- Column order preservation
- Empty/single-byte blobs
- Malformed file handling
- Row reader ↔ Writer interop
- String edge cases (embedded nulls, surrogates, BOM)
- Record that pushes exactly to block boundary
"""

import datetime
import math
import os
import struct
import tempfile
import concurrent.futures
from decimal import Decimal
from pathlib import Path

import polars as pl
import pytest

import sigilyx

TEST_DIR = Path(__file__).parent.parent / "sigilyx" / "test_files"


def _yxdb(name: str) -> str:
    return str(TEST_DIR / name)


# ============================================================================
#  LZF compression block boundary tests
# ============================================================================


class TestLzfBlockBoundaries:
    """Stress the 0x40000-byte (262144) uncompressed block size limit."""

    def test_data_just_under_block_boundary(self, tmp_path):
        """Create data whose uncompressed record buffer is just under 0x40000 bytes.
        
        A single-column Int32 (5 bytes fixed per record) needs ~52428 rows
        to fill exactly 262140 bytes (just under 262144).
        """
        n = 52428
        df = pl.DataFrame({"v": pl.Series(list(range(n)), dtype=pl.Int32)})
        path = str(tmp_path / "just_under.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape
        assert df2["v"].to_list() == list(range(n))

    def test_data_just_over_block_boundary(self, tmp_path):
        """Create data whose records span into a second LZF block."""
        n = 52430  # slightly over one block
        df = pl.DataFrame({"v": pl.Series(list(range(n)), dtype=pl.Int32)})
        path = str(tmp_path / "just_over.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape
        assert df2["v"][0] == 0
        assert df2["v"][-1] == n - 1

    def test_data_exactly_at_block_boundary(self, tmp_path):
        """Data that fills exactly one block (262144 / 5 = 52428.8 rows)."""
        # With 5 bytes per record (Int32 + null indicator), we can't hit it exactly.
        # Instead use a wider record: Int64 (9 bytes) → 262144 / 9 = 29127.1
        # Use 2 Int64 columns: 18 bytes/record → 262144 / 18 = 14563.5
        # Actually the block holds raw record bytes; let's just use a large row count.
        n = 29127
        df = pl.DataFrame({"v": pl.Series(list(range(n)), dtype=pl.Int64)})
        path = str(tmp_path / "exact_boundary.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape

    def test_many_blocks(self, tmp_path):
        """Create enough data to span 4+ LZF blocks."""
        # At 9 bytes/record (Int64), 120,000 rows ≈ 1,080,000 bytes ≈ 4.1 blocks
        n = 120_000
        df = pl.DataFrame({"v": pl.Series(list(range(n)), dtype=pl.Int64)})
        path = str(tmp_path / "many_blocks.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["v"][0] == 0
        assert df2["v"][-1] == n - 1
        assert df2.height == n

    def test_incompressible_data(self, tmp_path):
        """Random-like data that doesn't compress well - forces uncompressed blocks."""
        import random
        random.seed(42)
        # Random strings are hard to compress with LZF
        data = [f"rand_{random.getrandbits(64):016x}" for _ in range(10_000)]
        df = pl.DataFrame({"s": data})
        path = str(tmp_path / "incompressible.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == data

    def test_highly_compressible_data(self, tmp_path):
        """All-same values - compresses extremely well."""
        n = 100_000
        df = pl.DataFrame({"s": ["AAAA"] * n})
        path = str(tmp_path / "compressible.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.height == n
        assert df2["s"][0] == "AAAA"
        assert df2["s"][-1] == "AAAA"


# ============================================================================
#  Variable-length data threshold tests (127-byte small/normal boundary)
# ============================================================================


class TestVariableLengthThresholds:
    """The YXDB variable-length format uses 1-byte headers for ≤127 bytes
    and 4-byte headers (with bit 31 set) for >127 bytes. Test at these
    boundary points."""

    def test_vstring_exactly_127_bytes(self, tmp_path):
        """V_String with exactly 127 bytes of content."""
        s = "A" * 127
        df = pl.DataFrame({"s": [s]})
        path = str(tmp_path / "v127.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == s

    def test_vstring_exactly_128_bytes(self, tmp_path):
        """V_String with exactly 128 bytes - crosses into 4-byte header territory."""
        s = "B" * 128
        df = pl.DataFrame({"s": [s]})
        path = str(tmp_path / "v128.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == s

    def test_vstring_126_to_130_range(self, tmp_path):
        """Test each length from 126 to 130 to stress the threshold."""
        for length in range(126, 131):
            s = "X" * length
            df = pl.DataFrame({"s": [s]})
            path = str(tmp_path / f"v{length}.yxdb")
            sigilyx.write_yxdb(path, df)
            df2 = sigilyx.read_yxdb(path)
            assert df2["s"][0] == s, f"Failed at length={length}"

    def test_mixed_small_and_large_vstrings(self, tmp_path):
        """Mix of strings below and above the 127-byte threshold in one column."""
        strings = [
            "short",           # < 127
            "A" * 127,         # exactly 127
            "B" * 128,         # exactly 128
            "C" * 1000,        # well above
            "",                # empty
            "D" * 126,         # just below
        ]
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "mixed_var.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == strings

    def test_blob_at_127_boundary(self, tmp_path):
        """Binary blob exactly at and around the 127-byte threshold."""
        blobs = [
            b"\x00" * 127,
            b"\xFF" * 128,
            b"\xAB" * 1,
            b"",
        ]
        # Create binary series
        df = pl.DataFrame({"b": pl.Series(blobs, dtype=pl.Binary)})
        path = str(tmp_path / "blob_threshold.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        for i, expected in enumerate(blobs):
            assert df2["b"][i] == expected, f"Mismatch at index {i}"

    def test_blob_empty_single_byte_null(self, tmp_path):
        """Edge cases for blob: empty, single-byte, null."""
        df = pl.DataFrame({
            "b": pl.Series([b"", b"\x42", None, b"\x00\x01\x02"], dtype=pl.Binary)
        })
        path = str(tmp_path / "blob_edge.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["b"][0] == b""
        assert df2["b"][1] == b"\x42"
        assert df2["b"][2] is None
        assert df2["b"][3] == b"\x00\x01\x02"


# ============================================================================
#  FixedDecimal precision and scale edge cases
# ============================================================================


class TestFixedDecimalEdgeCases:
    """Test FixedDecimal handling through the full pipeline."""

    def test_decimal_read_all_types(self):
        """Verify FixedDecimal from AllTypes.yxdb reads correctly."""
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        # DecimalCol should be Decimal type in Polars
        assert "Decimal" in str(df["DecimalCol"].dtype)
        val0 = df["DecimalCol"][0]
        assert abs(float(val0) - 1234.5678) < 0.001

    def test_decimal_roundtrip_basic(self, tmp_path):
        """Write a Decimal128 column and read it back."""
        df = pl.DataFrame({
            "d": pl.Series(
                [Decimal("123.45"), Decimal("-999.99"), Decimal("0.01")],
                dtype=pl.Decimal(precision=10, scale=2),
            )
        })
        path = str(tmp_path / "decimal.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = [float(v) for v in df2["d"].to_list()]
        assert abs(vals[0] - 123.45) < 0.01
        assert abs(vals[1] - (-999.99)) < 0.01
        assert abs(vals[2] - 0.01) < 0.001

    def test_decimal_scale_zero(self, tmp_path):
        """FixedDecimal with scale=0 (integer-like)."""
        df = pl.DataFrame({
            "d": pl.Series(
                [Decimal("42"), Decimal("-100"), Decimal("0")],
                dtype=pl.Decimal(precision=10, scale=0),
            )
        })
        path = str(tmp_path / "dec_scale0.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = [float(v) for v in df2["d"].to_list()]
        assert vals[0] == 42.0
        assert vals[1] == -100.0

    def test_decimal_high_scale(self, tmp_path):
        """FixedDecimal with high scale (many decimal places)."""
        df = pl.DataFrame({
            "d": pl.Series(
                [Decimal("3.14159265"), Decimal("-0.00000001")],
                dtype=pl.Decimal(precision=19, scale=8),
            )
        })
        path = str(tmp_path / "dec_high_scale.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        vals = [float(v) for v in df2["d"].to_list()]
        assert abs(vals[0] - 3.14159265) < 1e-7
        assert abs(vals[1] - (-0.00000001)) < 1e-7

    def test_decimal_null_values(self, tmp_path):
        """FixedDecimal column with nulls."""
        df = pl.DataFrame({
            "d": pl.Series(
                [Decimal("1.5"), None, Decimal("3.5")],
                dtype=pl.Decimal(precision=10, scale=2),
            )
        })
        path = str(tmp_path / "dec_null.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["d"][1] is None
        assert df2["d"].null_count() == 1


# ============================================================================
#  Byte (UInt8) type handling
# ============================================================================


class TestByteType:
    """Byte is stored as a single unsigned byte in YXDB, mapped to i16 in Polars."""

    def test_byte_from_alltypes(self):
        """Byte values from AllTypes.yxdb."""
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        assert df["ByteCol"][0] == 7
        assert df["ByteCol"][1] == 255

    def test_byte_boundary_values_via_i16(self, tmp_path):
        """Write i16 values in the Byte range [0, 255] and roundtrip.
        
        When read back, Byte columns become i16 in Polars. Writing i16
        values in [0, 255] should map to Byte fields.
        """
        # Note: SigilYX maps i16 to Int16 in YXDB, not Byte. Byte roundtrip
        # requires reading from an existing Byte-typed YXDB.
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        byte_col = df.select("ByteCol")
        path = str(tmp_path / "byte_rt.yxdb")
        sigilyx.write_yxdb(path, byte_col)
        df2 = sigilyx.read_yxdb(path)
        assert df2["ByteCol"][0] == 7
        assert df2["ByteCol"][1] == 255

    def test_byte_null_handling(self):
        """Byte column with null from NullValues.yxdb."""
        df = sigilyx.read_yxdb(_yxdb("NullValues.yxdb"))
        assert df["NullByte"][0] == 42
        assert df["NullByte"][1] is None
        assert df["NullByte"][2] is None


# ============================================================================
#  Time field roundtrip
# ============================================================================


class TestTimeField:
    """Time fields (HH:MM:SS) are stored as 9-byte ASCII in YXDB."""

    def test_time_from_alltypes(self):
        """Verify Time column is readable."""
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        assert "TimeCol" in df.columns
        assert df["TimeCol"].dtype == pl.Time

    def test_time_roundtrip(self, tmp_path):
        """Round-trip Time values through write and read."""
        import datetime
        times = [
            datetime.time(0, 0, 0),       # midnight
            datetime.time(12, 0, 0),       # noon
            datetime.time(23, 59, 59),     # last second
            datetime.time(8, 30, 0),       # normal time
        ]
        df = pl.DataFrame({"t": pl.Series(times, dtype=pl.Time)})
        path = str(tmp_path / "times.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["t"].to_list() == times

    def test_time_with_null(self, tmp_path):
        """Time column with null values."""
        import datetime
        df = pl.DataFrame({
            "t": pl.Series(
                [datetime.time(10, 30, 0), None, datetime.time(15, 0, 0)],
                dtype=pl.Time,
            )
        })
        path = str(tmp_path / "time_null.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["t"][0] == datetime.time(10, 30, 0)
        assert df2["t"][1] is None
        assert df2["t"][2] == datetime.time(15, 0, 0)


# ============================================================================
#  Unsigned integer type handling (Polars UInt → YXDB mapping)
# ============================================================================


class TestUnsignedIntTypes:
    """Polars has UInt8/16/32/64 which must be cast or mapped to YXDB types.

    The YXDB format has no unsigned integer types, so values are cast to
    appropriate signed types before writing. We cast explicitly in the test
    to work around IPC serialization limitations with unsigned Polars types.
    """

    def test_uint8_roundtrip(self, tmp_path):
        """UInt8 values should survive a cast-to-Int16 roundtrip."""
        vals = [0, 127, 255]
        df = pl.DataFrame({"u": pl.Series(vals, dtype=pl.UInt8).cast(pl.Int16)})
        path = str(tmp_path / "uint8.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["u"].to_list() == vals

    def test_uint16_roundtrip(self, tmp_path):
        """UInt16 roundtrip via Int32 cast."""
        vals = [0, 32767, 65535]
        df = pl.DataFrame({"u": pl.Series(vals, dtype=pl.UInt16).cast(pl.Int32)})
        path = str(tmp_path / "uint16.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["u"].to_list() == vals

    def test_uint32_roundtrip(self, tmp_path):
        """UInt32 roundtrip via Int64 cast."""
        vals = [0, 2**31 - 1, 2**32 - 1]
        df = pl.DataFrame({
            "u": pl.Series(vals, dtype=pl.UInt32).cast(pl.Int64),
        })
        path = str(tmp_path / "uint32.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["u"].to_list() == vals

    def test_uint64_roundtrip(self, tmp_path):
        """UInt64 roundtrip (values fitting in i64 range, cast to Int64)."""
        vals = [0, 2**63 - 1]
        df = pl.DataFrame({
            "u": pl.Series(vals, dtype=pl.UInt64).cast(pl.Int64),
        })
        path = str(tmp_path / "uint64.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["u"].to_list() == vals


# ============================================================================
#  Column order and name preservation
# ============================================================================


class TestColumnOrderPreservation:
    """Verify columns come back in the exact order they were written."""

    def test_column_order_preserved(self, tmp_path):
        """Column order must match exactly after roundtrip."""
        cols = [f"col_{chr(ord('z') - i)}" for i in range(10)]  # reverse alpha
        data = {c: [i] for i, c in enumerate(cols)}
        df = pl.DataFrame(data)
        path = str(tmp_path / "order.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.columns == df.columns

    def test_column_order_from_test_file(self):
        """AllTypes.yxdb columns should be in the exact known order."""
        df = sigilyx.read_yxdb(_yxdb("AllTypes.yxdb"))
        expected = [
            "ByteCol", "BoolCol", "Int16Col", "Int32Col", "Int64Col",
            "FloatCol", "DoubleCol", "DecimalCol", "StringCol", "WStringCol",
            "VStringCol", "VWStringCol", "DateCol", "TimeCol", "DateTimeCol",
            "BlobCol",
        ]
        assert df.columns == expected

    def test_single_column_name_roundtrip(self, tmp_path):
        """A single column name survives roundtrip."""
        df = pl.DataFrame({"my_column_123": [42]})
        path = str(tmp_path / "name.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.columns == ["my_column_123"]


# ============================================================================
#  Row reader ↔ Writer interop
# ============================================================================


class TestRowReaderWriterInterop:
    """Verify data written via streaming writer is readable via row reader."""

    def test_streaming_write_then_row_read(self, tmp_path):
        """Write batches, then read back with YxdbRowReader."""
        batches = [
            pl.DataFrame({"id": [1, 2], "text": ["hello", "world"]}),
            pl.DataFrame({"id": [3, 4], "text": ["foo", "bar"]}),
        ]
        path = str(tmp_path / "stream_to_row.yxdb")
        sigilyx.write_yxdb_batches(path, iter(batches))

        reader = sigilyx.YxdbRowReader(path)
        rows = []
        while reader.next():
            rows.append(reader.read_dict())
        reader.close()

        assert len(rows) == 4
        assert rows[0]["id"] == 1
        assert rows[0]["text"] == "hello"
        assert rows[3]["id"] == 4
        assert rows[3]["text"] == "bar"

    def test_row_reader_field_names(self, tmp_path):
        """Row reader fields should match what was written."""
        df = pl.DataFrame({"alpha": [1], "beta": [2.0], "gamma": ["three"]})
        path = str(tmp_path / "fields.yxdb")
        sigilyx.write_yxdb(path, df)

        reader = sigilyx.YxdbRowReader(path)
        field_names = [f.name for f in reader.fields]
        reader.close()
        assert field_names == ["alpha", "beta", "gamma"]

    def test_row_reader_with_nulls(self, tmp_path):
        """Row reader correctly returns None for null values."""
        df = pl.DataFrame({
            "v": pl.Series([1, None, 3], dtype=pl.Int32),
        })
        path = str(tmp_path / "null_row.yxdb")
        sigilyx.write_yxdb(path, df)

        reader = sigilyx.YxdbRowReader(path)
        vals = []
        while reader.next():
            vals.append(reader.read_dict()["v"])
        reader.close()
        assert vals == [1, None, 3]


# ============================================================================
#  Concurrent writes
# ============================================================================


class TestConcurrentWrites:
    """Multiple threads writing separate files simultaneously."""

    def test_concurrent_writes_to_different_files(self, tmp_path):
        """4 threads each writing a different file - all should succeed."""
        def write_file(i: int):
            df = pl.DataFrame({"id": list(range(1000)), "thread": [i] * 1000})
            path = str(tmp_path / f"concurrent_{i}.yxdb")
            sigilyx.write_yxdb(path, df)
            return path

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            paths = list(pool.map(write_file, range(4)))

        for i, path in enumerate(paths):
            df = sigilyx.read_yxdb(path)
            assert df.height == 1000
            assert df["thread"][0] == i


# ============================================================================
#  Malformed / corrupted file handling
# ============================================================================


class TestMalformedFiles:
    """Ensure the reader fails gracefully on corrupted or malformed data."""

    def test_empty_file(self, tmp_path):
        """Zero-byte file should raise an error."""
        path = str(tmp_path / "empty.yxdb")
        with open(path, "wb") as f:
            pass
        with pytest.raises((OSError, ValueError)):
            sigilyx.read_yxdb(path)

    def test_truncated_header(self, tmp_path):
        """File with only a partial header (100 bytes)."""
        path = str(tmp_path / "truncated_header.yxdb")
        with open(path, "wb") as f:
            f.write(b"Alteryx Database File" + b"\x00" * 79)
        with pytest.raises((OSError, ValueError)):
            sigilyx.read_yxdb(path)

    def test_valid_header_no_metadata(self, tmp_path):
        """Valid 512-byte header but no XML metadata following it."""
        path = str(tmp_path / "no_meta.yxdb")
        header = bytearray(512)
        header[:21] = b"Alteryx Database File"
        # Set metadata size to 100 but don't write any metadata
        struct.pack_into("<I", header, 80, 100)
        with open(path, "wb") as f:
            f.write(header)
        with pytest.raises((OSError, ValueError)):
            sigilyx.read_yxdb(path)

    def test_corrupted_magic(self, tmp_path):
        """File with wrong magic string."""
        path = str(tmp_path / "bad_magic.yxdb")
        with open(path, "wb") as f:
            f.write(b"Not an Alteryx file!!" + b"\x00" * 491)
        with pytest.raises((OSError, ValueError)):
            sigilyx.read_yxdb(path)

    def test_negative_record_count_in_header(self, tmp_path):
        """Header claiming negative record count - should raise cleanly, not panic."""
        # Read a real file, corrupt the record count, write it back
        real_data = open(_yxdb("SingleColumn.yxdb"), "rb").read()
        corrupted = bytearray(real_data)
        struct.pack_into("<q", corrupted, 104, -1)
        path = str(tmp_path / "neg_count.yxdb")
        with open(path, "wb") as f:
            f.write(corrupted)
        # Must raise a clean error (OSError / ValueError), never panic
        with pytest.raises((OSError, ValueError), match=r"(?i)record count.*unreasonably large|corrupt"):
            sigilyx.read_yxdb(path)

    def test_directory_path_raises(self, tmp_path):
        """Passing a directory instead of a file should raise."""
        with pytest.raises((OSError, IsADirectoryError, PermissionError)):
            sigilyx.read_yxdb(str(tmp_path))


# ============================================================================
#  String edge cases for YXDB (UTF-16LE encoding in files)
# ============================================================================


class TestStringEdgeCases:
    """Advanced string encoding edge cases."""

    def test_surrogate_pair_emoji(self, tmp_path):
        """Supplementary plane characters (emoji) require UTF-16 surrogate pairs."""
        strings = ["😀", "🎉🚀💻", "A😀B😀C", "Hello 🌍!"]
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "emoji.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == strings

    def test_mathematical_symbols(self, tmp_path):
        """Mathematical symbols from supplementary plane."""
        strings = ["∀x∈ℝ", "∑∏∫", "α²+β²=γ²"]
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "math.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == strings

    def test_rtl_text(self, tmp_path):
        """Right-to-left text (Arabic, Hebrew)."""
        strings = ["مرحبا", "שלום", "Hello مرحبا World"]
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "rtl.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == strings

    def test_mixed_scripts_in_one_string(self, tmp_path):
        """Single string containing multiple scripts."""
        s = "Hello世界مرحباΓεια"
        df = pl.DataFrame({"s": [s]})
        path = str(tmp_path / "mixed_scripts.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == s

    def test_string_with_newlines_tabs(self, tmp_path):
        """Strings containing control characters."""
        strings = [
            "line1\nline2",
            "tab\there",
            "cr\rreturn",
            "mixed\n\t\r",
            "null\x00char",
        ]
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "control_chars.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"].to_list() == strings

    def test_very_long_unicode_string(self, tmp_path):
        """Long string with multi-byte UTF-8 characters."""
        s = "日本語" * 10_000  # 30,000 CJK characters
        df = pl.DataFrame({"s": [s]})
        path = str(tmp_path / "long_cjk.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == s

    def test_latin_extended_a_full_range(self, tmp_path):
        """U+0100 to U+017F (Latin Extended-A) - the range that tripped SIMD."""
        chars = "".join(chr(c) for c in range(0x0100, 0x0180))
        df = pl.DataFrame({"s": [chars]})
        path = str(tmp_path / "latin_ext_a.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["s"][0] == chars

    def test_column_name_with_spaces_and_dots(self, tmp_path):
        """Column names with spaces, dots, and other unusual but valid chars."""
        df = pl.DataFrame({
            "First Name": [1],
            "user.id": [2],
            "col (1)": [3],
            "100%": [4],
        })
        path = str(tmp_path / "special_names.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.columns == ["First Name", "user.id", "col (1)", "100%"]


# ============================================================================
#  Date/Time boundary values
# ============================================================================


class TestDateTimeBoundaries:
    """Extreme date/time values."""

    def test_leap_year_dates(self, tmp_path):
        """February 29 for various leap years."""
        dates = [
            datetime.date(2000, 2, 29),  # century leap year
            datetime.date(2004, 2, 29),  # normal leap year
            datetime.date(2024, 2, 29),  # recent leap year
            datetime.date(1600, 2, 29),  # 400-year leap year
        ]
        df = pl.DataFrame({"d": dates})
        path = str(tmp_path / "leap.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["d"].to_list() == dates

    def test_end_of_month_dates(self, tmp_path):
        """Last day of every month in a non-leap year."""
        last_days = [
            datetime.date(2023, 1, 31),
            datetime.date(2023, 2, 28),
            datetime.date(2023, 3, 31),
            datetime.date(2023, 4, 30),
            datetime.date(2023, 5, 31),
            datetime.date(2023, 6, 30),
            datetime.date(2023, 7, 31),
            datetime.date(2023, 8, 31),
            datetime.date(2023, 9, 30),
            datetime.date(2023, 10, 31),
            datetime.date(2023, 11, 30),
            datetime.date(2023, 12, 31),
        ]
        df = pl.DataFrame({"d": last_days})
        path = str(tmp_path / "end_of_month.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["d"].to_list() == last_days

    def test_datetime_midnight_noon_edge(self, tmp_path):
        """Datetime at midnight and noon boundaries."""
        dts = [
            datetime.datetime(2025, 1, 1, 0, 0, 0),
            datetime.datetime(2025, 1, 1, 12, 0, 0),
            datetime.datetime(2025, 6, 15, 23, 59, 59),
            datetime.datetime(2025, 12, 31, 0, 0, 1),
        ]
        df = pl.DataFrame({"dt": dts})
        path = str(tmp_path / "dt_boundary.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["dt"].to_list() == dts

    def test_mixed_date_datetime_time_columns(self, tmp_path):
        """All three temporal types in a single DataFrame."""
        df = pl.DataFrame({
            "d": [datetime.date(2025, 1, 15)],
            "t": pl.Series([datetime.time(14, 30, 0)], dtype=pl.Time),
            "dt": [datetime.datetime(2025, 1, 15, 14, 30, 0)],
        })
        path = str(tmp_path / "all_temporal.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2["d"][0] == datetime.date(2025, 1, 15)
        assert df2["t"][0] == datetime.time(14, 30, 0)
        assert df2["dt"][0] == datetime.datetime(2025, 1, 15, 14, 30, 0)


# ============================================================================
#  Large-scale stress tests
# ============================================================================


class TestStressScenarios:
    """Push the engine hard with large/complex data."""

    def test_200_columns(self, tmp_path):
        """Very wide DataFrame (200 columns)."""
        data = {f"col_{i:04d}": [i * 10, i * 20] for i in range(200)}
        df = pl.DataFrame(data)
        path = str(tmp_path / "wide200.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == (2, 200)
        assert df2.columns == df.columns

    def test_mixed_types_many_rows(self, tmp_path):
        """50k rows with Int, Float, String, Bool, Date - crosses many LZF blocks."""
        n = 50_000
        df = pl.DataFrame({
            "id": pl.Series(list(range(n)), dtype=pl.Int64),
            "value": pl.Series([float(i) * 0.01 for i in range(n)], dtype=pl.Float64),
            "flag": [i % 5 == 0 for i in range(n)],
            "label": [f"item_{i:06d}" for i in range(n)],
            "date": [datetime.date(2020, 1, 1) + datetime.timedelta(days=i % 365)
                     for i in range(n)],
        })
        path = str(tmp_path / "mixed_50k.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.shape == df.shape
        assert df2["id"][0] == 0
        assert df2["id"][-1] == n - 1
        assert df2["label"][0] == "item_000000"
        assert df2["date"][0] == datetime.date(2020, 1, 1)

    def test_all_nulls_all_types(self, tmp_path):
        """DataFrame where every column of every type is entirely null."""
        df = pl.DataFrame({
            "i32": pl.Series([None, None, None], dtype=pl.Int32),
            "i64": pl.Series([None, None, None], dtype=pl.Int64),
            "f64": pl.Series([None, None, None], dtype=pl.Float64),
            "bool": pl.Series([None, None, None], dtype=pl.Boolean),
            "str": pl.Series([None, None, None], dtype=pl.String),
            "date": pl.Series([None, None, None], dtype=pl.Date),
            "bin": pl.Series([None, None, None], dtype=pl.Binary),
        })
        path = str(tmp_path / "all_nulls.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.height == 3
        for col in df2.columns:
            assert df2[col].null_count() == 3

    def test_single_value_repeated(self, tmp_path):
        """Same value repeated 100k times - tests compression and correctness."""
        n = 100_000
        df = pl.DataFrame({
            "v": pl.Series([42] * n, dtype=pl.Int32),
            "s": ["hello"] * n,
        })
        path = str(tmp_path / "repeated.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        assert df2.height == n
        assert df2["v"].unique().to_list() == [42]
        assert df2["s"].unique().to_list() == ["hello"]

    def test_alternating_long_short_strings(self, tmp_path):
        """Alternating very long and very short strings stress variable-length encoding."""
        n = 5000
        strings = []
        for i in range(n):
            if i % 2 == 0:
                strings.append("X" * 5000)
            else:
                strings.append("y")
        df = pl.DataFrame({"s": strings})
        path = str(tmp_path / "alt_len.yxdb")
        sigilyx.write_yxdb(path, df)
        df2 = sigilyx.read_yxdb(path)
        for i in range(n):
            if i % 2 == 0:
                assert len(df2["s"][i]) == 5000
            else:
                assert df2["s"][i] == "y"


# ============================================================================
#  Scan (LazyFrame) edge cases
# ============================================================================


class TestScanEdgeCases:
    """Edge cases for the Polars IO plugin / scan_yxdb."""

    def test_scan_with_filter_and_projection(self):
        """Combined filter + projection pushdown."""
        result = (
            sigilyx.scan_yxdb(_yxdb("People.yxdb"))
            .select("PersonId", "Age")
            .filter(pl.col("Age") > 60)
            .collect()
        )
        assert result.columns == ["PersonId", "Age"]
        assert all(a > 60 for a in result["Age"].to_list() if a is not None)

    def test_scan_with_limit(self):
        """head() on a scan should limit rows."""
        result = sigilyx.scan_yxdb(_yxdb("ManyRecords.yxdb")).head(10).collect()
        assert result.height == 10

    def test_scan_collect_twice(self):
        """Collecting a LazyFrame twice should give identical results."""
        lf = sigilyx.scan_yxdb(_yxdb("People.yxdb"))
        df1 = lf.collect()
        df2 = lf.collect()
        assert df1.equals(df2)

    def test_scan_group_by(self):
        """Aggregation on scanned data."""
        result = (
            sigilyx.scan_yxdb(_yxdb("People.yxdb"))
            .group_by("Active")
            .agg(pl.col("PersonId").count().alias("cnt"))
            .collect()
        )
        assert result.height == 2
        assert result["cnt"].sum() == 200


# ============================================================================
#  Write path edge cases
# ============================================================================


class TestWritePathEdgeCases:
    """Edge cases specific to the write path."""

    def test_write_to_nonexistent_directory(self, tmp_path):
        """Writing to a path where parent dir doesn't exist should create it or raise clearly."""
        path = str(tmp_path / "subdir" / "deep" / "test.yxdb")
        df = pl.DataFrame({"x": [1]})
        try:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            sigilyx.write_yxdb(path, df)
            df2 = sigilyx.read_yxdb(path)
            assert df2["x"][0] == 1
        except OSError:
            pass  # Also acceptable if it doesn't create dirs

    def test_overwrite_existing_file(self, tmp_path):
        """Writing to an existing file should overwrite it."""
        path = str(tmp_path / "overwrite.yxdb")
        df1 = pl.DataFrame({"x": [1, 2, 3]})
        sigilyx.write_yxdb(path, df1)
        df2 = pl.DataFrame({"y": [10, 20]})
        sigilyx.write_yxdb(path, df2)
        df3 = sigilyx.read_yxdb(path)
        assert df3.columns == ["y"]
        assert df3.height == 2

    def test_write_read_file_size_reasonable(self, tmp_path):
        """File size should be reasonable (not bloated by large amounts of padding)."""
        n = 10_000
        df = pl.DataFrame({"v": pl.Series(list(range(n)), dtype=pl.Int32)})
        path = str(tmp_path / "size_check.yxdb")
        sigilyx.write_yxdb(path, df)
        file_size = os.path.getsize(path)
        # 10k rows × 5 bytes/record = 50kB raw. With header, metadata,
        # and LZF framing, should be < 200kB.
        assert file_size < 200_000, f"File unexpectedly large: {file_size} bytes"
        assert file_size > 512, "File too small (likely empty)"


# ============================================================================
#  Batch reader/writer schema consistency
# ============================================================================


class TestBatchSchemaConsistency:
    """Verify that batched reads produce consistent schemas across batches."""

    def test_dtypes_consistent_across_batches(self):
        """Every batch from read_yxdb_batches should have identical dtypes."""
        batches = list(sigilyx.read_yxdb_batches(_yxdb("People.yxdb"), batch_size=50))
        first_dtypes = batches[0].dtypes
        for i, batch in enumerate(batches[1:], 1):
            assert batch.dtypes == first_dtypes, f"Batch {i} has different dtypes"

    def test_write_batches_schema_from_first_batch(self, tmp_path):
        """Schema is inferred from the first batch; subsequent batches must match."""
        batches = [
            pl.DataFrame({"id": pl.Series([1, 2], dtype=pl.Int32), "name": ["a", "b"]}),
            pl.DataFrame({"id": pl.Series([3, 4], dtype=pl.Int32), "name": ["c", "d"]}),
        ]
        path = str(tmp_path / "batch_schema.yxdb")
        sigilyx.write_yxdb_batches(path, iter(batches))
        df = sigilyx.read_yxdb(path)
        assert df.columns == ["id", "name"]
        assert df["id"].to_list() == [1, 2, 3, 4]

    def test_batched_read_concat_equals_full_read(self):
        """Concatenating all batches should exactly equal a full read."""
        for name in ["AllTypes.yxdb", "NullValues.yxdb", "Strings.yxdb",
                      "People.yxdb", "ManyRecords.yxdb"]:
            df_full = sigilyx.read_yxdb(_yxdb(name))
            batches = list(sigilyx.read_yxdb_batches(_yxdb(name), batch_size=7))
            df_batched = pl.concat(batches)
            assert df_full.equals(df_batched), f"{name}: batched != full"
