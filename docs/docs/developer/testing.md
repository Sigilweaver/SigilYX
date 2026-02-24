---
sidebar_position: 3
---

# Testing

SigilYX has tests at both the Rust and Python levels.

## Rust Tests

```bash
cargo test --workspace
```

This runs all unit and integration tests across the workspace (core library, Python bindings crate, and benchmark crate). The test suite currently includes 65+ tests covering:

- Header parsing
- Field type decoding for all 17 types
- LZF compression/decompression round-trips
- Full read/write round-trips
- Column projection
- Batched reads
- Spatial (SHP-to-WKB) conversion
- Edge cases: empty files, null-heavy columns, maximum-length strings

### Run a specific test

```bash
cargo test --package sigilyx test_read_mixed
```

### With output

```bash
cargo test --workspace -- --nocapture
```

## Python Tests

```bash
pytest tests/ -v
```

Python tests verify the full stack from Python API through Rust bindings. They cover:

- `read_yxdb()` / `write_yxdb()` round-trips
- Polars namespace registration (`pl.read_yxdb`, `df.yxdb.write`)
- LazyFrame scan (`pl.scan_yxdb`)
- PyArrow and Pandas paths
- Batched reads with various options
- Metadata functions (`read_yxdb_fields`, `record_count`)
- Edge cases (empty DataFrames, null columns, special characters)

### Run specific tests

```bash
pytest tests/test_read.py -v
pytest tests/ -k "test_polars_plugin" -v
```

## Cross-Implementation Tests

If you have the C++ reference implementations available:

```bash
python benchmarks/test_cross_impl.py
```

This reads test files with both SigilYX and the C++ readers and verifies that the output matches exactly. See `benchmarks/README.md` for environment setup.

## Test Data

Test files live in `sigilyx/test_files/`. These are small YXDB files covering various field type combinations. Benchmark data (100K rows) is generated separately -- see [Benchmarks](/developer/benchmarks).

## Adding Tests

When adding a new feature or fixing a bug:

1. Add a Rust test in the appropriate module (e.g., `reader.rs`, `writer.rs`)
2. Add a Python test in `tests/` if the feature is exposed to Python
3. Run the full suite before submitting:

```bash
cargo test --workspace && pytest tests/ -v
```
