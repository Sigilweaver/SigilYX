# Changelog

All notable changes to SigilYX will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Streaming reads for E2 (AMP-engine) files. `read_yxdb_batches`,
  `scan_yxdb`, and the underlying `_YxdbBatchReader` now detect the file
  format and dispatch accordingly, so E2 files can be read in
  constant-memory batches with projection and row-limit pushdown. They
  previously rejected E2 files outright, since they always opened the E1
  reader.
- `E2Reader::next_batch(batch_size, columns)`, decoding up to
  `batch_size` records per call and resuming mid-block across calls;
  `E2Reader::into_dataframe_projected`, materialising only the named
  columns; and `E2Reader::count_records`, summing each block's declared
  record count without framing or decoding any records.
- `read_schema` and `record_count` accept E2 files.
- `DataFrame` is re-exported from the crate root, so consumers can name
  the readers' return type without matching the polars version
  themselves.

### Changed

- `E2Reader::into_dataframe` decodes in batches and stacks them instead
  of collecting every record's `FieldValue` form before building any
  column, bounding peak memory to one batch plus the frame under
  construction.
- E1 files carrying a spatial index are read in bounded chunks where
  possible. Such a file is read chunked, retried chunked with grid blocks
  filtered out, and only decompressed whole when neither succeeds - which
  happens when a single record is larger than a chunk and so cannot be
  walked in pieces at all.
- `read_yxdb_columns` pushes the projection into the E2 reader instead
  of reading every column and selecting afterwards.
- Opening an E2 file with the E1 reader reports that the file is E2 and
  points at the format-detecting entry points, rather than reporting a
  missing E1 magic string.

### Fixed

- Streaming an E1 file that carries a spatial index no longer skips record
  data. Grid-block detection reads a block as though a record starts at its
  first byte, and was applied to every block, so it also discarded blocks
  reached part-way through a record and blocks holding one record too large
  to fit. Detection now runs only where the format can place a grid block:
  on a record boundary that also ends a record-block-index group. Across the
  corpus this takes spatial-index files that stream successfully from 107 of
  155 to 155 of 155, each matching its eager read row for row.
- The E1 streaming reader read past the end of the block region and tried
  to decompress the record block index as if it were a block. Block reads
  now stop where the header locates that index, matching the bound the
  whole-file reader already applied.

## [0.3.3] - 2026-08-12

### Added

- A `cargo-fuzz` harness (`fuzz/`) over the YXDB parse path: targets for
  `detect_format`, `YxdbReader::open` (E1), and `E2Reader::open` (E2, with
  unverified field types enabled) exercised against arbitrary byte input.
  Runs in CI on every push/PR touching the parser, plus a longer nightly
  run, backing the third-party-input crash claim in SECURITY.md. Contributed
  by @Nabejo.
- Docs: a Python API reference page (`/python/api-reference`) listing every
  public class and function exported from the `sigilyx` package, with
  signatures, parameters, and return types. Registered in `sidebars.ts` and
  linked from the Python guide index. Contributed by @Nabejo.

### Fixed

- E1/E2 metadata reads (`meta_info_size`/`metadata_size`) and E2 block
  reads (`block_size`) sized a `Vec` from an untrusted `u32` read straight
  off the header/wire, before any bytes were actually read, letting a
  crafted or corrupt file trigger a 2-7 GiB allocation and OOM the
  process. The new fuzz harness above caught this within days. Added
  sanity-limit checks (64 MiB for XML metadata, 512 MiB for a single E2
  block) mirroring the existing `MAX_RECORDS`/`MAX_FIELDS` guards, plus
  regression tests reproducing the fuzzer's `u32::MAX`-style inputs.

## [0.3.2] - 2026-07-07

### Fixed

- Reading large LZF-compressed YXDB files (the common case - all writes use
  LZF) could OOM: the eager read path (`read_yxdb()` / `pl.read_yxdb()`)
  decompressed the entire file into one buffer before building any columns,
  holding both the full decompressed bytes and the full built DataFrame in
  memory at once. It now decompresses and builds columns in bounded-size
  chunks (~128 MiB), so peak memory is bounded by one chunk plus the
  DataFrame under construction instead of the whole file's decompressed
  size. Measured ~45% peak RSS reduction on a 4M-row / ~700 MB-decompressed
  benchmark file, with no throughput regression. The already-existing
  streaming APIs (`scan_yxdb()`, `read_yxdb_batches()`) are unaffected by
  this change and remain the right choice when even the final DataFrame
  wouldn't fit in memory.
