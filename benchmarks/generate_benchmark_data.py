#!/usr/bin/env python3
"""
Benchmark Data Generator
=========================

Generates YXDB files with varying data profiles and row counts for
cross-language benchmarking. Uses sigilyx's writer for deterministic,
reproducible output.

Profiles:
  - numeric:      5 numeric columns (Int32, Int64, Float32, Float64, Int16)
  - string_heavy: 5 string columns (short, medium, long, nullable, mixed)
  - mixed:        8 columns of mixed types (Int64, Float64, Utf8, Bool, Date, Datetime, UInt8, Utf8)
  - wide:         50 columns of mixed types (stress column count)
  - narrow:       2 columns (Int64, Float64)

Row counts: 1,000 / 10,000 / 100,000

Special modes:
  --per-type      Generate one file per YXDB field type (17 files) at 10,100,000 rows
  --giant         Generate a ~10 GB file

Output: benchmarks/data/bench_{profile}_{rows}.yxdb

Usage:
    python benchmarks/generate_benchmark_data.py
    python benchmarks/generate_benchmark_data.py --rows 1000 10000
    python benchmarks/generate_benchmark_data.py --profiles numeric mixed
    python benchmarks/generate_benchmark_data.py --per-type
    python benchmarks/generate_benchmark_data.py --giant
"""

from __future__ import annotations

import argparse
import os
import random
import sys
import time
from datetime import date, datetime, timedelta
from pathlib import Path

import numpy as np

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "python"))

import polars as pl
import sigilyx as yx

DATA_DIR = Path(__file__).resolve().parent / "data"
SEED = 42

DEFAULT_ROW_COUNTS = [1_000, 10_000, 100_000]
DEFAULT_PROFILES = ["numeric", "string_heavy", "mixed", "wide", "narrow"]


def _seeded_rng(seed: int = SEED) -> random.Random:
    return random.Random(seed)


def generate_numeric(n: int, rng: random.Random) -> pl.DataFrame:
    """5 numeric columns: Int32, Int64, Float32, Float64, Int16."""
    return pl.DataFrame({
        "id_i32": [rng.randint(-2_000_000, 2_000_000) for _ in range(n)],
        "value_i64": [rng.randint(-9_000_000_000, 9_000_000_000) for _ in range(n)],
        "measure_f32": [rng.uniform(-1e6, 1e6) for _ in range(n)],
        "amount_f64": [rng.uniform(-1e12, 1e12) for _ in range(n)],
        "small_i16": [rng.randint(-30_000, 30_000) for _ in range(n)],
    }).cast({
        "id_i32": pl.Int32,
        "value_i64": pl.Int64,
        "measure_f32": pl.Float32,
        "amount_f64": pl.Float64,
        "small_i16": pl.Int16,
    })


# Pre-built word lists for deterministic string generation
_WORDS = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
    "hotel", "india", "juliet", "kilo", "lima", "mike", "november",
    "oscar", "papa", "quebec", "romeo", "sierra", "tango", "uniform",
    "victor", "whiskey", "xray", "yankee", "zulu", "one", "two",
    "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "red", "blue", "green", "yellow", "orange", "purple", "black",
    "white", "silver", "golden", "copper", "bronze", "platinum",
]


def _random_string(rng: random.Random, min_len: int, max_len: int) -> str:
    target = rng.randint(min_len, max_len)
    parts = []
    length = 0
    while length < target:
        word = rng.choice(_WORDS)
        parts.append(word)
        length += len(word) + 1
    return " ".join(parts)[:target]


def generate_string_heavy(n: int, rng: random.Random) -> pl.DataFrame:
    """5 string columns of varying length/nullability."""
    short = [_random_string(rng, 5, 15) for _ in range(n)]
    medium = [_random_string(rng, 30, 60) for _ in range(n)]
    long_s = [_random_string(rng, 100, 250) for _ in range(n)]
    nullable = [_random_string(rng, 10, 40) if rng.random() > 0.3 else None for _ in range(n)]
    mixed = [_random_string(rng, 1, 100) for _ in range(n)]

    return pl.DataFrame({
        "short_str": short,
        "medium_str": medium,
        "long_str": long_s,
        "nullable_str": nullable,
        "mixed_len_str": mixed,
    })


