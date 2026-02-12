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

Output: benchmarks/data/bench_{profile}_{rows}.yxdb

Usage:
    python benchmarks/generate_benchmark_data.py
    python benchmarks/generate_benchmark_data.py --rows 1000 10000
    python benchmarks/generate_benchmark_data.py --profiles numeric mixed
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
    """Generate n random strings by joining words, using numpy for speed."""
    nprng = np.random.default_rng(SEED + hash((min_words, max_words, null_fraction)) & 0xFFFFFFFF)
    word_arr = np.array(_WORDS)

    # Generate word counts for each string
    word_counts = nprng.integers(min_words, max_words + 1, size=n)
    max_wc = int(word_counts.max())

    # Generate a block of random word indices
    all_indices = nprng.integers(0, len(_WORDS), size=(n, max_wc))

    # Build strings
    result: list[str | None] = []
    for i in range(n):
        wc = word_counts[i]
        words = word_arr[all_indices[i, :wc]]
        result.append(" ".join(words))

    # Apply nulls
    if null_fraction > 0:
        null_mask = nprng.random(n) < null_fraction
        for i in range(n):
            if null_mask[i]:
                result[i] = None

    return result


def generate_string_heavy_fast(n: int, rng: random.Random) -> pl.DataFrame:
    """5 string columns using numpy-accelerated generation."""
    print("(fast) ", end="", flush=True)
    short = _generate_random_strings_fast(n, 1, 2)
    medium = _generate_random_strings_fast(n, 4, 8)
    long_s = _generate_random_strings_fast(n, 15, 35)
    nullable = _generate_random_strings_fast(n, 2, 6, null_fraction=0.3)
    mixed = _generate_random_strings_fast(n, 1, 15)

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

    # Date columns: days offset from 2020-01-01
    base_date = date(2020, 1, 1)
    day_offsets = nprng.integers(0, 1501, size=n)
    dates = [base_date + timedelta(days=int(d)) for d in day_offsets]

    base_dt = datetime(2020, 1, 1, 0, 0, 0)
    sec_offsets = nprng.integers(0, 100_000_001, size=n)
    datetimes = [base_dt + timedelta(seconds=int(s)) for s in sec_offsets]

    # String columns
    names = _generate_random_strings_fast(n, 1, 4)
    descriptions = _generate_random_strings_fast(n, 3, 12)

    return pl.DataFrame({
        "id": ids,
        "amount": amounts,
        "name": names,
        "active": active.tolist(),
        "created_date": dates,
        "updated_at": datetimes,
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
        data[f"str_col_{i:02d}"] = _generate_random_strings_fast(n, 1, 4)
    for i in range(5):
        data[f"bool_col_{i:02d}"] = nprng.choice([True, False], size=n).tolist()
    for i in range(5):
        base_date = date(2020, 1, 1)
        day_offsets = nprng.integers(0, 1501, size=n)
        data[f"date_col_{i:02d}"] = [base_date + timedelta(days=int(d)) for d in day_offsets]

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
    args = parser.parse_args()

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

    total_files = 0
    total_bytes = 0

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

    print()
    print(f"  Generated {total_files} files ({total_bytes / 1024 / 1024:.1f} MB total)")
    print("=" * 70)


if __name__ == "__main__":
    main()