- `Blob`/`SpatialObj` columns no longer allocate a separate `Vec<u8>` per
  value (`Vec<Option<Vec<u8>>>`); they're now built directly into Polars'
  `BinaryChunkedBuilder`, cutting per-value overhead for blob/spatial-heavy
  large files.

### Performance

- The streaming/batch read path (`next_record`/`next_batch`, and therefore
  `scan_yxdb()`/`read_yxdb_batches()`) now decompresses each LZF block on a
  background thread, one block ahead of the caller's consumption of the
  current one, instead of decompressing synchronously on every block
  boundary. File I/O and spatial-index-block-skip detection stay on the
  caller's thread - the background thread only ever decompresses byte
  buffers it's handed, so this doesn't change the streaming path's bounded-
  memory characteristics (peak RSS unaffected in benchmarking; one extra
  ~256 KB block resident at a time). Measured ~17-20% wall-clock improvement
  reading a 6M-row benchmark file in streaming batches.

### Changed

- Dependencies bumped across the board (`cargo update`); `quick-xml` bumped
  to 0.41, fixing RUSTSEC-2026-0194/-0195 (quadratic runtime / DoS) in the
  copy SigilYX actually compiles.
- CI: `Security audit` now ignores RUSTSEC-2026-0176/-0177 (pyo3, fixed
  upstream in the unreleased `pyo3-polars`/polars monorepo HEAD but not yet
  cut into a release) and RUSTSEC-2026-0194/-0195 for a second, unreachable
  `quick-xml` instance pulled in by `object_store` (an optional dependency
  gated behind polars' `cloud` feature, which SigilYX never enables) - see
  `ci.yml` for the full reasoning.

## [0.3.1] - 2026-07-04

### Changed

- Release workflow now builds and publishes a source distribution
  (sdist) to PyPI alongside the wheels, enabling source-based
  installs and conda-forge packaging.

## [0.3.0] - 2026-05-31

### Added

- `CITATION.cff`: author identity (Nathan Riley + ORCID) and a
  scaffolded `identifiers:` block ready for the Zenodo concept DOI.

### Changed

- CI: `python-test` job moved off the warpbuild self-hosted runner
  back to the standard GitHub-hosted `ubuntu-latest` pool.
- Dependency bumps applied directly to `main` (closes dependabot
  PRs #2-#5, #7, #9, #11-#14, #16-#17): `actions/checkout` v6,
  `actions/download-artifact` v8, plus cargo / pip dependency
  refresh including `rustls-webpki 0.103.13` (RUSTSEC-2026-0049,
  -0098, -0099, -0104).
- Test suite: tests requiring missing `.yxdb` fixtures are now
  marked `#[ignore]` so CI is not blocked.
- `cargo clippy --fix` cleanups (collapsible if/match, useless
  conversions); `cargo fmt` normalisation.

## [0.2.1] - 2026-05-30

### Fixed

- Re-published as `0.2.1` because the `sigilyx-0.2.0-*` wheel
  filenames were reserved by an earlier deleted PyPI upload and
  could not be re-used. No code changes versus `0.2.0`.

## [0.2.0] - 2026-05-30

### Changed

- **License:** Relicensed from AGPL-3.0-only to Apache-2.0.
- **Positioning:** Removed performance-oriented marketing copy from the README
  and Python README. SigilYX is now described as a plain Rust + Python
  reader/writer for the YXDB format, in line with the rest of the
  Sigilweaver open-source portfolio.

### Added

- `SECURITY.md` with private vulnerability reporting policy.
- `CITATION.cff` for academic and software citation.

### Removed

- `PERFORMANCE.md` and the `benchmarks/` tree (Rust, Go, C#, C++ harnesses
  and supporting data-generation scripts). The benchmark workspace member
  is also removed from the root `Cargo.toml`.

## [0.1.3] - 2026-03-07

### Fixed

- **Writer:** LZF compressor now matches the reference liblzf implementation,
  fixing compressed output that could differ from upstream
- **Writer:** Improved record-boundary handling in LZF compression

### Changed

- **CI:** Moved compute-heavy jobs (Python tests, Linux/Windows wheel builds)
  to WarpBuild runners for faster builds
- **CI:** Refactored workflows to use a custom sccache composite action
- **CI:** Updated Python testing to use `astral-sh/setup-uv` and streamlined
  dependency installation
- **CI:** Added sccache probing, improved error handling, and conditional
  sccache configuration
- **CI:** Updated `aarch64` wheel builds to use native ARM64 runner
- **CI:** Added support for `aarch64` wheel builds with zig linker
- **CI:** Path-based ignores for CI triggers and improved environment setup
- **Docs:** Updated performance benchmarks (100 runs) and revised documentation
  for clarity
- **Tests:** Replaced test fixtures with synthetically generated data
- **Tests:** Added geopandas support in spatial feature tests (skipped if
  not installed)

## [0.1.2] - 2026-02-26

### Fixed

- **Spatial:** Removed dead/duplicated loop in MultiPolygon SHP→WKB conversion
  that could produce corrupt geometry output
- **Reader:** Added field offset validation after XML parsing to reject
  corrupt headers before memory mapping
- **Writer:** `YxdbWriter::finish()` marked `#[must_use]`; `Drop` impl warns
  if writer dropped without calling `finish()` when records were written
- **Record parsing:** `locate_var_data` returns `None` (null) for corrupt
  variable-length records instead of `Some(&[])` (empty)
- **Build:** `STRICT_ALIGN` in LZF decompression is now conditional on CPU
  architecture (disabled on x86_64, enabled on ARM)
- **Python:** `pl.Duration` columns now rejected with a clear error instead
  of being silently accepted
- **Python:** Replaced deprecated `pl.Utf8` references with `pl.String`
- **Python:** TypeError fallback in write path tightened to only match
  pyo3-polars compat errors, not arbitrary TypeErrors
- **PyO3:** `batch_size=0` now raises `ValueError` instead of causing a panic
- **PyO3:** Spatial mode parsing is now case-insensitive (`"WKB"` works)
- **PyO3:** `YxdbStreamWriter` supports Python context manager protocol
- **PyO3:** Deduplicated type override logic between direct and IPC write paths

### Changed

- **Python:** Split monolithic `__init__.py` (1345 lines) into focused
  sub-modules (`_types`, `_readers`, `_writers`, `_geo`, `_polars_plugin`)
  for maintainability
- **SPECIFICATION.md:** Corrected header offset table (file_id, meta_info_size,
  spatial_index_pos, record_block_index_pos, compression_version)

### Added

- `#![deny(unsafe_op_in_unsafe_fn)]` and
  `#![warn(clippy::undocumented_unsafe_blocks)]` crate-level lints
- `// SAFETY:` documentation on all unsafe blocks
- `cargo audit` step in CI pipeline
- `rust-toolchain.toml` for reproducible builds
- `THIRD_PARTY_LICENSES.md` for vendored LZF decompression code
- Tests for `write_yxdb_with_overrides()` API
- Strip debug symbols from release binaries
- MSRV (`rust-version = "1.75"`) declared in Cargo.toml
- `--locked` flag on all CI cargo commands
- Python 3.12 added to CI test matrix

## [0.1.1] - 2026-02-25

### Fixed

- **CI:** Publish workflows now sync package version from git tag automatically,
  removing the need to manually bump version numbers before release
- **CI:** Crate and PyPI publish steps allow dirty builds so the in-flight
  version rewrite does not block the build
- **CI:** macOS x86_64 wheels cross-compiled from ARM runner instead of
  using deprecated `macos-13` runner
- **PyO3:** Removed unused `write_yxdb_from_ipc_spatial` import

## [0.1.0] - 2026-02-25

### Added

- **Core Rust library** for reading and writing Alteryx YXDB files
  - Columnar reader with parallel LZF decompression and SIMD UTF-16 transcoding
  - Row-by-row streaming reader
  - Pipelined writer with background compression
  - Memory-mapped I/O
  - Support for all 17 YXDB field types
- **Python bindings** via PyO3 and pyo3-polars
  - `read_yxdb()` - Polars DataFrame (zero-copy via Arrow C Data Interface)
  - `read_yxdb_arrow()` - PyArrow Table
  - `read_yxdb_pandas()` - Pandas DataFrame
  - `write_yxdb()`, `write_yxdb_arrow()`, `write_yxdb_pandas()`
  - Batched streaming reads with `read_yxdb_batches()`
  - Lazy scan with `scan_yxdb()` (projection and n_rows pushdown)
- **Polars integration** - official namespace plugins
  - `pl.read_yxdb()`, `pl.scan_yxdb()`
  - `df.yxdb.write()`, `lf.yxdb.sink()`
- **Spatial support** - SpatialObj decoded to WKB, GeoArrow extension type metadata
- **YXDB format specification** documented in SPECIFICATION.md
- **Cross-language benchmark suite** (Rust, C++, Go, C#, Python)

[0.1.3]: https://github.com/Sigilweaver/sigilyx/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Sigilweaver/sigilyx/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Sigilweaver/sigilyx/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Sigilweaver/sigilyx/releases/tag/v0.1.0