def generate_mixed(n: int, rng: random.Random) -> pl.DataFrame:
    """8 columns of mixed types."""
    base_date = date(2020, 1, 1)
    base_dt = datetime(2020, 1, 1, 0, 0, 0)

    return pl.DataFrame({
        "id": list(range(n)),
        "amount": [rng.uniform(-10_000, 100_000) for _ in range(n)],
        "name": [_random_string(rng, 5, 30) for _ in range(n)],
        "active": [rng.choice([True, False]) for _ in range(n)],
        "created_date": [base_date + timedelta(days=rng.randint(0, 1500)) for _ in range(n)],
        "updated_at": [base_dt + timedelta(seconds=rng.randint(0, 100_000_000)) for _ in range(n)],
        "priority": [rng.randint(0, 255) for _ in range(n)],
        "description": [_random_string(rng, 20, 80) for _ in range(n)],
    }).cast({
        "id": pl.Int64,
        "amount": pl.Float64,
        "active": pl.Boolean,
        "created_date": pl.Date,
        "updated_at": pl.Datetime("us"),
        "priority": pl.Int16,
    })


def generate_wide(n: int, rng: random.Random) -> pl.DataFrame:
    """50 columns of mixed types (stress column count)."""
    data = {}
    for i in range(15):
        data[f"int_col_{i:02d}"] = [rng.randint(-1_000_000, 1_000_000) for _ in range(n)]
    for i in range(15):
        data[f"float_col_{i:02d}"] = [rng.uniform(-1e6, 1e6) for _ in range(n)]
    for i in range(10):
        data[f"str_col_{i:02d}"] = [_random_string(rng, 5, 30) for _ in range(n)]
    for i in range(5):
        data[f"bool_col_{i:02d}"] = [rng.choice([True, False]) for _ in range(n)]
    for i in range(5):
        base_date = date(2020, 1, 1)
        data[f"date_col_{i:02d}"] = [base_date + timedelta(days=rng.randint(0, 1500)) for _ in range(n)]

    df = pl.DataFrame(data)
    cast_map = {}
    for col in df.columns:
        if col.startswith("int_"):
            cast_map[col] = pl.Int64
        elif col.startswith("float_"):
            cast_map[col] = pl.Float64
        elif col.startswith("bool_"):
            cast_map[col] = pl.Boolean
        elif col.startswith("date_"):
            cast_map[col] = pl.Date
    return df.cast(cast_map)


def generate_narrow(n: int, rng: random.Random) -> pl.DataFrame:
    """2 columns only: Int64, Float64."""
    return pl.DataFrame({
        "id": list(range(n)),
        "value": [rng.gauss(0, 1000) for _ in range(n)],
    }).cast({
        "id": pl.Int64,
        "value": pl.Float64,
    })


# ── Fast generators (numpy-based, for large row counts) ──────────────────

FAST_THRESHOLD = 500_000  # Use fast generators above this row count


def generate_numeric_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """5 numeric columns using numpy for bulk generation."""
    nprng = np.random.default_rng(SEED)
    return pl.DataFrame({
        "id_i32": nprng.integers(-2_000_000, 2_000_001, size=n, dtype=np.int32),
        "value_i64": nprng.integers(-9_000_000_000, 9_000_000_001, size=n, dtype=np.int64),
        "measure_f32": nprng.uniform(-1e6, 1e6, size=n).astype(np.float32),
        "amount_f64": nprng.uniform(-1e12, 1e12, size=n),
        "small_i16": nprng.integers(-30_000, 30_001, size=n, dtype=np.int16),
    })


def _generate_random_strings_fast(n: int, min_words: int, max_words: int,
                                   null_fraction: float = 0.0) -> list[str | None]:
    """Generate n random strings by joining words using fully-vectorised numpy."""
    nprng = np.random.default_rng(SEED + hash((min_words, max_words, null_fraction)) & 0xFFFFFFFF)
    word_arr = np.array(_WORDS, dtype=object)
    nw = len(word_arr)

    if min_words == max_words:
        wc = min_words
        # shape (n, wc), look up words, join with space column-by-column
        indices = nprng.integers(0, nw, size=(n, wc))
        cols = [word_arr[indices[:, k]] for k in range(wc)]
        # start with first column then add " " + next column
        result_arr = cols[0].copy()
        for k in range(1, wc):
            result_arr = np.char.add(np.char.add(result_arr, " "), cols[k])
    else:
        max_wc = max_words
        indices = nprng.integers(0, nw, size=(n, max_wc))
        word_counts = nprng.integers(min_words, max_words + 1, size=n).astype(np.int32)
        cols = [word_arr[indices[:, k]] for k in range(max_wc)]
        # build base from first `min_words` columns (always included)
        result_arr = cols[0].copy()
        for k in range(1, min_words):
            result_arr = np.char.add(np.char.add(result_arr, " "), cols[k])
        # conditionally add each optional column
        for k in range(min_words, max_wc):
            mask = word_counts > k   # rows that include word k
            if mask.any():
                addition = np.char.add(" ", cols[k])
                result_arr = np.where(mask, np.char.add(result_arr, addition), result_arr)

    result: list[str | None] = result_arr.tolist()

    # Apply nulls
    if null_fraction > 0:
        null_indices = np.where(nprng.random(n) < null_fraction)[0]
        for i in null_indices:
            result[i] = None

    return result


