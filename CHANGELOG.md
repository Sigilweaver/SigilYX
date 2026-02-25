# Changelog

All notable changes to SigilYX will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-02-25

### Added

- **Core Rust library** for reading and writing Alteryx YXDB files
  - Columnar reader with parallel LZF decompression and SIMD UTF-16 transcoding
  - Row-by-row streaming reader
  - Pipelined writer with background compression
  - Memory-mapped I/O
  - Support for all 17 YXDB field types
- **Python bindings** via PyO3 and pyo3-polars
  - `read_yxdb()` — Polars DataFrame (zero-copy via Arrow C Data Interface)
  - `read_yxdb_arrow()` — PyArrow Table
  - `read_yxdb_pandas()` — Pandas DataFrame
  - `write_yxdb()`, `write_yxdb_arrow()`, `write_yxdb_pandas()`
  - Batched streaming reads with `read_yxdb_batches()`
  - Lazy scan with `scan_yxdb()` (projection and n_rows pushdown)
- **Polars integration** — official namespace plugins
  - `pl.read_yxdb()`, `pl.scan_yxdb()`
  - `df.yxdb.write()`, `lf.yxdb.sink()`
- **Spatial support** — SpatialObj decoded to WKB, GeoArrow extension type metadata
- **YXDB format specification** documented in SPECIFICATION.md
- **Cross-language benchmark suite** (Rust, C++, Go, C#, Python)

[Unreleased]: https://github.com/sigilweaver/sigilyx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sigilweaver/sigilyx/releases/tag/v0.1.0
