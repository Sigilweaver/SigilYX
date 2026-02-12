#!/usr/bin/env python3
"""
Generate Clean Benchmark Data for SigilYX

This script generates synthetic test data using the Faker library.
The data is saved in Arrow IPC format, which can then be converted to YXDB
using Alteryx Designer's Python Tool.

Usage:
    python benchmarks/generate_clean_data.py [--rows N] [--output PATH]

Default: 1,000,000 rows saved to benchmarks/clean_benchmark.arrow

To convert to YXDB (in Alteryx Designer Python Tool):
    
    import pandas as pd
    import pyarrow.ipc as ipc
    from ayx import Alteryx
    
    with open("clean_benchmark.arrow", "rb") as f:
        reader = ipc.open_file(f)
        table = reader.read_all()
    df = table.to_pandas()
    Alteryx.write(df, 1)  # Output to anchor 1

All generated data is synthetic and contains no real personal information.
"""

from __future__ import annotations

import argparse
import random
import sys
import time
from datetime import date, timedelta
from pathlib import Path

try:
    import polars as pl
except ImportError:
    print("ERROR: polars not installed. Run `pip install polars` first.")
    sys.exit(1)

try:
    from faker import Faker
except ImportError:
    print("ERROR: faker not installed. Run `pip install faker` first.")
    sys.exit(1)


def generate_clean_data(
    num_rows: int = 1_000_000,
    seed: int = 42,
) -> pl.DataFrame:
    """
    Generate synthetic benchmark data typical of Alteryx workflows.
    
    Columns:
        - Account_ID (Int64): Unique account identifier
        - Customer_Name (String): Full name (faker.name())
        - Transaction_Date (Date): Random date in past 2 years
        - Sales_Amount (Float64): Random amount 10.00 - 10000.00
        - Notes (String): Long text to test variable length buffers
    
    Parameters
    ----------
    num_rows : int
        Number of rows to generate.
    seed : int
        Random seed for reproducibility.
    
    Returns
    -------
    polars.DataFrame
        Generated synthetic data.
    """
    print(f"Generating {num_rows:,} rows of synthetic data...")
    start_time = time.perf_counter()
    
    # Set seeds for reproducibility
    random.seed(seed)
    Faker.seed(seed)
    fake = Faker()
    
    # Pre-compute date range
    end_date = date.today()
    start_date = end_date - timedelta(days=730)  # 2 years
    date_range_days = (end_date - start_date).days
    
    # Generate data in chunks for memory efficiency
    chunk_size = min(100_000, num_rows)
    chunks = []
    
    rows_generated = 0
    while rows_generated < num_rows:
        current_chunk_size = min(chunk_size, num_rows - rows_generated)
        
        # Generate data for this chunk
        account_ids = list(range(rows_generated + 1, rows_generated + current_chunk_size + 1))
        
        customer_names = [fake.name() for _ in range(current_chunk_size)]
        
        transaction_dates = [
            start_date + timedelta(days=random.randint(0, date_range_days))
            for _ in range(current_chunk_size)
        ]
        
        sales_amounts = [
            round(random.uniform(10.0, 10000.0), 2)
            for _ in range(current_chunk_size)
        ]
        
        # Generate varied-length notes to test V_String handling
        notes = []
        for _ in range(current_chunk_size):
            note_length = random.choice([0, 1, 2, 3])  # Vary length
            if note_length == 0:
                notes.append("")
            elif note_length == 1:
                notes.append(fake.sentence())
            elif note_length == 2:
                notes.append(fake.paragraph())
            else:
                notes.append(fake.text(max_nb_chars=500))
        
        chunk_df = pl.DataFrame({
            "Account_ID": account_ids,
            "Customer_Name": customer_names,
            "Transaction_Date": transaction_dates,
            "Sales_Amount": sales_amounts,
            "Notes": notes,
        })
        
        chunks.append(chunk_df)
        rows_generated += current_chunk_size
        
        # Progress indicator
        progress = rows_generated / num_rows * 100
        print(f"  Progress: {rows_generated:,} / {num_rows:,} ({progress:.1f}%)", end="\r")
    
    print()  # New line after progress
    
    # Combine all chunks
    df = pl.concat(chunks)
    
    elapsed = time.perf_counter() - start_time
    print(f"Generated {len(df):,} rows in {elapsed:.2f}s")
    print(f"Schema:")
    for name, dtype in zip(df.columns, df.dtypes):
        print(f"  - {name}: {dtype}")
    
    return df


def main():
    parser = argparse.ArgumentParser(
        description="Generate synthetic benchmark data for SigilYX"
    )
    parser.add_argument(
        "--rows",
        type=int,
        default=1_000_000,
        help="Number of rows to generate (default: 1,000,000)",
    )
    parser.add_argument(
        "--output",
        type=str,
        default=None,
        help="Output file path (default: benchmarks/clean_benchmark.arrow)",
    )
    parser.add_argument(
        "--format",
        choices=["arrow", "csv", "parquet"],
        default="arrow",
        help="Output format (default: arrow)",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=42,
        help="Random seed for reproducibility (default: 42)",
    )
    args = parser.parse_args()
    
    # Determine output path
    if args.output:
        output_path = Path(args.output)
    else:
        benchmarks_dir = Path(__file__).parent
        output_path = benchmarks_dir / f"clean_benchmark.{args.format}"
    
    # Ensure output directory exists
    output_path.parent.mkdir(parents=True, exist_ok=True)
    
    # Generate data
    df = generate_clean_data(num_rows=args.rows, seed=args.seed)
    
    # Save data
    print(f"\nSaving to {output_path}...")
    start_time = time.perf_counter()
    
    if args.format == "arrow":
        df.write_ipc(output_path)
    elif args.format == "csv":
        df.write_csv(output_path)
    elif args.format == "parquet":
        df.write_parquet(output_path)
    
    elapsed = time.perf_counter() - start_time
    file_size_mb = output_path.stat().st_size / (1024 * 1024)
    print(f"Saved in {elapsed:.2f}s ({file_size_mb:.1f} MB)")
    
    print(f"\n✓ Benchmark data ready: {output_path}")
    print(f"  Use this file as input for write benchmarks.")


if __name__ == "__main__":
    main()