def generate_string_heavy_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """5 string columns using pool-sampling (memory-efficient at any n)."""
    print("(fast) ", end="", flush=True)
    pool_short = _make_string_pool(5_000, 1, 2)
    pool_medium = _make_string_pool(5_000, 4, 8)
    pool_long = _make_string_pool(5_000, 5, 12)  # reasonable length, still varied
    pool_generic = _make_string_pool(5_000, 2, 6)
    rng2 = np.random.default_rng(SEED)

    short = _tile_strings_series(n, pool_short, rng2)
    medium = _tile_strings_series(n, pool_medium, rng2)
    long_s = _tile_strings_series(n, pool_long, rng2)
    # nullable: sample then mark ~30% as null
    nullable = _tile_strings_series(n, pool_generic, rng2)
    null_mask = pl.Series(rng2.random(n) < 0.3)
    nullable = nullable.set(null_mask, None)
    mixed = _tile_strings_series(n, pool_generic, rng2)

    return pl.DataFrame({
        "short_str": short,
        "medium_str": medium,
        "long_str": long_s,
        "nullable_str": nullable,
        "mixed_len_str": mixed,
    })


def generate_mixed_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """8 columns of mixed types using numpy."""
    print("(fast) ", end="", flush=True)
    nprng = np.random.default_rng(SEED)

    # Numeric columns
    ids = np.arange(n, dtype=np.int64)
    amounts = nprng.uniform(-10_000, 100_000, size=n)
    priorities = nprng.integers(0, 256, size=n, dtype=np.int16)
    active = nprng.choice([True, False], size=n)

    # Date: Polars Date stores days since epoch; 2020-01-01 = day 18262
    base_day = 18262
    day_offsets = nprng.integers(0, 1501, size=n).astype(np.int32)
    dates_int = (base_day + day_offsets)

    # Datetime: Polars Datetime("us") stores microseconds since epoch
    # 2020-01-01 = 1577836800 seconds = 1577836800_000000 us
    base_us = 1577836800_000000
    sec_offsets = nprng.integers(0, 100_000_001, size=n).astype(np.int64)
    datetimes_us = base_us + sec_offsets * 1_000_000

    # String columns
    pool = _make_string_pool(5_000, 1, 4)
    pool_desc = _make_string_pool(5_000, 3, 8)
    names = _tile_strings_series(n, pool, nprng)
    descriptions = _tile_strings_series(n, pool_desc, nprng)

    return pl.DataFrame({
        "id": ids,
        "amount": amounts,
        "name": names,
        "active": active.tolist(),
        "created_date": dates_int.astype(np.int32).tolist(),
        "updated_at": datetimes_us.tolist(),
        "priority": priorities,
        "description": descriptions,
    }).cast({
        "id": pl.Int64,
        "amount": pl.Float64,
        "active": pl.Boolean,
        "created_date": pl.Date,
        "updated_at": pl.Datetime("us"),
        "priority": pl.Int16,
    })


def generate_wide_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """50 columns of mixed types using numpy."""
    print("(fast) ", end="", flush=True)
    nprng = np.random.default_rng(SEED)
    data = {}

    for i in range(15):
        data[f"int_col_{i:02d}"] = nprng.integers(-1_000_000, 1_000_001, size=n, dtype=np.int64)
    for i in range(15):
        data[f"float_col_{i:02d}"] = nprng.uniform(-1e6, 1e6, size=n)
    for i in range(10):
        pool_s = _make_string_pool(5_000, 1, 4)
        data[f"str_col_{i:02d}"] = _tile_strings_series(n, pool_s, nprng)
    for i in range(5):
        data[f"bool_col_{i:02d}"] = nprng.choice([True, False], size=n).tolist()
    base_day = 18262  # 2020-01-01 in Polars Date (days since epoch)
    for i in range(5):
        day_offsets = nprng.integers(0, 1501, size=n).astype(np.int32)
        data[f"date_col_{i:02d}"] = (base_day + day_offsets).tolist()

    df = pl.DataFrame(data)
    cast_map = {}
    for col in df.columns:
        if col.startswith("int_"):
            cast_map[col] = pl.Int64
        elif col.startswith("float_"):
            cast_map[col] = pl.Float64
        elif col.startswith("bool_"):
            cast_map[col] = pl.Boolean
        elif col.startswith("date_"):
            cast_map[col] = pl.Date
    return df.cast(cast_map)


def generate_narrow_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """2 columns using numpy."""
    nprng = np.random.default_rng(SEED)
    return pl.DataFrame({
        "id": np.arange(n, dtype=np.int64),
        "value": nprng.normal(0, 1000, size=n),
    }).cast({
        "id": pl.Int64,
        "value": pl.Float64,
    })


# Fast generator lookup (used when n > FAST_THRESHOLD)
FAST_GENERATORS = {
    "numeric": generate_numeric_fast,
    "string_heavy": generate_string_heavy_fast,
    "mixed": generate_mixed_fast,
    "wide": generate_wide_fast,
    "narrow": generate_narrow_fast,
}


# ── Per-YXDB-type generators ────────────────────────────────────────────
# Each returns a DataFrame with 5 columns of that single YXDB type.
# For types that require explicit schema overrides (String, WString, VString),
# the generator returns (DataFrame, type_overrides_dict).

PER_TYPE_ROWS = 10_100_000

_YXDB_TYPES = [
    "Bool", "Byte", "Int16", "Int32", "Int64", "Float", "Double",
    "FixedDecimal", "String", "WString", "VString", "VWString",
    "Date", "Time", "DateTime", "Blob", "SpatialObj",
]


def _gen_type_bool(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"bool_{i}": nprng.choice([True, False], size=n).tolist() for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Boolean for c in data})


def _gen_type_byte(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"byte_{i}": nprng.integers(0, 128, size=n, dtype=np.int8) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Int8 for c in data})


def _gen_type_int16(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"i16_{i}": nprng.integers(-30_000, 30_001, size=n, dtype=np.int16) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Int16 for c in data})


def _gen_type_int32(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"i32_{i}": nprng.integers(-2_000_000, 2_000_001, size=n, dtype=np.int32) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Int32 for c in data})


def _gen_type_int64(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"i64_{i}": nprng.integers(-9_000_000_000, 9_000_000_001, size=n, dtype=np.int64) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Int64 for c in data})


def _gen_type_float(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"f32_{i}": nprng.uniform(-1e6, 1e6, size=n).astype(np.float32) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Float32 for c in data})


def _gen_type_double(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    data = {f"f64_{i}": nprng.uniform(-1e12, 1e12, size=n) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Float64 for c in data})


def _gen_type_fixeddecimal(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    # Generate as float64 then cast to Decimal(19,4)
    data = {f"dec_{i}": nprng.uniform(-1e10, 1e10, size=n) for i in range(5)}
    df = pl.DataFrame(data)
    return df.cast({c: pl.Decimal(precision=19, scale=4) for c in data})


def _make_string_pool(pool_size: int, min_words: int, max_words: int, seed_offset: int = 0) -> np.ndarray:
    """Build a small pool of strings, then tile/index into it to fill n rows cheaply."""
    return np.array(_generate_random_strings_fast(pool_size, min_words, max_words), dtype=object)


def _tile_strings_series(n: int, pool: np.ndarray, rng: np.random.Generator) -> pl.Series:
    """Sample with replacement from a pool — stays in Polars Series (fast, low RAM)."""
    pool_series = pl.Series(pool.tolist())  # only done once per call
    idx = rng.integers(0, len(pool), size=n)
    return pool_series.gather(idx)


def _gen_type_vwstring(n: int) -> pl.DataFrame:
    """VWString = Polars String (default mapping)."""
    rng = np.random.default_rng(SEED)
    pool = _make_string_pool(10_000, 2, 8)
    cols = {f"vws_{i}": _tile_strings_series(n, pool, rng) for i in range(5)}
    return pl.DataFrame(cols)


def _gen_type_string(n: int) -> tuple[pl.DataFrame, dict]:
    """Fixed-width narrow String — requires type_overrides."""
    rng = np.random.default_rng(SEED)
    pool = _make_string_pool(10_000, 2, 6)
    col_names = [f"str_{i}" for i in range(5)]
    cols = {c: _tile_strings_series(n, pool, rng) for c in col_names}
    df = pl.DataFrame(cols)
    overrides = {c: {"type": "String", "size": 64} for c in col_names}
    return df, overrides


def _gen_type_wstring(n: int) -> tuple[pl.DataFrame, dict]:
    """Fixed-width wide WString — requires type_overrides."""
    rng = np.random.default_rng(SEED)
    pool = _make_string_pool(10_000, 2, 6)
    col_names = [f"wstr_{i}" for i in range(5)]
    cols = {c: _tile_strings_series(n, pool, rng) for c in col_names}
    df = pl.DataFrame(cols)
    overrides = {c: {"type": "WString", "size": 64} for c in col_names}
    return df, overrides


def _gen_type_vstring(n: int) -> tuple[pl.DataFrame, dict]:
    """Variable-length narrow VString — requires type_overrides."""
    rng = np.random.default_rng(SEED)
    pool = _make_string_pool(10_000, 2, 8)
    col_names = [f"vs_{i}" for i in range(5)]
    cols = {c: _tile_strings_series(n, pool, rng) for c in col_names}
    df = pl.DataFrame(cols)
    overrides = {c: {"type": "V_String", "size": 262144} for c in col_names}
    return df, overrides


def _gen_type_date(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    # Polars Date is days since 1970-01-01; 2000-01-01 = day 10957
    base_day = 10957
    data = {f"date_{i}": (nprng.integers(0, 10_000, size=n) + base_day).astype(np.int32) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Date for c in data})


def _gen_type_time(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    # Polars Time is nanoseconds since midnight
    data = {f"time_{i}": (nprng.integers(0, 86400, size=n) * 1_000_000_000).astype(np.int64) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Time for c in data})


def _gen_type_datetime(n: int) -> pl.DataFrame:
    nprng = np.random.default_rng(SEED)
    # Polars Datetime("us") is microseconds since 1970-01-01 00:00:00
    # 2000-01-01 = 10957 days = 946684800 seconds = 946684800_000000 microseconds
    base_us = 946684800_000000
    data = {f"dt_{i}": (nprng.integers(0, 500_000_000, size=n) * 1_000_000 + base_us).astype(np.int64) for i in range(5)}
    return pl.DataFrame(data).cast({c: pl.Datetime("us") for c in data})


def _gen_type_blob(n: int) -> pl.DataFrame:
    """Blob — pool of 1000 distinct 32-byte values, sampled with replacement (memory-efficient)."""
    nprng = np.random.default_rng(SEED)
    pool_size = 1_000
    blob_size = 32
    pool_raw = nprng.bytes(pool_size * blob_size)
    pool_np = np.array([pool_raw[j*blob_size:(j+1)*blob_size] for j in range(pool_size)], dtype=object)
    data = {}
    for i in range(5):
        idx = nprng.integers(0, pool_size, size=n)
        data[f"blob_{i}"] = pool_np[idx].tolist()
    return pl.DataFrame(data).cast({c: pl.Binary for c in data})


def _gen_type_spatialobj(n: int) -> pl.DataFrame:
    """SpatialObj — pool of 1000 distinct WKB points, sampled with replacement."""
    nprng = np.random.default_rng(SEED)
    import struct
    pool_size = 1_000
    header = struct.pack("<bI", 1, 1)
    lons = nprng.uniform(-180, 180, size=pool_size)
    lats = nprng.uniform(-90, 90, size=pool_size)
    pool_np = np.array([header + struct.pack("<dd", lons[j], lats[j]) for j in range(pool_size)], dtype=object)
    data = {}
    for i in range(5):
        idx = nprng.integers(0, pool_size, size=n)
        data[f"geom_{i}"] = pool_np[idx].tolist()
    return pl.DataFrame(data).cast({c: pl.Binary for c in data})


# Map YXDB type name → generator function
# Generators that return a tuple (df, overrides) need special handling
_TYPE_GENERATORS = {
    "Bool": _gen_type_bool,
    "Byte": _gen_type_byte,
    "Int16": _gen_type_int16,
    "Int32": _gen_type_int32,
    "Int64": _gen_type_int64,
    "Float": _gen_type_float,
    "Double": _gen_type_double,
    "FixedDecimal": _gen_type_fixeddecimal,
    "String": _gen_type_string,
    "WString": _gen_type_wstring,
    "VString": _gen_type_vstring,
    "VWString": _gen_type_vwstring,
    "Date": _gen_type_date,
    "Time": _gen_type_time,
    "DateTime": _gen_type_datetime,
    "Blob": _gen_type_blob,
    "SpatialObj": _gen_type_spatialobj,
}

# Types where the generator returns (df, overrides) tuple
_OVERRIDE_TYPES = {"String", "WString", "VString"}
# Types where spatial_columns must be set
_SPATIAL_TYPES = {"SpatialObj"}


def generate_per_type_files(n_rows: int = PER_TYPE_ROWS) -> tuple[int, int]:
    """Generate one file per YXDB type. Returns (file_count, total_bytes)."""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    total_files = 0
    total_bytes = 0

    print(f"\n{'='*70}")
    print(f"Per-Type Benchmarks ({n_rows:,} rows × 5 cols each)")
    print(f"{'='*70}")

    for type_name in _YXDB_TYPES:
        gen_func = _TYPE_GENERATORS[type_name]
        fname = f"bench_type_{type_name.lower()}_{n_rows}.yxdb"
        fpath = DATA_DIR / fname

        if fpath.exists():
            fsize = os.path.getsize(fpath)
            size_str = (f"{fsize/1024/1024:.1f} MB" if fsize < 1024**3
                        else f"{fsize/1024/1024/1024:.2f} GB")
            print(f"  Skipping  {fname} (already exists, {size_str})")
            total_files += 1
            total_bytes += fsize
            continue

        print(f"  Generating {fname}...", end=" ", flush=True)
        t0 = time.perf_counter()
        result = gen_func(n_rows)
        t_gen = time.perf_counter() - t0

        t0 = time.perf_counter()
        if type_name in _OVERRIDE_TYPES:
            df, overrides = result
            spatial = list(df.columns) if type_name in _SPATIAL_TYPES else None
            yx.write_yxdb_with_overrides(str(fpath), df, overrides, spatial_columns=spatial)
        elif type_name in _SPATIAL_TYPES:
            df = result
            yx.write_yxdb(str(fpath), df, spatial_columns=list(df.columns))
        else:
            df = result
            yx.write_yxdb(str(fpath), df)
        t_write = time.perf_counter() - t0

        fsize = os.path.getsize(fpath)
        total_files += 1
        total_bytes += fsize

        if fsize < 1024 * 1024:
            size_str = f"{fsize / 1024:.1f} KB"
        elif fsize < 1024 * 1024 * 1024:
            size_str = f"{fsize / 1024 / 1024:.1f} MB"
        else:
            size_str = f"{fsize / 1024 / 1024 / 1024:.2f} GB"

        print(f"{n_rows:,} rows × 5 cols, "
              f"{size_str}, "
              f"gen: {t_gen:.3f}s, write: {t_write:.3f}s")

        # Free memory before next iteration (important after large types like FixedDecimal)
        del result, df
        if "overrides" in dir():
            del overrides
        import gc; gc.collect()

    return total_files, total_bytes


# ── Giant (~10 GB) profile ───────────────────────────────────────────────

def generate_giant() -> tuple[int, int]:
    """Generate a ~10 GB YXDB file with a wide mixed schema.

    Schema: 20 columns of mixed types, enough rows to reach ~10 GB.
    At ~100 bytes/row (avg), ~100M rows ≈ 10 GB on disk.
    """
    DATA_DIR.mkdir(parents=True, exist_ok=True)

    # Estimate: 20 cols × ~5 bytes avg fixed + variable strings ≈ 100-120 bytes/row
    # ~90M rows should produce roughly 10 GB
    target_rows = 90_000_000
    fname = f"bench_giant_{target_rows}.yxdb"
    fpath = DATA_DIR / fname

    print(f"\n{'='*70}")
    print(f"Giant Benchmark (~10 GB target)")
    print(f"{'='*70}")
    print(f"  File:   {fname}")
    print(f"  Rows:   {target_rows:,}")
    print(f"  Schema: 20 columns (mixed types)")
    print()

    # Generate in batches to avoid OOM
    batch_size = 5_000_000
    nprng = np.random.default_rng(SEED)

    def make_batch(offset: int, n: int) -> pl.DataFrame:
        """Generate a batch of n rows using vectorized approaches (no Python loops)."""
        # String columns: use pool sampling (stays in Polars, low RAM)
        str_pool_short  = _make_string_pool(1_000, 1, 3,  seed_offset=1)
        str_pool_medium = _make_string_pool(1_000, 3, 8,  seed_offset=2)
        str_pool_long   = _make_string_pool(1_000, 8, 20, seed_offset=3)
        str_pool_tag    = _make_string_pool(1_000, 1, 4,  seed_offset=4)

        # Date: cast int32 offsets directly to pl.Date
        base_day = 10957  # 2000-01-01 as Polars Date integer
        date_ints = (nprng.integers(0, 10_000, size=n) + base_day).astype(np.int32)

        # Datetime: cast int64 microseconds directly to pl.Datetime("us")
        base_us = 946684800_000000  # 2000-01-01 00:00:00 in microseconds
        dt_us = (nprng.integers(0, 500_000_000, size=n).astype(np.int64) * 1_000_000 + base_us)

        data = {
            "id": np.arange(offset, offset + n, dtype=np.int64),
            "i32_a": nprng.integers(-2_000_000, 2_000_001, size=n, dtype=np.int32),
            "i32_b": nprng.integers(-2_000_000, 2_000_001, size=n, dtype=np.int32),
            "i64_a": nprng.integers(-9_000_000_000, 9_000_000_001, size=n, dtype=np.int64),
            "f32_a": nprng.uniform(-1e6, 1e6, size=n).astype(np.float32),
            "f32_b": nprng.uniform(-1e6, 1e6, size=n).astype(np.float32),
            "f64_a": nprng.uniform(-1e12, 1e12, size=n),
            "f64_b": nprng.uniform(-1e12, 1e12, size=n),
            "bool_a": nprng.integers(0, 2, size=n, dtype=np.int8),
            "bool_b": nprng.integers(0, 2, size=n, dtype=np.int8),
            "i16_a": nprng.integers(-30_000, 30_001, size=n, dtype=np.int16),
            "byte_a": nprng.integers(0, 256, size=n, dtype=np.int16),
            "date_a": date_ints,
            "datetime_a": dt_us,
            "active": nprng.integers(0, 2, size=n, dtype=np.int8),
            "score_a": nprng.uniform(0, 100, size=n),
        }

        df = pl.DataFrame(data).cast({
            "id": pl.Int64,
            "i32_a": pl.Int32,
            "i32_b": pl.Int32,
            "i64_a": pl.Int64,
            "f32_a": pl.Float32,
            "f32_b": pl.Float32,
            "f64_a": pl.Float64,
            "f64_b": pl.Float64,
            "bool_a": pl.Boolean,
            "bool_b": pl.Boolean,
            "i16_a": pl.Int16,
            "byte_a": pl.Int16,
            "date_a": pl.Date,
            "datetime_a": pl.Datetime("us"),
            "active": pl.Boolean,
            "score_a": pl.Float64,
        })

        # Add string columns via Polars pool sampling
        df = df.with_columns([
            _tile_strings_series(n, str_pool_short,  nprng).alias("str_short"),
            _tile_strings_series(n, str_pool_medium, nprng).alias("str_medium"),
            _tile_strings_series(n, str_pool_long,   nprng).alias("str_long"),
            _tile_strings_series(n, str_pool_tag,    nprng).alias("tag"),
        ])

        return df

    # Write first batch to initialize the writer, then stream the rest
    n_batches = (target_rows + batch_size - 1) // batch_size
    t0_total = time.perf_counter()

    print(f"  Writing {n_batches} batches of {batch_size:,} rows...", flush=True)

    def batch_generator():
        rows_so_far = 0
        for batch_idx in range(n_batches):
            remaining = target_rows - rows_so_far
            n = min(batch_size, remaining)
            if n <= 0:
                break
            batch = make_batch(rows_so_far, n)
            rows_so_far += n
            if (batch_idx + 1) % 5 == 0 or batch_idx == n_batches - 1:
                elapsed = time.perf_counter() - t0_total
                pct = rows_so_far / target_rows * 100
                print(f"    Batch {batch_idx + 1}/{n_batches} "
                      f"({pct:.0f}%, {rows_so_far:,} rows, {elapsed:.1f}s)", flush=True)
            yield batch

    rows_written = yx.write_yxdb_batches(str(fpath), batch_generator())
    t_total = time.perf_counter() - t0_total

    fsize = os.path.getsize(fpath)
    size_gb = fsize / 1024 / 1024 / 1024
    # Get column count from a small sample batch
    sample = make_batch(0, 1)
    print(f"\n  Done: {rows_written:,} rows × {sample.width} cols, "
          f"{size_gb:.2f} GB, {t_total:.1f}s")

    return 1, fsize


GENERATORS = {
    "numeric": generate_numeric,
    "string_heavy": generate_string_heavy,
    "mixed": generate_mixed,
    "wide": generate_wide,
    "narrow": generate_narrow,
}


def main():
    parser = argparse.ArgumentParser(
        description="Generate benchmark YXDB data files",
    )
    parser.add_argument("--rows", nargs="+", type=int, default=None,
                        help=f"Row counts to generate (default: {DEFAULT_ROW_COUNTS})")
    parser.add_argument("--profiles", nargs="+", default=None,
                        help=f"Data profiles to generate (default: {DEFAULT_PROFILES})")
    parser.add_argument("--per-type", action="store_true",
                        help=f"Generate one file per YXDB type ({len(_YXDB_TYPES)} types) "
                             f"at {PER_TYPE_ROWS:,} rows")
    parser.add_argument("--per-type-rows", type=int, default=PER_TYPE_ROWS,
                        help=f"Row count for per-type files (default: {PER_TYPE_ROWS:,})")
    parser.add_argument("--giant", action="store_true",
                        help="Generate a ~10 GB benchmark file")
    args = parser.parse_args()

    # If --per-type or --giant specified alone, skip default profiles
    run_default = not args.per_type and not args.giant
    if args.rows or args.profiles:
        run_default = True

    total_files = 0
    total_bytes = 0

    if run_default:
        row_counts = args.rows or DEFAULT_ROW_COUNTS
        profiles = args.profiles or DEFAULT_PROFILES

        DATA_DIR.mkdir(parents=True, exist_ok=True)

        print("=" * 70)
        print("SigilYX Benchmark Data Generator")
        print("=" * 70)
        print(f"  Profiles:   {', '.join(profiles)}")
        print(f"  Row counts: {', '.join(str(r) for r in row_counts)}")
        print(f"  Output dir: {DATA_DIR}")
        print(f"  Seed:       {SEED}")
        print()

        for profile in profiles:
            if profile not in GENERATORS:
                print(f"  WARNING: Unknown profile '{profile}', skipping")
                continue

            gen_func = GENERATORS[profile]

            for n_rows in row_counts:
                rng = _seeded_rng(SEED)
                fname = f"bench_{profile}_{n_rows}.yxdb"
                fpath = DATA_DIR / fname

                # Use fast generators for large row counts
                if n_rows >= FAST_THRESHOLD and profile in FAST_GENERATORS:
                    gen_func = FAST_GENERATORS[profile]
                else:
                    gen_func = GENERATORS[profile]

                print(f"  Generating {fname}...", end=" ", flush=True)
                t0 = time.perf_counter()
                df = gen_func(n_rows, rng)
                t_gen = time.perf_counter() - t0

                t0 = time.perf_counter()
                yx.write_yxdb(str(fpath), df)
                t_write = time.perf_counter() - t0

                fsize = os.path.getsize(fpath)
                total_files += 1
                total_bytes += fsize

                size_str = f"{fsize / 1024:.1f} KB" if fsize < 1024 * 1024 else f"{fsize / 1024 / 1024:.1f} MB"
                print(f"{df.height:,} rows x {df.width} cols, "
                      f"{size_str}, "
                      f"gen: {t_gen:.3f}s, write: {t_write:.3f}s")

    if args.per_type:
        pf, pb = generate_per_type_files(args.per_type_rows)
        total_files += pf
        total_bytes += pb

    if args.giant:
        gf, gb = generate_giant()
        total_files += gf
        total_bytes += gb

    print()
    if total_bytes < 1024 * 1024 * 1024:
        print(f"  Generated {total_files} files ({total_bytes / 1024 / 1024:.1f} MB total)")
    else:
        print(f"  Generated {total_files} files ({total_bytes / 1024 / 1024 / 1024:.2f} GB total)")
    print("=" * 70)


if __name__ == "__main__":
    main()
